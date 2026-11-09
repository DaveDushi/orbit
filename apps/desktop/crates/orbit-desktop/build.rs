//! Windows: embeds the Orbit icon and the version info in the exe, so Explorer, the taskbar and the
//! Start-menu shortcut show the icon. Other targets need nothing here.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/orbit.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../packaging/windows/orbit.ico").set("ProductName", "Orbit").set("FileDescription", "Orbit");
    if let Err(e) = resource.compile() {
        panic!("cannot embed the Windows icon: {e}");
    }
}
