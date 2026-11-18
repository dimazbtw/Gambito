// Puts the Gambito icon and version info into the .exe (Explorer, taskbar, shortcuts).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Gambito");
        res.set("FileDescription", "Gambito");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no icon embedded: {e}");
        }
    }
}
