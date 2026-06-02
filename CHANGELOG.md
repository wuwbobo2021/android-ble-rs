# Changes

## 0.2.0
* Bumped `jni` to 0.22.4, removed workarounds for `java-spaghetti` 0.2.0; reduced `unsafe` usages.
* All GATT operations are now posted to the Android main looper.
* Existing GATT connections now prevent the global event receiver from being stopped.
* (Breaking) Removed JVM pointer and bluetooth manager pointer settings in `AdapterConfig`, and `Adapter::with_config` becomes incompatible with the `unstable` API of `bluest` 0.6.x.
* (Breaking) Removed `JavaNullResult` in `NativeError`; the throwable object is no longer available in `NativeError::JavaError`.
* (Breaking) `Adapter::default` now returns `Result` instead of `Option`.

## 0.1.2
* Fixed potential dead lock problem in `async_util::Excluder`.

## 0.1.1
* (Breaking) Fixed `Adapter::default` to be compatible with the `unstable` API of `bluest` 0.6.x.
* Implement `Send` for `AdapterConfig`.
* Improved `async_util::Excluder`.
* Fixed doc.rs build problem: `aarch64-linux-android` target is used now.

## 0.1.0
* Initial release.
