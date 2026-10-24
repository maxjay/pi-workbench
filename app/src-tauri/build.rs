use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    bundle_agent();
    tauri_build::build();
}

/// The wb-agent the Windows app installs into WSL: the Linux build that
/// `WB_AGENT_BUNDLE` names, or nothing, in which case the app runs whichever
/// wb-agent WSL already has.
fn bundle_agent() {
    println!("cargo:rerun-if-env-changed=WB_AGENT_BUNDLE");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("wb-agent");
    match env::var_os("WB_AGENT_BUNDLE") {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            fs::copy(&path, &out).expect("WB_AGENT_BUNDLE should name the Linux wb-agent");
        }
        None => fs::write(&out, []).expect("OUT_DIR is writable"),
    }
}
