fn main() {
    println!("cargo:rerun-if-changed=web/icon.ico");
    // Windows: embed the icon and version info in the .exe (Explorer, taskbar, Start menu)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("web/icon.ico")
            .set("ProductName", "Katana Desktop")
            .set("FileDescription", "Katana Desktop - unofficial Nonograms Katana client")
            .set("CompanyName", "Katana Desktop");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon (no resource compiler?): {e}");
        }
    }
}
