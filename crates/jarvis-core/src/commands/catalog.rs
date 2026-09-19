//! The catalogue the «Команды» page is built from.
//!
//! It reads the same packs the voice host loads, with the same parser
//! ([`parse_command_document`]), and turns each command into a card: an
//! identifier, the pack it came from, a category, the phrases a person can say,
//! the slots the command needs, the risk level the safety gate will read, whether
//! it needs a spoken confirmation, and — when it cannot run — why.
//!
//! Two rules decide the shape of everything here:
//!
//! * **a path never crosses this boundary.** The pack's directory, the
//!   executable, the script and the arguments are read to answer "can this run",
//!   and are not part of the answer. A pack is named by its logical name, which is
//!   the name of its directory and nothing more.
//! * **a pack the loader cannot read is listed, not hidden.** The voice host reads
//!   `command.toml` only, so a pack that ships `command.yaml` is invisible to it.
//!   The catalogue says so, with a reason code and the logical name, instead of
//!   leaving a person to wonder why a phrase does nothing.

use std::fs;
use std::path::Path;

use serde::Serialize;

use super::{parse_command_document, JCommand};
use crate::safety::RiskLevel;
use crate::{config, APP_DIR};

/// The categories the filter offers, in the order the page shows them.
pub const CATEGORIES: [&str; 8] = [
    "applications",
    "sound",
    "windows",
    "screenshots",
    "timers",
    "system",
    "weather",
    "global_voice_input",
];

/// The category of a pack.
///
/// A pack is the unit the loader has: one directory, one document, one subject.
/// The mapping is a table and not a guess — an unknown pack is `system`, and a
/// pack that is not about anything in the table keeps its place there rather than
/// being sorted by the words of its phrases.
pub fn category_of(pack: &str) -> &'static str {
    match pack {
        "browser" | "calculator" | "steam" | "applications" => "applications",
        "volume" | "sound" => "sound",
        "windows" | "window" => "windows",
        "screenshot" | "screenshots" => "screenshots",
        "timer" | "timers" => "timers",
        "weather" => "weather",
        "dictation" | "voice_input" | "global_voice_input" => "global_voice_input",
        _ => "system",
    }
}

/// A slot a command declares, by name and by the entity it expects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogSlot {
    pub name: String,
    pub entity: String,
}

/// One card on the page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogEntry {
    /// The identifier the matcher answers with.
    pub id: String,
    /// The logical name of the pack: the name of its directory, never a path.
    pub pack: String,
    /// One of [`CATEGORIES`].
    pub category: String,
    /// `pack` for a command the loader read, `settings` for one the person
    /// configured elsewhere.
    pub source: String,
    /// The description the pack carries, in the language of the page.
    pub description: String,
    /// The phrases a person can say, as the pack writes them. A phrase that
    /// carries a slot keeps its placeholder, because that is what the pack says.
    pub phrases: Vec<String>,
    /// The slots the command needs, by name.
    pub slots: Vec<CatalogSlot>,
    /// `safe`, `confirm` or `forbidden`, from [`crate::safety::RiskLevel`].
    pub risk_level: String,
    /// Whether the safety gate will ask for a spoken confirmation.
    pub requires_confirmation: bool,
    /// Whether the command can run as it is.
    pub enabled: bool,
    /// The one word for the card: `ready`, `configuration_required`, `disabled`,
    /// `forbidden`, `executor_missing`, `dependency_missing`.
    pub status: String,
    /// Why it cannot, as a stable code: `no_phrases`, `allowlist_required`,
    /// `executable_missing`, `script_missing`, `unsupported_type`,
    /// `disabled_in_settings`, `forbidden_by_policy`.
    pub unavailable_reason: Option<String>,
}

/// A pack the loader could not read, with its logical name and nothing else.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnreadablePack {
    /// The name of the directory, never a path.
    pub pack: String,
    /// `parse_failed`, `unsupported_format`, `missing_document`, `unreadable`.
    pub reason: String,
}

/// Everything the page needs, in one answer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CommandCatalog {
    pub language: String,
    /// The filter's vocabulary, always all of it: a category with no card yet is
    /// an honest empty state, not an absent category.
    pub categories: Vec<String>,
    pub entries: Vec<CatalogEntry>,
    pub unreadable: Vec<UnreadablePack>,
}

impl CommandCatalog {
    /// The entries of one category.
    pub fn in_category(&self, category: &str) -> Vec<&CatalogEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.category == category)
            .collect()
    }
}

/// The catalogue of the installed packs, for the language the page is shown in.
pub fn load_catalog(language: &str) -> CommandCatalog {
    build_catalog(&APP_DIR.join(config::COMMANDS_PATH), language)
}

/// The same, for a directory given instead of the installed one, so a test can
/// lay out packs it controls.
pub fn build_catalog(directory: &Path, language: &str) -> CommandCatalog {
    let mut entries = Vec::new();
    let mut unreadable = Vec::new();

    let mut packs: Vec<fs::DirEntry> = match fs::read_dir(directory) {
        Ok(read) => read.flatten().collect(),
        Err(_) => Vec::new(),
    };
    packs.sort_by_key(|entry| entry.file_name());

    for pack in packs {
        let path = pack.path();
        if !path.is_dir() {
            continue;
        }
        let pack_name = pack.file_name().to_string_lossy().to_string();
        let document = path.join("command.toml");
        if !document.is_file() {
            // The loader reads `command.toml`. A pack in another format is not
            // missing and not broken: it is simply not read, and the page says so.
            let reason = if path.join("command.yaml").is_file() {
                "unsupported_format"
            } else {
                "missing_document"
            };
            unreadable.push(UnreadablePack {
                pack: pack_name,
                reason: reason.to_string(),
            });
            continue;
        }
        match fs::read_to_string(&document) {
            Ok(text) => match parse_command_document(&text) {
                Ok(list) => {
                    for command in list.commands {
                        entries.push(entry_of(&pack_name, &path, &command, language));
                    }
                }
                // The error text can quote the document; only a code is kept.
                Err(_) => unreadable.push(UnreadablePack {
                    pack: pack_name,
                    reason: "parse_failed".to_string(),
                }),
            },
            Err(_) => unreadable.push(UnreadablePack {
                pack: pack_name,
                reason: "unreadable".to_string(),
            }),
        }
    }

    entries.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then(left.pack.cmp(&right.pack))
            .then(left.id.cmp(&right.id))
    });

    CommandCatalog {
        language: language.to_string(),
        categories: CATEGORIES.iter().map(|key| key.to_string()).collect(),
        entries,
        unreadable,
    }
}

/// Turns one parsed command into a card.
pub fn entry_of(pack: &str, pack_path: &Path, command: &JCommand, language: &str) -> CatalogEntry {
    let phrases = phrases_for(command, language);
    let (available, reason) = availability(pack_path, command, &phrases);
    // A forbidden command never reaches an executor, whatever it names: the card
    // says so instead of showing an executor that will not be used.
    let (enabled, unavailable_reason) = if command.risk_level == RiskLevel::Forbidden {
        (false, Some("forbidden_by_policy".to_string()))
    } else {
        (available, reason)
    };
    let status = status_of(command, enabled, unavailable_reason.as_deref());

    let mut slots: Vec<CatalogSlot> = command
        .slots
        .iter()
        .map(|(name, definition)| CatalogSlot {
            name: name.clone(),
            entity: definition.entity.clone(),
        })
        .collect();
    slots.sort_by(|left, right| left.name.cmp(&right.name));

    CatalogEntry {
        id: command.id.clone(),
        pack: pack.to_string(),
        category: category_of(pack).to_string(),
        source: "pack".to_string(),
        description: command.description.clone(),
        phrases,
        slots,
        risk_level: command.risk_level.as_str().to_string(),
        requires_confirmation: command.risk_level == RiskLevel::ConfirmationRequired,
        enabled,
        status,
        unavailable_reason,
    }
}

/// The one word a card shows, derived from the risk and from what is missing.
///
/// MATCHED and EXECUTABLE are separate questions and stay separate here: a command
/// whose phrase is listed and whose executor is missing is *not* ready, and it is
/// not "unknown" either — it says which of the two is missing.
fn status_of(command: &JCommand, enabled: bool, reason: Option<&str>) -> String {
    if command.risk_level == RiskLevel::Forbidden {
        return "forbidden".to_string();
    }
    if enabled {
        return "ready".to_string();
    }
    match reason {
        Some("executable_missing") | Some("unsupported_type") => "executor_missing".to_string(),
        Some("script_missing") => "dependency_missing".to_string(),
        Some("allowlist_required") => "configuration_required".to_string(),
        _ => "disabled".to_string(),
    }
}

/// The global voice input as the catalogue sees it.
///
/// It is not a pack and it is not executed by the command list: the person
/// configures the phrase in the settings, the listener recognises it, and the
/// dictation runs in the window process. It is listed here so the page answers
/// "what can I say" completely, and it is marked as coming from the settings so
/// nothing suggests it lives in a pack.
pub fn global_voice_input_entry(phrase: &str, enabled: bool) -> CatalogEntry {
    let normalized = super::normalize_phrase(phrase);
    let has_phrase = !normalized.is_empty();
    let unavailable_reason = if !has_phrase {
        Some("no_phrases".to_string())
    } else if !enabled {
        Some("disabled_in_settings".to_string())
    } else {
        None
    };
    CatalogEntry {
        id: "global_voice_input".to_string(),
        pack: "settings".to_string(),
        category: "global_voice_input".to_string(),
        source: "settings".to_string(),
        description: String::new(),
        phrases: if has_phrase {
            vec![normalized]
        } else {
            Vec::new()
        },
        slots: Vec::new(),
        risk_level: RiskLevel::Safe.as_str().to_string(),
        requires_confirmation: false,
        enabled: enabled && has_phrase,
        status: if enabled && has_phrase {
            "ready".to_string()
        } else {
            "disabled".to_string()
        },
        unavailable_reason,
    }
}

/// The phrases of a command, in the language of the page and with the same
/// fallback the pack uses, because a pack that has no phrases for a language is
/// still a command a person may say in another one.
fn phrases_for(command: &JCommand, language: &str) -> Vec<String> {
    let mut phrases = command.get_phrases(language).to_vec();
    if phrases.is_empty() {
        phrases = command.get_phrases("ru").to_vec();
    }
    if phrases.is_empty() {
        phrases = command.get_phrases("en").to_vec();
    }
    phrases
}

/// Whether a command can run as it is, and why not when it cannot.
fn availability(
    pack_path: &Path,
    command: &JCommand,
    phrases: &[String],
) -> (bool, Option<String>) {
    // A command nobody can say is unreachable by voice whatever its files are:
    // that is the first thing worth reporting, not the last.
    if phrases.is_empty() {
        return (false, Some("no_phrases".to_string()));
    }
    match command.cmd_type.as_str() {
        // A typed action goes through the pipeline this build already runs, so the
        // only question is whether it needs something the user has to configure.
        "native" => match &command.native {
            Some(action) => {
                if action.needs_allowed_application() {
                    // The pack asks for a role; the user's allowlist answers it. Until
                    // an entry matches, the command is not ready and says so.
                    (false, Some("allowlist_required".to_string()))
                } else {
                    (true, None)
                }
            }
            None => (false, Some("unsupported_type".to_string())),
        },
        // A typed event of the application needs nothing but the host.
        "internal" => match command.internal {
            Some(_) => (true, None),
            None => (false, Some("unsupported_type".to_string())),
        },
        "ahk" => {
            if resolves(pack_path, &command.exe_path) {
                (true, None)
            } else {
                (false, Some("executable_missing".to_string()))
            }
        }
        "lua" => {
            let script = if command.script.trim().is_empty() {
                "script.lua"
            } else {
                command.script.as_str()
            };
            if resolves(pack_path, script) {
                (true, None)
            } else {
                (false, Some("script_missing".to_string()))
            }
        }
        "cli" => {
            if command.cli_cmd.trim().is_empty() {
                (false, Some("executable_missing".to_string()))
            } else {
                (true, None)
            }
        }
        // A phrase the listener answers with a sound, the end of a chain, and the
        // terminator need no file at all.
        "voice" | "terminate" | "stop_chaining" => (true, None),
        _ => (false, Some("unsupported_type".to_string())),
    }
}

/// Whether the file a command names is where the command names it. The executor
/// tries the path as written and then next to the pack, and so does this, so the
/// page and the runner agree.
fn resolves(pack_path: &Path, named: &str) -> bool {
    if named.trim().is_empty() {
        return false;
    }
    let candidate = Path::new(named);
    if candidate.is_absolute() {
        candidate.is_file()
    } else {
        pack_path.join(candidate).is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> CommandCatalog {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("resources")
            .join("commands");
        build_catalog(&directory, "ru")
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("jarvis-catalog-{name}"));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("a fixture directory");
        directory
    }

    #[test]
    fn the_filter_is_the_eight_categories() {
        let catalog = installed();
        assert_eq!(catalog.categories.len(), 8);
        for required in [
            "applications",
            "sound",
            "windows",
            "screenshots",
            "timers",
            "system",
            "weather",
            "global_voice_input",
        ] {
            assert!(
                catalog.categories.iter().any(|key| key == required),
                "the filter must offer {required}"
            );
        }
        let mut unique = catalog.categories.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 8, "a category is listed twice");
        assert_eq!(category_of("volume"), "sound");
        assert_eq!(category_of("weather"), "weather");
        assert_eq!(category_of("browser"), "applications");
        assert_eq!(category_of("something-new"), "system");
    }

    #[test]
    fn every_installed_command_is_a_card_with_a_category_and_no_path() {
        let catalog = installed();
        assert!(
            catalog.entries.len() >= 5,
            "the installed packs must produce cards, got {}",
            catalog.entries.len()
        );
        for entry in &catalog.entries {
            assert!(
                CATEGORIES.contains(&entry.category.as_str()),
                "{} has an unknown category {}",
                entry.id,
                entry.category
            );
            assert!(!entry.id.is_empty(), "a card needs an identifier");
            assert_eq!(entry.source, "pack");
            assert!(
                !entry.pack.contains('/') && !entry.pack.contains('\\'),
                "a pack is named by its logical name, not by {}",
                entry.pack
            );
            assert!(
                ["safe", "confirm", "forbidden"].contains(&entry.risk_level.as_str()),
                "{} has an unknown risk level {}",
                entry.id,
                entry.risk_level
            );
        }
        // The weather command is there with its slot and its placeholder, which is
        // exactly the shape that used to be unreachable by voice.
        let weather = catalog
            .entries
            .iter()
            .find(|entry| entry.id == "weather")
            .expect("the weather command");
        assert_eq!(weather.category, "weather");
        assert!(weather.slots.iter().any(|slot| slot.name == "city"));
        assert!(weather
            .phrases
            .iter()
            .any(|phrase| phrase.contains("{city}")));
    }

    #[test]
    fn a_pack_the_loader_cannot_read_is_listed_with_a_reason_and_no_path() {
        let directory = fixture("unreadable");
        fs::create_dir_all(directory.join("good")).expect("a pack");
        fs::write(
            directory.join("good").join("command.toml"),
            "[[commands]]\nid = \"fine\"\ntype = \"voice\"\n\n[commands.phrases]\nru = [\"привет\"]\n",
        )
        .expect("a document");
        fs::create_dir_all(directory.join("legacy")).expect("a pack");
        fs::write(directory.join("legacy").join("command.yaml"), "list: []").expect("a document");
        fs::create_dir_all(directory.join("broken")).expect("a pack");
        fs::write(
            directory.join("broken").join("command.toml"),
            "this is not a command document",
        )
        .expect("a document");
        fs::create_dir_all(directory.join("empty")).expect("a pack");

        let catalog = build_catalog(&directory, "ru");
        assert_eq!(catalog.entries.len(), 1, "only the readable pack has cards");
        assert_eq!(catalog.entries[0].id, "fine");

        let reason = |pack: &str| {
            catalog
                .unreadable
                .iter()
                .find(|entry| entry.pack == pack)
                .map(|entry| entry.reason.clone())
        };
        assert_eq!(reason("legacy").as_deref(), Some("unsupported_format"));
        assert_eq!(reason("broken").as_deref(), Some("parse_failed"));
        assert_eq!(reason("empty").as_deref(), Some("missing_document"));
        for entry in &catalog.unreadable {
            assert!(
                !entry.pack.contains('/') && !entry.pack.contains('\\'),
                "an unreadable pack is named by its logical name"
            );
        }
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_command_that_cannot_run_says_why() {
        let directory = fixture("unavailable");
        let pack = directory.join("browser");
        fs::create_dir_all(&pack).expect("a pack");

        let mut command = JCommand::for_test("browser_open", "ahk");
        command.exe_path = "ahk/Run browser.exe".to_string();
        command
            .phrases
            .insert("ru".to_string(), vec!["открой браузер".to_string()]);

        let entry = entry_of("browser", &pack, &command, "ru");
        assert!(!entry.enabled);
        assert_eq!(
            entry.unavailable_reason.as_deref(),
            Some("executable_missing")
        );

        // The same command with its file where the pack says it is.
        fs::create_dir_all(pack.join("ahk")).expect("the folder");
        fs::write(pack.join("ahk").join("Run browser.exe"), b"stub").expect("the file");
        let entry = entry_of("browser", &pack, &command, "ru");
        assert!(entry.enabled);
        assert_eq!(entry.unavailable_reason, None);

        // A command nobody can say is unreachable before its files are even asked
        // about. A second command is built for this, because `get_phrases` caches
        // what it has already resolved.
        let mut silent = JCommand::for_test("browser_open", "ahk");
        silent.exe_path = "ahk/Run browser.exe".to_string();
        let entry = entry_of("browser", &pack, &silent, "ru");
        assert!(!entry.enabled);
        assert_eq!(entry.unavailable_reason.as_deref(), Some("no_phrases"));

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_risky_command_is_marked_as_needing_confirmation() {
        let directory = fixture("risk");
        let pack = directory.join("browser");
        fs::create_dir_all(&pack).expect("a pack");
        let mut command = JCommand::for_test("browser_close", "voice");
        command.risk_level = RiskLevel::ConfirmationRequired;
        command
            .phrases
            .insert("ru".to_string(), vec!["закрой браузер".to_string()]);
        let entry = entry_of("browser", &pack, &command, "ru");
        assert_eq!(entry.risk_level, "confirm");
        assert!(entry.requires_confirmation);
        assert!(entry.enabled, "a phrase command needs no file");

        command.risk_level = RiskLevel::Forbidden;
        let entry = entry_of("browser", &pack, &command, "ru");
        assert_eq!(entry.risk_level, "forbidden");
        // Forbidden is the gate's business, not a confirmation: it never runs.
        assert!(!entry.requires_confirmation);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_card_says_which_question_is_unanswered() {
        // MATCHED and EXECUTABLE are separate. Every installed command is a card
        // with a status from the closed set, and "ready" is never claimed for a
        // command that has no executor or that the policy refuses.
        const STATUSES: [&str; 6] = [
            "ready",
            "configuration_required",
            "disabled",
            "forbidden",
            "executor_missing",
            "dependency_missing",
        ];
        let catalog = installed();
        for entry in &catalog.entries {
            assert!(
                STATUSES.contains(&entry.status.as_str()),
                "{} has an unknown status {}",
                entry.id,
                entry.status
            );
            if entry.status == "ready" {
                assert!(entry.enabled, "{} claims ready while disabled", entry.id);
                assert!(
                    entry.unavailable_reason.is_none(),
                    "{} claims ready with a reason",
                    entry.id
                );
            } else {
                assert!(
                    entry.unavailable_reason.is_some(),
                    "{} is not ready and does not say why",
                    entry.id
                );
            }
            if entry.risk_level == "forbidden" {
                assert_eq!(entry.status, "forbidden", "{}", entry.id);
                assert!(!entry.enabled);
            }
        }

        let status = |id: &str| {
            catalog
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap_or_else(|| panic!("{id} must be installed"))
                .status
                .clone()
        };
        // Typed actions that need nothing from the user are ready.
        for id in [
            "volume_get",
            "volume_mute",
            "volume_unmute",
            "volume_min",
            "volume_mid",
            "volume_max",
            "windows_screenshot",
            "windows_lock",
            "windows_list",
            "stop_listening",
        ] {
            assert_eq!(status(id), "ready", "{id}");
        }
        // A launch waits for the user's allowlist, and the card says so.
        for id in [
            "browser_open",
            "calculator_open",
            "steam_open",
            "windows_task_manager",
        ] {
            assert_eq!(status(id), "configuration_required", "{id}");
        }
        // The policy refuses these, whichever executor they name.
        assert_eq!(status("jarvis_reboot"), "forbidden");
        assert_eq!(status("calculator_close"), "forbidden");
        assert_eq!(status("browser_close"), "forbidden");
        // A command whose compiled helper is not in the repository is not ready.
        for id in [
            "open_google",
            "steam_close",
            "windows_empty_trash",
            "windows_sleep",
            "windows_clipboard",
            "windows_keyboard_layout",
        ] {
            assert_eq!(status(id), "executor_missing", "{id}");
        }
        // The terminator asks for a confirmation, and is still usable.
        let terminate = catalog
            .entries
            .iter()
            .find(|entry| entry.id == "terminate")
            .expect("the terminator");
        assert_eq!(terminate.status, "ready");
        assert!(terminate.requires_confirmation);
    }

    #[test]
    fn a_command_that_cannot_run_is_described_and_never_guessed_at() {
        // A native launch is configuration, not a broken command.
        let mut launch = JCommand::for_test("browser_open", "native");
        launch.native = Some(crate::commands::NativeAction::LaunchApplication {
            role: "browser".to_string(),
        });
        launch
            .phrases
            .insert("ru".to_string(), vec!["открой браузер".to_string()]);
        let directory = fixture("status");
        let entry = entry_of("browser", &directory, &launch, "ru");
        assert_eq!(entry.status, "configuration_required");
        assert_eq!(
            entry.unavailable_reason.as_deref(),
            Some("allowlist_required")
        );

        // The same shape with an action that needs nothing is ready.
        launch.native = Some(crate::commands::NativeAction::GetVolume);
        let entry = entry_of("browser", &directory, &launch, "ru");
        assert_eq!(entry.status, "ready");
        assert!(entry.enabled);

        // A native command with no action at all is not ready and never runs.
        launch.native = None;
        let entry = entry_of("browser", &directory, &launch, "ru");
        assert_eq!(entry.status, "executor_missing");
        assert_eq!(
            entry.unavailable_reason.as_deref(),
            Some("unsupported_type")
        );

        // An internal command needs nothing but the host.
        let mut internal = JCommand::for_test("stop_listening", "internal");
        internal.internal = Some(crate::commands::InternalEvent::StopChaining);
        internal
            .phrases
            .insert("ru".to_string(), vec!["хватит".to_string()]);
        let entry = entry_of("stop", &directory, &internal, "ru");
        assert_eq!(entry.status, "ready");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_global_voice_input_is_a_card_from_the_settings() {
        let entry = global_voice_input_entry("Джарвис, продиктую текст", true);
        assert_eq!(entry.category, "global_voice_input");
        assert_eq!(entry.source, "settings");
        assert_eq!(entry.pack, "settings");
        assert!(entry.enabled);
        assert_eq!(entry.status, "ready");
        assert_eq!(entry.phrases, vec!["продиктую текст".to_string()]);

        let disabled = global_voice_input_entry("Джарвис, продиктую текст", false);
        assert!(!disabled.enabled);
        assert_eq!(disabled.status, "disabled");
        assert_eq!(
            disabled.unavailable_reason.as_deref(),
            Some("disabled_in_settings")
        );

        let empty = global_voice_input_entry("   ", true);
        assert!(!empty.enabled);
        assert_eq!(empty.unavailable_reason.as_deref(), Some("no_phrases"));
    }
}
