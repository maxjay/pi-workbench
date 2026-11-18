use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    bundle_wbd();
    tauri_build::build();
}

/// The wbd the Windows app installs into WSL: the Linux build that
/// `WBD_BUNDLE` names. Without one the app carries nothing and runs
/// whichever wbd WSL already has, as in development.
fn bundle_wbd() {
    println!("cargo:rerun-if-env-changed=WBD_BUNDLE");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("wbd");
    match env::var_os("WBD_BUNDLE") {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            fs::copy(&path, &out).expect("WBD_BUNDLE should name the Linux wbd");
        }
        None => fs::write(&out, []).expect("OUT_DIR is writable"),
    }
}
