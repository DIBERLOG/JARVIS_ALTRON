//! Small platform boundary; plans are testable without launching desktop programs.
use std::{path::{Path, PathBuf}, process::Command};

pub fn command_on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

pub fn resource_root(executable: &Path, override_root: Option<PathBuf>) -> PathBuf {
    if let Some(root) = override_root { return root; }
    let directory = executable.parent().unwrap_or(Path::new("."));
    let candidates = [directory.to_path_buf(), directory.join("../Resources"),
        directory.join("../lib/jarvis"), directory.join("../share/jarvis")];
    candidates.into_iter().find(|root| root.join("resources").is_dir())
        .unwrap_or_else(|| directory.to_path_buf())
}

pub fn unix_command(id: &str, arguments: &[String], mac: bool) -> Option<(String, Vec<String>)> {
    let url = match id {
        "browser_open" | "open_google" => Some("https://www.google.com/"),
        "open_youtube" => Some("https://www.youtube.com/"),
        "open_chatgpt" => Some("https://chatgpt.com/"),
        "twitch_open" => Some("https://www.twitch.tv/"),
        _ => None,
    };
    if let Some(url) = url {
        return Some((if mac { "open" } else { "xdg-open" }.into(), vec![url.into()]));
    }
    let (application, linux) = match id {
        "calculator_open" => ("Calculator", "gnome-calculator"),
        "telegram_open" => ("Telegram", "telegram-desktop"),
        "discord_open" => ("Discord", "discord"),
        "vscode_open" => ("Visual Studio Code", "code"),
        "game_mode" => ("Steam", "steam"),
        _ => return None,
    };
    let _ = arguments; // Windows executable arguments must never be reused on Unix.
    Some(if mac { ("open".into(), vec!["-a".into(), application.into()]) }
        else { (linux.into(), vec![]) })
}

pub fn system_speech(text: &str) -> Option<Command> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/say");
        command.args(["--", text]);
        return Some(command);
    }
    #[cfg(target_os = "linux")]
    {
        let program = command_on_path("espeak-ng").or_else(|| command_on_path("espeak"))?;
        let mut command = Command::new(program);
        command.args(["-v", "ru", "--", text]);
        return Some(command);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    { let _ = text; None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unix_launches_have_literal_arguments_and_no_windows_helpers() {
        assert_eq!(unix_command("vscode_open", &[], true), Some(("open".into(), vec!["-a".into(), "Visual Studio Code".into()])));
        assert_eq!(unix_command("calculator_open", &[], false).unwrap().0, "gnome-calculator");
        assert!(unix_command("computer_restart", &[], false).is_none());
        assert!(unix_command("browser_close", &[], true).is_none());
        assert_eq!(unix_command("open_youtube", &["& evil".into()], false).unwrap().1, ["https://www.youtube.com/"]);
    }
    #[test]
    fn bundled_and_explicit_resources_are_resolved() {
        let temp = tempfile::tempdir().unwrap();
        let mac = temp.path().join("JARVIS.app/Contents");
        std::fs::create_dir_all(mac.join("Resources/resources")).unwrap();
        assert!(resource_root(&mac.join("MacOS/jarvis-gui"), None).ends_with("../Resources"));
        assert_eq!(resource_root(Path::new("/bin/jarvis"), Some(temp.path().into())), temp.path());
    }
}
