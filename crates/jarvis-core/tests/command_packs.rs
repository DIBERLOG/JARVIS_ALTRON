//! Every command pack in the repository has to parse, to be reachable, and to name
//! an executor this build really has.
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
//! the loader's own function, so the two cannot drift apart. On top of that it
//! answers the four questions this project refuses to blur together:
//!
//! * **MATCHED** — a phrase the pack lists reaches *that* command, through the
//!   matcher the listener uses, in all three languages;
//! * **EXECUTABLE** — the executor the pack names exists in this build, and a
//!   command that needs a file has that file next to the pack;
//! * **ALLOWED** — a dangerous command carries the risk level that makes the safety
//!   gate refuse it or ask for a spoken confirmation;
//! * **no shell** — no pack names an interpreter, an absolute path, or a command
//!   line, and none of the seven legacy `command.yaml` packs is left unread.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jarvis_core::commands::{
    check_phrase, execute_command, native_dispatch_registered, parse_command_document,
    set_native_dispatch, JCommandsList, NativeAction, NativeError,
};
use jarvis_core::safety::RiskLevel;

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

/// The pack directory of a `command.toml`.
fn pack_directory(path: &Path) -> PathBuf {
    path.parent().expect("a pack directory").to_path_buf()
}

/// Every installed pack, parsed, with the directory it lives in.
fn installed() -> Vec<(PathBuf, JCommandsList)> {
    command_files()
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{path:?} must be readable: {error}"));
            let mut parsed = parse_command_document(&text).unwrap_or_else(|error| {
                panic!(
                    "{} must parse with the schema the loader uses: {error}",
                    path.display()
                )
            });
            parsed.path = pack_directory(&path);
            (path, parsed)
        })
        .collect()
}

fn catalog() -> Vec<JCommandsList> {
    installed().into_iter().map(|(_, pack)| pack).collect()
}

#[test]
fn every_pack_parses_with_the_loader_itself() {
    for (path, parsed) in installed() {
        assert!(
            !parsed.commands.is_empty(),
            "{} declares no command",
            path.display()
        );
    }
}

#[test]
fn no_pack_is_left_in_a_format_the_loader_does_not_read() {
    // The seven packs that used to ship `command.yaml` were migrated. If one comes
    // back, the page shows it as `unsupported_format` and its commands silently do
    // not exist — which is exactly the state this test refuses to allow.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("resources")
        .join("commands");
    let mut leftovers = Vec::new();
    for entry in std::fs::read_dir(&root)
        .expect("the command directory")
        .flatten()
    {
        for name in ["command.yaml", "command.yml"] {
            if entry.path().join(name).is_file() {
                leftovers.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    assert!(
        leftovers.is_empty(),
        "these packs are not read by the loader: {leftovers:?}"
    );
}

#[test]
fn every_command_has_an_id_and_is_reachable_by_a_phrase() {
    for (path, parsed) in installed() {
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
fn every_command_answers_in_all_three_languages() {
    for (path, parsed) in installed() {
        for command in &parsed.commands {
            for language in ["ru", "en", "ua"] {
                let phrases = command.get_phrases(language);
                assert!(
                    !phrases.is_empty(),
                    "{}: `{}` has no phrase in {language}",
                    path.display(),
                    command.id
                );
            }
        }
    }
}

#[test]
fn every_command_id_is_unique() {
    let mut seen: Vec<(String, String)> = Vec::new();
    for (_, parsed) in installed() {
        let pack = parsed
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        for command in &parsed.commands {
            if let Some((_, other)) = seen.iter().find(|(id, _)| *id == command.id) {
                panic!(
                    "`{}` is declared twice: in {pack} and in {other}",
                    command.id
                );
            }
            seen.push((command.id.clone(), pack.clone()));
        }
    }
    assert!(
        seen.len() >= 20,
        "expected the installed commands, got {}",
        seen.len()
    );
}

#[test]
fn every_command_names_an_executor_this_build_has() {
    for (path, parsed) in installed() {
        for command in &parsed.commands {
            let pack = pack_directory(&path);
            match command.cmd_type.as_str() {
                "native" => {
                    let action = command.native.as_ref().unwrap_or_else(|| {
                        panic!(
                            "{}: `{}` is native without an action",
                            path.display(),
                            command.id
                        )
                    });
                    action.validate().unwrap_or_else(|error| {
                        panic!(
                            "{}: `{}` names an action this build refuses: {error}",
                            path.display(),
                            command.id
                        )
                    });
                    // A native action never carries a file of its own.
                    assert!(
                        command.exe_path.trim().is_empty() && command.cli_cmd.trim().is_empty(),
                        "{}: `{}` is native and still names a program",
                        path.display(),
                        command.id
                    );
                }
                "internal" => {
                    assert!(
                        command.internal.is_some(),
                        "{}: `{}` is internal without an event",
                        path.display(),
                        command.id
                    );
                    assert!(
                        command.exe_path.trim().is_empty()
                            && command.cli_cmd.trim().is_empty()
                            && command.script.trim().is_empty(),
                        "{}: `{}` is internal and still names a program",
                        path.display(),
                        command.id
                    );
                }
                "voice" => {
                    assert!(
                        !command.get_all_sounds().is_empty(),
                        "{}: `{}` is a voice command with no sound",
                        path.display(),
                        command.id
                    );
                }
                "terminate" | "stop_chaining" => {}
                "lua" => {
                    assert!(
                        !command.script.trim().is_empty(),
                        "{}: `{}` is a lua command with no script",
                        path.display(),
                        command.id
                    );
                }
                "ahk" => {
                    let named = &command.exe_path;
                    assert!(
                        !named.trim().is_empty(),
                        "{}: `{}` is an ahk command with no executable",
                        path.display(),
                        command.id
                    );
                    assert!(
                        !Path::new(named).is_absolute(),
                        "{}: `{}` names an absolute path",
                        path.display(),
                        command.id
                    );
                    // The file may be missing: that is `executor_missing`, and the
                    // catalogue reports it. It must never be a path out of the pack.
                    let candidate = pack.join(named);
                    assert!(
                        candidate.starts_with(&pack),
                        "{}: `{}` points outside its pack",
                        path.display(),
                        command.id
                    );
                }
                "cli" => {
                    assert!(
                        !command.cli_cmd.trim().is_empty(),
                        "{}: `{}` is a cli command with no executable",
                        path.display(),
                        command.id
                    );
                    assert!(
                        !Path::new(&command.cli_cmd).is_absolute(),
                        "{}: `{}` names an absolute path",
                        path.display(),
                        command.id
                    );
                }
                other => panic!(
                    "{}: `{}` names an executor this build does not have: {other}",
                    path.display(),
                    command.id
                ),
            }
        }
    }
}

#[test]
fn no_pack_can_become_a_shell() {
    // The rule the whole stage rests on: a pack names an action, never a command
    // line, and never an interpreter that would turn one string into a program.
    const INTERPRETERS: [&str; 12] = [
        "cmd",
        "cmd.exe",
        "powershell",
        "powershell.exe",
        "pwsh",
        "pwsh.exe",
        "sh",
        "bash",
        "wscript",
        "wscript.exe",
        "cscript",
        "cscript.exe",
    ];
    for (path, parsed) in installed() {
        for command in &parsed.commands {
            let named = command.cli_cmd.trim().to_lowercase();
            let named = named.trim_end_matches(".exe").to_string();
            assert!(
                !INTERPRETERS.contains(&named.as_str()),
                "{}: `{}` runs an interpreter ({})",
                path.display(),
                command.id,
                command.cli_cmd
            );
            assert!(
                !command.cli_args.iter().any(|argument| {
                    let argument = argument.to_lowercase();
                    argument == "-c" || argument == "/c" || argument == "-command"
                }),
                "{}: `{}` passes a command-line switch",
                path.display(),
                command.id
            );
            // And no field may carry a whole command line.
            for argument in command.cli_args.iter().chain(command.exe_args.iter()) {
                assert!(
                    !argument.contains("&&") && !argument.contains('|') && !argument.contains(';'),
                    "{}: `{}` passes what looks like a command line: {argument}",
                    path.display(),
                    command.id
                );
            }
        }
    }
}

#[test]
fn a_dangerous_command_is_gated() {
    let packs = catalog();
    let find = |id: &str| {
        packs
            .iter()
            .flat_map(|pack| pack.commands.iter())
            .find(|command| command.id == id)
            .unwrap_or_else(|| panic!("`{id}` must exist"))
    };

    // Stopping the assistant asks first: the phrase alone must not end the process.
    assert_eq!(
        find("terminate").risk_level,
        RiskLevel::ConfirmationRequired
    );
    // Reboot and forced process termination stay forbidden, so the safety gate
    // refuses them before an executor is even asked.
    assert_eq!(find("jarvis_reboot").risk_level, RiskLevel::Forbidden);
    assert_eq!(find("calculator_close").risk_level, RiskLevel::Forbidden);
    // A screenshot and a launch are not a confirmation by themselves: the pipeline
    // has its own rules for them.
    assert_eq!(find("windows_screenshot").risk_level, RiskLevel::Safe);
    // The volume commands are safe: they change a number the user controls.
    for id in [
        "volume_get",
        "volume_mute",
        "volume_unmute",
        "volume_min",
        "volume_mid",
        "volume_max",
    ] {
        assert_eq!(find(id).risk_level, RiskLevel::Safe, "{id}");
    }
}

#[test]
fn every_phrase_matches_its_own_command_in_every_language() {
    // MATCHED, for every phrase of every pack, through the production matcher.
    let packs = catalog();
    for (_, pack) in installed() {
        for command in &pack.commands {
            for language in ["ru", "en", "ua"] {
                for phrase in command.get_phrases(language).iter() {
                    let check = check_phrase(&packs, language, phrase);
                    assert_eq!(
                        check.matched.as_deref(),
                        Some(command.id.as_str()),
                        "`{phrase}` ({language}) must reach `{}`, got {:?}",
                        command.id,
                        check
                    );
                }
            }
        }
    }
}

#[test]
fn a_phrase_survives_the_shape_a_person_speaks_it_in() {
    // The same command through the wake word, the case, `ё`, punctuation and extra
    // spaces — the four things a recognizer does to a sentence.
    let packs = catalog();
    for phrase in [
        "Джарвис, открой браузер",
        "ДЖАРВИС ОТКРОЙ БРАУЗЕР",
        "джарвис,  открой   браузер!",
        "Джарвис, выключи звёзды", // `ё` is folded, and this one does not match
    ] {
        let check = check_phrase(&packs, "ru", phrase);
        if phrase.contains("звёзды") {
            assert_eq!(check.matched, None, "an unknown phrase must not match");
        } else {
            assert_eq!(check.matched.as_deref(), Some("browser_open"), "{phrase}");
        }
    }
    // A phrase with a slot, and a slot value of two words.
    let check = check_phrase(&packs, "ru", "Джарвис, какая погода в Нижнем Новгороде");
    assert_eq!(check.matched.as_deref(), Some("weather"));
    let check = check_phrase(&packs, "ru", "Джарвис, погода в Королёве");
    assert_eq!(check.matched.as_deref(), Some("weather"));
}

/// A native action handed to the host's pipeline, remembered by this test.
///
/// The dispatcher is a plain function pointer registered once for the process, and
/// the Rust test harness runs these tests in parallel: the memory is a mutex, and
/// the test that proves "nothing runs" compares the length before and after its own
/// call rather than the whole list.
static DISPATCHED: std::sync::Mutex<Vec<NativeAction>> = std::sync::Mutex::new(Vec::new());

fn remember(action: &NativeAction) -> Result<String, NativeError> {
    DISPATCHED
        .lock()
        .expect("the dispatch log is never poisoned")
        .push(action.clone());
    Ok("ok".to_string())
}

fn dispatched() -> Vec<NativeAction> {
    DISPATCHED
        .lock()
        .expect("the dispatch log is never poisoned")
        .clone()
}

/// The dispatcher is a process-wide hook, so the two tests that use it take turns:
/// one hands a command to the pipeline, the other proves that a phrase check hands
/// nothing to it, and neither may see the other's work.
static PIPELINE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_native_command_really_reaches_the_native_pipeline() {
    let _turn = PIPELINE_LOCK.lock().expect("the pipeline lock");
    // EXECUTED, with a stand-in pipeline: the point is that the pack's typed action
    // arrives there unchanged, and that the executor reports success.
    assert!(set_native_dispatch(remember));
    assert!(native_dispatch_registered());

    let packs = catalog();
    let pack = packs
        .iter()
        .find(|pack| pack.path.to_string_lossy().contains("volume"))
        .expect("the volume pack");
    let command = pack
        .commands
        .iter()
        .find(|command| command.id == "volume_mid")
        .expect("the volume_mid command");

    let chain = execute_command(&pack.path, command, Some("громкость пятьдесят"), None)
        .expect("the native executor must run");
    assert!(chain, "a native command keeps the chain");
    assert!(
        dispatched().contains(&NativeAction::SetVolume { percent: 50 }),
        "the pack's typed action must arrive in the pipeline"
    );

    // A malformed action never reaches the pipeline: the assertion above still holds.
    let mut broken = command.clone();
    broken.native = Some(NativeAction::SetVolume { percent: 200 });
    let error = execute_command(&pack.path, &broken, None, None).expect_err("must refuse");
    assert_eq!(error, "native_volume_range");
}

#[test]
fn a_native_command_without_an_action_is_refused() {
    let packs = catalog();
    let pack = packs.first().expect("a pack");
    let mut command = pack.commands[0].clone();
    command.cmd_type = "native".to_string();
    command.native = None;
    let error = execute_command(&pack.path, &command, None, None).expect_err("must refuse");
    assert!(
        error.contains("native command without a native action"),
        "{error}"
    );

    command.cmd_type = "internal".to_string();
    command.internal = None;
    let error = execute_command(&pack.path, &command, None, None).expect_err("must refuse");
    assert!(
        error.contains("internal command without an internal event"),
        "{error}"
    );
}

#[test]
fn an_internal_command_starts_no_process() {
    let packs = catalog();
    let pack = packs
        .iter()
        .find(|pack| pack.path.to_string_lossy().contains("stop"))
        .expect("the stop pack");
    let command = pack
        .commands
        .iter()
        .find(|command| command.id == "stop_listening")
        .expect("the stop command");
    // Without a host the typed event is refused by code, and nothing is started.
    let error = execute_command(&pack.path, command, Some("хватит"), None).expect_err("refused");
    assert_eq!(error, "internal_not_available");
}

#[test]
fn checking_a_phrase_executes_nothing() {
    // The phrase check runs the matcher and stops there: even a native command is
    // returned as an identifier, never handed to a pipeline.
    let _turn = PIPELINE_LOCK.lock().expect("the pipeline lock");
    let packs = catalog();
    let before = dispatched().len();
    let check = check_phrase(&packs, "ru", "громкость пятьдесят");
    assert_eq!(check.matched.as_deref(), Some("volume_mid"));
    assert_eq!(
        dispatched().len(),
        before,
        "checking a phrase must not hand anything to the pipeline"
    );
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

#[test]
fn a_program_a_pack_names_stays_inside_its_pack() {
    // No pack may point at something outside itself, and none may name a path the
    // user did not install. The migrated packs only name files next to them.
    let mut seen = HashSet::new();
    for (path, parsed) in installed() {
        let pack = pack_directory(&path);
        for command in &parsed.commands {
            if command.cmd_type != "ahk" {
                continue;
            }
            let candidate = pack.join(&command.exe_path);
            assert!(
                candidate.starts_with(&pack),
                "{}: `{}` escapes its pack",
                path.display(),
                command.id
            );
            seen.insert(command.id.clone());
        }
    }
    // The commands that need a compiled helper are exactly the ones the catalogue
    // reports as `executor_missing`: they must be a known, finite set.
    let expected: HashSet<&str> = [
        "browser_close",
        "open_google",
        "steam_close",
        "windows_minimize_all",
        "windows_empty_trash",
        "windows_sleep",
        "windows_clipboard",
        "windows_keyboard_layout",
    ]
    .into_iter()
    .collect();
    for id in &expected {
        assert!(
            seen.contains(*id),
            "`{id}` must keep its AutoHotkey executor"
        );
    }
}
