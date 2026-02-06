# android-ble-rs

Android Bluetooth API wrapper, currently supporting BLE client role operations.

A few portions of the code (especially `L2capChannel`) is orginally written by
[Dirbaio](https://github.com/Dirbaio).

Version 0.1.x of this crate is supposed to be API-compatible with version 0.6.x of
the [bluest](https://docs.rs/crate/bluest/0.6.9) library. In fact, some type definitions
are copied from `bluest`. Anything incompatible with `bluest` in the API may be reported as a bug.

This crate uses [ndk_context](https://crates.io/crates/ndk-context), which is automatically
initialized in [android_activity](https://crates.io/crates/android_activity).

## Test

Make sure the Android SDK, NDK, Rust target `aarch64-linux-android` and
[cargo-apk](https://crates.io/crates/cargo-apk) are installed.
Note that [cargo-apk2](https://crates.io/crates/cargo-apk2) can also be used.

Create `android-ble-test` according to the template provided below, and build it with `cargo apk build -r`.
Note: `-r` means building for the release profile, which produces a much smaller package.

Install the `target/release/apk/android-ble-test.apk` on the Android device, and enable permissions
manually on the device.

Start the `android-ble-test` on the device, then check the log output with `adb logcat android_ble_test:D '*:S'`.

### `cargo-apk` template

`Cargo.toml`:

```toml
[package]
name = "android-ble-test"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
log = "0.4"
android-ble = "0.1"
android_logger = "0.15.1"
ndk-context = "0.1.1"
android-activity = { version = "0.6", features = ["native-activity"] }
# jni-min-helper = { version = "0.3", features = ["futures"] }
futures-lite = "2.6"
async-channel = "2.2.0"

[lib]
crate-type = ["cdylib"]

[package.metadata.android]
package = "com.example.android_ble_test"

build_targets = ["aarch64-linux-android"]

# For `cargo-apk2`:
# put <https://docs.rs/crate/jni-min-helper/0.3.2/source/java/PermActivity.java> in this folder
# java_sources = "java"

# Android 12 or above may require runtime permission request. Use `cargo-apk2` for performing this.
# <https://developer.android.com/develop/connectivity/bluetooth/bt-permissions>
# <https://docs.rs/jni-min-helper/0.3.2/jni_min_helper/struct.PermissionRequest.html>
[package.metadata.android.sdk]
min_sdk_version = 23
target_sdk_version = 33

[[package.metadata.android.uses_feature]]
name = "android.hardware.bluetooth_le"
required = true

[[package.metadata.android.uses_permission]]
name = "android.permission.BLUETOOTH_SCAN"
min_sdk_version = 31

[[package.metadata.android.uses_permission]]
name = "android.permission.BLUETOOTH_CONNECT"
min_sdk_version = 31

[[package.metadata.android.uses_permission]]
name = "android.permission.ACCESS_FINE_LOCATION"
# TODO: uncomment this line when `usesPermissionFlags` becomes supported in `cargo-apk2`.
# max_sdk_version = 30

[[package.metadata.android.uses_permission]]
name = "android.permission.BLUETOOTH"
max_sdk_version = 30

[[package.metadata.android.uses_permission]]
name = "android.permission.BLUETOOTH_ADMIN"
max_sdk_version = 30

# configurations below are for `cargo-apk2`

# [[package.metadata.android.application.activity]]
# name = "android.app.NativeActivity"

# [[package.metadata.android.application.activity.intent_filter]]
# actions = ["android.intent.action.VIEW", "android.intent.action.MAIN"]
# categories = ["android.intent.category.LAUNCHER"]

# [[package.metadata.android.application.activity.meta_data]]
# name = "android.app.lib_name"
# value = "android_ble_test"

# [[package.metadata.android.application.activity]]
# name = "rust.jniminhelper.PermActivity"
```

`src/lib.rs`:

```rust
use android_ble as bluest;

use android_activity::{AndroidApp, MainEvent, PollEvent};
use futures_lite::{FutureExt, StreamExt};
use log::info;

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    // Currently this requires `cargo-apk2` instead of `cargo-apk` to work.
    // But this is required if the user chooses to confirm permission on every startup.
    /*
    let req = jni_min_helper::PermissionRequest::request(
        "BLE Test",
        [
            "android.permission.BLUETOOTH_SCAN",
            "android.permission.BLUETOOTH_CONNECT",
            "android.permission.ACCESS_FINE_LOCATION",
        ],
    )?;
    if let Some(req) = req {
        info!("requesting permissions...");
        let result = req.await;
        for (perm_name, granted) in result.unwrap_or_default() {
            if !granted {
                eprintln!("{perm_name} is denied by the user.");
                return Ok(());
            }
        }
    };
    */

    // View tracing log on the host PC with `adb logcat android_ble_test:D '*:S'`.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("android_ble_test".as_bytes()),
    );

    // calling `block_on` with bluetooth operations in `android_main` thread may block forever.
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        let res = futures_lite::future::block_on(async_main().or(async {
            let _ = rx.recv().await;
            info!("async thread received stop signal.");
            Ok(())
        }));
        if let Err(e) = res {
            info!("async thread's `block_on` received error: {e}");
        } else {
            info!("async thread terminates itself.");
        }
    });

    let mut on_destroy = false;
    loop {
        app.poll_events(None, |event| match event {
            PollEvent::Main(MainEvent::Stop) => {
                info!("Main Stop Event.");
                let _ = tx.send(());
            }
            PollEvent::Main(MainEvent::Destroy) => {
                on_destroy = true;
            }
            _ => (),
        });
        if on_destroy {
            return;
        }
    }
}

// Please put your new test case here.
async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = bluest::Adapter::default().await?;
    adapter.wait_available().await?;
    info!("starting scan...");
    let mut scan = adapter.scan(&[]).await?;
    info!("scan started.");
    while let Some(discovered) = scan.next().await {
        info!("found a device...");
        info!("{:?}", discovered);
    }
    Ok(())
}
```
