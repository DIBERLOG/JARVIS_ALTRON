fn main() {
    println!("cargo:rerun-if-env-changed=VOSK_LIBRARY_DIR");
    if let Some(directory) = std::env::var_os("VOSK_LIBRARY_DIR") {
        println!("cargo:rustc-link-search=native={}", std::path::PathBuf::from(directory).display());
    } else if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let native = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("../../lib/windows/amd64");
        println!("cargo:rustc-link-search=native={}", native.display());
    }
}
