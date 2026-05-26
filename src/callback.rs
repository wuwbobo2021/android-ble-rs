use std::sync::{Arc, OnceLock};

use jni::{
    bind_java_type, jni_str,
    objects::{JByteArray, JClassLoader, JList, LoaderContext},
    refs::Global,
    sys::jlong,
};

use crate::bindings;

#[cfg(target_os = "android")]
const DEX_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/classes.dex"));

#[cfg(not(target_os = "android"))]
const DEX_DATA: &[u8] = &[]; // dummy data

fn get_dex_class_loader() -> Result<&'static JClassLoader<'static>, jni::errors::Error> {
    static CLASS_LOADER: OnceLock<Global<JClassLoader<'static>>> = OnceLock::new();
    if CLASS_LOADER.get().is_none() {
        let loader = jni_min_helper::jni_with_env(|env| {
            use jni_min_helper::DexClassLoader;
            let loader = JClassLoader::get_system_class_loader(env)?;
            let dex_loader = loader.load_dex(env, DEX_DATA)?;
            env.new_global_ref(dex_loader)
        })?;
        let _ = CLASS_LOADER.set(loader);
    }
    Ok(CLASS_LOADER.get().unwrap())
}

bind_java_type! {
    pub(crate) ScanCallbackJavaProxy => "com.github.alexmoon.bluest.proxy.android.bluetooth.le.ScanCallback",
    fields {
        ptr: jlong,
    },
    constructors {
        fn new(ptr: jlong),
    },
    type_map = {
        bindings::ScanResult => "android.bluetooth.le.ScanResult",
        bindings::ScanCallback => "android.bluetooth.le.ScanCallback",
    },
    is_instance_of = {
        bindings::ScanCallback,
    },
    native_methods_error_policy = jni::errors::LogErrorAndDefault,
    native_methods {
        fn native_finalize {
            name = "native_finalize",
            sig = (ptr: jlong),
        },
        fn native_on_batch_scan_results {
            name = "native_onBatchScanResults",
            sig = (ptr: jlong, arg0: JList),
        },
        fn native_on_scan_failed {
            name = "native_onScanFailed",
            sig = (ptr: jlong, arg0: jint),
        },
        fn native_on_scan_result {
            name = "native_onScanResult",
            sig = (ptr: jlong, arg0: jint, arg1: bindings::ScanResult),
        },
    },
    hooks = {
        load_class = |env, _load_context, initialize| {
            let class_loader = get_dex_class_loader()?;
            let loader_context = LoaderContext::Loader(class_loader);
            loader_context.load_class(
                env,
                jni_str!("com.github.alexmoon.bluest.proxy.android.bluetooth.le.ScanCallback"),
                initialize
            )
        },
    },
}

pub trait ScanCallbackProxy: Send + Sync + 'static {
    fn on_scan_result<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: i32,
        arg1: bindings::ScanResult<'local>,
    ) -> Result<(), jni::errors::Error>;
    fn on_batch_scan_results<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: JList<'local>,
    ) -> Result<(), jni::errors::Error>;
    fn on_scan_failed<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: i32,
    ) -> Result<(), jni::errors::Error>;
}

impl<'local> ScanCallbackJavaProxy<'local> {
    pub fn new_proxy(
        env: &mut jni::Env<'local>,
        rust_proxy: std::sync::Arc<dyn ScanCallbackProxy>,
    ) -> Result<Self, jni::errors::Error> {
        let b = Box::new(rust_proxy);
        let ptr = Box::into_raw(b);
        ScanCallbackJavaProxy::new(env, ptr.expose_provenance() as jlong)
    }
}

impl ScanCallbackJavaProxyAPI {
    /// SAFETY: only use this in `ScanCallbackJavaProxyNativeInterface` impl methods.
    #[inline(always)]
    unsafe fn get_arc<'local>(
        _env: &mut jni::Env<'local>,
        ptr: jni::sys::jlong,
    ) -> &'local Arc<dyn ScanCallbackProxy> {
        let ptr: *const std::sync::Arc<dyn ScanCallbackProxy> =
            std::ptr::with_exposed_provenance(ptr as usize);
        unsafe { &*ptr }
    }
    /// SAFETY: only use this on finalize.
    unsafe fn drop_arc(ptr: jni::sys::jlong) {
        let ptr: *mut Arc<dyn ScanCallbackProxy> =
            std::ptr::with_exposed_provenance_mut(ptr as usize);
        let _ = unsafe { Box::from_raw(ptr) };
    }
}

impl ScanCallbackJavaProxyNativeInterface for ScanCallbackJavaProxyAPI {
    type Error = jni::errors::Error;
    fn native_on_scan_result<'local>(
        env: &mut jni::Env<'local>,
        _this: ScanCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: jni::sys::jint,
        arg1: bindings::ScanResult<'local>,
    ) -> Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_scan_result(env, arg0, arg1)
    }
    fn native_on_batch_scan_results<'local>(
        env: &mut jni::Env<'local>,
        _this: ScanCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: jni::objects::JList<'local>,
    ) -> Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_batch_scan_results(env, arg0)
    }

    fn native_on_scan_failed<'local>(
        env: &mut jni::Env<'local>,
        _this: ScanCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: jni::sys::jint,
    ) -> Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_scan_failed(env, arg0)
    }

    fn native_finalize<'local>(
        _env: &mut jni::Env<'local>,
        _this: ScanCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
    ) -> Result<(), Self::Error> {
        unsafe {
            Self::drop_arc(ptr);
        }
        Ok(())
    }
}

bind_java_type! {
    pub(crate) BluetoothGattCallbackJavaProxy => "com.github.alexmoon.bluest.proxy.android.bluetooth.BluetoothGattCallback",
    type_map = {
        bindings::BluetoothGatt => "android.bluetooth.BluetoothGatt",
        bindings::BluetoothGattCharacteristic => "android.bluetooth.BluetoothGattCharacteristic",
        bindings::BluetoothGattDescriptor => "android.bluetooth.BluetoothGattDescriptor",
        bindings::BluetoothGattCallback => "android.bluetooth.BluetoothGattCallback",
    },
    constructors {
        fn new(ptr: jlong),
    },
    fields {
        ptr: jlong,
    },
    is_instance_of = {
        bindings::BluetoothGattCallback,
    },
    native_methods_error_policy = jni::errors::LogErrorAndDefault,
    native_methods {
        fn native_finalize {
            name = "native_finalize",
            sig = (ptr: jlong),
        },
        fn native_on_characteristic_changed_old {
            name = "native_onCharacteristicChanged",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattCharacteristic),
        },
        #[allow(non_snake_case)]
        fn native_on_characteristic_changed {
            name = "native_onCharacteristicChanged",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattCharacteristic, arg2: jbyte[]),
        },
        fn native_on_characteristic_read_old {
            name = "native_onCharacteristicRead",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattCharacteristic, arg2: jint),
        },
        #[allow(non_snake_case)]
        fn native_on_characteristic_read {
            name = "native_onCharacteristicRead",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattCharacteristic, arg2: jbyte[], arg3: jint),
        },
        fn native_on_characteristic_write {
            name = "native_onCharacteristicWrite",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattCharacteristic, arg2: jint),
        },
        fn native_on_connection_state_change {
            name = "native_onConnectionStateChange",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: jint, arg2: jint),
        },
        fn native_on_descriptor_read_old {
            name = "native_onDescriptorRead",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattDescriptor, arg2: jint),
        },
        #[allow(non_snake_case)]
        fn native_on_descriptor_read {
            name = "native_onDescriptorRead",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattDescriptor, arg2: jint, arg3: jbyte[]),
        },
        fn native_on_descriptor_write {
            name = "native_onDescriptorWrite",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: bindings::BluetoothGattDescriptor, arg2: jint),
        },
        fn native_on_mtu_changed {
            name = "native_onMtuChanged",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: jint, arg2: jint),
        },
        fn native_on_phy_read {
            name = "native_onPhyRead",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: jint, arg2: jint, arg3: jint),
        },
        fn native_on_phy_update {
            name = "native_onPhyUpdate",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: jint, arg2: jint, arg3: jint),
        },
        fn native_on_read_remote_rssi {
            name = "native_onReadRemoteRssi",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: jint, arg2: jint),
        },
        fn native_on_reliable_write_completed {
            name = "native_onReliableWriteCompleted",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: jint),
        },
        fn native_on_service_changed {
            name = "native_onServiceChanged",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt),
        },
        fn native_on_services_discovered {
            name = "native_onServicesDiscovered",
            sig = (ptr: jlong, arg0: bindings::BluetoothGatt, arg1: jint),
        },
    },
    hooks = {
        load_class = |env, _load_context, initialize| {
            let class_loader = get_dex_class_loader()?;
            let loader_context = LoaderContext::Loader(class_loader);
            loader_context.load_class(
                env,
                jni_str!("com.github.alexmoon.bluest.proxy.android.bluetooth.BluetoothGattCallback"),
                initialize
            )
        },
    },
}

pub trait BluetoothGattCallbackProxy: Send + Sync + 'static {
    fn on_phy_update<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: i32,
        arg2: i32,
        arg3: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_phy_read<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: i32,
        arg2: i32,
        arg3: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_connection_state_change<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: i32,
        arg2: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_services_discovered<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_characteristic_read_old<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_characteristic_read<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: JByteArray<'local>,
        arg3: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_characteristic_write<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_characteristic_changed_old<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
    ) -> Result<(), jni::errors::Error>;
    fn on_characteristic_changed<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: JByteArray<'local>,
    ) -> Result<(), jni::errors::Error>;
    fn on_descriptor_read_old<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattDescriptor<'local>,
        arg2: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_descriptor_read<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattDescriptor<'local>,
        arg2: i32,
        arg3: JByteArray<'local>,
    ) -> Result<(), jni::errors::Error>;
    fn on_descriptor_write<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattDescriptor<'local>,
        arg2: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_reliable_write_completed<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_read_remote_rssi<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: i32,
        arg2: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_mtu_changed<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: i32,
        arg2: i32,
    ) -> Result<(), jni::errors::Error>;
    fn on_service_changed<'local>(
        &self,
        env: &mut jni::Env<'local>,
        arg0: bindings::BluetoothGatt<'local>,
    ) -> Result<(), jni::errors::Error>;
}

impl<'local> BluetoothGattCallbackJavaProxy<'local> {
    pub fn new_proxy(
        env: &mut jni::Env<'local>,
        rust_proxy: std::sync::Arc<dyn BluetoothGattCallbackProxy>,
    ) -> Result<Self, jni::errors::Error> {
        let b = Box::new(rust_proxy);
        let ptr = Box::into_raw(b);
        BluetoothGattCallbackJavaProxy::new(env, ptr.expose_provenance() as jlong)
    }
}

impl BluetoothGattCallbackJavaProxyAPI {
    /// SAFETY: only use this in `BluetoothGattCallbackJavaProxyNativeInterface` impl methods.
    #[inline(always)]
    unsafe fn get_arc<'local>(
        _env: &mut jni::Env<'local>,
        ptr: jni::sys::jlong,
    ) -> &'local Arc<dyn BluetoothGattCallbackProxy> {
        let ptr: *const std::sync::Arc<dyn BluetoothGattCallbackProxy> =
            std::ptr::with_exposed_provenance(ptr as usize);
        unsafe { &*ptr }
    }
    /// SAFETY: only use this on finalize.
    unsafe fn drop_arc(ptr: jni::sys::jlong) {
        let ptr: *mut Arc<dyn BluetoothGattCallbackProxy> =
            std::ptr::with_exposed_provenance_mut(ptr as usize);
        let _ = unsafe { Box::from_raw(ptr) };
    }
}

impl BluetoothGattCallbackJavaProxyNativeInterface for BluetoothGattCallbackJavaProxyAPI {
    type Error = jni::errors::Error;

    fn native_on_characteristic_changed_old<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_characteristic_changed_old(env, arg0, arg1)
    }

    fn native_on_characteristic_changed<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: jni::objects::JPrimitiveArray<'local, jni::sys::jbyte>,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_characteristic_changed(env, arg0, arg1, arg2)
    }

    fn native_on_characteristic_read_old<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_characteristic_read_old(env, arg0, arg1, arg2)
    }

    fn native_on_characteristic_read<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: jni::objects::JPrimitiveArray<'local, jni::sys::jbyte>,
        arg3: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_characteristic_read(env, arg0, arg1, arg2, arg3)
    }

    fn native_on_characteristic_write<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattCharacteristic<'local>,
        arg2: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_characteristic_write(env, arg0, arg1, arg2)
    }

    fn native_on_connection_state_change<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: jni::sys::jint,
        arg2: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_connection_state_change(env, arg0, arg1, arg2)
    }

    fn native_on_descriptor_read_old<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattDescriptor<'local>,
        arg2: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_descriptor_read_old(env, arg0, arg1, arg2)
    }

    fn native_on_descriptor_read<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattDescriptor<'local>,
        arg2: jni::sys::jint,
        arg3: jni::objects::JPrimitiveArray<'local, jni::sys::jbyte>,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_descriptor_read(env, arg0, arg1, arg2, arg3)
    }

    fn native_on_descriptor_write<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: bindings::BluetoothGattDescriptor<'local>,
        arg2: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_descriptor_write(env, arg0, arg1, arg2)
    }

    fn native_on_mtu_changed<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: jni::sys::jint,
        arg2: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_mtu_changed(env, arg0, arg1, arg2)
    }

    fn native_on_phy_read<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: jni::sys::jint,
        arg2: jni::sys::jint,
        arg3: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_phy_read(env, arg0, arg1, arg2, arg3)
    }

    fn native_on_phy_update<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: jni::sys::jint,
        arg2: jni::sys::jint,
        arg3: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_phy_update(env, arg0, arg1, arg2, arg3)
    }

    fn native_on_read_remote_rssi<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: jni::sys::jint,
        arg2: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_read_remote_rssi(env, arg0, arg1, arg2)
    }

    fn native_on_reliable_write_completed<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_reliable_write_completed(env, arg0, arg1)
    }

    fn native_on_service_changed<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_service_changed(env, arg0)
    }

    fn native_on_services_discovered<'local>(
        env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
        arg0: bindings::BluetoothGatt<'local>,
        arg1: jni::sys::jint,
    ) -> ::std::result::Result<(), Self::Error> {
        let arc = unsafe { Self::get_arc(env, ptr) };
        arc.on_services_discovered(env, arg0, arg1)
    }

    fn native_finalize<'local>(
        _env: &mut jni::Env<'local>,
        _this: BluetoothGattCallbackJavaProxy<'local>,
        ptr: jni::sys::jlong,
    ) -> ::std::result::Result<(), Self::Error> {
        unsafe {
            Self::drop_arc(ptr);
        }
        Ok(())
    }
}
