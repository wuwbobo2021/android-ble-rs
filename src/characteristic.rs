use std::sync::Arc;

use futures_core::Stream;
use java_spaghetti::ByteArray;
use uuid::Uuid;

use super::bindings::android::bluetooth::BluetoothGattCharacteristic;
use super::descriptor::Descriptor;
use super::error::ErrorKind;
use super::gatt_tree::{CachedWeak, CharacteristicInner, GattTree};
use super::jni::{ByteArrayExt, Monitor};
use super::util::{BoolExt, IntExt, OptionExt};
use super::vm_context::{android_api_level, jni_with_env};
use super::{CharacteristicProperties, DeviceId, Result};

/// A Bluetooth GATT characteristic.
#[derive(Debug, Clone)]
pub struct Characteristic {
    dev_id: DeviceId,
    service_id: Uuid,
    char_id: Uuid,
    inner: CachedWeak<CharacteristicInner>,
}

impl PartialEq for Characteristic {
    fn eq(&self, other: &Self) -> bool {
        self.dev_id == other.dev_id
            && self.service_id == other.service_id
            && self.char_id == other.char_id
    }
}

impl Eq for Characteristic {}

impl std::hash::Hash for Characteristic {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.dev_id.hash(state);
        self.service_id.hash(state);
        self.char_id.hash(state);
    }
}

impl Characteristic {
    pub(crate) fn new(dev_id: DeviceId, service_id: Uuid, char_id: Uuid) -> Self {
        Self {
            dev_id,
            service_id,
            char_id,
            inner: CachedWeak::new(),
        }
    }

    /// The [Uuid] identifying the type of this GATT characteristic.
    pub fn uuid(&self) -> Uuid {
        self.char_id
    }

    /// This method is kept for compatibility with `bluest`.
    pub async fn uuid_async(&self) -> Result<Uuid> {
        Ok(self.char_id)
    }

    /// The properties of this this GATT characteristic.
    ///
    /// Characteristic properties indicate which operations (e.g. read, write, notify, etc)
    /// may be performed on this characteristic.
    pub async fn properties(&self) -> Result<CharacteristicProperties> {
        jni_with_env(|env| {
            let val = self.get_inner()?.char.as_ref(env).getProperties()?;
            Ok(CharacteristicProperties::from_bits(val.cast_unsigned()))
        })
    }

    /// The cached value of this characteristic. Returns an error if the value has not yet been read.
    pub async fn value(&self) -> Result<Vec<u8>> {
        self.get_inner()?
            .read
            .last_value()
            .ok_or(crate::Error::new(
                ErrorKind::NotReady,
                None,
                "please call `Characteristic::read` at first",
            ))?
    }

    // NOTE: the sequence of gaining read lock and write lock should be the same
    // in `read` and `write` methods, otherwise deadlock may occur.
    //
    // To make `wait_unlock` exit on device disconnection, `drop((conn, inner))`
    // cannot be removed here.

    /// Read the value of this characteristic from the device.
    pub async fn read(&self) -> Result<Vec<u8>> {
        let conn = GattTree::check_connection(&self.dev_id)?;
        let inner = self.get_inner()?;
        let read_lock = inner.read.lock().await;
        let _write_lock = inner.write.lock().await;
        jni_with_env(|env| {
            let gatt = &conn.gatt.as_ref(env);
            let gatt = Monitor::new(gatt);
            gatt.readCharacteristic(inner.char.as_ref(env))
                .map_err(|e| e.into())
                .and_then(|b| b.non_false())
        })?;
        drop((conn, inner));
        read_lock
            .wait_unlock()
            .await
            .ok_or_check_conn(&self.dev_id)?
    }

    /// Write `value` to this characteristic on the device and request the device to return a response
    /// indicating a successful write.
    pub async fn write(&self, value: &[u8]) -> Result<()> {
        // NOTE: It is tested that `AttError::INVALID_ATTRIBUTE_VALUE_LENGTH` is returned if the data length
        // is too long; a successful write means it is not truncated. Is this really guaranteed?
        self.write_internal(value, true).await
    }

    /// Write `value` to this characteristic on the device without requesting a response.
    pub async fn write_without_response(&self, value: &[u8]) -> Result<()> {
        // NOTE: It is tested that writing *without response* may never cause an error from the Android API
        // even if the write length is horrible.
        //
        // See <https://developer.android.com/reference/android/bluetooth/BluetoothGatt#requestMtu(int)>:
        // When performing a write request operation (write without response), the data sent is truncated
        // to the MTU size.
        if value.len() <= self.max_write_len()? {
            self.write_internal(value, false).await
        } else {
            Err(crate::Error::new(
                ErrorKind::InvalidParameter,
                None,
                "write length probably exceeded the MTU's limitation",
            ))
        }
    }

    async fn write_internal(&self, value: &[u8], with_response: bool) -> Result<()> {
        let conn = GattTree::check_connection(&self.dev_id)?;
        let inner = self.get_inner()?;
        let _read_lock = inner.read.lock().await;
        let write_lock = inner.write.lock().await;
        jni_with_env(|env| {
            let gatt = conn.gatt.as_ref(env);
            let gatt = Monitor::new(&gatt);
            let char = inner.char.as_ref(env);
            let array = ByteArray::from_slice(env, value);
            let write_type = if with_response {
                BluetoothGattCharacteristic::WRITE_TYPE_DEFAULT
            } else {
                BluetoothGattCharacteristic::WRITE_TYPE_NO_RESPONSE
            };
            char.setWriteType(write_type)?;
            if android_api_level() >= 33 {
                gatt.writeCharacteristic_BluetoothGattCharacteristic_byte_array_int(
                    char, array, write_type,
                )?
                .check_status_code()
            } else {
                #[allow(deprecated)]
                char.setValue_byte_array(array)?;
                #[allow(deprecated)]
                gatt.writeCharacteristic_BluetoothGattCharacteristic(char)
                    .map_err(|e| e.into())
                    .and_then(|b| b.non_false())
            }
        })?;
        drop((conn, inner));
        write_lock
            .wait_unlock()
            .await
            .ok_or_check_conn(&self.dev_id)?
    }

    /// Get the maximum amount of data that can be written in a single packet for this characteristic.
    ///
    /// The Android API does not provide a method to query the current MTU value directly;
    /// instead, `BluetoothGatt.requestMtu()` may be called in `Adapter::connect_device`
    /// to have a possible maximum MTU in the callback. This can be configured with
    /// [crate::AdapterConfig::request_mtu_on_connect].
    pub fn max_write_len(&self) -> Result<usize> {
        let conn = GattTree::check_connection(&self.dev_id)?;
        let mtu = conn.mtu_changed_received.last_value().unwrap_or(23);
        Ok(mtu - 5)
    }

    /// This method is kept for compatibility with `bluest`.
    pub async fn max_write_len_async(&self) -> Result<usize> {
        self.max_write_len()
    }

    /// Enables notification of value changes for this GATT characteristic.
    ///
    /// Returns a stream of values for the characteristic sent from the device.
    pub async fn notify(&self) -> Result<impl Stream<Item = Result<Vec<u8>>> + Send + Unpin + '_> {
        let conn = GattTree::check_connection(&self.dev_id)?;
        let inner = self.get_inner()?;
        let inner_2 = inner.clone();
        let (gatt_for_stop, char_for_stop) = (conn.gatt.clone(), inner.char.clone());
        inner
            .notify
            .subscribe(
                move || {
                    jni_with_env(|env| {
                        let gatt = conn.gatt.as_ref(env);
                        let gatt = Monitor::new(&gatt);
                        let result =
                            gatt.setCharacteristicNotification(inner_2.char.as_ref(env), true)?;
                        result.non_false()
                    })
                },
                move || {
                    jni_with_env(|env| {
                        let gatt = gatt_for_stop.as_ref(env);
                        let gatt = Monitor::new(&gatt);
                        let _ =
                            gatt.setCharacteristicNotification(char_for_stop.as_ref(env), false);
                    })
                },
            )
            .await
    }

    /// Is the device currently sending notifications for this characteristic?
    pub async fn is_notifying(&self) -> Result<bool> {
        Ok(self.get_inner()?.notify.is_notifying())
    }

    /// This method is kept for compatibility with `bluest`.
    pub async fn discover_descriptors(&self) -> Result<Vec<Descriptor>> {
        self.descriptors().await
    }

    /// Get previously discovered descriptors.
    pub async fn descriptors(&self) -> Result<Vec<Descriptor>> {
        Ok(self
            .get_inner()?
            .descs
            .keys()
            .map(|id| Descriptor::new(self.dev_id.clone(), self.service_id, self.char_id, *id))
            .collect())
    }

    fn get_inner(&self) -> Result<Arc<CharacteristicInner>, crate::Error> {
        self.inner.get_or_find(|| {
            GattTree::find_characteristic(&self.dev_id, self.service_id, self.char_id)
                .ok_or_check_conn(&self.dev_id)
        })
    }
}
