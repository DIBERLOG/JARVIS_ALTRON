use std::{path::PathBuf, process::Command};

#[tauri::command]
pub fn show_in_folder(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.exists() { return Err("Файл или папка больше не существует".into()); }
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(format!("/select,{}", path.display()));
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-R").arg(&path);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(if path.is_dir() { path.as_path() } else { path.parent().ok_or("Папка файла недоступна")? });
        command
    };
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    { return Err("Открытие системной папки недоступно на этой платформе".into()); }
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    command.spawn().map(|_| ()).map_err(|_| "Не удалось открыть файловый менеджер".into())
}
