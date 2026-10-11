fn main() {
    println!("cargo:rerun-if-env-changed=VOSK_LIBRARY_DIR");
    if let Some(directory) = std::env::var_os("VOSK_LIBRARY_DIR") {
        println!("cargo:rustc-link-search=native={}", std::path::PathBuf::from(directory).display());
    }
    println!("cargo:rerun-if-changed=../../resources/icons");
    // Tauri build
    tauri_build::build()
}
