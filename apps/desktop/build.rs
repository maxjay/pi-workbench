//! For Windows, Glitch is the program's icon too: in Explorer, on shortcuts,
//! and pinned to the taskbar. Decided by the target, so a cross-build for
//! Windows gets it and a Windows build of something else does not.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/glitch.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icon/glitch.ico");
    if let Err(err) = resource.compile() {
        // Without the Windows SDK's resource compiler the program still
        // builds, without its icon.
        println!("cargo:warning=No icon embedded: {err}");
    }
}
