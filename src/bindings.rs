//! Notes for this module:
//! * Extra names and system service names should be declared in fields of `bind_java_type`,
//!   all other constant values should be defined directly in the Rust impl.
//! * All `XXX`s can be resolved when <https://github.com/jni-rs/jni-rs/issues/764> is resolved:
//!   avoid finding and caching their method IDs on new platforms.
//! * `non_null` marks for methods don't mean they can't return null, these marks just makes
//!   sure that null returning values will be converted to JNI error.
//! * API structs for all java class bindings should be initialized in `jni_init`, so that
//!   any error in the binding signature may be found as early as possible. This should be
//!   tested on Android 13.0 and Android 7.0 (Android 17.0 support is to be added).

use jni::{bind_java_type, sys::jint};
use jni_min_helper::android_api_level;

use crate::callback;

pub(crate) fn jni_init(
    env: &mut jni::Env,
    loader: &jni::refs::LoaderContext,
) -> jni::errors::Result<()> {
    ArrayListAPI::get(env, loader)?;
    UUIDAPI::get(env, loader)?;
    InputStreamAPI::get(env, loader)?;
    OutputStreamAPI::get(env, loader)?;
    ContextAPI::get(env, loader)?;
    SparseArrayAPI::get(env, loader)?;
    ParcelUuidAPI::get(env, loader)?;
    LooperAPI::get(env, loader)?;
    BluetoothManagerAPI::get(env, loader)?;
    BluetoothAdapterAPI::get(env, loader)?;
    BluetoothProfileAPI::get(env, loader)?;
    BluetoothDeviceAPI::get(env, loader)?;
    BluetoothSocketAPI::get(env, loader)?;
    BluetoothLeScannerAPI::get(env, loader)?;
    ScanCallbackAPI::get(env, loader)?;
    ScanSettingsAPI::get(env, loader)?;
    ScanSettingsBuilderAPI::get(env, loader)?;
    ScanFilterAPI::get(env, loader)?;
    ScanFilterBuilderAPI::get(env, loader)?;
    ScanResultAPI::get(env, loader)?;
    ScanRecordAPI::get(env, loader)?;
    BluetoothGattAPI::get(env, loader)?;
    BluetoothGattCallbackAPI::get(env, loader)?;
    BluetoothGattServiceAPI::get(env, loader)?;
    BluetoothGattCharacteristicAPI::get(env, loader)?;
    BluetoothGattDescriptorAPI::get(env, loader)?;
    if android_api_level() >= 33 {
        BluetoothGattNewApiAPI::get(env, loader)?;
    } else {
        BluetoothGattOldApiAPI::get(env, loader)?;
        BluetoothGattCharacteristicOldApiAPI::get(env, loader)?;
        BluetoothGattDescriptorOldApiAPI::get(env, loader)?;
    }

    callback::ScanCallbackJavaProxyAPI::get(env, loader)?;
    callback::BluetoothGattCallbackJavaProxyAPI::get(env, loader)?;
    Ok(())
}

bind_java_type! {
    pub ArrayList => "java.util.ArrayList",
    constructors {
        fn new(),
    },
    methods {
        fn add {
            name = "add",
            sig = (arg0: JObject) -> jboolean,
        },
    },
    is_instance_of = {
        JList,
    },
}

bind_java_type! {
    pub UUID => "java.util.UUID",
    methods {
        non_null static fn from_string(arg0: JString) -> UUID,
        non_null fn to_string() -> JString,
    }
}

bind_java_type! {
    pub InputStream => "java.io.InputStream",
    methods {
        fn read {
            name = "read",
            sig = (arg0: jbyte[]) -> jint,
        },
    },
}

bind_java_type! {
    pub OutputStream => "java.io.OutputStream",
    methods {
        fn write {
            name = "write",
            sig = (arg0: jbyte[]),
        },
    },
}

bind_java_type! {
    pub Context => "android.content.Context",
    fields {
        #[allow(non_snake_case)]
        static BLUETOOTH_SERVICE {
            sig = JString,
            get = BLUETOOTH_SERVICE,
        },
    },
    methods {
        non_null fn get_system_service(name: JString) -> JObject,
    }
}

bind_java_type! {
    pub SparseArray => "android.util.SparseArray",
    methods {
        fn key_at(arg0: jint) -> jint,
        fn value_at(arg0: jint) -> JObject,
        fn size() -> jint,
    }
}

bind_java_type! {
    pub ParcelUuid => "android.os.ParcelUuid",
    methods {
        non_null static fn from_string(arg0: JString) -> ParcelUuid,
        non_null fn to_string() -> JString,
    },
}

bind_java_type! {
    pub Looper => "android.os.Looper",
    methods {
        non_null static fn get_main_looper() -> Looper,
        non_null fn get_thread() -> JThread,
    }
}

bind_java_type! {
    pub BluetoothManager => "android.bluetooth.BluetoothManager",
    type_map = {
        BluetoothAdapter => "android.bluetooth.BluetoothAdapter",
        BluetoothDevice => "android.bluetooth.BluetoothDevice",
    },
    methods {
        non_null fn get_adapter() -> BluetoothAdapter,
        non_null fn get_connected_devices(arg0: jint) -> JList,
        fn get_connection_state(arg0: BluetoothDevice, arg1: jint) -> jint,
        non_null fn get_devices_matching_connection_states(arg0: jint, arg1: jint[]) -> JList,
    },
}

bind_java_type! {
    pub BluetoothAdapter => "android.bluetooth.BluetoothAdapter",
    type_map = {
        BluetoothAdapter => "android.bluetooth.BluetoothAdapter",
        BluetoothDevice => "android.bluetooth.BluetoothDevice",
        BluetoothLeScanner => "android.bluetooth.le.BluetoothLeScanner",
    },
    fields {
        #[allow(non_snake_case)]
        static EXTRA_STATE {
            sig = JString,
            get = EXTRA_STATE,
        },
    },
    methods {
        fn get_address() -> JString,
        fn get_remote_device {
            name = "getRemoteDevice",
            sig = (arg0: JString) -> BluetoothDevice,
            non_null = true,
        },
        fn get_scan_mode() -> jint,
        fn get_state() -> jint,
        fn is_enabled() -> jboolean,
        non_null fn get_bluetooth_le_scanner() -> BluetoothLeScanner,
    },
}

impl<'a> BluetoothAdapter<'a> {
    pub const ACTION_DISCOVERY_FINISHED: &'static str =
        "android.bluetooth.adapter.action.DISCOVERY_FINISHED";
    pub const ACTION_STATE_CHANGED: &'static str = "android.bluetooth.adapter.action.STATE_CHANGED";
    pub const STATE_OFF: i32 = 10;
    pub const STATE_ON: i32 = 12;
}

bind_java_type! {
    pub BluetoothProfile => "android.bluetooth.BluetoothProfile",
}
impl<'a> BluetoothProfile<'a> {
    pub const GATT: i32 = 7;
    pub const STATE_CONNECTED: i32 = 2;
    pub const STATE_DISCONNECTED: i32 = 0;
}

bind_java_type! {
    pub BluetoothDevice => "android.bluetooth.BluetoothDevice",
    type_map = {
        BluetoothGatt => "android.bluetooth.BluetoothGatt",
        BluetoothGattCallback => "android.bluetooth.BluetoothGattCallback",
        BluetoothSocket => "android.bluetooth.BluetoothSocket",
        Context => "android.content.Context",
    },
    fields {
        #[allow(non_snake_case)]
        static EXTRA_DEVICE {
            sig = JString,
            get = EXTRA_DEVICE,
        },
        #[allow(non_snake_case)]
        static EXTRA_BOND_STATE {
            sig = JString,
            get = EXTRA_BOND_STATE,
        },
        #[allow(non_snake_case)]
        static EXTRA_PREVIOUS_BOND_STATE {
            sig = JString,
            get = EXTRA_PREVIOUS_BOND_STATE,
        },
    },
    methods {
        // XXX: this is deprecated on API level 37
        fn connect_gatt {
            name = "connectGatt",
            sig = (arg0: Context, arg1: jboolean, arg2: BluetoothGattCallback) -> BluetoothGatt,
            non_null = true, // this should be true on success for any device that supports BLE
        },
        fn create_bond() -> jboolean,
        fn equals(arg0: JObject) -> jboolean,
        non_null fn get_address() -> JString,
        fn get_bond_state() -> jint,
        non_null fn get_name() -> JString,
        // XXX: L2CAP requires API level 29 or above
        // non_null fn create_insecure_l2cap_channel(arg0: jint) -> BluetoothSocket,
        // non_null fn create_l2cap_channel(arg0: jint) -> BluetoothSocket,
    },
}

impl<'a> BluetoothDevice<'a> {
    pub const ACTION_ACL_CONNECTED: &'static str = "android.bluetooth.device.action.ACL_CONNECTED";
    pub const ACTION_ACL_DISCONNECTED: &'static str =
        "android.bluetooth.device.action.ACL_DISCONNECTED";
    pub const ACTION_BOND_STATE_CHANGED: &'static str =
        "android.bluetooth.device.action.BOND_STATE_CHANGED";
    pub const BOND_BONDED: i32 = 12;
    pub const BOND_BONDING: i32 = 11;
    pub const BOND_NONE: i32 = 10;
    pub const TRANSPORT_LE: i32 = 2;
    // XXX: This is added in API level 33
    pub const EXTRA_TRANSPORT: &'static str = "android.bluetooth.device.extra.TRANSPORT";
}

bind_java_type! {
    pub BluetoothSocket => "android.bluetooth.BluetoothSocket",
    type_map = {
        BluetoothDevice => "android.bluetooth.BluetoothDevice",
        InputStream => "java.io.InputStream",
        OutputStream => "java.io.OutputStream",
    },
    methods {
        fn close(),
        fn connect(),
        fn finalize(),
        fn get_connection_type() -> jint,
        non_null fn get_input_stream() -> InputStream,
        fn get_max_receive_packet_size() -> jint,
        fn get_max_transmit_packet_size() -> jint,
        non_null fn get_output_stream() -> OutputStream,
        non_null fn get_remote_device() -> BluetoothDevice,
        fn is_connected() -> jboolean,
    },
}

bind_java_type! {
    pub BluetoothLeScanner => "android.bluetooth.le.BluetoothLeScanner",
    type_map = {
        ScanSettings => "android.bluetooth.le.ScanSettings",
        ScanCallback => "android.bluetooth.le.ScanCallback",
    },
    methods {
        fn start_scan {
            name = "startScan",
            sig = (arg0: JList, arg1: ScanSettings, arg2: ScanCallback),
        },
        fn stop_scan {
            name = "stopScan",
            sig = (arg0: ScanCallback),
        },
    }
}

bind_java_type! {
    pub ScanCallback => "android.bluetooth.le.ScanCallback",
}

bind_java_type! {
    pub ScanSettings => "android.bluetooth.le.ScanSettings",
}
impl<'a> ScanSettings<'a> {
    pub const SCAN_MODE_LOW_LATENCY: i32 = 2;
}

bind_java_type! {
    pub ScanSettingsBuilder => "android.bluetooth.le.ScanSettings$Builder",
    type_map = {
        ScanSettings => "android.bluetooth.le.ScanSettings",
    },
    constructors {
        fn new(),
    },
    methods {
        non_null fn build() -> ScanSettings,
        non_null fn set_scan_mode(arg0: jint) -> ScanSettingsBuilder,
    },
}

bind_java_type! {
    pub ScanFilter => "android.bluetooth.le.ScanFilter",
}

bind_java_type! {
    pub ScanFilterBuilder => "android.bluetooth.le.ScanFilter$Builder",
    type_map = {
        ParcelUuid => "android.os.ParcelUuid",
        ScanFilter => "android.bluetooth.le.ScanFilter",
    },
    constructors {
        fn new(),
    },
    methods {
        non_null fn set_service_uuid {
            name = "setServiceUuid",
            sig = (arg0: ParcelUuid) -> ScanFilterBuilder,
        },
        non_null fn build() -> ScanFilter,
    }
}

bind_java_type! {
    pub ScanResult => "android.bluetooth.le.ScanResult",
    type_map = {
        BluetoothDevice => "android.bluetooth.BluetoothDevice",
        ScanRecord => "android.bluetooth.le.ScanRecord",
    },
    methods {
        non_null fn get_device() -> BluetoothDevice,
        non_null fn get_scan_record() -> ScanRecord,
        fn get_rssi() -> jint,
        // XXX: `is_connectable` requires API level 26 or higher
        // fn is_connectable() -> jboolean,
    }
}

bind_java_type! {
    pub ScanRecord => "android.bluetooth.le.ScanRecord",
    type_map = {
        SparseArray => "android.util.SparseArray",
    },
    methods {
        fn get_device_name() -> JString,
        fn get_tx_power_level() -> jint,
        fn get_service_uuids() -> JList,
        non_null fn get_service_data {
            name = "getServiceData",
            sig = () -> JMap,
        },
        non_null fn get_manufacturer_specific_data {
            name = "getManufacturerSpecificData",
            sig = () -> SparseArray,
        },
    }
}

bind_java_type! {
    pub BluetoothGatt => "android.bluetooth.BluetoothGatt",
    type_map = {
        BluetoothDevice => "android.bluetooth.BluetoothDevice",
        BluetoothGattCharacteristic => "android.bluetooth.BluetoothGattCharacteristic",
        BluetoothGattDescriptor => "android.bluetooth.BluetoothGattDescriptor",
    },
    methods {
        non_null fn get_device() -> BluetoothDevice,
        fn read_remote_rssi() -> jboolean,
        fn request_mtu(arg0: jint) -> jboolean,
        fn discover_services() -> jboolean,
        fn get_services() -> JList,
        fn read_characteristic(arg0: BluetoothGattCharacteristic) -> jboolean,
        fn set_characteristic_notification(arg0: BluetoothGattCharacteristic, arg1: jboolean) -> jboolean,
        fn read_descriptor(arg0: BluetoothGattDescriptor) -> jboolean,
        fn disconnect(),
        fn close(),
    },
}

// XXX: <https://github.com/jni-rs/jni-rs/issues/821> explains why such `unsafe`
// blocks are needed for avoiding runtime checks under this purpose.
impl<'local> BluetoothGatt<'local> {
    pub fn as_new_api<'env: 'local>(
        &self,
        env: &jni::Env<'env>,
    ) -> jni::objects::Cast<'_, '_, BluetoothGattNewApi<'local>> {
        unsafe { env.as_cast_unchecked::<BluetoothGattNewApi>(self) }
    }
    pub fn as_old_api<'env: 'local>(
        &self,
        env: &jni::Env<'env>,
    ) -> jni::objects::Cast<'_, '_, BluetoothGattOldApi<'local>> {
        unsafe { env.as_cast_unchecked::<BluetoothGattOldApi>(self) }
    }
}

// XXX
bind_java_type! {
    pub BluetoothGattNewApi => "android.bluetooth.BluetoothGatt",
    type_map = {
        BluetoothGattCharacteristic => "android.bluetooth.BluetoothGattCharacteristic",
        BluetoothGattDescriptor => "android.bluetooth.BluetoothGattDescriptor",
    },
    methods {
        fn write_characteristic {
            name = "writeCharacteristic",
            sig = (arg0: BluetoothGattCharacteristic, arg1: jbyte[], arg2: jint) -> jint,
        },
        fn write_descriptor {
            name = "writeDescriptor",
            sig = (arg0: BluetoothGattDescriptor, arg1: jbyte[]) -> jint,
        },
    },
}

// XXX
bind_java_type! {
    pub BluetoothGattOldApi => "android.bluetooth.BluetoothGatt",
    type_map = {
        BluetoothGattCharacteristic => "android.bluetooth.BluetoothGattCharacteristic",
        BluetoothGattDescriptor => "android.bluetooth.BluetoothGattDescriptor",
    },
    methods {
        fn write_characteristic {
            name = "writeCharacteristic",
            sig = (arg0: BluetoothGattCharacteristic) -> jboolean,
        },
        fn write_descriptor {
            name = "writeDescriptor",
            sig = (arg0: BluetoothGattDescriptor) -> jboolean,
        },
    },
}

bind_java_type! {
    pub BluetoothGattCallback => "android.bluetooth.BluetoothGattCallback",
    type_map = {
        ScanCallback => "android.bluetooth.le.ScanCallback",
        ScanSettings => "android.bluetooth.le.ScanSettings",
    },
}

bind_java_type! {
    pub BluetoothGattService => "android.bluetooth.BluetoothGattService",
    type_map = {
        UUID => "java.util.UUID",
    },
    methods {
        fn get_type() -> jint,
        non_null fn get_uuid() -> UUID,
        non_null fn get_included_services() -> JList,
        non_null fn get_characteristics() -> JList,
    }
}

impl<'a> BluetoothGattService<'a> {
    pub const SERVICE_TYPE_PRIMARY: i32 = 0;
}

bind_java_type! {
    pub BluetoothGattCharacteristic => "android.bluetooth.BluetoothGattCharacteristic",
    type_map = {
        UUID => "java.util.UUID",
        BluetoothGattService => "android.bluetooth.BluetoothGattService",
    },
    methods {
        non_null fn get_uuid() -> UUID,
        non_null fn get_service() -> BluetoothGattService,
        non_null fn get_descriptors() -> JList,
        fn get_properties() -> jint,
        fn set_write_type(arg0: jint),
    },
}

// XXX
impl<'local> BluetoothGattCharacteristic<'local> {
    pub fn as_old_api<'env: 'local>(
        &self,
        env: &jni::Env<'env>,
    ) -> jni::objects::Cast<'_, '_, BluetoothGattCharacteristicOldApi<'local>> {
        unsafe { env.as_cast_unchecked::<BluetoothGattCharacteristicOldApi>(self) }
    }
}

// XXX
bind_java_type! {
    pub BluetoothGattCharacteristicOldApi => "android.bluetooth.BluetoothGattCharacteristic",
    methods {
        fn set_value {
            name = "setValue",
            sig = (arg0: jbyte[]) -> jboolean,
        },
        fn get_value() -> jbyte[],
    }
}

impl<'a> BluetoothGattCharacteristic<'a> {
    pub const WRITE_TYPE_DEFAULT: i32 = 2;
    pub const WRITE_TYPE_NO_RESPONSE: i32 = 1;
}

bind_java_type! {
    pub BluetoothGattDescriptor => "android.bluetooth.BluetoothGattDescriptor",
    type_map = {
        UUID => "java.util.UUID",
        BluetoothGattCharacteristic => "android.bluetooth.BluetoothGattCharacteristic",
    },
    methods {
        non_null fn get_uuid() -> UUID,
        non_null fn get_characteristic() -> BluetoothGattCharacteristic,
    },
    fields {
        #[allow(non_snake_case)]
        static ENABLE_INDICATION_VALUE {
            sig = jbyte[],
            get = ENABLE_INDICATION_VALUE,
        },
        #[allow(non_snake_case)]
        static ENABLE_NOTIFICATION_VALUE {
            sig = jbyte[],
            get = ENABLE_NOTIFICATION_VALUE,
        },
    }
}

// XXX
impl<'local> BluetoothGattDescriptor<'local> {
    pub fn as_old_api<'env: 'local>(
        &self,
        env: &jni::Env<'env>,
    ) -> jni::objects::Cast<'_, '_, BluetoothGattDescriptorOldApi<'local>> {
        unsafe { env.as_cast_unchecked::<BluetoothGattDescriptorOldApi>(self) }
    }
}

// XXX
bind_java_type! {
    pub BluetoothGattDescriptorOldApi => "android.bluetooth.BluetoothGattDescriptor",
    methods {
        fn set_value(arg0: jbyte[]) -> jboolean,
        fn get_value() -> jbyte[]
    }
}

pub struct BluetoothStatusCodes;
impl BluetoothStatusCodes {
    pub const ERROR_BLUETOOTH_NOT_ALLOWED: jint = 0x00000002;
    pub const ERROR_BLUETOOTH_NOT_ENABLED: jint = 0x00000001;
    pub const ERROR_DEVICE_NOT_BONDED: jint = 0x00000003;
    pub const ERROR_GATT_WRITE_NOT_ALLOWED: jint = 0x000000c8;
    pub const ERROR_GATT_WRITE_REQUEST_BUSY: jint = 0x000000c9;
    pub const ERROR_MISSING_BLUETOOTH_CONNECT_PERMISSION: jint = 0x00000006;
    pub const ERROR_PROFILE_SERVICE_NOT_BOUND: jint = 0x00000009;
    pub const ERROR_UNKNOWN: jint = 0x7fffffff;
    pub const FEATURE_NOT_SUPPORTED: jint = 0x0000000b;
    #[allow(unused)]
    pub const SUCCESS: jint = 0;
}
