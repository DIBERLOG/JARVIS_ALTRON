//! Where optional downloaded components live and whether they are ready.
//! The GUI downloads them on first launch; the voice core only reads them.
use std::path::PathBuf;

use crate::APP_DIR;

pub const S2_MODEL_ID: &str = "jarvis-s2";
pub const S2_PORT: u16 = 29871;
/// `JARVIS_S2_PORT` lets tests run a separate server next to a running JARVIS.
pub fn s2_port() -> u16 {
    std::env::var("JARVIS_S2_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(S2_PORT)
}
/// Reference voice and its transcript, shipped in resources/tts.
pub const S2_REFERENCE_TEXT: &str = "Сэр, напоминание установлено на завтра, на девять часов утра. При необходимости я изменю время";

/// `%LOCALAPPDATA%/JARVIS-ALTRON/components`, or `JARVIS_COMPONENTS_DIR` for tests.
pub fn root() -> PathBuf {
    if let Some(dir) = std::env::var_os("JARVIS_COMPONENTS_DIR") { return PathBuf::from(dir); }
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
        .join("JARVIS-ALTRON").join("components")
}

pub fn s2_dir() -> PathBuf { root().join("s2") }

#[derive(Clone, Debug)]
pub struct S2Assets {
    pub server: PathBuf,
    pub model: PathBuf,
    /// Server config; written for downloaded components, kept as-is for the developer setup.
    pub config: PathBuf,
    pub workdir: PathBuf,
    pub cache: PathBuf,
    pub device: Option<u32>,
}

/// Downloaded components first, then the developer's local S2 test folder.
pub fn s2_assets() -> Option<S2Assets> {
    let dir = s2_dir();
    let ready = std::fs::read_to_string(dir.join("ready.json")).ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    if let Some(ready) = ready {
        let server = dir.join("runtime").join("audiocpp_server.exe");
        let model = dir.join("model.gguf");
        if server.is_file() && model.is_file() {
            return Some(S2Assets { server, model, config: dir.join("server.json"), workdir: dir.clone(),
                cache: dir.join("cache"), device: ready["device"].as_u64().map(|d| d as u32) });
        }
    }
    let dev = APP_DIR.parent()?.parent()?.join("tools/voice_training/s2_local_test");
    let server = dev.join("runtime/audiocpp_server.exe");
    let model = dev.join("models/Fish-Audio-S2-Pro-GGUF/fish-audio-s2-pro-q4_k-preserved-v2.gguf");
    let config = dev.join("resident-server.json");
    (server.is_file() && model.is_file() && config.is_file()).then(|| S2Assets {
        server, model, config, workdir: dev.clone(), cache: s2_dir().join("cache"), device: None,
    })
}

pub fn s2_reference() -> PathBuf { APP_DIR.join("resources/tts/xtts-references/jarvis_ru_012.wav") }

/// Server config for downloaded components: loads on first request, frees memory after 10 idle minutes.
pub fn s2_server_config(assets: &S2Assets) -> serde_json::Value {
    let path = |p: &PathBuf| p.to_string_lossy().replace('\\', "/");
    serde_json::json!({
        "host": "127.0.0.1", "port": s2_port(), "backend": "vulkan", "device": assets.device.unwrap_or(0),
        "threads": 4, "lazy_load": true, "idle_unload_ms": 600000, "max_loaded_models": 1,
        "log_request_body": false,
        "models": [{
            "id": S2_MODEL_ID, "family": "fish_audio", "path": path(&assets.model), "task": "tts", "mode": "offline",
            "default_voice_preset": { "voice_ref": path(&s2_reference()), "reference_text": S2_REFERENCE_TEXT }
        }]
    })
}
