//! Commands the user adds in Settings: a phrase that opens an app or a site.
//! Stored as JSON in the `custom_commands_v1` setting.

use serde::{Deserialize, Serialize};

pub const MAX_COMMANDS: usize = 100;
const MAX_PHRASES: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CustomCommand {
    pub id: String,
    /// Ways to say it, e.g. ["открой ютуб", "ютуб"].
    pub phrases: Vec<String>,
    /// "app" or "site".
    pub kind: String,
    /// Path to a program/shortcut or an http(s) address.
    pub target: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool { true }

/// Lowercase, ё→е, punctuation to spaces, single spaces, without the wake word.
pub fn normalize(text: &str) -> String {
    let text: String = text.to_lowercase().replace('ё', "е").chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let words: Vec<&str> = text.split_whitespace().collect();
    let words = match words.first() { Some(&"джарвис") => &words[1..], _ => &words[..] };
    words.join(" ")
}

pub fn valid_site(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false };
    matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some()
        && parsed.username().is_empty() && parsed.password().is_none()
}

/// Validates the stored JSON; returns the commands on success.
pub fn parse(json: &str) -> Result<Vec<CustomCommand>, String> {
    if json.trim().is_empty() { return Ok(Vec::new()); }
    if json.len() > 200_000 { return Err("Слишком много своих команд".into()); }
    let commands: Vec<CustomCommand> = serde_json::from_str(json).map_err(|_| "Некорректный список своих команд")?;
    if commands.len() > MAX_COMMANDS { return Err(format!("Можно добавить не больше {MAX_COMMANDS} команд")); }
    for command in &commands {
        if command.id.is_empty() || command.id.len() > 64 { return Err("Некорректный идентификатор команды".into()); }
        if command.phrases.is_empty() || command.phrases.len() > MAX_PHRASES {
            return Err(format!("У команды должно быть от 1 до {MAX_PHRASES} фраз"));
        }
        if command.phrases.iter().any(|p| { let n = normalize(p).chars().count(); !(2..=80).contains(&n) }) {
            return Err("Каждая фраза — от 2 до 80 символов".into());
        }
        match command.kind.as_str() {
            "site" => if !valid_site(&command.target) { return Err(format!("«{}» — не адрес сайта. Нужна ссылка http:// или https://", command.target)); },
            "app" => if command.target.trim().is_empty() || command.target.len() > 1000 || command.target.contains('\0') {
                return Err("Укажите путь к программе".into());
            },
            _ => return Err("Тип команды: приложение или сайт".into()),
        }
    }
    Ok(commands)
}

const OPEN_VERBS: [&str; 6] = ["открой", "запусти", "включи", "зайди на", "перейди на", "открыть"];

/// The command whose phrase matches what was said. The user may say the phrase
/// alone or with an opening verb: "ютуб" also answers to "открой ютуб".
pub fn find<'a>(commands: &'a [CustomCommand], said: &str) -> Option<&'a CustomCommand> {
    let said = normalize(said);
    if said.is_empty() { return None; }
    let bare = OPEN_VERBS.iter().find_map(|verb| said.strip_prefix(verb).and_then(|rest| rest.strip_prefix(' '))).unwrap_or(&said);
    commands.iter().filter(|c| c.enabled).find(|command| command.phrases.iter().any(|phrase| {
        let phrase = normalize(phrase);
        let phrase_bare = OPEN_VERBS.iter().find_map(|verb| phrase.strip_prefix(verb).and_then(|rest| rest.strip_prefix(' '))).unwrap_or(&phrase);
        phrase == said || phrase_bare == bare
            || (phrase_bare.chars().count() >= 6 && close(phrase_bare, bare))
    }))
}

/// One-letter slip of the recognizer ("ютюб" for "ютуб" is too short, "телеграмм" fits).
fn close(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 { return false; }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let value = (previous + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            previous = row[j + 1];
            row[j + 1] = value;
        }
    }
    row[b.len()] <= 1
}

pub fn load() -> Vec<CustomCommand> {
    crate::db::latest_settings().and_then(|s| parse(&s.custom_commands).ok()).unwrap_or_default()
}

/// Opens the target without a shell, so characters in a URL or path are never
/// interpreted as commands.
pub fn run(command: &CustomCommand) -> Result<(), String> {
    use std::process::Command;
    if command.kind == "site" {
        if !valid_site(&command.target) { return Err("Некорректный адрес сайта".into()); }
        #[cfg(windows)]
        let spawned = Command::new("explorer.exe").arg(&command.target).spawn();
        #[cfg(not(windows))]
        let spawned = Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(&command.target).spawn();
        return spawned.map(|_| ()).map_err(|e| format!("Не удалось открыть сайт: {e}"));
    }
    let path = std::path::PathBuf::from(command.target.trim().trim_matches('"'));
    if !path.exists() { return Err(format!("Не найден файл: {}", path.display())); }
    let is_exe = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    let spawned = if is_exe || !cfg!(windows) && !cfg!(target_os = "macos") {
        let mut process = Command::new(&path);
        if let Some(dir) = path.parent() { process.current_dir(dir); }
        process.spawn()
    } else if cfg!(windows) {
        // Shortcuts (.lnk), .url and documents open with their associated program.
        Command::new("explorer.exe").arg(&path).spawn()
    } else {
        Command::new("open").arg(&path).spawn()
    };
    spawned.map(|_| ()).map_err(|e| format!("Не удалось запустить: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(phrases: &[&str], kind: &str, target: &str) -> CustomCommand {
        CustomCommand { id: "1".into(), phrases: phrases.iter().map(|p| p.to_string()).collect(), kind: kind.into(), target: target.into(), enabled: true }
    }

    #[test]
    fn matches_phrase_with_or_without_verb() {
        let list = [command(&["ютуб"], "site", "https://youtube.com"), command(&["открой телеграмм"], "app", "C:/t.exe")];
        assert_eq!(find(&list, "Джарвис, открой ютуб!").unwrap().kind, "site");
        assert_eq!(find(&list, "ютуб").unwrap().kind, "site");
        assert_eq!(find(&list, "запусти телеграмм").unwrap().kind, "app");
        assert_eq!(find(&list, "открой телеграм").unwrap().kind, "app");
        assert!(find(&list, "что такое ютуб").is_none());
        assert!(find(&list, "открой браузер").is_none());
    }

    #[test]
    fn disabled_commands_are_skipped() {
        let mut item = command(&["ютуб"], "site", "https://youtube.com");
        item.enabled = false;
        assert!(find(&[item], "ютуб").is_none());
    }

    #[test]
    fn validation_rejects_unsafe_targets() {
        let ok = serde_json::to_string(&[command(&["ютуб"], "site", "https://youtube.com/watch?v=1&t=2")]).unwrap();
        assert_eq!(parse(&ok).unwrap().len(), 1);
        for bad in [command(&["x1"], "site", "javascript:alert(1)"), command(&["сайт"], "site", "https://user:pass@site.ru"),
                    command(&["сайт"], "script", "C:/a.exe"), command(&[], "site", "https://a.ru"), command(&["сайт"], "app", " ")] {
            assert!(parse(&serde_json::to_string(&[bad]).unwrap()).is_err());
        }
        assert!(parse("").unwrap().is_empty());
    }
}
