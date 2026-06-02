use std::sync::Arc;

use crate::characteristic::Characteristic;
use crate::gatt_tree::{GattTree, ServiceInner};
use crate::util::{CachedWeak, JIteratorExt, OptionExt, UuidExt, jni_with_env};
use crate::{DeviceId, Result, Uuid, bindings};

/// A Bluetooth GATT service.
#[derive(Debug, Clone)]
pub struct Service {
    dev_id: DeviceId,
    service_id: Uuid,
    inner: CachedWeak<ServiceInner>,
}

impl PartialEq for Service {
    fn eq(&self, other: &Self) -> bool {
        self.dev_id == other.dev_id && self.service_id == other.service_id
    }
}

impl Eq for Service {}

impl std::hash::Hash for Service {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.dev_id.hash(state);
        self.service_id.hash(state);
    }
}

impl Service {
    pub(crate) fn new(dev_id: DeviceId, service_id: Uuid) -> Self {
        Self {
            dev_id,
            service_id,
            inner: CachedWeak::new(),
        }
    }

    /// The [Uuid] identifying the type of this GATT service.
    pub fn uuid(&self) -> Uuid {
        self.service_id
    }

    /// This method is kept for compatibility with `bluest`.
    pub async fn uuid_async(&self) -> Result<Uuid> {
        Ok(self.service_id)
    }

    /// Whether this is a primary service of the device.
    pub async fn is_primary(&self) -> Result<bool> {
        jni_with_env(|env| {
            Ok(self.get_inner()?.service.get_type(env)?
                == bindings::BluetoothGattService::SERVICE_TYPE_PRIMARY)
        })
    }

    /// Returns all characteristics associated with this service.
    ///
    /// This method is kept for compatibility with `bluest`.
    pub async fn discover_characteristics(&self) -> Result<Vec<Characteristic>> {
        self.characteristics().await
    }

    /// Returns the characteristic(s) with the given [Uuid].
    ///
    /// This method is kept for compatibility with `bluest`.
    pub async fn discover_characteristics_with_uuid(
        &self,
        uuid: Uuid,
    ) -> Result<Vec<Characteristic>> {
        Ok(self
            .characteristics()
            .await?
            .into_iter()
            .filter(|ch| ch.uuid() == uuid)
            .collect())
    }

    /// Get previously discovered characteristics.
    pub async fn characteristics(&self) -> Result<Vec<Characteristic>> {
        Ok(self
            .get_inner()?
            .chars
            .keys()
            .map(|id| Characteristic::new(self.dev_id.clone(), self.service_id, *id))
            .collect())
    }

    /// Returns the included services of this service.
    ///
    /// This method is kept for compatibility with `bluest`.
    pub async fn discover_included_services(&self) -> Result<Vec<Service>> {
        self.included_services().await
    }

    /// Returns the included service(s) with the given [Uuid].
    pub async fn discover_included_services_with_uuid(&self, uuid: Uuid) -> Result<Vec<Service>> {
        Ok(self
            .included_services()
            .await?
            .into_iter()
            .filter(|ch| ch.uuid() == uuid)
            .collect())
    }

    /// Returns the included services of this service.
    pub async fn included_services(&self) -> Result<Vec<Service>> {
        jni_with_env(|env| {
            let inner = self.get_inner()?;
            let service = &inner.service;
            let includes = service.get_included_services(env)?;
            let jiter_includes = includes.iter(env)?;
            let mut vec = Vec::new();
            while let Some(serv) = jiter_includes.check_next(env)? {
                let serv = env.as_cast::<bindings::BluetoothGattService>(&serv)?;
                let uuid = serv.get_uuid(env)?;
                let uuid = Uuid::from_java(env, &uuid)?;
                vec.push(Service::new(self.dev_id.clone(), uuid));
            }
            Ok(vec)
        })
    }

    fn get_inner(&self) -> Result<Arc<ServiceInner>, crate::Error> {
        self.inner.get_or_find(|| {
            GattTree::find_service(&self.dev_id, self.service_id).ok_or_check_conn(&self.dev_id)
        })
    }
}
