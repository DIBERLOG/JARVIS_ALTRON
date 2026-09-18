use std::path::{Path, PathBuf};

/// Native libraries that `vosk` (and the MinGW runtime it was built with) needs
/// at run time on Windows.
const RUNTIME_DLLS: &[&str] = &[
    "libvosk.dll",
    "libgcc_s_seh-1.dll",
    "libstdc++-6.dll",
    "libwinpthread-1.dll",
];

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let lib_dir = Path::new(&manifest_dir)
        .join("..")
        .join("..")
        .join("lib")
        .join("windows")
        .join("amd64");

    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows")) {
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!(
            "cargo:rerun-if-changed={}",
            lib_dir.join("libvosk.lib").display()
        );
        stage_runtime_dlls(&lib_dir);
    }
}

/// Copies the native DLLs next to the built binaries.
///
/// Windows resolves a DLL from the directory of the executable first, so
/// placing them in the cargo profile directory and in `deps/` makes both the
/// application and every test binary start without the user editing `PATH`.
fn stage_runtime_dlls(lib_dir: &Path) {
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    // OUT_DIR is <target>/<profile>/build/<package>-<hash>/out.
    let Some(profile_dir) = Path::new(&out_dir).ancestors().nth(3) else {
        return;
    };
    let destinations = [profile_dir.to_path_buf(), profile_dir.join("deps")];

    for name in RUNTIME_DLLS {
        let source = lib_dir.join(name);
        if !source.is_file() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", source.display());
        let Ok(source_len) = source.metadata().map(|meta| meta.len()) else {
            continue;
        };
        for destination_dir in &destinations {
            let destination = destination_dir.join(name);
            // Skip the 26 MB copy when the file is already staged.
            if destination
                .metadata()
                .map(|meta| meta.len() == source_len)
                .unwrap_or(false)
            {
                continue;
            }
            if std::fs::create_dir_all(destination_dir).is_err() {
                continue;
            }
            if let Err(error) = std::fs::copy(&source, &destination) {
                println!(
                    "cargo:warning=could not stage {} for tests: {}",
                    display_name(name),
                    error
                );
            }
        }
    }
}

fn display_name(name: &str) -> String {
    PathBuf::from(name)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string())
}
