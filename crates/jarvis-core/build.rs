fn main() {
    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows")) {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
        let lib_dir = std::path::Path::new(&manifest_dir)
            .join("..")
            .join("..")
            .join("lib")
            .join("windows")
            .join("amd64");

        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!(
            "cargo:rerun-if-changed={}",
            lib_dir.join("libvosk.lib").display()
        );
    }
}
