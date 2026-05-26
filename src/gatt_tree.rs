use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use futures_core::Stream;
use jni::{refs::Global, Env};
use log::{error, info};

use crate::async_util::{Excluder, Notifier, ResultWaiter};
use crate::bindings;
use crate::device::Device;
use crate::error::{AttError, Error};
use crate::util::{
    android_api_level, is_current_thread_main_looper, jni_with_env, post_to_main_looper, BoolExt,
    JByteArrayExt, ReferenceExt, UuidExt,
};
use crate::{Adapter, ConnectionEvent, DeviceId, Uuid};

static GATT_CONNECTIONS: LazyLock<Mutex<HashMap<DeviceId, Arc<GattConnection>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static CONNECTION_EVENTS: Notifier<(DeviceId, ConnectionEvent)> = Notifier::new(32);

pub(crate) struct GattConnection {
    pub(super) gatt: Global<bindings::BluetoothGatt<'static>>,
    pub(super) callback_hdl_weak: Weak<BluetoothGattCallbackProxy>,
    pub(super) gatt_connect: Excluder<()>,
    pub(super) services: Mutex<HashMap<Uuid, Arc<ServiceInner>>>,
    pub(super) discover_services: Excluder<Result<(), Error>>,
    pub(super) read_rssi: Excluder<Result<i16, Error>>,
    pub(super) services_changes: Notifier<()>,
    pub(super) mtu_changed_received: Excluder<usize>,
}

pub(crate) struct ServiceInner {
    pub(super) service: Global<bindings::BluetoothGattService<'static>>,
    pub(super) chars: HashMap<Uuid, Arc<CharacteristicInner>>,
}

pub(crate) struct CharacteristicInner {
    pub(super) char: Global<bindings::BluetoothGattCharacteristic<'static>>,
    pub(super) descs: HashMap<Uuid, Arc<DescriptorInner>>,
    pub(super) notify: Notifier<Result<Vec<u8>, Error>>,
    pub(super) read: Excluder<Result<Vec<u8>, Error>>,
    pub(super) write: Excluder<Result<(), Error>>,
}

pub(crate) struct DescriptorInner {
    pub(super) desc: Global<bindings::BluetoothGattDescriptor<'static>>,
    pub(super) read: Excluder<Result<Vec<u8>, Error>>,
    pub(super) write: Excluder<Result<(), Error>>,
}

/// Manages all existing GATT connections handled by this crate.
pub(crate) struct GattTree;

impl GattTree {
    /// Gets all devices registered here.
    pub fn registered_devices() -> Result<Vec<crate::Device>, crate::Error> {
        let connections = GATT_CONNECTIONS.lock().unwrap();
        let mut devices = Vec::with_capacity(connections.len());
        jni_with_env(|env| {
            for conn in connections.values() {
                let java_dev = conn.gatt.get_device(env)?;
                devices.push(Device::from_java(env, &java_dev, true)?);
            }
            Ok(devices)
        })
    }

    /// Called from `Adapter::connect_device`.
    pub fn register_connection(
        dev_id: &DeviceId,
        gatt: Global<bindings::BluetoothGatt<'static>>,
        callback_hdl: &Arc<BluetoothGattCallbackProxy>,
    ) {
        let _ = GATT_CONNECTIONS.lock().unwrap().insert(
            dev_id.clone(),
            Arc::new(GattConnection {
                gatt,
                callback_hdl_weak: Arc::downgrade(callback_hdl),
                // Inspired by `CONNECTION_TIMEOUT_THRESHOLD` in `Android-BLE-Library`.
                gatt_connect: Excluder::new(Duration::from_secs(20)),
                services: Mutex::new(HashMap::new()),
                discover_services: Excluder::new(Duration::from_secs(10)),
                read_rssi: Excluder::default(),
                services_changes: Notifier::new(16),
                mtu_changed_received: Excluder::default(),
            }),
        );
    }

    /// Call it *once* right after calling `register_connection`.
    /// Returns `None` if it's still disconnected.
    pub async fn wait_connection_available(dev_id: &DeviceId) -> Result<(), crate::Error> {
        let conn = Self::check_connection(dev_id)?;
        let connect_lock = conn.gatt_connect.lock().await;
        if conn.gatt_connect.last_value().is_none() {
            drop(conn);
            if connect_lock.wait_unlock().await.is_some() {
                Ok(())
            } else {
                let _ = Self::check_connection(dev_id)?;
                Err(crate::Error::from(crate::error::ErrorKind::Timeout))
            }
        } else {
            Ok(())
        }
    }

    /// Call this when the actual disconnection is realized.
    pub fn deregister_connection(dev_id: &DeviceId) -> bool {
        let deregistered = GATT_CONNECTIONS.lock().unwrap().remove(dev_id);
        if let Some(conn) = deregistered {
            let _ = jni_with_env(|env| {
                conn.gatt.close(env)?; // releases resources
                Ok(())
            });
            CONNECTION_EVENTS.notify((dev_id.clone(), ConnectionEvent::Disconnected));
            true
        } else {
            false
        }
    }

    pub async fn connection_events() -> impl Stream<Item = (DeviceId, ConnectionEvent)> {
        CONNECTION_EVENTS
            .subscribe(async { Ok::<_, ()>(()) }, || ())
            .await
            .unwrap()
    }

    /// Call this on adapter disabling event.
    pub fn clear_connections() -> bool {
        let mut conns = GATT_CONNECTIONS.lock().unwrap();
        if !conns.is_empty() {
            conns.clear();
            true
        } else {
            false
        }
    }

    pub fn check_connection(dev_id: &DeviceId) -> Result<Arc<GattConnection>, crate::Error> {
        Self::find_connection(dev_id).ok_or(crate::error::ErrorKind::NotConnected.into())
    }

    pub fn find_connection(dev_id: &DeviceId) -> Option<Arc<GattConnection>> {
        let conn = GATT_CONNECTIONS.lock().unwrap().get(dev_id).cloned()?;
        if conn.callback_hdl_weak.strong_count() > 0 {
            Some(conn)
        } else {
            Self::deregister_connection(dev_id);
            info!("deregistered connection with {dev_id} in find_connection()");
            None
        }
    }

    pub fn find_service(dev_id: &DeviceId, service_id: Uuid) -> Option<Arc<ServiceInner>> {
        Self::find_connection(dev_id)
            .and_then(|conn| conn.services.lock().unwrap().get(&service_id).cloned())
    }

    pub fn find_characteristic(
        dev_id: &DeviceId,
        service_id: Uuid,
        char_id: Uuid,
    ) -> Option<Arc<CharacteristicInner>> {
        Self::find_service(dev_id, service_id)
            .and_then(|service| service.chars.get(&char_id).cloned())
    }

    pub fn find_descriptor(
        dev_id: &DeviceId,
        service_id: Uuid,
        char_id: Uuid,
        desc_id: Uuid,
    ) -> Option<Arc<DescriptorInner>> {
        Self::find_characteristic(dev_id, service_id, char_id)
            .and_then(|char| char.descs.get(&desc_id).cloned())
    }
}

impl GattTree {
    /// Posts the GATT operation to the main looper, then waits for the result.
    ///
    /// Notes:
    /// - If current thread is the main looper thread, `f` will be called immediately.
    /// - The `BluetoothGatt` is locked with JNI monitor during the operation.
    /// - If `lock_adapter` is provided, the Java adapter object will be locked with
    ///   monitor before locking the `BluetoothGatt` with monitor.
    #[inline(always)]
    pub async fn jni_with_locked_gatt<R: Send + Sync + 'static>(
        lock_adapter: Option<&Adapter>,
        dev_id: &DeviceId,
        f: impl for<'local> FnOnce(&GattConnection, &mut Env<'local>) -> Result<R, crate::Error>
            + Send
            + Sync
            + 'static,
    ) -> Result<R, crate::Error> {
        let adapter = lock_adapter.cloned();
        let dev_id = dev_id.clone();
        let get_result = move || {
            jni_with_env(|env| {
                let conn = GattTree::check_connection(&dev_id)?;
                let mut _lock_adapter = None;
                if let Some(adapter) = adapter {
                    _lock_adapter.replace(env.lock_obj(adapter.java_adapter())?);
                }
                let _lock_gatt = env.lock_obj(&conn.gatt)?;
                f(&conn, env)
            })
        };
        if is_current_thread_main_looper()? {
            return get_result();
        }
        let (tx, rx) = async_channel::bounded(1);
        post_to_main_looper(move |_| {
            let result = get_result();
            // this should work because the channel was empty
            let _ = tx.try_send(result);
            Ok(())
        })
        .map_err(|e| e.into())
        .and_then(|res| res.non_false())?;
        rx.recv().await.unwrap_or(Err(Error::new(
            crate::error::ErrorKind::Internal,
            None,
            "failed to get the execution result from the main looper thread",
        )))
    }
}

// Code below is written for the callback implementation.

impl GattConnection {
    /// Refresh available services according to the result of `BluetoothGatt.getServices()`.
    /// This does not perform real device discovering.
    pub fn refresh_services(&self) -> Result<(), crate::Error> {
        let mut services = self.services.lock().unwrap();
        let mut current_services_ids = Vec::new();
        jni_with_env(|env| {
            let gatt = &self.gatt;
            let jlist_services = gatt.get_services(env)?;
            let jiter = jlist_services.iter(env)?;
            while let Some(obj) = jiter.next(env)? {
                let service_obj = env.cast_local::<bindings::BluetoothGattService>(obj)?;
                let java_uuid = service_obj.get_uuid(env)?;
                let service_id = Uuid::from_java(env, &java_uuid)?;
                current_services_ids.push(service_id);
                if services.get(&service_id).is_none() {
                    services.insert(
                        service_id,
                        Arc::new(construct_service_tree(env, &service_obj)?),
                    );
                }
            }
            services.retain(|id, _| current_services_ids.contains(id));
            Ok(())
        })
    }
}

fn construct_service_tree<'env: 'local, 'local>(
    env: &mut Env<'env>,
    service_obj: &bindings::BluetoothGattService<'local>,
) -> Result<ServiceInner, crate::Error> {
    let mut chars = HashMap::new();
    env.with_local_frame(32, |env| {
        let jlist_chars = service_obj.get_characteristics(env)?;
        let jiter_chars = jlist_chars.iter(env)?;
        while let Some(obj) = jiter_chars.next(env)? {
            let char_obj = env.cast_local::<bindings::BluetoothGattCharacteristic>(obj)?;
            let java_uuid = char_obj.get_uuid(env)?;
            let char_id = Uuid::from_java(env, &java_uuid)?;

            let mut descs = HashMap::new();
            env.with_local_frame(32, |env| {
                let jlist_descs = char_obj.get_descriptors(env)?;
                let jiter_descs = jlist_descs.iter(env)?;
                while let Some(obj) = jiter_descs.next(env)? {
                    let desc_obj = env.cast_local::<bindings::BluetoothGattDescriptor>(obj)?;
                    let java_uuid = desc_obj.get_uuid(env)?;
                    let desc_id = Uuid::from_java(env, &java_uuid)?;
                    descs.insert(
                        desc_id,
                        Arc::new(DescriptorInner {
                            desc: env.new_global_ref(desc_obj)?,
                            read: Excluder::default(),
                            write: Excluder::default(),
                        }),
                    );
                }
                Ok::<_, crate::Error>(())
            })?;
            chars.insert(
                char_id,
                Arc::new(CharacteristicInner {
                    char: env.new_global_ref(char_obj)?,
                    descs,
                    notify: Notifier::new(128),
                    read: Excluder::default(),
                    write: Excluder::default(),
                }),
            );
        }
        Ok(ServiceInner {
            service: env.new_global_ref(service_obj)?,
            chars,
        })
    })
}

fn callback_find_char<'local>(
    env: &mut Env<'local>,
    dev_id: &DeviceId,
    char_obj: &bindings::BluetoothGattCharacteristic<'local>,
) -> Result<Option<Arc<CharacteristicInner>>, jni::errors::Error> {
    if char_obj.is_null() {
        return Err(jni::errors::Error::NullPtr(
            "unexpected null characteristic in GATT callback",
        ));
    }
    let service_id = {
        let service_obj = char_obj.get_service(env)?;
        let java_uuid = service_obj.get_uuid(env)?;
        Uuid::from_java(env, &java_uuid)?
    };
    let char_id = {
        let java_uuid = char_obj.get_uuid(env)?;
        Uuid::from_java(env, &java_uuid)?
    };
    Ok(GattTree::find_characteristic(dev_id, service_id, char_id))
}

fn callback_find_desc<'local>(
    env: &mut Env<'local>,
    dev_id: &DeviceId,
    desc_obj: &bindings::BluetoothGattDescriptor<'local>,
) -> Result<Option<Arc<DescriptorInner>>, jni::errors::Error> {
    if desc_obj.is_null() {
        return Err(jni::errors::Error::NullPtr(
            "unexpected null descriptor in GATT callback",
        ));
    }
    let char_obj = desc_obj.get_characteristic(env)?;
    let Some(char) = callback_find_char(env, dev_id, &char_obj)? else {
        return Ok(None);
    };
    let java_uuid = desc_obj.get_uuid(env)?;
    let desc_id = Uuid::from_java(env, &java_uuid)?;
    Ok(char.descs.get(&desc_id).cloned())
}

fn gatt_error_check(status: i32) -> Result<(), Error> {
    if status == AttError::SUCCESS.as_u8() as i32 {
        Ok(())
    } else if let Ok(status) = u8::try_from(status) {
        Err(AttError::from_u8(status).into())
    } else {
        Err(AttError::UNLIKELY_ERROR.into())
    }
}

pub struct BluetoothGattCallbackProxy {
    dev_id: DeviceId,
    discover_services_on_change: Mutex<Option<ResultWaiter<Result<(), Error>>>>,
}

impl BluetoothGattCallbackProxy {
    pub fn new(dev_id: DeviceId) -> Arc<Self> {
        Arc::new(Self {
            dev_id,
            discover_services_on_change: Mutex::new(None),
        })
    }
}

impl super::callback::BluetoothGattCallbackProxy for BluetoothGattCallbackProxy {
    fn on_phy_update<'local>(
        &self,
        _env: &mut jni::Env<'local>,
        _arg0: bindings::BluetoothGatt<'local>,
        _arg1: i32,
        _arg2: i32,
        _arg3: i32,
    ) -> Result<(), jni::errors::Error> {
        Ok(())
    }
    fn on_phy_read<'local>(
        &self,
        _env: &mut jni::Env<'local>,
        _arg0: bindings::BluetoothGatt<'local>,
        _arg1: i32,
        _arg2: i32,
        _arg3: i32,
    ) -> Result<(), jni::errors::Error> {
        Ok(())
    }

    fn on_connection_state_change<'local>(
        &self,
        _env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        _status: i32,
        new_state: i32,
    ) -> Result<(), jni::errors::Error> {
        #[allow(clippy::collapsible_if)]
        if new_state == bindings::BluetoothProfile::STATE_CONNECTED {
            CONNECTION_EVENTS.notify((self.dev_id.clone(), ConnectionEvent::Connected));
            if let Some(conn) = GattTree::find_connection(&self.dev_id) {
                conn.gatt_connect.unlock(());
            }
        } else if new_state == bindings::BluetoothProfile::STATE_DISCONNECTED {
            if GattTree::deregister_connection(&self.dev_id) {
                info!(
                    "deregistered connection with {} in onConnectionStateChange()",
                    &self.dev_id
                );
            }
        }
        Ok(())
    }

    fn on_services_discovered<'local>(
        &self,
        _env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        status: i32,
    ) -> Result<(), jni::errors::Error> {
        info!("onServicesDiscovered of {}, status {status}", self.dev_id);
        let Some(conn) = GattTree::find_connection(&self.dev_id) else {
            return Ok(());
        };
        if let Err(e) = conn.refresh_services() {
            error!("refresh_services failed during onServicesDiscovered(): {e}");
        }
        let status = gatt_error_check(status);
        if let Err(e) = &status {
            error!("onServicesDiscovered() with error status: {e}");
        }
        conn.discover_services.unlock(status);

        // see onServiceChanged().
        let _ = self.discover_services_on_change.lock().unwrap().take();
        conn.services_changes.notify(());
        Ok(())
    }

    fn on_characteristic_read<'local>(
        &self,
        env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        char: bindings::BluetoothGattCharacteristic<'local>,
        data: jni::objects::JByteArray<'local>,
        status: i32,
    ) -> Result<(), jni::errors::Error> {
        let Some(char_item) = callback_find_char(env, &self.dev_id, &char)? else {
            return Ok(());
        };
        let result = gatt_error_check(status).and_then(|_| Ok(data.non_null()?.to_vec(env)?));
        char_item.read.unlock(result);
        Ok(())
    }

    fn on_characteristic_read_old<'local>(
        &self,
        env: &mut jni::Env<'local>,
        gatt: bindings::BluetoothGatt<'local>,
        char: bindings::BluetoothGattCharacteristic<'local>,
        status: i32,
    ) -> Result<(), jni::errors::Error> {
        if android_api_level() >= 33 {
            return Ok(());
        }

        let Some(char_item) = callback_find_char(env, &self.dev_id, &char)? else {
            return Ok(());
        };
        if let Err(e) = gatt_error_check(status) {
            char_item.read.unlock(Err(e));
            return Ok(());
        }
        let _lock_gatt = env.lock_obj(gatt)?;
        let get_data = || {
            char.non_null()?
                .as_old_api()
                .get_value(env)?
                .non_null()?
                .to_vec(env)
                .map_err(crate::Error::from)
        };
        char_item.read.unlock(get_data());
        Ok(())
    }

    fn on_characteristic_write<'local>(
        &self,
        env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        char: bindings::BluetoothGattCharacteristic<'local>,
        status: i32,
    ) -> Result<(), jni::errors::Error> {
        let Some(char_item) = callback_find_char(env, &self.dev_id, &char)? else {
            return Ok(());
        };
        char_item.write.unlock(gatt_error_check(status));
        Ok(())
    }

    fn on_characteristic_changed<'local>(
        &self,
        env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        char: bindings::BluetoothGattCharacteristic<'local>,
        data: jni::objects::JByteArray<'local>,
    ) -> Result<(), jni::errors::Error> {
        let Some(char_item) = callback_find_char(env, &self.dev_id, &char)? else {
            return Ok(());
        };
        let result = data
            .non_null()
            .and_then(|jarr| jarr.to_vec(env))
            .map_err(crate::Error::from);
        char_item.notify.notify(result);
        Ok(())
    }

    fn on_characteristic_changed_old<'local>(
        &self,
        env: &mut jni::Env<'local>,
        gatt: bindings::BluetoothGatt<'local>,
        char: bindings::BluetoothGattCharacteristic<'local>,
    ) -> Result<(), jni::errors::Error> {
        if android_api_level() >= 33 {
            return Ok(());
        }

        let Some(char_item) = callback_find_char(env, &self.dev_id, &char)? else {
            return Ok(());
        };
        let _lock_gatt = env.lock_obj(gatt)?;
        let get_data = || {
            char.non_null()?
                .as_old_api()
                .get_value(env)?
                .non_null()?
                .to_vec(env)
                .map_err(crate::Error::from)
        };
        char_item.notify.notify(get_data());
        Ok(())
    }

    fn on_descriptor_read<'local>(
        &self,
        env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        desc: bindings::BluetoothGattDescriptor<'local>,
        status: i32,
        data: jni::objects::JByteArray<'local>,
    ) -> Result<(), jni::errors::Error> {
        let Some(desc_item) = callback_find_desc(env, &self.dev_id, &desc)? else {
            return Ok(());
        };
        let result = gatt_error_check(status).and_then(|_| Ok(data.non_null()?.to_vec(env)?));
        desc_item.read.unlock(result);
        Ok(())
    }

    fn on_descriptor_read_old<'local>(
        &self,
        env: &mut jni::Env<'local>,
        gatt: bindings::BluetoothGatt<'local>,
        desc: bindings::BluetoothGattDescriptor<'local>,
        status: i32,
    ) -> Result<(), jni::errors::Error> {
        if android_api_level() >= 33 {
            return Ok(());
        }

        let Some(desc_item) = callback_find_desc(env, &self.dev_id, &desc)? else {
            return Ok(());
        };
        if let Err(e) = gatt_error_check(status) {
            desc_item.read.unlock(Err(e));
            return Ok(());
        }
        let _lock_gatt = env.lock_obj(gatt)?;
        let get_data = || {
            desc.non_null()?
                .as_old_api()
                .get_value(env)?
                .non_null()?
                .to_vec(env)
                .map_err(crate::Error::from)
        };
        desc_item.read.unlock(get_data());
        Ok(())
    }

    fn on_descriptor_write<'local>(
        &self,
        env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        desc: bindings::BluetoothGattDescriptor<'local>,
        status: i32,
    ) -> Result<(), jni::errors::Error> {
        let Some(desc_item) = callback_find_desc(env, &self.dev_id, &desc)? else {
            return Ok(());
        };
        desc_item.write.unlock(gatt_error_check(status));
        Ok(())
    }

    fn on_reliable_write_completed<'local>(
        &self,
        _env: &mut jni::Env<'local>,
        _arg0: bindings::BluetoothGatt<'local>,
        _arg1: i32,
    ) -> Result<(), jni::errors::Error> {
        Ok(())
    }
    fn on_read_remote_rssi<'local>(
        &self,
        _env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        rssi: i32,
        status: i32,
    ) -> Result<(), jni::errors::Error> {
        let Some(conn) = GattTree::find_connection(&self.dev_id) else {
            return Ok(());
        };
        conn.read_rssi
            .unlock(gatt_error_check(status).map(|_| rssi as _));
        Ok(())
    }

    fn on_mtu_changed<'local>(
        &self,
        _env: &mut jni::Env<'local>,
        _gatt: bindings::BluetoothGatt<'local>,
        mtu: i32,
        _status: i32,
    ) -> Result<(), jni::errors::Error> {
        let Some(conn) = GattTree::find_connection(&self.dev_id) else {
            return Ok(());
        };
        // this should be true
        if let Ok(mtu) = usize::try_from(mtu) {
            info!("onMtuChanged of {}, mtu is {mtu}", self.dev_id);
            conn.mtu_changed_received.unlock(mtu);
        }
        Ok(())
    }

    fn on_service_changed<'local>(
        &self,
        env: &mut jni::Env<'local>,
        gatt: bindings::BluetoothGatt<'local>,
    ) -> Result<(), jni::errors::Error> {
        let Some(conn) = GattTree::find_connection(&self.dev_id) else {
            return Ok(());
        };
        info!("onServiceChanged of {}", self.dev_id);
        if let Some(disc_lock) = conn.discover_services.try_lock() {
            let _lock_gatt = env.lock_obj(&gatt)?;
            match gatt.discover_services(env) {
                Ok(true) => (),
                Ok(false) => {
                    error!("failed to call BluetoothGatt.discoverServices() on onServiceChanged");
                    return Ok(());
                }
                Err(e) => {
                    error!(
                        "failed to call BluetoothGatt.discoverServices() on onServiceChanged: {e}"
                    );
                    return Err(e);
                }
            }

            // see onServicesDiscovered().
            self.discover_services_on_change
                .lock()
                .unwrap()
                .replace(disc_lock);
        }
        Ok(())
    }
}
