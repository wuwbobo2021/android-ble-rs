use std::sync::Arc;

use futures_core::Stream;
use futures_lite::StreamExt;
use jni::{objects::JString, refs::Global};
use log::warn;
use uuid::Uuid;

use crate::async_util::StreamUntil;
use crate::bindings;
use crate::device::Device;
use crate::error::ErrorKind;
use crate::event_receiver::{EventReceiver, GlobalEvent};
use crate::gatt_tree::{BluetoothGattCallbackProxy, GattTree};
use crate::scanner::Scanner;
use crate::util::{
    JIteratorExt, android_api_level, android_context, android_has_permission, jni_with_env,
};
use crate::{AdapterEvent, AdvertisingDevice, ConnectionEvent, DeviceId, Error, Result, callback};

/// The system’s Bluetooth adapter interface.
#[derive(Clone)]
pub struct Adapter {
    inner: Arc<AdapterInner>,
}

struct AdapterInner {
    #[allow(unused)]
    manager: Global<bindings::BluetoothManager<'static>>,
    adapter: Global<bindings::BluetoothAdapter<'static>>,
    request_mtu_on_connect: bool,
    allow_multiple_connections: bool,
}

static CONN_MUTEX: async_lock::Mutex<()> = async_lock::Mutex::new(());

/// Configuration for creating an interface to the default Bluetooth adapter of the system.
///
/// [ndk-context](https://docs.rs/ndk-context/0.1.1/ndk_context) is used for obtaining the
/// JNI `JavaVM` pointer, it is not configurable here.
///
/// TODO:
/// - add an option for enforcing all operations of a device to lock the same mutex,
///   improving compatibility for old devices.
/// - have adjustable timeout values for device connection and GATT operations.
pub struct AdapterConfig {
    request_mtu_on_connect: bool,
    allow_multiple_connections: bool,
}

impl AdapterConfig {
    /// If enabled, this library will request the BLE ATT MTU to 517 bytes during [Adapter::connect_device].
    /// See <https://developer.android.com/about/versions/14/behavior-changes-all#mtu-set-to-517>.
    ///
    /// If disabled, [crate::Characteristic::max_write_len] may always return `18`.
    ///
    /// This is enabled by default; disable it if the firmware of the device to be connected is problematic.
    pub fn request_mtu_on_connect(mut self, enabled: bool) -> Self {
        self.request_mtu_on_connect = enabled;
        self
    }

    /// If enabled, connections with devices already connected outside this library instance will
    /// be permitted. Note that another `android.bluetooth.BluetoothGatt` object will not be created
    /// if the device is already connected in the current library instance.
    ///
    /// This is enabled by default; this should be okay on well-implemented Android API implementations,
    /// but disabling it might improve Android compatibility.
    pub fn allow_multiple_connections(mut self, enabled: bool) -> Self {
        self.allow_multiple_connections = enabled;
        self
    }
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            request_mtu_on_connect: true,
            allow_multiple_connections: true,
        }
    }
}

fn check_scan_permission() -> Result<(), crate::Error> {
    let has_perm = if android_api_level() >= 31 {
        if android_has_permission("android.permission.BLUETOOTH_SCAN")? {
            if !android_has_permission("android.permission.ACCESS_FINE_LOCATION")? {
                warn!(
                    "Please ensure `neverForLocation` is included in `android:usesPermissionFlags`."
                )
            }
            true // XXX
        } else {
            false
        }
    } else if android_api_level() >= 29 {
        android_has_permission("android.permission.ACCESS_FINE_LOCATION")?
            && android_has_permission("android.permission.BLUETOOTH_ADMIN")?
    } else {
        (android_has_permission("android.permission.ACCESS_COARSE_LOCATION")?
            || android_has_permission("android.permission.ACCESS_FINE_LOCATION")?)
            && android_has_permission("android.permission.BLUETOOTH_ADMIN")?
    };
    if !has_perm {
        return Err(crate::Error::new(
            ErrorKind::NotAuthorized,
            None,
            "Bluetooth scanning permission is not granted",
        ));
    }
    Ok(())
}

fn check_connection_permission() -> Result<(), crate::Error> {
    if !android_has_permission(if android_api_level() >= 31 {
        "android.permission.BLUETOOTH_CONNECT"
    } else {
        "android.permission.BLUETOOTH"
    })? {
        return Err(crate::Error::new(
            ErrorKind::NotAuthorized,
            None,
            "Bluetooth connection permission is not granted",
        ));
    }
    Ok(())
}

impl Adapter {
    /// Creates an interface to a Bluetooth adapter using the default config.
    pub async fn default() -> Result<Self, crate::Error> {
        Adapter::with_config(AdapterConfig::default()).await
    }

    /// Creates an interface to a Bluetooth adapter.
    pub async fn with_config(config: AdapterConfig) -> Result<Self> {
        jni_with_env(|env| {
            bindings::jni_init(env, &jni::objects::LoaderContext::None)?;
            let context = android_context(env);
            let service_name = bindings::Context::BLUETOOTH_SERVICE(env)?;
            let manager = context
                .get_system_service(env, &service_name)
                .map_err(|e| {
                    Error::new(
                        ErrorKind::AdapterUnavailable,
                        Some(e.into()),
                        "Failed to get the system service BLUETOOTH_SERVICE",
                    )
                })?;
            let manager = env.new_cast_global_ref::<bindings::BluetoothManager>(manager)?;

            let adapter = manager.get_adapter(env)?;
            Ok(Self {
                inner: Arc::new(AdapterInner {
                    adapter: env.new_global_ref(adapter)?,
                    manager,
                    request_mtu_on_connect: config.request_mtu_on_connect,
                    allow_multiple_connections: config.allow_multiple_connections,
                }),
            })
        })
    }

    /// A stream of [AdapterEvent] which allows the application to identify when the adapter is enabled or disabled.
    pub async fn events(
        &self,
    ) -> Result<impl Stream<Item = Result<AdapterEvent>> + Send + Unpin + '_> {
        Ok(EventReceiver::subscribe()
            .await?
            .filter_map(|event| {
                if let GlobalEvent::AdapterStateChanged(val) = event {
                    match val {
                        bindings::BluetoothAdapter::STATE_ON => Some(AdapterEvent::Available),
                        bindings::BluetoothAdapter::STATE_OFF => Some(AdapterEvent::Unavailable),
                        _ => None, // XXX: process "turning on" and "turning off" events
                    }
                } else {
                    None
                }
            })
            .map(Ok))
    }

    /// Asynchronously blocks until the adapter is available.
    pub async fn wait_available(&self) -> Result<()> {
        while !self.is_available().await? {
            let mut events = self.events().await?;
            while let Some(Ok(event)) = events.next().await {
                if event == AdapterEvent::Available {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Check if the adapter is available.
    pub async fn is_available(&self) -> Result<bool> {
        jni_with_env(|env| Ok(self.inner.adapter.is_enabled(env)?))
    }

    /// Attempts to create the device identified by `id`.
    pub async fn open_device(&self, id: &DeviceId) -> Result<Device> {
        if let Some(dev) = self
            .connected_devices()
            .await?
            .into_iter()
            .find(|d| &d.id() == id)
        {
            return Ok(dev);
        }
        jni_with_env(|env| {
            let addr = JString::new(env, &id.0)?;
            let device = self.inner.adapter.get_remote_device(env, &addr)?;
            Ok(Device::from_java(env, &device, false)?)
        })
    }

    /// Finds all connected Bluetooth LE devices.
    ///
    /// NOTE: there might be BLE devices connected outside this library.
    /// If [AdapterConfig::allow_multiple_connections] is set to true, this method will call
    /// `BluetoothManager.getConnectedDevices()` and ensure GATT connections are created
    /// for them in this library instance.
    pub async fn connected_devices(&self) -> Result<Vec<Device>> {
        check_connection_permission()?;
        if self.inner.allow_multiple_connections {
            let mut device_items = Vec::new();
            jni_with_env(|env| {
                let manager = &self.inner.manager;
                let devices =
                    manager.get_connected_devices(env, bindings::BluetoothProfile::GATT)?;
                let iter_devices = devices.iter(env)?;
                while let Some(device) = iter_devices.check_next(env)? {
                    let device = env.cast_local::<bindings::BluetoothDevice>(device)?;
                    let device_item = Device::from_java(env, &device, true)?;
                    device_items.push(device_item);
                }
                Ok(())
            })?;
            for device_item in &device_items {
                if GattTree::find_connection(&device_item.id).is_none() {
                    self.connect_device(device_item).await?;
                }
            }
            Ok(device_items)
        } else {
            GattTree::registered_devices()
        }
    }

    /// Finds all connected devices providing any service in `service_ids`.
    pub async fn connected_devices_with_services(
        &self,
        service_ids: &[Uuid],
    ) -> Result<Vec<Device>> {
        let mut devices_found = Vec::new();
        for device in self.connected_devices().await? {
            device.discover_services().await?;
            let device_services = device.services().await?;
            if service_ids
                .iter()
                .any(|&id| device_services.iter().any(|serv| serv.uuid() == id))
            {
                devices_found.push(device);
            }
        }
        Ok(devices_found)
    }

    /// Starts scanning for Bluetooth advertising packets.
    ///
    /// Returns a stream of [`AdvertisingDevice`] structs which contain the data from the advertising packet and the
    /// [`Device`] which sent it. Scanning is automatically stopped when the stream is dropped. Inclusion of duplicate
    /// packets is a platform-specific implementation detail.
    ///
    /// If `service_ids` is not empty, returns advertisements including at least one GATT service with a UUID in
    /// `services`. Otherwise returns all advertisements.
    pub async fn scan(
        &self,
        service_ids: &[Uuid],
    ) -> Result<impl Stream<Item = AdvertisingDevice> + Send + Unpin + 'static> {
        check_scan_permission()?;
        Scanner::scan(&self.inner.adapter, service_ids).await
    }

    /// Finds Bluetooth devices providing any service in `services`.
    ///
    /// Returns a stream of [`Device`] structs with matching connected devices returned first. If the stream is not
    /// dropped before all matching connected devices are consumed then scanning will begin for devices advertising any
    /// of the `services`. Scanning will continue until the stream is dropped. Inclusion of duplicate devices is a
    /// platform-specific implementation detail.
    pub async fn discover_devices<'a>(
        &'a self,
        services: &'a [Uuid],
    ) -> Result<impl Stream<Item = Result<Device>> + Send + Unpin + 'a> {
        use futures_lite::stream;
        let connected = stream::iter(self.connected_devices_with_services(services).await?).map(Ok);

        // try_unfold is used to ensure we do not start scanning until the connected devices have been consumed
        let advertising = Box::pin(stream::try_unfold(None, |state| async {
            let mut stream = match state {
                Some(stream) => stream,
                None => self.scan(services).await?,
            };
            Ok(stream.next().await.map(|x| (x.device, Some(stream))))
        }));

        Ok(connected.chain(advertising))
    }

    /// Connects to the [`Device`].
    pub async fn connect_device(&self, device: &Device) -> Result<()> {
        check_connection_permission()?;
        let _conn_lock = CONN_MUTEX.lock().await;
        if device.is_connected().await {
            return Ok(());
        }
        if !self.inner.allow_multiple_connections && self.is_actually_connected(&device.id())? {
            return Err(Error::new(
                ErrorKind::ConnectionFailed,
                None,
                "device is connected outside the current `android_ble` library",
            ));
        }
        let event_receiver = EventReceiver::subscribe().await?;
        let callback_hdl = BluetoothGattCallbackProxy::new(device.id());
        jni_with_env(|env| {
            let context = android_context(env);
            let adapter = &self.inner.adapter;
            let _lock_adapter = env.lock_obj(adapter)?;
            let device_obj = &device.device;
            let proxy =
                callback::BluetoothGattCallbackJavaProxy::new_proxy(env, callback_hdl.clone())?;
            let gatt = device_obj.connect_gatt(env, &context, false, proxy)?;
            GattTree::register_connection(
                &device.id(),
                env.new_global_ref(gatt)?,
                &callback_hdl,
                &event_receiver,
            );
            Ok(())
        })?;
        if !self.is_actually_connected(&device.id())? {
            GattTree::wait_connection_available(&device.id()).await?;
        }
        if self.inner.request_mtu_on_connect {
            let conn = GattTree::check_connection(&device.id())?;
            let mtu_lock = conn.mtu_changed_received.lock().await;
            GattTree::jni_with_locked_gatt(None, &device.id, |conn, env| {
                Ok(conn.gatt.request_mtu(env, 517)?)
            })
            .await?;
            let _ = mtu_lock.wait_unlock().await;
        }
        // validates GATT tree API objects again upon reconnection
        if device.once_connected.get().is_some() {
            let _ = device.discover_services().await?;
        }
        let _ = device.once_connected.set(());
        Ok(())
    }

    /// Disconnects from the [`Device`].
    ///
    /// XXX: manage to call this internally when all API wrapper objects for the device are dropped.
    pub async fn disconnect_device(&self, device: &Device) -> Result<()> {
        let _conn_lock = CONN_MUTEX.lock().await;
        GattTree::jni_with_locked_gatt(Some(self), &device.id, |conn, env| {
            Ok(conn.gatt.disconnect(env)?)
        })
        .await?;
        let mut conn_events = self.device_connection_events(device).await?;
        if GattTree::deregister_connection(&device.id()) {
            while let Some(event) = conn_events.next().await {
                if event == ConnectionEvent::Disconnected {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Monitors a device for connection/disconnection events.
    ///
    /// This monitors only devices connected/disconnected in this library instance,
    /// even if [AdapterConfig::allow_multiple_connections] is set to true.
    ///
    /// This does not work with random address devices.
    pub async fn device_connection_events<'a>(
        &'a self,
        device: &'a Device,
    ) -> Result<impl Stream<Item = ConnectionEvent> + Send + Unpin + 'a> {
        Ok(StreamUntil::create(
            GattTree::connection_events()
                .await
                .filter_map(|(dev_id, ev)| {
                    if dev_id == device.id() {
                        Some(ev)
                    } else {
                        None
                    }
                }),
            self.events().await?,
            |e| matches!(e, Ok(AdapterEvent::Unavailable)),
        ))
    }

    pub(crate) fn java_adapter(&self) -> &bindings::BluetoothAdapter<'static> {
        &self.inner.adapter
    }

    // NOTE: this returns true even if the device is connected outside this crate.
    pub(crate) fn is_actually_connected(&self, dev_id: &DeviceId) -> Result<bool> {
        jni_with_env(|env| {
            let manager = &self.inner.manager;
            let devices = manager.get_connected_devices(env, bindings::BluetoothProfile::GATT)?;
            let jiter_devices = devices.iter(env)?;
            while let Some(device) = jiter_devices.check_next(env)? {
                let device = env.as_cast::<bindings::BluetoothDevice>(&device)?;
                let addr = DeviceId::from_java_dev(env, &device)?;
                if dev_id == &addr {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }
}

impl PartialEq for Adapter {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for Adapter {}

impl std::hash::Hash for Adapter {
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {}
}

impl std::fmt::Debug for Adapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Adapter").finish()
    }
}
