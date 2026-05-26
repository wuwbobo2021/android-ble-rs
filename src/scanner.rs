use std::pin::Pin;
use std::{collections::HashMap, sync::Arc};

use futures_core::Stream;
use jni::{
    jni_sig, jni_str,
    objects::{JByteArray, JList, JMapEntry, JString},
    refs::Global,
    Env,
};
use log::{debug, warn};
use uuid::Uuid;

use crate::async_util::StreamUntil;
use crate::event_receiver::{EventReceiver, GlobalEvent};
use crate::util::{android_api_level, jni_with_env, JByteArrayExt, ReferenceExt, UuidExt};
use crate::{
    bindings, callback, AdvertisementData, AdvertisingDevice, Device, ManufacturerData, Result,
};

pub struct Scanner {
    le_scanner: Global<bindings::BluetoothLeScanner<'static>>,
    filter_list: Global<bindings::ArrayList<'static>>,
    callback_proxy: Global<callback::ScanCallbackJavaProxy<'static>>,
    start_receiver: Pin<Box<async_channel::Receiver<Result<()>>>>,
    device_receiver: Pin<Box<async_channel::Receiver<AdvertisingDevice>>>,
    adapter: Global<bindings::BluetoothAdapter<'static>>,
}

impl Scanner {
    fn build(adapter: &bindings::BluetoothAdapter<'static>) -> Result<Self, crate::Error> {
        jni_with_env(|env| {
            let (start_sender, start_receiver) = async_channel::bounded(1);
            let (device_sender, device_receiver) = async_channel::bounded(16);

            let callback_proxy = callback::ScanCallbackJavaProxy::new_proxy(
                env,
                Arc::new(ScanCallbackProxy {
                    device_sender,
                    start_sender,
                }),
            )?;

            let _lock_adapter = env.lock_obj(adapter)?;
            let scanner = adapter.get_bluetooth_le_scanner(env)?;

            Ok(Self {
                le_scanner: env.new_global_ref(scanner)?,
                filter_list: Global::null(),
                callback_proxy: env.new_global_ref(callback_proxy)?,
                start_receiver: Box::pin(start_receiver),
                device_receiver: Box::pin(device_receiver),
                adapter: env.new_global_ref(adapter)?,
            })
        })
    }

    fn add_service_id_filter(&mut self, service_ids: &[Uuid]) -> Result<(), crate::Error> {
        if service_ids.is_empty() {
            return Ok(());
        }
        jni_with_env(|env| {
            if self.filter_list.is_null() {
                let filter_list = bindings::ArrayList::new(env)?;
                self.filter_list = env.new_global_ref(filter_list)?;
            }

            let filter_builder = bindings::ScanFilterBuilder::new(env)?;
            for uuid in service_ids {
                let uuid_string = JString::new(env, uuid.to_string())?;
                let parcel_uuid = bindings::ParcelUuid::from_string(env, &uuid_string)?;
                filter_builder.set_service_uuid(env, &parcel_uuid)?;
                let filter = filter_builder.build(env)?;
                self.filter_list.add(env, &filter)?;
            }
            Ok(())
        })
    }

    fn start_scan_internal(&self) -> Result<(), crate::Error> {
        jni_with_env(|env| {
            let settings_builder = bindings::ScanSettingsBuilder::new(env)?;
            let settings_builder = settings_builder
                .set_scan_mode(env, bindings::ScanSettings::SCAN_MODE_LOW_LATENCY)?;
            let settings = settings_builder.build(env)?;
            self.le_scanner
                .start_scan(env, &self.filter_list, settings, &self.callback_proxy)?;
            Ok(())
        })
    }

    async fn start_scan(
        adapter: &bindings::BluetoothAdapter<'static>,
        service_ids: &[Uuid],
    ) -> Result<Self, crate::Error> {
        let mut scanner = Self::build(adapter)?;
        scanner.add_service_id_filter(service_ids)?;
        scanner.start_scan_internal()?;

        // Wait for scan started or failed.
        match scanner.start_receiver.recv().await {
            Ok(Ok(())) => (),
            Ok(Err(e)) => return Err(e),
            Err(e) => {
                return Err(crate::Error::new(
                    crate::error::ErrorKind::Internal,
                    None,
                    format!("receiving failed while waiting for start: {e:?}"),
                ))
            }
        }
        Ok(scanner)
    }

    pub(crate) async fn scan(
        adapter: &bindings::BluetoothAdapter<'static>,
        service_ids: &[Uuid],
    ) -> Result<impl Stream<Item = AdvertisingDevice> + Send + Unpin + 'static, crate::Error> {
        let scanner = Self::start_scan(adapter, service_ids).await?;
        #[rustfmt::skip]
        let stream = StreamUntil::create(
            scanner,
            EventReceiver::subscribe().await?,
            |event| {
                matches!(
                    event,
                      GlobalEvent::DiscoveryFinished
                    | GlobalEvent::AdapterStateChanged(
                          bindings::BluetoothAdapter::STATE_OFF
                      )
                )
            },
        );
        Ok(stream)
    }
}

impl futures_core::Stream for Scanner {
    type Item = AdvertisingDevice;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.device_receiver.as_mut().as_mut().poll_next(cx)
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        let _ = jni_with_env(|env| {
            let adapter_enabled = self.adapter.is_enabled(env).unwrap_or_else(|_| {
                env.exception_clear();
                false
            });
            if adapter_enabled {
                match self.le_scanner.stop_scan(env, &self.callback_proxy) {
                    Ok(()) => debug!("stopped scan"),
                    Err(e) => {
                        let ex = env.exception_catch().err().unwrap_or(e);
                        warn!("failed to stop scan: {:?}", ex);
                    }
                };
            };
            Ok(())
        });
    }
}

struct ScanCallbackProxy {
    start_sender: async_channel::Sender<Result<(), crate::Error>>,
    device_sender: async_channel::Sender<AdvertisingDevice>,
}

impl super::callback::ScanCallbackProxy for ScanCallbackProxy {
    fn on_scan_failed<'local>(
        &self,
        _env: &mut Env<'local>,
        error_code: i32,
    ) -> Result<(), jni::errors::Error> {
        let e = crate::Error::new(
            crate::error::ErrorKind::Internal,
            None,
            format!("Scan failed to start with error code {error_code}"),
        );
        if let Err(e) = self.start_sender.try_send(Err(e)) {
            warn!("onScanFailed failed to send error: {e:?}");
        }
        Ok(())
    }

    fn on_batch_scan_results<'local>(
        &self,
        env: &mut Env<'local>,
        scan_results: JList<'local>,
    ) -> Result<(), jni::errors::Error> {
        let Some(scan_results) = scan_results.to_option() else {
            warn!("onBatchScanResults: ignoring null scan_results");
            return Ok(());
        };
        if let Some(jiter_results) = scan_results.iter(env)?.to_option() {
            while let Some(scan_result) = jiter_results.next(env)? {
                let scan_result = env.cast_local::<bindings::ScanResult>(scan_result)?;
                self.on_scan_result(env, 0, scan_result)?; // NOTE: this `0` is meaningless!
            }
        }
        Ok(())
    }

    fn on_scan_result<'local>(
        &self,
        env: &mut Env<'local>,
        _callback_type: i32,
        scan_result: bindings::ScanResult<'local>,
    ) -> Result<(), jni::errors::Error> {
        let Some(scan_result) = scan_result.to_option() else {
            warn!("onScanResult: ignoring null scan_result");
            return Ok(());
        };

        let scan_record = scan_result.get_scan_record(env)?;
        let device = scan_result.get_device(env)?;

        let rssi = scan_result.get_rssi(env)?;
        let is_connectable = if android_api_level() >= 26 {
            env.call_method(
                &scan_result,
                jni_str!("isConnectable"),
                jni_sig!(() -> jboolean),
                &[],
            )?
            .z()?
        } else {
            true // XXX: try to check `eventType` via `ScanResult.toString()`
        };
        let local_name = scan_record
            .get_device_name(env)?
            .to_option()
            .map(|n| n.to_string());
        let tx_power_level = scan_record.get_tx_power_level(env)?;

        // Services
        let mut services = Vec::new();
        let jlist_serv_uuids = scan_record.get_service_uuids(env)?;
        if !jlist_serv_uuids.is_null() {
            env.with_local_frame(32, |env| {
                let jiter_serv_uuids = jlist_serv_uuids.iter(env)?;
                while let Some(parcel_uuid) = jiter_serv_uuids.next(env)? {
                    let parcel_uuid = env.as_cast::<bindings::ParcelUuid>(&parcel_uuid)?;
                    if let Ok(uuid) = Uuid::from_andriod_parcel(env, &parcel_uuid) {
                        services.push(uuid);
                    }
                }
                Ok::<(), jni::errors::Error>(())
            })?;
        }

        // Service data
        let mut service_data = HashMap::new();
        let sd = scan_record.get_service_data(env)?;
        let sd = sd.entry_set(env)?;
        let jiter_sd = sd.iterator(env)?;
        env.with_local_frame(32, |env| {
            while let Some(entry) = jiter_sd.next(env)? {
                let entry = env.cast_local::<JMapEntry>(entry)?;
                let (key, val) = (entry.key(env)?, entry.value(env)?);
                let (key, val) = (
                    env.cast_local::<bindings::ParcelUuid>(key)?,
                    env.cast_local::<JByteArray>(val)?,
                );
                if let Ok(uuid) = Uuid::from_andriod_parcel(env, &key) {
                    service_data.insert(uuid, val.to_vec(env)?);
                }
            }
            Ok::<(), jni::errors::Error>(())
        })?;

        // Manufacturer data
        let mut manufacturer_data = None;
        let msd = scan_record.get_manufacturer_specific_data(env)?;
        // XXX: there can be multiple manufacturer data entries, but the API (compatible with bluest)
        // only supports one. So grab just the first.
        if msd.size(env)? != 0 {
            let jarr_msd = msd.value_at(env, 0)?;
            let jarr_msd = env.cast_local::<JByteArray>(jarr_msd)?;
            manufacturer_data = Some(ManufacturerData {
                company_id: msd.key_at(env, 0)? as _,
                data: jarr_msd.to_vec(env)?,
            });
        }

        let d = AdvertisingDevice {
            device: Device::from_java(env, &device, false)?,
            adv_data: AdvertisementData {
                is_connectable,
                local_name,
                manufacturer_data,
                service_data,
                services,
                tx_power_level: Some(tx_power_level as _),
            },
            rssi: Some(rssi as _),
        };

        self.start_sender.try_send(Ok(())).ok();
        self.device_sender.try_send(d).ok();

        Ok(())
    }
}
