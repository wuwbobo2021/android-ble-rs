//! Android Bluetooth API wrapper, currently supporting BLE client role operations.
//!
//! Version 0.1.x of this crate is supposed to be API-compatible with version 0.6.x of `bluest` library.
//! Anything incompatible with `bluest` in the API may be reported as a bug.
//!
//! This crate uses `ndk_context::AndroidContext`, which is automatically initialized by `android_activity`.
//! The basic Android test template is provided in the crate page.

pub use adapter::{Adapter, AdapterConfig};
pub use btuuid::BluetoothUuidExt;
pub use characteristic::Characteristic;
pub use descriptor::Descriptor;
pub use device::{Device, ServicesChanged};
pub use error::Error;
pub use l2cap_channel::{L2capChannel, L2capChannelReader, L2capChannelWriter};
pub use service::Service;

/// Convenience alias for a result with [`Error`].
pub type Result<T, E = Error> = core::result::Result<T, E>;

// These are migrated from `bluest` for maintaining API compatibility with that library.
pub use uuid::Uuid;
pub mod btuuid;
pub mod error;
mod types;
pub use types::*;

mod adapter;
mod async_util;
mod characteristic;
mod descriptor;
mod device;
mod event_receiver;
mod gatt_tree;
mod l2cap_channel;
mod service;
mod util;

// **NOTE**: it is important to use `jni_get_vm` or `jni_with_env` instead of `Global::vm`
// so that a few bugs in `java-spaghetti` 0.2.0 may be avoided.
#[allow(mismatched_lifetime_syntaxes)]
mod bindings;
mod callback;
mod jni;
mod vm_context;
