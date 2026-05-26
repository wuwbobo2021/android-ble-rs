use std::sync::Arc;

use jni::objects::JByteArray;

use crate::error::ErrorKind;
use crate::gatt_tree::{DescriptorInner, GattTree};
use crate::util::{android_api_level, BoolExt, CachedWeak, IntExt, JByteArrayExt, OptionExt};
use crate::{DeviceId, Result, Uuid};

/// A Bluetooth GATT descriptor.
#[derive(Debug, Clone)]
pub struct Descriptor {
    dev_id: DeviceId,
    service_id: Uuid,
    char_id: Uuid,
    desc_id: Uuid,
    inner: CachedWeak<DescriptorInner>,
}

impl PartialEq for Descriptor {
    fn eq(&self, other: &Self) -> bool {
        self.dev_id == other.dev_id
            && self.service_id == other.service_id
            && self.char_id == other.char_id
            && self.desc_id == other.desc_id
    }
}

impl Eq for Descriptor {}

impl Descriptor {
    pub(crate) fn new(dev_id: DeviceId, service_id: Uuid, char_id: Uuid, desc_id: Uuid) -> Self {
        Self {
            dev_id,
            service_id,
            char_id,
            desc_id,
            inner: CachedWeak::new(),
        }
    }

    /// The [Uuid] identifying the type of this GATT descriptor.
    pub fn uuid(&self) -> Uuid {
        self.desc_id
    }

    /// This method is kept for compatibility with `bluest`.
    pub async fn uuid_async(&self) -> Result<Uuid> {
        Ok(self.desc_id)
    }

    /// The cached value of this descriptor. Returns an error if the value has not yet been read.
    pub async fn value(&self) -> Result<Vec<u8>> {
        self.get_inner()?
            .read
            .last_value()
            .ok_or(crate::Error::new(
                ErrorKind::NotReady,
                None,
                "please call `Descriptor::read` at first",
            ))?
    }

    // NOTE: the sequence of gaining read lock and write lock should be the same
    // in `read` and `write` methods, otherwise deadlock may occur.

    /// Read the value of this descriptor from the device.
    pub async fn read(&self) -> Result<Vec<u8>> {
        let inner = self.get_inner()?;
        let read_lock = inner.read.lock().await;
        let _write_lock = inner.write.lock().await;
        GattTree::jni_with_locked_gatt(None, &self.dev_id, move |conn, env| {
            conn.gatt
                .read_descriptor(env, &inner.desc) // `inner` moved here
                .map_err(|e| e.into())
                .and_then(|b| b.non_false())
        })
        .await?;
        read_lock
            .wait_unlock()
            .await
            .ok_or_check_conn(&self.dev_id)?
    }

    /// Write the `value` to this descriptor on the device.
    pub async fn write(&self, value: &[u8]) -> Result<()> {
        let value = value.to_vec();
        let inner = self.get_inner()?;
        let _read_lock = inner.read.lock().await;
        let write_lock = inner.write.lock().await;
        GattTree::jni_with_locked_gatt(None, &self.dev_id, move |conn, env| {
            let desc = &inner.desc; // `inner` moved here
            let array = JByteArray::from_slice(env, &value)?;
            if android_api_level() >= 33 {
                conn.gatt
                    .as_new_api()
                    .write_descriptor(env, desc, array)?
                    .check_status_code()
            } else {
                desc.as_old_api().set_value(env, array)?;
                conn.gatt
                    .as_old_api()
                    .write_descriptor(env, desc)
                    .map_err(|e| e.into())
                    .and_then(|b| b.non_false())
            }
        })
        .await?;
        write_lock
            .wait_unlock()
            .await
            .ok_or_check_conn(&self.dev_id)?
    }

    fn get_inner(&self) -> Result<Arc<DescriptorInner>, crate::Error> {
        self.inner.get_or_find(|| {
            GattTree::find_descriptor(&self.dev_id, self.service_id, self.char_id, self.desc_id)
                .ok_or_check_conn(&self.dev_id)
        })
    }
}
