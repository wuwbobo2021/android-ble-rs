use std::env;
use std::path::PathBuf;

use android_build::{Dexer, JavaBuild};

fn main() {
    if !env::var("TARGET").unwrap().contains("android") {
        return;
    }

    let java_srcs = [
        "java/BluetoothGattCallback.java",
        "java/ScanCallback.java",
        "java/BroadcastReceiver.java",
    ];
    let prebuilt_dex = "java/classes.dex";

    let out_dir: PathBuf = env::var_os("OUT_DIR").unwrap().into();
    let out_class_dir = out_dir.join("java");

    if out_class_dir.try_exists().unwrap_or(false) {
        let _ = std::fs::remove_dir_all(&out_class_dir);
    }
    std::fs::create_dir_all(&out_class_dir)
        .unwrap_or_else(|e| panic!("Cannot create output directory {out_class_dir:?} - {e}"));

    let Some(android_jar) = android_build::android_jar(None) else {
        println!(
            "cargo::warning=Android SDK is not found, falling back to the unmanaged prebuilt dex."
        );
        let out_dex_path = out_dir.join("classes.dex");
        std::fs::copy(prebuilt_dex, out_dex_path).expect("Failed to access the prebuilt dex file");
        return;
    };

    // Compile the Java file into .class files
    let o = JavaBuild::new()
        .files(&java_srcs)
        .class_path(&android_jar)
        .classes_out_dir(&out_class_dir)
        .java_source_version(8)
        .java_target_version(8)
        .command()
        .unwrap_or_else(|e| panic!("Could not generate the java compiler command: {e}"))
        .args(["-encoding", "UTF-8"])
        .output()
        .unwrap_or_else(|e| panic!("Could not run the java compiler: {e}"));

    if !o.status.success() {
        panic!(
            "Java compilation failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    let o = Dexer::new()
        .android_jar(&android_jar)
        .class_path(&out_class_dir)
        .collect_classes(&out_class_dir)
        .unwrap()
        .android_min_api(20) // disable multidex for single dex file output
        .out_dir(out_dir)
        .command()
        .unwrap_or_else(|e| panic!("Could not generate the D8 command: {e}"))
        .output()
        .unwrap_or_else(|e| panic!("Error running D8: {e}"));

    if !o.status.success() {
        panic!(
            "Dex conversion failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    for java_src in java_srcs {
        println!("cargo:rerun-if-changed={java_src}");
    }
}
