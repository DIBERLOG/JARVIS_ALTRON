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
    /// Whether the command can run once its requirements are met.
    pub enabled: bool,
    /// The one status a person sees, and the only one: `ready`,
    /// `confirmation_required`, `allowlist_required`, `forbidden`,
    /// `executor_missing` or `disabled`. The six are mutually exclusive, every
    /// installed command has exactly one of them, and they add up to the total.
    pub status: String,
    /// Why it cannot, as a stable code: `no_phrases`, `allowlist_required`,
    /// `executable_missing`, `script_missing`, `unsupported_type`,
    /// `disabled_in_settings`, `forbidden_by_policy`.
    pub unavailable_reason: Option<String>,
    /// Whether a person can reach it at all: it has phrases in this language.
    pub recognized: bool,
    /// Whether the executor exists and is usable — the second question, answered
    /// separately from the first.
    pub executor_ready: bool,
    /// Whether the policy permits it.
    pub allowed: bool,
    /// Whether both halves the automatic suite asserts hold for it: a phrase
    /// reaches this command, and its executor exists. A pack cannot be installed
    /// with a phrase that does not match, because the suite walks every installed
    /// pack.
    pub verified: bool,
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

    // The four questions, each answered on its own and in its own terms:
    //
    // * `recognized`  — does a phrase in this language reach this command?
    // * `executor_ready` — does the executor this command names exist in this build?
    //   This is a fact about the build and the files next to the pack. The policy is
    //   not consulted: a forbidden command whose executable is present reports the
    //   executable as present, because that is the truth.
    // * `allowed` — does the policy permit it?
    // * `verified` — all of the above hold and nothing is waiting for configuration,
    //   which is exactly `enabled`.
    let recognized = !phrases.is_empty();
    let (executor_ready, executor_reason) = executor_state(pack_path, command);
    let allowed = command.risk_level != RiskLevel::Forbidden;
    let configuration = needs_configuration(command);

    let (enabled, unavailable_reason) = if !allowed {
        (false, Some("forbidden_by_policy".to_string()))
    } else if !recognized {
        (false, Some("no_phrases".to_string()))
    } else if !executor_ready {
        (
            false,
            Some(executor_reason.unwrap_or("unsupported_type").to_string()),
        )
    } else if let Some(code) = configuration {
        (false, Some(code.to_string()))
    } else {
        (true, None)
    };
    let status = status_of(command, enabled, unavailable_reason.as_deref());
    let verified = enabled && recognized;

    debug_assert_eq!(
        status == "ready" || status == "confirmation_required",
        enabled,
        "`{}` must be usable exactly when it is ready or waiting for a confirmation",
        command.id
    );

    let mut slots: Vec<CatalogSlot> = command
        .slots
        .iter()
        .map(|(name, definition)| CatalogSlot {
            name: name.clone(),
            entity: definition.entity.clone(),
        })
        .collect();
    slots.sort_by(|left, right| left.name.cmp(&right.name));

    // The four questions, each answered on its own. `recognized` is about the
    // matcher, `executor_ready` about this build, `allowed` about the policy, and
    // `verified` claims only that the command would really run.
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
        recognized,
        executor_ready,
        allowed,
        verified,
        unavailable_reason,
    }
}

/// The one status a person sees.
///
/// The six are mutually exclusive, and the order below is the precedence — the
/// first question that has an answer wins:
///
/// 1. **forbidden** — the policy refuses it. It is named first because it is the
///    one answer that does not change when the files change; a command that is both
///    forbidden and missing its helper is reported as forbidden, and the card still
///    shows the executor as missing in its own indicator. This is the one
///    documented overlap, and it is deliberate: "why does this not work" has two
///    true answers there, and hiding one of them would be worse.
/// 2. **disabled** — nobody can say it: no phrases in this language.
/// 3. **executor_missing** — the executor does not exist in this build (a compiled
///    helper that is not there, a script that is not there, an action this build
///    does not have).
/// 4. **allowlist_required** — the executor exists, and the user has to allow the
///    application it should start.
/// 5. **confirmation_required** — it runs, after a spoken confirmation.
/// 6. **ready** — it runs.
fn status_of(command: &JCommand, enabled: bool, reason: Option<&str>) -> String {
    if command.risk_level == RiskLevel::Forbidden {
        return "forbidden".to_string();
    }
    match reason {
        Some("no_phrases") | Some("disabled_in_settings") => "disabled".to_string(),
        Some("executable_missing") | Some("script_missing") | Some("unsupported_type") => {
            "executor_missing".to_string()
        }
        Some("allowlist_required") => "allowlist_required".to_string(),
        _ if command.risk_level == RiskLevel::ConfirmationRequired => {
            "confirmation_required".to_string()
        }
        _ if enabled => "ready".to_string(),
        // Nothing above answered: the command is not usable and the catalogue does
        // not know why, which must never be shown as "ready".
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
        recognized: has_phrase,
        // The listener recognises the phrase and the dictation engine runs in the
        // window process: there is nothing else to be ready.
        executor_ready: true,
        allowed: true,
        verified: enabled && has_phrase,
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

/// Whether the executor a command names exists in this build, and which part of it
/// is missing when it does not.
///
/// This is a fact about the build and the files next to the pack, and it is
/// deliberately blind to the policy and to the user's configuration: a forbidden
/// command whose executable is present reports the executable as present, and a
/// launch that waits for the allowlist reports its executor as present too. Mixing
/// those answers into one is exactly what makes a page lie about *why* a command
/// does nothing.
fn executor_state(pack_path: &Path, command: &JCommand) -> (bool, Option<&'static str>) {
    match command.cmd_type.as_str() {
        // A typed action is executed by this build; the action itself is the executor.
        "native" => match command.native {
            Some(_) => (true, None),
            None => (false, Some("unsupported_type")),
        },
        // A typed event of the application needs nothing but the host.
        "internal" => match command.internal {
            Some(_) => (true, None),
            None => (false, Some("unsupported_type")),
        },
        "ahk" => {
            if resolves(pack_path, &command.exe_path) {
                (true, None)
            } else {
                (false, Some("executable_missing"))
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
                (false, Some("script_missing"))
            }
        }
        "cli" => {
            // The executable is resolved by the operating system at launch, so its
            // presence cannot be checked from here; an empty name is the one thing
            // that is certainly not an executor.
            if command.cli_cmd.trim().is_empty() {
                (false, Some("executable_missing"))
            } else {
                (true, None)
            }
        }
        // A phrase the listener answers with a sound, the end of a chain, and the
        // terminator need no file at all.
        "voice" | "terminate" | "stop_chaining" => (true, None),
        _ => (false, Some("unsupported_type")),
    }
}

/// What the user still has to configure before this command can run, if anything.
///
/// It is a separate question from "does the executor exist": the launch pipeline is
/// there, and it will start the file the user allowed — once the user has allowed
/// one.
fn needs_configuration(command: &JCommand) -> Option<&'static str> {
    match &command.native {
        Some(action) if action.needs_allowed_application() => Some("allowlist_required"),
        _ => None,
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
    fn the_statuses_are_mutually_exclusive_and_add_up_to_the_total() {
        // One command, one status. The six add up to the number of commands, so the
        // page can never show a count that does not match what it lists.
        const STATUSES: [&str; 6] = [
            "ready",
            "confirmation_required",
            "allowlist_required",
            "forbidden",
            "executor_missing",
            "disabled",
        ];
        let catalog = installed();
        let total = catalog.entries.len();
        assert_eq!(total, 32, "the installed packs declare 32 commands");

        let mut counts = std::collections::BTreeMap::new();
        for status in STATUSES {
            counts.insert(status, 0usize);
        }
        for entry in &catalog.entries {
            let count = counts.get_mut(entry.status.as_str()).unwrap_or_else(|| {
                panic!(
                    "`{}` has a status outside the six: {}",
                    entry.id, entry.status
                )
            });
            *count += 1;
        }
        let sum: usize = counts.values().sum();
        assert_eq!(sum, total, "the statuses must add up to the total");

        // The exact picture of the installed packs, so a change to a pack or to the
        // rules has to be a deliberate one.
        assert_eq!(counts["ready"], 17, "ready: {counts:?}");
        assert_eq!(counts["confirmation_required"], 4, "{counts:?}");
        assert_eq!(counts["allowlist_required"], 4, "{counts:?}");
        assert_eq!(counts["forbidden"], 0, "{counts:?}");
        assert_eq!(counts["executor_missing"], 7, "{counts:?}");
        assert_eq!(counts["disabled"], 0, "{counts:?}");

        // A command that is usable exactly when it is ready or asking for a
        // confirmation, and never otherwise.
        for entry in &catalog.entries {
            let usable = entry.status == "ready" || entry.status == "confirmation_required";
            assert_eq!(usable, entry.enabled, "{}", entry.id);
            if entry.status == "confirmation_required" {
                assert!(entry.requires_confirmation, "{}", entry.id);
            }
            if matches!(
                entry.status.as_str(),
                "forbidden" | "allowlist_required" | "executor_missing" | "disabled"
            ) {
                assert!(
                    entry.unavailable_reason.is_some(),
                    "{} is not usable and must say why",
                    entry.id
                );
                assert!(!entry.enabled, "{}", entry.id);
            } else {
                assert!(
                    entry.unavailable_reason.is_none(),
                    "{} is usable and has nothing to explain",
                    entry.id
                );
            }
        }
    }

    #[test]
    fn the_three_system_commands_now_ask_instead_of_refusing() {
        // The three commands that used to be forbidden are executable and ask for a
        // spoken confirmation: reboot, and closing the calculator's and the
        // browser's windows the graceful way. Nothing kills a process.
        let catalog = installed();
        let find = |id: &str| {
            catalog
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap_or_else(|| panic!("{id} must be installed"))
        };
        for id in ["jarvis_reboot", "calculator_close", "browser_close"] {
            let entry = find(id);
            assert_eq!(entry.status, "confirmation_required", "{id}");
            assert!(entry.enabled, "{id}");
            assert!(entry.requires_confirmation, "{id}");
            assert!(entry.allowed, "{id}");
            assert!(entry.executor_ready, "{id}");
            assert!(entry.verified, "{id}");
            assert_eq!(entry.unavailable_reason, None, "{id}");
        }
    }

    #[test]
    fn a_card_says_which_question_is_unanswered() {
        // MATCHED and EXECUTABLE are separate, and each installed command is pinned
        // to the one status it must show.
        const STATUSES: [&str; 6] = [
            "ready",
            "confirmation_required",
            "allowlist_required",
            "forbidden",
            "executor_missing",
            "disabled",
        ];
        let catalog = installed();
        for entry in &catalog.entries {
            assert!(
                STATUSES.contains(&entry.status.as_str()),
                "{} has an unknown status {}",
                entry.id,
                entry.status
            );
            let usable = entry.status == "ready" || entry.status == "confirmation_required";
            assert_eq!(usable, entry.enabled, "{}", entry.id);
            if !usable {
                assert!(
                    entry.unavailable_reason.is_some(),
                    "{} is not usable and does not say why",
                    entry.id
                );
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
            "counter",
            "weather",
            "set_city",
            "test_greet_name",
            "jarvis_thanks",
            "jarvis_joke",
            "jarvis_insult",
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
            assert_eq!(status(id), "allowlist_required", "{id}");
        }
        // These run after a spoken confirmation, and they run.
        for id in [
            "terminate",
            "jarvis_reboot",
            "calculator_close",
            "browser_close",
        ] {
            assert_eq!(status(id), "confirmation_required", "{id}");
        }
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
        // Nothing in the installed packs is forbidden any more: the three that were
        // now ask for a confirmation instead.
        assert!(
            catalog
                .entries
                .iter()
                .all(|entry| entry.status != "forbidden"),
            "no installed command is forbidden"
        );
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
        assert_eq!(entry.status, "allowlist_required");
        assert_eq!(
            entry.unavailable_reason.as_deref(),
            Some("allowlist_required")
        );

        // Closing windows names a role and needs no allowlist: the windows exist or
        // they do not, at the moment the phrase is said.
        launch.native = Some(crate::commands::NativeAction::CloseApplicationWindows {
            role: "browser".to_string(),
        });
        let entry = entry_of("browser", &directory, &launch, "ru");
        assert_eq!(entry.status, "ready");
        assert!(entry.executor_ready);
        assert_eq!(entry.unavailable_reason, None);

        // A command that asks for a confirmation is usable, and the status says what
        // it is waiting for rather than calling it ready.
        launch.risk_level = crate::safety::RiskLevel::ConfirmationRequired;
        let entry = entry_of("browser", &directory, &launch, "ru");
        assert_eq!(entry.status, "confirmation_required");
        assert!(entry.enabled);
        assert!(entry.requires_confirmation);
        assert_eq!(entry.unavailable_reason, None);
        launch.risk_level = crate::safety::RiskLevel::Safe;

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
    fn the_four_questions_are_answered_separately() {
        let catalog = installed();
        for entry in &catalog.entries {
            // A phrase-checked fact and an executor fact are never the same fact.
            assert_eq!(
                entry.recognized,
                !entry.phrases.is_empty(),
                "{} must say whether a phrase reaches it",
                entry.id
            );
            assert_eq!(
                entry.allowed,
                entry.risk_level != "forbidden",
                "{} must say whether the policy permits it",
                entry.id
            );
            assert_eq!(
                entry.verified,
                entry.enabled && entry.recognized,
                "{} claims verified only when it would really run",
                entry.id
            );
            if entry.status == "executor_missing" {
                assert!(
                    entry.recognized && !entry.executor_ready,
                    "{} has a phrase but no executor, and must say exactly that",
                    entry.id
                );
                assert!(
                    !entry.verified,
                    "{} cannot be verified without an executor",
                    entry.id
                );
            }
            // A forbidden command is refused by the policy, and that is a different
            // sentence from "its executor is missing": the executable may well be
            // there, and the card says so. This is the one documented overlap.
            if entry.status == "forbidden" {
                assert!(!entry.allowed, "{} is refused by policy", entry.id);
                assert!(!entry.enabled, "{} must not claim to be ready", entry.id);
            }
            // A launch that waits for the allowlist has an executor — the pipeline —
            // and needs configuration. Two different answers.
            if entry.status == "allowlist_required" {
                assert!(entry.executor_ready, "{} has the pipeline", entry.id);
                assert!(entry.allowed, "{} is not forbidden", entry.id);
                assert!(!entry.enabled);
                assert_eq!(
                    entry.unavailable_reason.as_deref(),
                    Some("allowlist_required"),
                    "{}",
                    entry.id
                );
            }
        }

        let find = |id: &str| {
            catalog
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap_or_else(|| panic!("{id} must be installed"))
        };

        // The concrete case: the phrase reaches a command whose helper is missing.
        let open_google = find("open_google");
        assert!(open_google.recognized);
        assert!(!open_google.executor_ready);
        assert!(!open_google.verified);
        assert!(open_google.allowed);

        // A command that asks for a confirmation: the executor is there, the policy
        // permits it, and the phrase alone is not enough.
        let reboot = find("jarvis_reboot");
        assert!(reboot.recognized);
        assert!(
            reboot.executor_ready,
            "shutdown.exe is a program like any other"
        );
        assert!(reboot.allowed);
        assert!(reboot.verified, "it does run, after the confirmation");
        assert_eq!(reboot.status, "confirmation_required");
        assert_eq!(reboot.unavailable_reason, None);

        // A command that closes a window gracefully: nothing is started, the window
        // has to exist, and the phrase asks first.
        let browser_close = find("browser_close");
        assert!(browser_close.executor_ready);
        assert!(browser_close.allowed);
        assert!(browser_close.verified);
        assert_eq!(browser_close.status, "confirmation_required");
    }

    #[test]
    fn the_runtime_layout_is_what_is_read() {
        // The application reads `resources/commands/<pack>/command.toml` next to its
        // own executable, in a debug build and in a release bundle alike: the path
        // is relative to the runtime directory and never to a developer's checkout.
        let runtime = fixture("runtime").join("debug");
        let packs = runtime.join(config::COMMANDS_PATH);
        let pack = packs.join("volume");
        fs::create_dir_all(&pack).expect("a runtime pack");
        fs::write(
            pack.join("command.toml"),
            "[[commands]]\nid = \"volume_mid\"\ntype = \"native\"\nrisk_level = \"safe\"\n\n[commands.native]\naction = \"set_volume\"\npercent = 50\n\n[commands.phrases]\nru = [\"громкость пятьдесят\"]\nen = [\"volume fifty\"]\nua = [\"гучність п'ятдесят\"]\n",
        )
        .expect("a runtime document");
        // A second pack the loader does not read: reported, not hidden.
        fs::create_dir_all(packs.join("legacy")).expect("a runtime pack");
        fs::write(packs.join("legacy").join("command.yaml"), "list: []").expect("a document");

        let catalog = build_catalog(&packs, "ru");
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].id, "volume_mid");
        assert_eq!(catalog.entries[0].status, "ready");
        assert_eq!(catalog.unreadable.len(), 1);
        assert_eq!(catalog.unreadable[0].pack, "legacy");

        // Nothing in the answer carries the runtime directory.
        let serialized = serde_json::to_string(&catalog).expect("the answer is a document");
        assert!(
            !serialized.contains(&runtime.to_string_lossy().to_string()),
            "the answer must not carry a path"
        );
        assert!(
            !serialized.contains("resources"),
            "the answer must not carry a path"
        );
        let _ = fs::remove_dir_all(runtime.parent().expect("the fixture root"));
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
