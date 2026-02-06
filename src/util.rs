use crate::bindings;
use crate::error::{BluetoothStatusCode, ErrorKind, NativeError};
use crate::{gatt_tree::GattTree, DeviceId};
use bindings::android::os::ParcelUuid;
use java_spaghetti::Local;

use std::mem::ManuallyDrop;
use std::num::NonZeroI32;

pub struct ScopeGuard<F: FnOnce()> {
    dropfn: ManuallyDrop<F>,
}

impl<F: FnOnce()> ScopeGuard<F> {
    #[allow(unused)]
    pub fn defuse(mut self) {
        unsafe { ManuallyDrop::drop(&mut self.dropfn) }
        std::mem::forget(self)
    }
}

impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        // SAFETY: This is OK because `dropfn` is `ManuallyDrop` which will not be dropped by the compiler.
        let dropfn = unsafe { ManuallyDrop::take(&mut self.dropfn) };
        dropfn();
    }
}

pub fn defer<F: FnOnce()>(dropfn: F) -> ScopeGuard<F> {
    ScopeGuard {
        dropfn: ManuallyDrop::new(dropfn),
    }
}

pub trait UuidExt {
    fn from_java(
        value: java_spaghetti::Ref<'_, bindings::java::util::UUID>,
    ) -> Result<uuid::Uuid, crate::Error>;
    fn from_andriod_parcel(uuid: Local<'_, ParcelUuid>) -> Result<uuid::Uuid, crate::Error>;
}

impl UuidExt for uuid::Uuid {
    fn from_java(
        value: java_spaghetti::Ref<'_, bindings::java::util::UUID>,
    ) -> Result<Self, crate::Error> {
        uuid::Uuid::parse_str(value.toString()?.non_null()?.to_string_lossy().trim()).map_err(|e| {
            crate::Error::new(
                ErrorKind::Internal,
                None,
                format!("`Uuid::parse_str` failed: {e:?}"),
            )
        })
    }

    fn from_andriod_parcel(uuid: Local<'_, ParcelUuid>) -> Result<Self, crate::Error> {
        // doing 1 JNI method call, probably faster than 3 method calls:
        // getUuid(), getLeastSignificantBits(), getMostSignificantBits()
        uuid::Uuid::parse_str(uuid.toString()?.non_null()?.to_string_lossy().trim()).map_err(|e| {
            crate::Error::new(
                ErrorKind::Internal,
                None,
                format!("`Uuid::parse_str` failed: {e:?}"),
            )
        })
    }
}

pub struct JavaIterator<'env>(pub Local<'env, bindings::java::util::Iterator>);

impl<'env> Iterator for JavaIterator<'env> {
    type Item = Local<'env, bindings::java::lang::Object>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.hasNext().unwrap() {
            let obj = self.0.next().unwrap().unwrap();
            // upgrade lifetime to the original env.
            let obj = unsafe { Local::from_raw(self.0.env(), obj.into_raw()) };
            Some(obj)
        } else {
            None
        }
    }
}

// TODO: make use of the caller information in these track caller methods.

pub(crate) trait OptionExt<T> {
    fn non_null(self) -> Result<T, crate::Error>;
    fn ok_or_check_conn(self, dev_id: &DeviceId) -> Result<T, crate::Error>;
}

impl<T> OptionExt<T> for Option<T> {
    #[track_caller]
    fn non_null(self) -> Result<T, crate::Error> {
        self.ok_or_else(|| NativeError::JavaNullResult.into())
    }

    #[track_caller]
    fn ok_or_check_conn(self, dev_id: &DeviceId) -> Result<T, crate::Error> {
        self.ok_or_else(|| {
            if GattTree::find_connection(dev_id).is_none() {
                ErrorKind::NotConnected.into()
            } else {
                ErrorKind::ServiceChanged.into()
            }
        })
    }
}

pub(crate) trait BoolExt {
    fn non_false(self) -> Result<(), crate::Error>;
}

impl BoolExt for bool {
    #[track_caller]
    fn non_false(self) -> Result<(), crate::Error> {
        self.then_some(())
            .ok_or_else(|| NativeError::JavaCallReturnedFalse.into())
    }
}

pub(crate) trait IntExt {
    fn check_status_code(self) -> Result<(), crate::Error>;
}

impl IntExt for i32 {
    #[track_caller]
    fn check_status_code(self) -> Result<(), crate::Error> {
        let Some(code) = NonZeroI32::new(self) else {
            return Ok(());
        };
        Err(NativeError::BluetoothStatusCode(BluetoothStatusCode::from(code)).into())
    }
}
