use jni::Env;
use jni::objects::{JByteArray, JIterator, JObject, JThread};
use log::error;
use std::num::NonZeroI32;
use std::panic;
use std::sync::OnceLock;

use crate::{
    DeviceId, bindings,
    error::{BluetoothStatusCode, ErrorKind, NativeError},
    gatt_tree::GattTree,
};

pub(crate) use jni_min_helper::android_api_level;

pub(crate) use unsafe_cached_weak::CachedWeak;
mod unsafe_cached_weak {
    use std::fmt::Debug;
    use std::sync::atomic::{AtomicPtr, Ordering};
    use std::sync::{Arc, Weak};

    /// Reusable weak storage.
    pub struct CachedWeak<T> {
        ptr: AtomicPtr<T>,
    }

    impl<T> CachedWeak<T> {
        fn get_raw(&self) -> *mut T {
            self.ptr.load(Ordering::SeqCst)
        }
        fn get_weak(&self) -> Weak<T> {
            // Safety: the raw pointer is got from `Weak::into_raw`.
            let weak = unsafe { Weak::from_raw(self.get_raw()) };
            let weak_cloned = weak.clone();
            let _ = weak.into_raw(); // preserve the ownership of the stored weak
            weak_cloned
        }
        pub fn new() -> Self {
            Self {
                ptr: AtomicPtr::new(Weak::<T>::new().into_raw().cast_mut()),
            }
        }
        pub fn get(&self) -> Option<Arc<T>> {
            self.get_weak().upgrade()
        }
        pub fn get_or_find<E>(
            &self,
            finder: impl FnOnce() -> Result<Arc<T>, E>,
        ) -> Result<Arc<T>, E> {
            if let Some(arc) = self.get() {
                return Ok(arc);
            }
            let arc = finder()?;
            self.ptr
                .store(Arc::downgrade(&arc).into_raw().cast_mut(), Ordering::SeqCst);
            Ok(arc)
        }
    }

    impl<T> Clone for CachedWeak<T> {
        fn clone(&self) -> Self {
            Self {
                ptr: AtomicPtr::new(self.get_weak().into_raw().cast_mut()),
            }
        }
    }

    impl<T> Debug for CachedWeak<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_fmt(format_args!("CachedWeak {{ ptr: {:?} }}", self.get_raw()))
        }
    }
}

/// Alternative for `jni_min_helper::jni_with_env` that deals with `crate::Error`.
/// This is responsible for catching pending Java exception on `NativeError::JavaError`.
///
/// Note that if `f` returns the error with inner `JavaError`, it's usually converted
/// by the simple `impl From<jni::errors::Error> for NativeError`, such convertion
/// is usually done before catching the exception, and that `from` function has no
/// cheaply available `env` for getting the exception info. If needed in the future,
/// the better exception-to-error convertion can be implemented as a subroutine
/// of this function.
#[inline(always)]
pub(crate) fn jni_with_env<R>(
    f: impl FnOnce(&mut Env) -> Result<R, crate::Error>,
) -> Result<R, crate::Error> {
    let vm = jni_min_helper::jni_get_vm();
    vm.attach_current_thread(|env| match f(env) {
        Ok(res) => Ok(res),
        Err(e) => {
            if let Some(native) = e.source.as_ref()
                && matches!(native, NativeError::JavaError(_))
                && let Err(ex) = env.exception_catch()
                && matches!(e.kind(), ErrorKind::Internal | ErrorKind::Other)
            {
                return Err(ex.into());
            }
            Err(e)
        }
    })
}

pub(crate) fn android_context<'local>(
    env: &mut Env<'local>,
) -> jni::refs::Cast<'local, 'local, bindings::Context<'local>> {
    env.as_cast::<bindings::Context>(jni_min_helper::android_context())
        .unwrap()
}

pub(crate) fn android_has_permission(permission: &str) -> Result<bool, jni::errors::Error> {
    jni_min_helper::PermissionRequest::has_permission(permission)
}

// This is a workaround for `jni_min_helper`'s `post_to_main_looper` which doesn't support `FnOnce`.
pub(crate) fn post_to_main_looper(
    runnable: impl FnOnce(&mut jni::Env) -> Result<(), jni::errors::Error> + Send + Sync + 'static,
) -> Result<bool, jni::errors::Error> {
    let runnable = std::sync::Mutex::new(Some(runnable));
    jni_min_helper::DynamicProxy::post_to_main_looper(move |env| {
        if let Some(runnable) = runnable.lock().unwrap().take() {
            runnable(env)
        } else {
            Ok(())
        }
    })
}

pub(crate) fn is_current_thread_main_looper() -> Result<bool, jni::errors::Error> {
    static MAIN_LOOPER_TID: OnceLock<jni::sys::jlong> = OnceLock::new();
    if MAIN_LOOPER_TID.get().is_none() {
        jni_min_helper::jni_with_env(|env| {
            let main_looper = bindings::Looper::get_main_looper(env)?;
            let main_thread = main_looper.get_thread(env)?;
            let _ = MAIN_LOOPER_TID.set(main_thread.get_id(env)?);
            Ok(())
        })?;
    }
    let cur_id = current_java_thread_id()?;
    Ok(cur_id == *MAIN_LOOPER_TID.get().unwrap())
}

pub(crate) fn current_java_thread_id() -> Result<i64, jni::errors::Error> {
    jni_min_helper::jni_with_env(|env| {
        let current_thread = JThread::current_thread(env)?;
        current_thread.get_id(env)
    })
}

/// Workaround for <https://github.com/jni-rs/jni-rs/issues/827>.
/// Use this instead of the original `JIterator::next` before that issue is resolved.
pub trait JIteratorExt {
    fn check_next<'local>(
        &self,
        env: &mut Env<'local>,
    ) -> Result<Option<JObject<'local>>, jni::errors::Error>;
}

impl<'local> JIteratorExt for JIterator<'local> {
    fn check_next<'env_local>(
        &self,
        env: &mut Env<'env_local>,
    ) -> Result<Option<JObject<'env_local>>, jni::errors::Error> {
        if !self.has_next(env)? {
            return Ok(None);
        }
        self.next(env)
    }
}

pub trait JByteArrayExt {
    fn from_slice<'local>(
        env: &mut Env<'local>,
        data: &[u8],
    ) -> Result<JByteArray<'local>, jni::errors::Error>;
    fn to_vec<'local>(&self, env: &mut Env<'local>) -> Result<Vec<u8>, jni::errors::Error>;
}

impl<'local> JByteArrayExt for JByteArray<'local> {
    fn from_slice<'env>(
        env: &mut Env<'env>,
        data: &[u8],
    ) -> Result<JByteArray<'env>, jni::errors::Error> {
        let arr = JByteArray::new(env, data.len())?;
        // Safety: <https://doc.rust-lang.org/reference/expressions/operator-expr.html#r-expr.as.numeric.int-same-size>
        arr.set_region(env, 0, unsafe {
            std::slice::from_raw_parts(data.as_ptr().cast(), data.len())
        })?;
        Ok(arr)
    }
    fn to_vec<'env>(&self, env: &mut Env<'env>) -> Result<Vec<u8>, jni::errors::Error> {
        if self.is_null() {
            return Ok(Vec::new()); // XXX: to be reconsidered
        }
        let mut buf = vec![0; self.len(env)?];
        self.get_region(env, 0, &mut buf)?;
        Ok(Vec::from_iter(buf.iter().map(|&b| b as u8)))
    }
}

impl DeviceId {
    pub fn from_java_dev<'env: 'local, 'local>(
        env: &mut Env<'env>,
        dev: impl AsRef<bindings::BluetoothDevice<'local>>,
    ) -> Result<Self, jni::errors::Error> {
        let addr = dev.as_ref().get_address(env)?.to_string();
        Ok(Self(addr.trim().to_string()))
    }
}

pub trait UuidExt {
    fn from_java<'env: 'local, 'local>(
        env: &mut Env<'env>,
        value: &bindings::UUID<'local>,
    ) -> Result<uuid::Uuid, jni::errors::Error>;
    fn from_andriod_parcel<'env: 'local, 'local>(
        env: &mut Env<'env>,
        uuid: &bindings::ParcelUuid<'local>,
    ) -> Result<uuid::Uuid, jni::errors::Error>;
}

impl UuidExt for uuid::Uuid {
    fn from_java<'env: 'local, 'local>(
        env: &mut Env<'env>,
        value: &bindings::UUID<'local>,
    ) -> Result<uuid::Uuid, jni::errors::Error> {
        let uuid_string = value.to_string(env)?.to_string();
        uuid::Uuid::parse_str(uuid_string.trim()).map_err(|e| {
            jni::errors::Error::ParseFailed(format!("`Uuid::parse_str` failed: {e:?}"))
        })
    }
    fn from_andriod_parcel<'env: 'local, 'local>(
        env: &mut Env<'env>,
        uuid: &bindings::ParcelUuid<'local>,
    ) -> Result<uuid::Uuid, jni::errors::Error> {
        let uuid_string = uuid.to_string(env)?.to_string();
        uuid::Uuid::parse_str(uuid_string.trim()).map_err(|e| {
            jni::errors::Error::ParseFailed(format!("`Uuid::parse_str` failed: {e:?}"))
        })
    }
}

pub(crate) trait ReferenceExt<T> {
    fn non_null(self) -> Result<T, jni::errors::Error>;
    fn to_option(self) -> Option<T>;
}

impl<T: jni::refs::Reference> ReferenceExt<T> for T {
    #[track_caller]
    fn non_null(self) -> Result<T, jni::errors::Error> {
        if self.is_null() {
            let loc = panic::Location::caller();
            error!("non-null check at {}:{} failed", loc.file(), loc.line());
            Err(jni::errors::Error::NullPtr("unexpected null value"))
        } else {
            Ok(self)
        }
    }
    fn to_option(self) -> Option<T> {
        if self.is_null() { None } else { Some(self) }
    }
}

pub(crate) trait OptionExt<T> {
    /// Turns `Some` into `Ok`; with a `None` input, this checks the connection of device
    /// with `dev_id`, returns error of `ErrorKind::NotConnected` if it is disconnected,
    /// or an error of kind `err_if_connected` if it is still connected.
    fn ok_or_check_conn(
        self,
        dev_id: &DeviceId,
        err_if_connected: ErrorKind,
    ) -> Result<T, crate::Error>;
}

impl<T> OptionExt<T> for Option<T> {
    #[track_caller]
    fn ok_or_check_conn(
        self,
        dev_id: &DeviceId,
        err_if_connected: ErrorKind,
    ) -> Result<T, crate::Error> {
        let loc = panic::Location::caller();
        self.ok_or_else(|| {
            if GattTree::find_connection(dev_id).is_none() {
                error!(
                    "disconnection of {dev_id} realized at {}:{}",
                    loc.file(),
                    loc.line()
                );
                ErrorKind::NotConnected.into()
            } else {
                error!(
                    "error {err_if_connected:?} produced at {}:{}",
                    loc.file(),
                    loc.line()
                );
                err_if_connected.into()
            }
        })
    }
}

pub(crate) trait BoolExt {
    /// Call this right after calling a Java method that simply returns false
    /// for some errors.
    fn non_false(self) -> Result<(), crate::Error>;
}

impl BoolExt for bool {
    #[track_caller]
    fn non_false(self) -> Result<(), crate::Error> {
        if self {
            Ok(())
        } else {
            let loc = panic::Location::caller();
            error!(
                "JNI Java call at {}:{} returned false",
                loc.file(),
                loc.line()
            );
            Err(NativeError::JavaCallReturnedFalse.into())
        }
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
