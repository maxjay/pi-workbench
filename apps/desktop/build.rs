//! On Windows, Glitch is the program's icon too: in Explorer, on shortcuts,
//! and pinned to the taskbar.
fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icon/glitch.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon/glitch.ico");
        if let Err(err) = resource.compile() {
            // Without the Windows SDK the program still builds, without it.
            println!("cargo:warning=No icon embedded: {err}");
        }
    }
}
