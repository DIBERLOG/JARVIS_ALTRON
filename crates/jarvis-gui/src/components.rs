//! First-launch downloads: the Fish S2 voice and local AI (Ollama + models).
//! JARVIS works without them; they are fetched in the background and resume after interruptions.
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use jarvis_core::components;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};

const S2_RUNTIME_URL: &str = "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.0/audio-v0.9.0-bin-windows-x64-vulkan.zip";
const S2_RUNTIME_SHA256: &str = "f884538138e44528a0bf17bb75dbe7ff7350cb91a14cb72c3e9cc1d4a31d6f1f";
// Pinned revision so the checksum below never changes under us.
const S2_MODEL_URL: &str = "https://huggingface.co/audio-cpp/audio.cpp-gguf/resolve/69492fe6b3f35ec13e49e5d2ed388bc9c24473d4/Fish-Audio-S2-Pro-GGUF/fish-audio-s2-pro-q8_0.gguf";
const S2_MODEL_SHA256: &str = "4ffc169447b7a26df8bf49e8637adb4000bfa763a22c018b6c03968564259d0b";
const S2_MODEL_BYTES: u64 = 6_317_911_232;
/// The Q8 model needs about 6.5 GB of video memory; slower cards keep the system voice.
const S2_MIN_VRAM_MB: u64 = 7_500;
const OLLAMA_SETUP_URL: &str = "https://ollama.com/download/OllamaSetup.exe";
const TRANSLATION_MODEL: &str = "qwen3:4b-instruct";
const GB: u64 = 1024 * 1024 * 1024;

#[derive(Serialize, Clone, Default)]
pub struct Item {
    /// waiting | checking | downloading | installing | ready | unsupported | error
    pub state: String,
    pub detail: String,
    pub done: u64,
    pub total: u64,
}

#[derive(Serialize, Clone, Default)]
pub struct Status { pub running: bool, pub s2: Item, pub ollama: Item }

static STATUS: Lazy<Mutex<Status>> = Lazy::new(|| Mutex::new(Status {
    running: false,
    s2: Item { state: "waiting".into(), detail: "Проверка начнётся после запуска".into(), ..Default::default() },
    ollama: Item { state: "waiting".into(), detail: "Проверка начнётся после запуска".into(), ..Default::default() },
}));
static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
enum Which { S2, Ollama }

fn set(which: Which, state: &str, detail: impl Into<String>, done: u64, total: u64) {
    let mut status = STATUS.lock();
    let item = match which { Which::S2 => &mut status.s2, Which::Ollama => &mut status.ollama };
    *item = Item { state: state.into(), detail: detail.into(), done, total };
}

#[tauri::command]
pub fn components_status() -> Status { STATUS.lock().clone() }

/// Starts (or retries) the downloads; does nothing if they are already running.
#[tauri::command]
pub fn components_start() { start_in_background(Duration::ZERO); }

pub fn start_in_background(delay: Duration) {
    if RUNNING.swap(true, Ordering::SeqCst) { return; }
    STATUS.lock().running = true;
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if let Err(error) = install_s2() { log::warn!("S2 voice download: {error}"); set(Which::S2, "error", error, 0, 0); }
        if let Err(error) = install_ollama() { log::warn!("Ollama setup: {error}"); set(Which::Ollama, "error", error, 0, 0); }
        STATUS.lock().running = false;
        RUNNING.store(false, Ordering::SeqCst);
    });
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(20)).timeout(None::<Duration>)
        .user_agent("JARVIS-ALTRON-components").build().map_err(|e| e.to_string())
}

#[cfg(windows)]
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let mut existing = path.to_path_buf();
    while !existing.exists() { existing = existing.parent()?.to_path_buf(); }
    let wide: Vec<u16> = existing.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}
#[cfg(not(windows))]
fn free_bytes(_: &Path) -> Option<u64> { None }

fn need_space(path: &Path, bytes: u64) -> Result<(), String> {
    match free_bytes(path) {
        Some(free) if free < bytes => Err(format!("Не хватает места на диске: нужно {:.1} ГБ, свободно {:.1} ГБ. Освободите место и нажмите «Повторить».", bytes as f64 / GB as f64, free as f64 / GB as f64)),
        _ => Ok(()),
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 4 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 { break; }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Downloads to `<dest>.part`, resuming what an earlier attempt left; checks the SHA-256 when given.
fn download(which: Which, label: &str, url: &str, dest: &Path, sha256: Option<&str>) -> Result<(), String> {
    if dest.is_file() { return Ok(()); }
    fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = dest.with_extension(format!("{}.part", dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")));
    let client = client()?;
    let mut attempt = 0;
    loop {
        attempt += 1;
        match download_once(which, label, &client, url, &part) {
            Ok(()) => break,
            Err(error) if attempt < 4 => {
                log::warn!("{label}: {error}; retrying");
                set(which, "downloading", format!("{label}: связь прервалась, продолжаю…"), 0, 0);
                std::thread::sleep(Duration::from_secs(5 * attempt));
            }
            Err(error) => return Err(format!("{label}: {error}. Проверьте интернет и нажмите «Повторить» — загрузка продолжится с того же места.")),
        }
    }
    if let Some(expected) = sha256 {
        set(which, "checking", format!("{label}: проверяю целостность файла…"), 0, 0);
        if sha256_file(&part)? != expected {
            let _ = fs::remove_file(&part);
            return Err(format!("{label}: файл повреждён при загрузке. Нажмите «Повторить», он скачается заново."));
        }
    }
    fs::rename(&part, dest).map_err(|e| e.to_string())
}

fn download_once(which: Which, label: &str, client: &reqwest::blocking::Client, url: &str, part: &Path) -> Result<(), String> {
    let existing = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut request = client.get(url);
    if existing > 0 { request = request.header(reqwest::header::RANGE, format!("bytes={existing}-")); }
    let mut response = request.send().map_err(|e| e.to_string())?;
    let status = response.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE { return Ok(()); } // Already complete.
    if !status.is_success() { return Err(format!("сервер ответил {status}")); }
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let start = if resumed { existing } else { 0 };
    let total = response.content_length().map(|len| len + start).unwrap_or(0);
    let mut file = if resumed { OpenOptions::new().append(true).open(part) } else { File::create(part) }.map_err(|e| e.to_string())?;
    let mut done = start;
    let mut buffer = vec![0u8; 1 << 20];
    let mut shown = Instant::now() - Duration::from_secs(1);
    loop {
        let read = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 { break; }
        file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
        done += read as u64;
        if shown.elapsed() > Duration::from_millis(400) {
            shown = Instant::now();
            set(which, "downloading", label.to_string(), done, total);
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    if total > 0 && done < total { return Err("загрузка оборвалась".into()); }
    Ok(())
}

fn no_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

/// Largest dedicated video memory reported by the display drivers, in MB.
fn max_vram_mb() -> u64 {
    let output = no_window(Command::new("reg").args(["query",
        r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}", "/s", "/v", "HardwareInformation.qwMemorySize"]))
        .output();
    let Ok(output) = output else { return 0; };
    String::from_utf8_lossy(&output.stdout).lines()
        .filter(|line| line.contains("REG_QWORD"))
        .filter_map(|line| line.split_whitespace().last())
        .filter_map(|hex| u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok())
        .max().unwrap_or(0) / (1024 * 1024)
}

/// Index of the first discrete Vulkan GPU, as audio.cpp numbers them.
fn discrete_vulkan_device(server: &Path) -> Option<u32> {
    let output = no_window(Command::new(server).arg("--list-devices").current_dir(server.parent()?)).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).to_string() + "
" + &String::from_utf8_lossy(&output.stderr);
    parse_devices(&text)
}

fn parse_devices(text: &str) -> Option<u32> {
    text.lines().map(str::trim).filter(|line| line.starts_with("Vulkan:") && line.ends_with("[GPU]"))
        .find_map(|line| line["Vulkan:".len()..].split_whitespace().next()?.parse().ok())
}

fn extract_zip(zip_path: &Path, target: &Path) -> Result<(), String> {
    let temporary = target.with_extension("tmp");
    let _ = fs::remove_dir_all(&temporary);
    let mut archive = zip::ZipArchive::new(File::open(zip_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let Some(relative) = entry.enclosed_name() else { continue; }; // Never write outside the target.
        let out = temporary.join(relative);
        if entry.is_dir() { fs::create_dir_all(&out).map_err(|e| e.to_string())?; continue; }
        fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut File::create(&out).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_dir_all(target);
    fs::rename(&temporary, target).map_err(|e| e.to_string())
}

fn install_s2() -> Result<(), String> {
    if jarvis_core::s2::available() {
        set(Which::S2, "ready", "Голос Fish S2 установлен", 0, 0);
        return Ok(());
    }
    set(Which::S2, "checking", "Проверяю видеокарту…", 0, 0);
    let vram = max_vram_mb();
    if vram < S2_MIN_VRAM_MB {
        set(Which::S2, "unsupported", format!("Для голоса Fish S2 нужна видеокарта с 8 ГБ памяти (найдено {:.1} ГБ). Джарвис говорит системным голосом.", vram as f64 / 1024.0), 0, 0);
        return Ok(());
    }
    let dir = components::s2_dir();
    let model = dir.join("model.gguf");
    let model_left = if model.is_file() { 0 } else { S2_MODEL_BYTES };
    need_space(&dir, model_left + GB)?;
    let server = dir.join("runtime").join("audiocpp_server.exe");
    if !server.is_file() {
        let zip = dir.join("runtime.zip");
        download(Which::S2, "Голосовой сервер (60 МБ)", S2_RUNTIME_URL, &zip, Some(S2_RUNTIME_SHA256))?;
        set(Which::S2, "installing", "Распаковываю голосовой сервер…", 0, 0);
        extract_zip(&zip, &dir.join("runtime"))?;
        let _ = fs::remove_file(&zip);
    }
    let Some(device) = discrete_vulkan_device(&server) else {
        set(Which::S2, "unsupported", "Голосовой сервер не нашёл подходящую видеокарту (Vulkan). Джарвис говорит системным голосом.", 0, 0);
        return Ok(());
    };
    download(Which::S2, "Модель голоса Fish S2 (6,3 ГБ)", S2_MODEL_URL, &model, Some(S2_MODEL_SHA256))?;
    let ready = serde_json::json!({"device": device, "model": "fish-audio-s2-pro-q8_0"});
    fs::write(dir.join("ready.json"), ready.to_string()).map_err(|e| e.to_string())?;
    set(Which::S2, "ready", "Голос Fish S2 установлен", 0, 0);
    Ok(())
}

fn ollama_exe() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?.join("Programs/Ollama/ollama.exe");
    if local.is_file() { return Some(local); }
    let output = no_window(Command::new("where").arg("ollama")).output().ok()?;
    String::from_utf8_lossy(&output.stdout).lines().map(PathBuf::from).find(|p| p.is_file())
}

/// Only an installer signed by Ollama is ever run.
fn signed_by_ollama(path: &Path) -> bool {
    let script = "$s=Get-AuthenticodeSignature -LiteralPath $env:JARVIS_CHECK_FILE; if($s.Status -eq 'Valid' -and $s.SignerCertificate.Subject -match 'Ollama'){'ok'}";
    no_window(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).env("JARVIS_CHECK_FILE", path))
        .output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "ok").unwrap_or(false)
}

fn installed_models() -> Result<Vec<String>, String> {
    let response = client()?.get("http://127.0.0.1:11434/api/tags").timeout(Duration::from_secs(10)).send().map_err(|e| e.to_string())?;
    let data: serde_json::Value = response.json().map_err(|e| e.to_string())?;
    Ok(data["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str().map(String::from)).collect())
}

fn has_model(installed: &[String], name: &str) -> bool {
    installed.iter().any(|m| m == name || (!name.contains(':') && m == &format!("{name}:latest")))
}

fn pull_model(name: &str) -> Result<(), String> {
    let label = format!("Модель ИИ {name}");
    set(Which::Ollama, "downloading", label.clone(), 0, 0);
    let response = client()?.post("http://127.0.0.1:11434/api/pull")
        .json(&serde_json::json!({"model": name, "stream": true})).send().map_err(|e| e.to_string())?;
    if !response.status().is_success() { return Err(format!("{label}: Ollama ответила {}", response.status())); }
    let mut shown = Instant::now() - Duration::from_secs(1);
    for line in BufReader::new(response).lines() {
        let line = line.map_err(|e| format!("{label}: связь прервалась ({e})"))?;
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else { continue; };
        if let Some(error) = event["error"].as_str() { return Err(format!("{label}: {error}")); }
        if event["status"] == "success" { return Ok(()); }
        if shown.elapsed() > Duration::from_millis(400) {
            shown = Instant::now();
            set(Which::Ollama, "downloading", label.clone(), event["completed"].as_u64().unwrap_or(0), event["total"].as_u64().unwrap_or(0));
        }
    }
    Err(format!("{label}: загрузка прервалась"))
}

fn install_ollama() -> Result<(), String> {
    set(Which::Ollama, "checking", "Проверяю Ollama…", 0, 0);
    if ollama_exe().is_none() {
        let dir = components::root().join("ollama");
        need_space(&dir, 12 * GB)?;
        let setup = dir.join("OllamaSetup.exe");
        download(Which::Ollama, "Ollama (1,9 ГБ)", OLLAMA_SETUP_URL, &setup, None)?;
        set(Which::Ollama, "installing", "Устанавливаю Ollama…", 0, 0);
        if !signed_by_ollama(&setup) {
            let _ = fs::remove_file(&setup);
            return Err("Установщик Ollama не прошёл проверку подписи и удалён. Нажмите «Повторить».".into());
        }
        let status = no_window(Command::new(&setup).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"]))
            .status().map_err(|e| format!("Не удалось запустить установщик Ollama: {e}"))?;
        let _ = fs::remove_file(&setup);
        if !status.success() || ollama_exe().is_none() { return Err("Ollama не установилась. Нажмите «Повторить».".into()); }
    }
    crate::ollama::ensure_running()?;
    let chat = jarvis_core::db::latest_settings().map(|s| s.local_chat_model).unwrap_or_else(|| "qwen3:8b".into());
    let mut wanted = vec![TRANSLATION_MODEL.to_string()];
    if !chat.trim().is_empty() && chat != TRANSLATION_MODEL { wanted.push(chat); }
    let installed = installed_models()?;
    for model in wanted.iter().filter(|m| !has_model(&installed, m)) { pull_model(model)?; }
    set(Which::Ollama, "ready", format!("Ollama и модели готовы: {}", wanted.join(", ")), 0, 0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_discrete_gpu_not_the_integrated_one() {
        let text = "Vulkan:0 \"AMD Radeon(TM) Graphics\" [IGPU]\nVulkan:1 \"NVIDIA GeForce RTX 5060\" [GPU]\nCPU:0 \"Ryzen\" [CPU]";
        assert_eq!(parse_devices(text), Some(1));
        assert_eq!(parse_devices("Vulkan:0 \"AMD\" [IGPU]"), None);
    }

    #[test]
    fn model_names_match_with_or_without_latest() {
        let installed = vec!["qwen3:8b".to_string(), "gemma:latest".to_string()];
        assert!(has_model(&installed, "qwen3:8b"));
        assert!(has_model(&installed, "gemma"));
        assert!(!has_model(&installed, "qwen3:4b-instruct"));
    }

    #[test]
    #[ignore = "downloads about 6.4 GB into JARVIS_COMPONENTS_DIR"]
    fn downloads_the_s2_voice() {
        assert!(std::env::var_os("JARVIS_COMPONENTS_DIR").is_some(), "set JARVIS_COMPONENTS_DIR to a scratch folder");
        install_s2().unwrap();
        let status = components_status();
        assert_eq!(status.s2.state, "ready", "{}", status.s2.detail);
        assert!(jarvis_core::components::s2_assets().unwrap().device.is_some());
    }

    #[test]
    #[ignore = "reads the real display driver registry"]
    fn this_pc_reports_video_memory() {
        assert!(max_vram_mb() > 0, "no video memory reported");
        println!("max VRAM {} MB", max_vram_mb());
    }
}
