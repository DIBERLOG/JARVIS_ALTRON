use std::{path::PathBuf, process::Command};
use serde::Serialize;

#[derive(Serialize)]
pub struct FoundFile { name:String, path:String }
#[derive(Serialize)]
pub struct FileResults { files:Vec<FoundFile>, limited:bool }

#[tauri::command]
pub async fn everyday_find_files(query:String, yesterday:bool) -> Result<FileResults,String> {
    let query=query.trim().to_lowercase();
    if query.chars().count()<2 || query.len()>200 {return Err("Укажите от 2 до 200 символов имени файла.".into())}
    tauri::async_runtime::spawn_blocking(move || {
        let home=std::env::var_os(if cfg!(windows){"USERPROFILE"}else{"HOME"}).ok_or("Домашняя папка недоступна")?;
        let mut stack:Vec<_>=["Documents","Downloads","Desktop"].iter().map(|folder|PathBuf::from(&home).join(folder)).collect();
        let mut files=Vec::new();let mut scanned=0;let mut limited=false;
        let previous=chrono::Local::now().date_naive()-chrono::Duration::days(1);
        while let Some(dir)=stack.pop() {
            let Ok(entries)=std::fs::read_dir(dir) else{continue};
            for entry in entries.flatten() {
                scanned+=1;if scanned>30000 {limited=true;break}
                let Ok(kind)=entry.file_type()else{continue};if kind.is_symlink(){continue}
                let name=entry.file_name().to_string_lossy().to_string();
                if kind.is_dir(){if !["node_modules","target",".git",".venv","AppData"].contains(&name.as_str()){stack.push(entry.path())}continue}
                if !kind.is_file()||!name.to_lowercase().contains(&query){continue}
                if yesterday {let Ok(modified)=entry.metadata().and_then(|m|m.modified())else{continue};let date:chrono::DateTime<chrono::Local>=modified.into();if date.date_naive()!=previous{continue}}
                files.push(FoundFile{name,path:entry.path().to_string_lossy().to_string()});
                if files.len()>=20 {limited=true;break}
            }
            if limited{break}
        }
        files.sort_by(|a,b|a.name.cmp(&b.name));Ok(FileResults{files,limited})
    }).await.map_err(|_|"Не удалось завершить поиск файлов.".to_string())?
}

#[derive(Serialize)]
pub struct ClipboardText { text:String, from_selection:bool }

/// Copies the selection of the active window with Ctrl+C, so "переведи это"
/// works on highlighted text. Returns true only if the clipboard really changed.
#[cfg(windows)]
fn copy_selection() -> bool {
    use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_C};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let window=GetForegroundWindow();
        if window.is_null(){return false}
        let mut pid=0;GetWindowThreadProcessId(window,&mut pid);
        if pid==std::process::id(){return false} // typed into JARVIS itself
        let mut class=[0u16;64];let length=GetClassNameW(window,class.as_mut_ptr(),class.len() as i32).max(0) as usize;
        let class=String::from_utf16_lossy(&class[..length]);
        // Ctrl+C in a terminal interrupts the running program instead of copying.
        if ["ConsoleWindowClass","CASCADIA_HOSTING_WINDOW_CLASS","mintty","PuTTY","VirtualConsoleClass"].contains(&class.as_str()){return false}
        let before=GetClipboardSequenceNumber();
        let key=|vk:u16,flags:u32|INPUT{r#type:INPUT_KEYBOARD,Anonymous:INPUT_0{ki:KEYBDINPUT{wVk:vk,wScan:0,dwFlags:flags,time:0,dwExtraInfo:0}}};
        let inputs=[key(VK_CONTROL,0),key(VK_C,0),key(VK_C,KEYEVENTF_KEYUP),key(VK_CONTROL,KEYEVENTF_KEYUP)];
        if SendInput(inputs.len() as u32,inputs.as_ptr(),std::mem::size_of::<INPUT>() as i32)!=inputs.len() as u32{return false}
        for _ in 0..12 {std::thread::sleep(std::time::Duration::from_millis(40));if GetClipboardSequenceNumber()!=before{return true}}
        false
    }
}
#[cfg(not(windows))]
fn copy_selection() -> bool { false }

#[tauri::command]
pub async fn everyday_clipboard_text() -> Result<ClipboardText,String> {
    tauri::async_runtime::spawn_blocking(|| {
        let from_selection=copy_selection();
        #[cfg(windows)]
        let output={
            use std::os::windows::process::CommandExt;
            Command::new("powershell.exe").creation_flags(0x08000000).args(["-NoProfile","-NonInteractive","-Command","[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); Get-Clipboard -Raw"]).output()
        };
        #[cfg(not(windows))]
        let output=if cfg!(target_os="macos"){Command::new("pbpaste").output()}else{Command::new("xclip").args(["-selection","clipboard","-o"]).output()};
        let output=output.map_err(|_|"Не удалось прочитать буфер обмена")?;
        if !output.status.success(){return Err("Скопируйте текст через Ctrl+C и повторите команду.".into())}
        let text=String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.chars().count()>12000{return Err("Выделите или скопируйте не больше 12 000 символов.".into())}Ok(ClipboardText{text,from_selection})
    }).await.map_err(|_|"Не удалось прочитать текст".to_string())?
}

/// Login pages by the canonical site name from voice recognition. Fixed URLs only:
/// recognized speech never becomes part of a URL. Credentials are filled by the
/// browser's password manager after it is unlocked; JARVIS never sees them.
fn login_url(site:&str) -> Option<(&'static str,&'static str)> {
    Some(match site {
        "github"=>("GitHub","https://github.com/login"),
        "gitlab"=>("GitLab","https://gitlab.com/users/sign_in"),
        "google"=>("Google","https://accounts.google.com/"),
        "яндекс"=>("Яндекс","https://passport.yandex.ru/auth"),
        "вконтакте"=>("ВКонтакте","https://vk.com/login"),
        "telegram"=>("Telegram","https://web.telegram.org/"),
        "discord"=>("Discord","https://discord.com/login"),
        "steam"=>("Steam","https://store.steampowered.com/login/"),
        "twitch"=>("Twitch","https://www.twitch.tv/login"),
        "youtube"=>("YouTube","https://accounts.google.com/ServiceLogin?service=youtube"),
        "microsoft"=>("Microsoft","https://login.live.com/"),
        "chatgpt"=>("ChatGPT","https://chatgpt.com/auth/login"),
        "notion"=>("Notion","https://www.notion.so/login"),
        "figma"=>("Figma","https://www.figma.com/login"),
        "госуслуги"=>("Госуслуги","https://www.gosuslugi.ru/"),
        _=>return None,
    })
}

fn open_url(url:&'static str) -> bool {
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        Command::new("cmd.exe").creation_flags(0x08000000).args(["/C","start","",url]).status().is_ok_and(|s|s.success())
    }
    #[cfg(not(windows))] {
        Command::new(if cfg!(target_os="macos"){"open"}else{"xdg-open"}).arg(url).spawn().is_ok()
    }
}

#[tauri::command]
pub async fn everyday_open_login(site:String) -> Result<String,String> {
    let (name,url)=login_url(site.trim()).ok_or("Этого сайта нет в списке входа. Доступны: GitHub, GitLab, Google, Яндекс, ВКонтакте, Telegram, Discord, Steam, Twitch, YouTube, Microsoft, ChatGPT, Notion, Figma, Госуслуги.")?;
    let opened=tauri::async_runtime::spawn_blocking(move||open_url(url)).await.map_err(|_|"Не удалось открыть браузер.")?;
    if !opened{return Err(format!("Не удалось открыть страницу входа {name}."))}
    Ok(format!("Открываю вход в {name}. Пароль подставит менеджер паролей браузера после разблокировки — диктовать его не нужно, я его не вижу и не сохраняю."))
}

#[tauri::command]
pub async fn everyday_work_mode() -> Result<String,String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut results=Vec::new();
        #[cfg(windows)] {
            let candidates=[std::env::var_os("LOCALAPPDATA").map(|p|PathBuf::from(p).join("Programs/Microsoft VS Code/Code.exe")),std::env::var_os("ProgramFiles").map(|p|PathBuf::from(p).join("Microsoft VS Code/Code.exe"))];
            let code=candidates.into_iter().flatten().find(|p|p.is_file());
            results.push(if code.is_some_and(|p|Command::new(p).spawn().is_ok()){ "VS Code запущен." }else{"VS Code не найден или не запустился."});
            // Fixed URL, never shell-interpolate recognized speech.
            use std::os::windows::process::CommandExt;
            let opened=Command::new("cmd.exe").creation_flags(0x08000000).args(["/C","start","","https://music.yandex.ru/"]).status().is_ok_and(|s|s.success());
            results.push(if opened{"Яндекс Музыка открыта в браузере. Нажмите воспроизведение на сайте."}else{"Не удалось открыть Яндекс Музыку."});
        }
        #[cfg(not(windows))] {
            results.push(if Command::new("code").spawn().is_ok(){"VS Code запущен."}else{"Не удалось запустить VS Code."});
            let opener=if cfg!(target_os="macos"){"open"}else{"xdg-open"};
            results.push(if Command::new(opener).arg("https://music.yandex.ru/").spawn().is_ok(){"Яндекс Музыка открыта в браузере."}else{"Не удалось открыть Яндекс Музыку."});
        }
        Ok(results.join(" "))
    }).await.map_err(|_|"Не удалось включить режим работы.".to_string())?
}
