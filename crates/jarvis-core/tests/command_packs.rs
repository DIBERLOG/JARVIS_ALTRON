//! Every command pack in the repository has to parse.
//!
//! The loader treats a pack it cannot parse as a warning and carries on, which is
//! the right behaviour at run time — one broken pack must not stop the
//! application — and the wrong behaviour in a repository, where it means a whole
//! feature silently disappears. The weather pack did exactly that: its second
//! command wrote `phrases = [...]`, a sequence, where the schema wants a map of
//! language to phrases, so `invalid type: sequence, expected a map` was logged and
//! the pack was dropped.
//!
//! This test walks `resources/commands/**/command.toml` and parses each one with
//! the loader's own function, so the two cannot drift apart. It also checks the
//! parts of a pack a person actually uses: an identifier, at least one phrase, and
//! the three languages this project ships.

use std::path::{Path, PathBuf};

use jarvis_core::commands::parse_command_document;

/// Every `command.toml` under `resources/commands`, sorted.
fn command_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("resources")
        .join("commands");
    let mut files = Vec::new();
    let entries = std::fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("the command directory must exist at {root:?}: {error}"));
    for entry in entries.flatten() {
        let candidate = entry.path().join("command.toml");
        if candidate.is_file() {
            files.push(candidate);
        }
    }
    files.sort();
    assert!(!files.is_empty(), "there must be at least one command pack");
    files
}

#[test]
fn every_pack_parses_with_the_loader_itself() {
    for path in command_files() {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{path:?} must be readable: {error}"));
        let parsed = parse_command_document(&text).unwrap_or_else(|error| {
            panic!(
                "{} must parse with the schema the loader uses: {error}",
                path.display()
            )
        });
        assert!(
            !parsed.commands.is_empty(),
            "{} declares no command",
            path.display()
        );
    }
}

#[test]
fn every_command_has_an_id_and_is_reachable_by_a_phrase() {
    for path in command_files() {
        let text = std::fs::read_to_string(&path).expect("readable");
        let parsed = parse_command_document(&text).expect("parsed");
        for command in &parsed.commands {
            assert!(
                !command.id.trim().is_empty(),
                "{}: a command without an id cannot be called",
                path.display()
            );
            let phrases = command.get_all_phrases();
            assert!(
                !phrases.is_empty(),
                "{}: `{}` has no phrase, so nothing can reach it",
                path.display(),
                command.id
            );
            for phrase in &phrases {
                assert!(
                    !phrase.trim().is_empty(),
                    "{}: `{}` has an empty phrase",
                    path.display(),
                    command.id
                );
            }
        }
    }
}

#[test]
fn the_weather_pack_answers_in_all_three_languages() {
    let path = command_files()
        .into_iter()
        .find(|path| path.to_string_lossy().contains("weather"))
        .expect("the weather pack must exist");
    let text = std::fs::read_to_string(&path).expect("readable");
    let parsed = parse_command_document(&text).expect("the weather pack must parse");

    for language in ["ru", "en", "ua"] {
        let phrases = parsed
            .commands
            .iter()
            .flat_map(|command| command.get_phrases(language).to_vec())
            .collect::<Vec<_>>();
        assert!(
            !phrases.is_empty(),
            "the weather pack must answer in {language}"
        );
        // And each language is a real list, not a fallback to another one.
        assert!(
            phrases.iter().any(|phrase| phrase.contains("{city}")),
            "the weather pack must accept a city in {language}: {phrases:?}"
        );
    }

    // The city slot is what makes the phrase a query rather than a wish.
    let weather = parsed
        .commands
        .iter()
        .find(|command| command.id == "weather")
        .expect("the weather command");
    assert!(
        weather.slots.contains_key("city"),
        "the weather command needs a city slot"
    );
    let setting = parsed
        .commands
        .iter()
        .find(|command| command.id == "set_city")
        .expect("the set_city command");
    assert!(setting.slots.contains_key("city"));
}

#[test]
fn a_sequence_where_the_schema_wants_a_language_map_is_refused() {
    // The exact defect, kept as a test so the schema and the packs stay honest.
    let broken = r#"
[[commands]]
id = "broken"
type = "lua"
script = "script.lua"
phrases = ["установи город", "set city"]
"#;
    let error = parse_command_document(broken)
        .err()
        .expect("a sequence must not be accepted where a map is expected");
    assert!(
        error.contains("expected a map") || error.contains("invalid type"),
        "{error}"
    );

    // And the corrected shape parses.
    let fixed = r#"
[[commands]]
id = "fixed"
type = "lua"
script = "script.lua"

[commands.phrases]
ru = ["установи город"]
en = ["set city"]
ua = ["встанови місто"]
"#;
    let parsed = parse_command_document(fixed).expect("the map shape parses");
    assert_eq!(parsed.commands.len(), 1);
    assert_eq!(
        parsed.commands[0].get_phrases("ua"),
        std::sync::Arc::new(vec!["встанови місто".to_string()])
    );
}
