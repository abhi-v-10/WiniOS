fn main() {
    println!("cargo:rerun-if-changed=resources/winios.ico");
    println!("cargo:rerun-if-changed=resources/winios.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let manifest = std::fs::read_to_string("resources/winios.manifest")
        .expect("read WiniOS manifest")
        .replace("@WINIOS_VERSION@", &format!("{version}.0"));
    winresource::WindowsResource::new()
        .set_icon("resources/winios.ico")
        .set_manifest(&manifest)
        .set("ProductName", "WiniOS")
        .set("FileDescription", "WiniOS clipboard companion")
        .set("CompanyName", "Abhishyanth")
        .compile()
        .expect("compile WiniOS Windows resources");
}
