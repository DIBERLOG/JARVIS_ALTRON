//! Compatibility adapter between the application and the ALTRON kernel.
//! Catalog loading, phrase selection and action planning live in the new crate.
use altron_command_engine::{
    catalog,
    dispatch::{self, Definition, Plan},
    matching::{self, Candidate, Selection},
    process,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Child,
};

pub mod center;
mod structs;
use crate::{config, i18n, APP_DIR};
pub use structs::*;
pub const KERNEL_VERSION: &str = altron_command_engine::VERSION;
#[cfg(feature = "lua")]
use crate::lua::{self, CommandContext, SandboxLevel};

pub fn parse_commands() -> Result<Vec<JCommandsList>, String> {
    let loaded = catalog::load::<JCommand>(&APP_DIR.join(config::COMMANDS_PATH))
        .map_err(|error| format!("ALTRON command catalog: {error}"))?;
    for warning in loaded.warnings {
        warn!("ALTRON command catalog: {warning}");
    }
    let mut packs: Vec<_> = loaded
        .packs
        .into_iter()
        .map(|pack| JCommandsList {
            path: pack.directory,
            commands: pack.definitions,
        })
        .collect();
    adapt_catalog(&mut packs);
    debug!("ALTRON kernel: loaded {} command packs", packs.len());
    Ok(packs)
}

fn adapt_catalog(packs: &mut Vec<JCommandsList>) {
    if cfg!(windows) { return; }
    for pack in packs.iter_mut() {
        pack.commands.retain_mut(|command| {
            if let Some((program, arguments)) = crate::platform::unix_command(&command.id, &command.exe_args, cfg!(target_os = "macos")) {
                command.cmd_type = "cli".into();
                command.cli_cmd = program;
                command.cli_args = arguments;
                return true;
            }
            let supported = !matches!(command.cmd_type.as_str(), "ahk" | "restart")
                && !matches!(command.cli_cmd.to_ascii_lowercase().as_str(), "calc" | "taskkill" | "cmd" | "powershell.exe" | "powershell" | "shutdown")
                && !command.cli_cmd.to_ascii_lowercase().ends_with(".exe");
            if !supported { debug!("Command {} is Windows-only; hidden on this platform", command.id); }
            supported
        });
    }
    packs.retain(|pack| !pack.commands.is_empty());
}

/// Retain the existing model-cache fingerprint during the migration.
pub fn commands_hash(commands: &[JCommandsList]) -> String {
    use sha2::{Digest, Sha256};
    let lang = i18n::get_language();
    let mut hasher = Sha256::new();
    hasher.update(lang.as_bytes());
    hasher.update(b"|");
    let mut entries: Vec<_> = commands
        .iter()
        .flat_map(|pack| pack.commands.iter())
        .map(|command| (command.id.as_str(), command.get_phrases(&lang)))
        .collect();
    entries.sort_by_key(|(id, _)| *id);
    for (id, phrases) in entries {
        hasher.update(id.as_bytes());
        for phrase in phrases.iter() {
            hasher.update(phrase.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

pub enum CommandSelection<'a> {
    Found(&'a PathBuf, &'a JCommand),
    Ambiguous(Vec<&'a str>),
    Missing,
}

fn resolve_in_language<'a>(
    phrase: &str,
    packs: &'a [JCommandsList],
    language: &str,
    fuzzy: bool,
) -> CommandSelection<'a> {
    let entries: Vec<_> = packs
        .iter()
        .flat_map(|pack| {
            pack.commands
                .iter()
                .map(move |command| (&pack.path, command))
        })
        .collect();
    let candidates: Vec<_> = entries
        .iter()
        .enumerate()
        .flat_map(|(key, (_, command))| {
            command
                .get_phrases(language)
                .iter()
                .map(|alias| Candidate {
                    key,
                    id: command.id.clone(),
                    phrase: alias.clone(),
                    allow_fuzzy: dispatch::allows_approximate(&command.id, &command.cmd_type),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    match matching::select(phrase, &candidates, fuzzy) {
        Selection::Found(key) => {
            let (path, command) = entries[key];
            CommandSelection::Found(path, command)
        }
        Selection::Ambiguous(keys) => CommandSelection::Ambiguous(
            keys.into_iter()
                .map(|key| entries[key].1.id.as_str())
                .collect(),
        ),
        Selection::Missing => CommandSelection::Missing,
    }
}

pub fn resolve_command<'a>(
    phrase: &str,
    packs: &'a [JCommandsList],
    fuzzy: bool,
) -> CommandSelection<'a> {
    resolve_in_language(phrase, packs, &i18n::get_language(), fuzzy)
}

pub fn fetch_command<'a>(
    phrase: &str,
    packs: &'a [JCommandsList],
) -> Option<(&'a PathBuf, &'a JCommand)> {
    match resolve_command(phrase, packs, true) {
        CommandSelection::Found(path, command) => Some((path, command)),
        _ => None,
    }
}

pub fn fetch_exact_command<'a>(
    phrase: &str,
    packs: &'a [JCommandsList],
) -> Option<(&'a PathBuf, &'a JCommand)> {
    match resolve_command(phrase, packs, false) {
        CommandSelection::Found(path, command) => Some((path, command)),
        _ => None,
    }
}

pub fn is_negated(phrase: &str) -> bool {
    matching::is_negated(phrase)
}

pub fn allow_intent_candidate(command: &JCommand) -> bool {
    dispatch::allows_approximate(&command.id, &command.cmd_type)
}

pub fn execute_exe(exe: &str, args: &[String]) -> std::io::Result<Child> {
    process::launch(Path::new(exe), args)
}

pub fn execute_cli(exe: &str, args: &[String]) -> std::io::Result<Child> {
    process::launch(Path::new(exe), args)
}

fn command_cli_arguments(pack: &Path, arguments: &[String]) -> Vec<String> {
    arguments.iter().map(|argument| argument.replace("{command_dir}", &pack.to_string_lossy())).collect()
}

pub fn execute_command(
    cmd_path: &PathBuf,
    command: &JCommand,
    phrase: Option<&str>,
    slots: Option<&HashMap<String, SlotValue>>,
) -> Result<bool, String> {
    // Explicit resource placeholder, not shell expansion: commands remain portable across installs.
    let cli_arguments = command_cli_arguments(cmd_path, &command.cli_args);
    let definition = Definition {
        kind: &command.cmd_type,
        executable: &command.exe_path,
        executable_arguments: &command.exe_args,
        cli: &command.cli_cmd,
        cli_arguments: &cli_arguments,
        script: &command.script,
    };
    let plan = dispatch::plan(cmd_path, &definition)?;
    info!(
        "ALTRON kernel: executing {} ({})",
        command.id, command.cmd_type
    );
    match plan {
        Plan::Acknowledge => Ok(true),
        Plan::EndChain => Ok(false),
        Plan::Launch { program, arguments } => process::launch(&program, &arguments)
            .map(|_| true)
            .map_err(|error| format!("Command process: {error}")),
        Plan::Restart {
            program,
            arguments,
            grace,
        } => {
            process::launch(&program, &arguments)
                .map_err(|error| format!("Restart helper: {error}"))?;
            std::thread::spawn(move || {
                std::thread::sleep(grace);
                std::process::exit(0);
            });
            Ok(false)
        }
        Plan::Exit { grace } => {
            std::thread::sleep(grace);
            std::process::exit(0);
        }
        Plan::Script(script_path) => {
            #[cfg(feature = "lua")]
            {
                let context = CommandContext {
                    phrase: phrase.unwrap_or_default().into(),
                    command_id: command.id.clone(),
                    command_path: cmd_path.clone(),
                    language: i18n::get_language(),
                    slots: slots.cloned(),
                };
                lua::execute(
                    &script_path,
                    context,
                    SandboxLevel::from_str(&command.sandbox),
                    std::time::Duration::from_millis(command.timeout),
                )
                .map(|result| result.chain)
                .map_err(|error| error.to_string())
            }
            #[cfg(not(feature = "lua"))]
            {
                let _ = (script_path, phrase, slots);
                Err("Lua backend is not enabled in this process".into())
            }
        }
    }
}

pub fn get_command_by_id<'a>(
    packs: &'a [JCommandsList],
    id: &str,
) -> Option<(&'a PathBuf, &'a JCommand)> {
    packs.iter().find_map(|pack| {
        pack.commands
            .iter()
            .find(|command| command.id == id)
            .map(|command| (&pack.path, command))
    })
}

pub fn list_paths(packs: &[JCommandsList]) -> Vec<&Path> {
    packs.iter().map(|pack| pack.path.as_path()).collect()
}

#[cfg(test)]
mod phrase_tests {
    use super::*;
    #[test]
    fn exact_phrases_ignore_punctuation_and_yo() {
        assert_eq!(matching::normalize("  Открой Дискорд! "), "открой дискорд");
        assert_eq!(matching::normalize("счётчик"), "счетчик");
    }
    #[test]
    fn name_templates_need_a_nonempty_name() {
        assert!(matching::template_matches(
            "поздоровайся с иваном",
            "поздоровайся с {name}"
        ));
        assert!(matching::template_matches("привет анна", "привет {name}"));
        assert!(!matching::template_matches("привет", "привет {name}"));
    }
    #[test]
    fn dialogue_examples_resolve_to_commands() {
        let manifests = [
            include_str!("../../../resources/commands/browser/command.toml"),
            include_str!("../../../resources/commands/discord/command.toml"),
            include_str!("../../../resources/commands/steam/command.toml"),
            include_str!("../../../resources/commands/weather/command.toml"),
        ];
        let packs: Vec<JCommandsList> = manifests
            .iter()
            .map(|manifest| toml::from_str(manifest).unwrap())
            .collect();
        for (phrase, expected) in [
            ("открой браузер", "browser_open"),
            ("открой дискорд", "discord_open"),
            ("открой стим", "game_mode"),
            ("открой расписание", "open_schedule"),
            ("погода в Москве", "weather"),
        ] {
            let CommandSelection::Found(_, command) =
                resolve_in_language(phrase, &packs, "ru", false)
            else {
                panic!("Unresolved: {phrase}");
            };
            assert_eq!(command.id, expected);
        }
    }
    #[test]
    fn real_catalog_preserves_all_unique_aliases() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let loaded = catalog::load::<JCommand>(&root).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let packs: Vec<_> = loaded
            .packs
            .into_iter()
            .map(|pack| JCommandsList {
                path: pack.directory,
                commands: pack.definitions,
            })
            .collect();
        let mut checked = 0;
        for language in ["ru", "en", "ua"] {
            let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
            for command in packs.iter().flat_map(|pack| &pack.commands) {
                for phrase in command
                    .get_phrases(language)
                    .iter()
                    .filter(|phrase| !phrase.contains('{'))
                {
                    let ids = aliases.entry(matching::request(phrase)).or_default();
                    if !ids.contains(&command.id) {
                        ids.push(command.id.clone());
                    }
                }
            }
            for (alias, ids) in aliases {
                if ids.len() == 1 {
                    let CommandSelection::Found(_, command) =
                        resolve_in_language(&alias, &packs, language, false)
                    else {
                        panic!("Unresolved {language}: {alias}");
                    };
                    assert_eq!(command.id, ids[0], "{language}: {alias}");
                    checked += 1;
                } else {
                    assert!(
                        matches!(
                            resolve_in_language(&alias, &packs, language, false),
                            CommandSelection::Ambiguous(_)
                        ),
                        "Conflicting alias: {alias}"
                    );
                }
            }
        }
        assert!(checked > 100, "Too few aliases tested: {checked}");
        println!("ALTRON compatibility: {checked} unique aliases checked in ru/en/ua");
    }
    #[test]
    fn manifest_execution_contracts_are_unchanged() {
        let manifest = include_str!("../../../resources/commands/windows/command.toml");
        let commands: JCommandsList = toml::from_str(manifest).unwrap();
        for command in &commands.commands {
            let definition = Definition {
                kind: &command.cmd_type,
                executable: &command.exe_path,
                executable_arguments: &command.exe_args,
                cli: &command.cli_cmd,
                cli_arguments: &command.cli_args,
                script: &command.script,
            };
            let Plan::Launch { program, arguments } =
                dispatch::plan(Path::new("."), &definition).unwrap()
            else {
                panic!("Expected launch plan");
            };
            assert_eq!(program, PathBuf::from("powershell.exe"));
            assert_eq!(arguments, command.cli_args);
        }
        // Only constructing plans: this test never minimizes real windows.
    }
    #[test]
    fn windows_restore_aliases_and_resource_path() {
        let pack: JCommandsList = toml::from_str(include_str!("../../../resources/commands/windows/command.toml")).unwrap();
        let packs=vec![pack];
        for phrase in ["верни все окна", "разверни все окна", "восстанови все окна", "разверни окна обратно"] {
            let CommandSelection::Found(_, command)=resolve_in_language(phrase,&packs,"ru",false) else { panic!("Missing restore alias: {phrase}") };
            assert_eq!(command.id,"windows_restore_all");
        }
        let arguments=command_cli_arguments(Path::new("C:/Folder With Spaces/windows"), &["{command_dir}/windows.ps1".into(),"a & b".into()]);
        assert_eq!(arguments,vec!["C:/Folder With Spaces/windows/windows.ps1","a & b"]);
    }
}

/// Commands exposed to paired devices: fixed catalog IDs without user parameters.
pub fn remote_allowed(id: &str) -> bool {
    matches!(id, "browser_open" | "open_youtube" | "open_chatgpt" | "open_google" | "calculator_open" | "telegram_open" | "discord_open" | "vscode_open" | "windows_minimize_all" | "windows_restore_all" | "center_open" | "center_news_open" | "center_calendar" | "center_schedule" | "center_notes" | "center_timer_open" | "center_reminders" | "center_birthdays" | "center_weather" | "center_habits" | "center_training" | "center_stats" | "center_history")
}
