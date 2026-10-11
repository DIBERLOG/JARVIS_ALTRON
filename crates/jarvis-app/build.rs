fn main() {
    // link to Vosk lib
    // println!("cargo:rustc-link-lib=libvosk.dll");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-env-changed=VOSK_LIBRARY_DIR");
    if let Some(directory) = std::env::var_os("VOSK_LIBRARY_DIR") {
        println!("cargo:rustc-link-search=native={}", std::path::PathBuf::from(directory).display());
    } else if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let lib_path = std::path::Path::new(&manifest_dir).join("../../lib/windows/amd64");
        println!("cargo:rustc-link-search=native={}", lib_path.display());
    }
}
