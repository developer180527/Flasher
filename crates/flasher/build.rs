//! On Windows, puts the icon, version information and application manifest
//! into `flasher.exe`, so Explorer, the taskbar and the installer show
//! Flasher's icon and name. Nothing to do on other systems.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/flasher.ico");
    println!("cargo:rerun-if-changed=windows.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

#[cfg(windows)]
fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icons/flasher.ico")
        .set("ProductName", "Flasher")
        .set("FileDescription", "Flasher")
        .set("CompanyName", "Venu Gopal")
        .set_manifest_file("windows.manifest");
    res.compile()
        .expect("embedding the Windows icon and manifest");
}

/// Building for Windows from another OS: the resource compiler is a
/// Windows tool, so the exe goes without (it still runs). Release builds
/// are made on Windows.
#[cfg(not(windows))]
fn windows_resources() {
    println!("cargo:warning=not embedding the Windows icon: build on Windows for that");
}
