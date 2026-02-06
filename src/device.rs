use std::sync::{Arc, OnceLock};

use futures_core::Stream;
use futures_lite::StreamExt;
use java_spaghetti::Global;
use log::info;
use uuid::Uuid;

use super::bindings::android::bluetooth::BluetoothDevice;
use super::error::ErrorKind;
use super::event_receiver::GlobalEvent;
use super::gatt_tree::{CachedWeak, GattConnection, GattTree};
use super::jni::Monitor;
use super::service::Service;
use super::util::{BoolExt, OptionExt};
use super::vm_context::{android_api_level, jni_with_env};
use super::{DeviceId, Result};

/// A Bluetooth LE device.
#[derive(Clone)]
pub struct Device {
    pub(super) id: DeviceId,
    pub(super) device: Global<BluetoothDevice>,
    pub(super) connection: CachedWeak<GattConnection>,
    pub(super) once_connected: Arc<OnceLock<()>>,
}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Device {}

impl std::hash::Hash for Device {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut f = f.debug_struct("Device");
        f.field("name", &self.name().unwrap_or("(Unknown name)".into()));
        f.field("id", &self.id());
        f.finish()
    }
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name().as_deref().unwrap_or("(Unknown name)"))
    }
}

impl Device {
    /// Returns this device’s unique identifier.
    pub fn id(&self) -> DeviceId {
        self.id.clone()
    }

    /// The local name for this device.
    pub fn name(&self) -> Result<String> {
        jni_with_env(|env| {
            self.device
                .as_ref(env)
                .getName()
                .map_err(|e| e.into())
                .and_then(|s| s.non_null())
                .map(|s| s.to_string_lossy())
        })
    }

    /// This method is kept for compatibility with `bluest`.
    pub async fn name_async(&self) -> Result<String> {
        self.name()
    }

    /// The connection status for this device.
    ///
    /// NOTE: currently this just checks if it is registered in this library instance.
    pub async fn is_connected(&self) -> bool {
        self.get_connection().is_ok()
    }

    /// The pairing status for this device.
    pub async fn is_paired(&self) -> Result<bool> {
        jni_with_env(|env| {
            self.device
                .as_ref(env)
                .getBondState()
                .map_err(|e| e.into())
                .map(|i| i == BluetoothDevice::BOND_BONDED)
        })
    }

    /// Attempt to pair this device using the system default pairing UI.
    pub async fn pair(&self) -> Result<()> {
        let conn = self.get_connection()?;
        let mut receiver = self
            .get_connection()?
            .global_event_receiver
            .subscribe()
            .await?;

        let bond_state = jni_with_env(|env| {
            let device = self.device.as_ref(env);
            device.getBondState().map_err(crate::Error::from)
        })?;
        match bond_state {
            BluetoothDevice::BOND_BONDED => return Ok(()),
            BluetoothDevice::BOND_BONDING => (),
            _ => {
                jni_with_env(|env| {
                    let device = self.device.as_ref(env);
                    let gatt = conn.gatt.as_ref(env);
                    let _lock = Monitor::new(&gatt);
                    device.createBond()?.non_false()?;
                    Ok::<_, crate::Error>(())
                })?;
            }
        }
        drop(conn);

        // Inspired by <https://github.com/NordicSemiconductor/Android-BLE-Library>, BleManagerHandler.java
        while let Some(event) = receiver.next().await {
            match event {
                GlobalEvent::BondStateChanged(dev_id, prev_st, st) if dev_id == self.id => match st
                {
                    BluetoothDevice::BOND_BONDED => return Ok(()),
                    BluetoothDevice::BOND_NONE => {
                        if prev_st == BluetoothDevice::BOND_BONDING {
                            return Err(crate::Error::new(
                                ErrorKind::NotAuthorized,
                                None,
                                "pairing process failed",
                            ));
                        } else if prev_st == BluetoothDevice::BOND_BONDED {
                            info!("deregistered connection with {dev_id} in Device::pair");
                            GattTree::deregister_connection(&dev_id);
                            return Err(ErrorKind::NotConnected.into());
                        }
                    }
                    _ => (),
                },
                _ => (),
            }
        }
        Err(ErrorKind::NotConnected.into())
    }

    /// Discover the primary services of this device.
    pub async fn discover_services(&self) -> Result<Vec<Service>> {
        let conn = self.get_connection()?;
        let disc_lock = conn.discover_services.lock().await;
        jni_with_env(|env| {
            let gatt = conn.gatt.as_ref(env);
            let gatt = Monitor::new(&gatt);
            gatt.discoverServices()?.non_false()?;
            Ok::<_, crate::Error>(())
        })?;
        drop(conn);
        disc_lock.wait_unlock().await.ok_or_check_conn(&self.id)??;
        self.collect_discovered_services()
    }

    /// Discover the primary service(s) of this device with the given [Uuid].
    pub async fn discover_services_with_uuid(&self, uuid: Uuid) -> Result<Vec<Service>> {
        Ok(self
            .discover_services()
            .await?
            .into_iter()
            .filter(|serv| serv.uuid() == uuid)
            .collect())
    }

    /// Get previously discovered services.
    ///
    /// If no services have been discovered yet, this method will perform service discovery.
    pub async fn services(&self) -> Result<Vec<Service>> {
        let conn = self.get_connection()?;
        if conn.discover_services.last_value().is_some() {
            self.collect_discovered_services()
        } else {
            self.discover_services().await
        }
    }

    fn collect_discovered_services(&self) -> Result<Vec<Service>> {
        Ok(self
            .get_connection()?
            .services
            .lock()
            .unwrap()
            .keys()
            .map(|&service_id| Service::new(self.id.clone(), service_id))
            .collect())
    }

    /// **(Experimental)** Monitors the device for service changed indications.
    ///
    /// This requires Android API level 31 or higher.
    pub async fn service_changed_indications(
        &self,
    ) -> Result<impl Stream<Item = Result<ServicesChanged>> + Send + Unpin + '_> {
        if android_api_level() < 31 {
            return Err(crate::Error::new(
                ErrorKind::NotSupported,
                None,
                "this requires BluetoothGattCallback.onServiceChanged() introduced in API level 31",
            ));
        }
        let receiver = self
            .get_connection()?
            .services_changes
            .subscribe(|| Ok::<_, crate::Error>(()), || ())
            .await?;
        Ok(receiver.map(|_| {
            Ok(ServicesChanged {
                dev_id: self.id.clone(),
            })
        }))
    }

    /// Get the current signal strength from the device in dBm.
    pub async fn rssi(&self) -> Result<i16> {
        let conn = self.get_connection()?;
        let read_rssi_lock = conn.read_rssi.lock().await;
        jni_with_env(|env| {
            let gatt = conn.gatt.as_ref(env);
            let gatt = Monitor::new(&gatt);
            gatt.readRemoteRssi()?.non_false()?;
            Ok::<_, crate::Error>(())
        })?;
        drop(conn);
        read_rssi_lock
            .wait_unlock()
            .await
            .ok_or_check_conn(&self.id)?
    }

    /// Open an L2CAP connection-oriented channel (CoC) to this device.
    ///
    /// This requires Android API level 29 or higher.
    pub async fn open_l2cap_channel(
        &self,
        psm: u16,
        secure: bool,
    ) -> Result<super::l2cap_channel::L2capChannel> {
        use log::warn;
        if self.get_connection().is_ok() {
            warn!("trying to open L2CAP channel while there is a GATT connection.");
        }
        let (reader, writer) =
            super::l2cap_channel::open_l2cap_channel(self.device.clone(), psm, secure)?;
        Ok(super::l2cap_channel::L2capChannel { reader, writer })
    }

    pub(crate) fn get_connection(&self) -> Result<Arc<GattConnection>, crate::Error> {
        self.connection
            .get_or_find(|| GattTree::check_connection(&self.id))
    }
}

/// A services changed notification.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServicesChanged {
    dev_id: DeviceId, // XXX: this is not enough for a unique hash value
}

impl ServicesChanged {
    /// Check if `service` is currently invalidated.
    pub fn was_invalidated(&self, service: &Service) -> bool {
        GattTree::find_service(&self.dev_id, service.uuid()).is_none()
    }
}
