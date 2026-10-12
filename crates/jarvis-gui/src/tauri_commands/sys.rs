use sysinfo::{System, Pid, ProcessRefreshKind, RefreshKind, CpuRefreshKind, Components};
use peak_alloc::PeakAlloc;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use std::time::Duration;
use serde::Serialize;
use crate::AppState;

static OWNED_CORE: Mutex<Option<std::process::Child>> = Mutex::new(None);

pub fn stop_owned_jarvis_app() {
    if let Ok(mut owned) = OWNED_CORE.lock() {
        if let Some(mut child) = owned.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[tauri::command]
pub async fn get_jarvis_terminal_log() -> Result<String,String> {
    tauri::async_runtime::spawn_blocking(|| {
        let path=jarvis_core::APP_LOG_DIR.get().ok_or("Папка логов ещё не готова")?
            .join(jarvis_core::config::LOG_FILE_NAME);
        let mut file=match std::fs::File::open(path) {
            Ok(file)=>file,
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(String::new()),
            Err(_)=>return Err("Не удалось прочитать лог JARVIS. Повторите попытку.".into()),
        };
        read_log_tail(&mut file).map_err(|_|"Лог обновляется. Повторите попытку.".into())
    }).await.map_err(|_|"Не удалось загрузить лог JARVIS".to_string())?
}

fn read_log_tail(reader:&mut (impl std::io::Read + std::io::Seek)) -> std::io::Result<String> {
    use std::io::{Read, SeekFrom};
    let length=reader.seek(SeekFrom::End(0))?;
    let start=length.saturating_sub(64*1024);
    reader.seek(SeekFrom::Start(start))?;
    let mut bytes=Vec::new();
    (&mut *reader).take(64*1024).read_to_end(&mut bytes)?;
    // Skip the first partial line (and potentially split UTF-8 codepoint).
    let offset=if start>0 {bytes.iter().position(|byte|*byte==b'\n').map(|i|i+1).unwrap_or(bytes.len())} else {0};
    let text=String::from_utf8_lossy(&bytes[offset..]);
    let lines:Vec<_>=text.lines().rev().take(300).collect();
    Ok(lines.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod terminal_tests {
    use super::read_log_tail;
    #[test]
    fn tail_preserves_russian_and_limits_lines() {
        let text=(0..500).map(|i|format!("Запрос {i}\n")).collect::<String>();
        let result=read_log_tail(&mut std::io::Cursor::new(text.into_bytes())).unwrap();
        assert_eq!(result.lines().count(),300);
        assert!(result.starts_with("Запрос 200\n"));
        assert!(result.ends_with("Запрос 499"));
    }
    #[test]
    fn tail_handles_large_and_empty_files() {
        assert_eq!(read_log_tail(&mut std::io::Cursor::new(Vec::<u8>::new())).unwrap(),"");
        let text=format!("{}\nПоследний запрос\n","x".repeat(70000));
        assert_eq!(read_log_tail(&mut std::io::Cursor::new(text.into_bytes())).unwrap(),"Последний запрос");
    }
}

#[global_allocator]
static PEAK_ALLOC: PeakAlloc = PeakAlloc;

static SYS: Lazy<Mutex<System>> = Lazy::new(|| {
    Mutex::new(System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_memory().with_cpu())
            .with_cpu(CpuRefreshKind::everything())
    ))
});

static COMPONENTS: Lazy<Mutex<Components>> = Lazy::new(|| {
    Mutex::new(Components::new_with_refreshed_list())
});

/// Find jarvis-app process and return its PID
fn find_jarvis_app_pid(sys: &System) -> Option<Pid> {
    let expected = std::env::current_exe().ok()?.with_file_name(if cfg!(windows) { "jarvis-app-sync-v2-lab.exe" } else { "jarvis-app-sync-v2-lab" });
    for (pid, process) in sys.processes() {
        if process.exe().is_some_and(|path| path == expected) {
            return Some(*pid);
        }
    }
    None
}

#[derive(serde::Serialize)]
pub struct JarvisAppStats {
    pub running: bool,
    pub ram_mb: u64,
    pub cpu_usage: f32,
}

#[tauri::command]
pub fn get_jarvis_app_stats() -> JarvisAppStats {
    let mut sys = SYS.lock().unwrap();
    
    // refresh all processes to find jarvis-app
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    
    if let Some(pid) = find_jarvis_app_pid(&sys) {
        if let Some(proc) = sys.process(pid) {
            return JarvisAppStats {
                running: true,
                ram_mb: proc.memory() / 1024 / 1024,
                cpu_usage: proc.cpu_usage(),
            };
        }
    }
    
    JarvisAppStats {
        running: false,
        ram_mb: 0,
        cpu_usage: 0.0,
    }
}

#[tauri::command]
pub fn get_current_ram_usage() -> u64 {
    let mut sys = SYS.lock().unwrap();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    
    if let Some(pid) = find_jarvis_app_pid(&sys) {
        if let Some(proc) = sys.process(pid) {
            return proc.memory() / 1024 / 1024;
        }
    }
    
    0
}

#[tauri::command]
pub fn is_jarvis_app_running() -> bool {
    let mut sys = SYS.lock().unwrap();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    find_jarvis_app_pid(&sys).is_some()
}

#[tauri::command]
pub fn get_cpu_temp() -> String {
    let mut components = COMPONENTS.lock().unwrap();
    components.refresh(true);
    
    for component in components.iter() {
        let label = component.label().to_lowercase();
        if label.contains("cpu") || label.contains("core") || label.contains("package") {
            if let Some(temp) = component.temperature() {
                return format!("{:.1}", temp);
            }
        }
    }
    
    if let Some(component) = components.iter().next() {
        if let Some(temp) = component.temperature() {
            return format!("{:.1}", temp);
        }
    }
    
    String::from("N/A")
}

#[tauri::command]
pub fn get_cpu_usage() -> f32 {
    let mut sys = SYS.lock().unwrap();
    
    sys.refresh_cpu_all();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu_all();
    
    sys.global_cpu_usage()
}

#[tauri::command]
pub fn get_peak_ram_usage() -> String {
    format!("{}", PEAK_ALLOC.peak_usage_as_gb())
}

#[tauri::command]
pub fn run_jarvis_app() -> Result<(), String> {
    let mut owned = OWNED_CORE.lock().map_err(|_| "Voice process ownership lock failed")?;
    let exe_dir = std::env::current_exe()
        .map_err(|e| format!("Failed to get exe path: {}", e))?
        .parent()
        .ok_or("Failed to get exe directory")?
        .to_path_buf();
    
    #[cfg(target_os = "windows")]
    let jarvis_app_name = "jarvis-app-sync-v2-lab.exe";
    
    #[cfg(not(target_os = "windows"))]
    let jarvis_app_name = "jarvis-app-sync-v2-lab";
    
    let jarvis_app_path = exe_dir.join(jarvis_app_name);
    
    if !jarvis_app_path.exists() {
        return Err(format!("jarvis-app not found at: {}", jarvis_app_path.display()));
    }
    if is_jarvis_app_running() {
        return Ok(());
    }
    
    let mut command = std::process::Command::new(&jarvis_app_path);
    command.current_dir(&exe_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let child = command.spawn()
        .map_err(|e| format!("Failed to start jarvis-app: {}", e))?;
    *owned = Some(child);
    
    Ok(())
}

#[tauri::command]
pub fn ipc_credentials()->Result<String,String>{jarvis_core::ipc_credentials::read_credentials()}

#[derive(Serialize)]
pub struct HealthStatus {
    pub microphone_ready: bool,
    pub microphone_name: String,
    pub neural_ready: bool,
    pub tts_mode: String,
    pub tts_ready: bool,
    pub voicemod_running: bool,
    pub internet_available: bool,
}

#[tauri::command]
pub fn get_health_status(state: tauri::State<'_, AppState>) -> HealthStatus {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let assistant_running = find_jarvis_app_pid(&sys).is_some();
    let voicemod_running = sys.processes().values().any(|process| {
        process.name().to_string_lossy().eq_ignore_ascii_case("Voicemod.exe")
    });
    let devices = jarvis_core::recorder::get_audio_devices();
    let mic_index = state.settings.read("selected_microphone")
        .and_then(|value| value.parse::<i32>().ok()).unwrap_or(-1);
    let microphone_name = if mic_index < 0 { "Системный микрофон".to_string() }
        else { devices.get(mic_index as usize).cloned().unwrap_or_else(|| "Микрофон не найден".into()) };
    let microphone_ready = assistant_running && (mic_index < 0 || (mic_index as usize) < devices.len());
    let (tts_mode, tts_ready) = jarvis_core::tts::mode_status();
    let internet_available = match reqwest::blocking::Client::builder().timeout(Duration::from_secs(3)).build() {
        Ok(client) => client.get("https://html.duckduckgo.com/html/").send()
            .is_ok_and(|response| response.status().is_success())
            || client.get("https://www.google.com/generate_204").send()
                .is_ok_and(|response| response.status().is_success()),
        Err(_) => false,
    };
    HealthStatus {
        microphone_ready, microphone_name, neural_ready: assistant_running,
        tts_mode, tts_ready, voicemod_running, internet_available,
    }
}
