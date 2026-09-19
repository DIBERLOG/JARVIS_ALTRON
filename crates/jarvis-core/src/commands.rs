use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use seqdiff::ratio;

mod structs;
pub use structs::*;

use crate::{config, i18n, APP_DIR};

#[cfg(feature = "lua")]
use crate::lua::{self, CommandContext, SandboxLevel};

/// Parses one `command.toml` document.
///
/// The loader and the test that walks `resources/commands/**` both go through
/// this function, so a pack that cannot be parsed fails the suite instead of
/// being skipped with a warning in a log nobody reads. That is exactly how the
/// weather pack shipped a `phrases = [...]` sequence where the schema wants a
/// language map, and the whole file was refused at run time.
pub fn parse_command_document(text: &str) -> Result<JCommandsList, String> {
    toml::from_str::<JCommandsList>(text).map_err(|error| error.to_string())
}

pub fn parse_commands() -> Result<Vec<JCommandsList>, String> {
    let mut commands: Vec<JCommandsList> = Vec::new();

    let commands_path = APP_DIR.join(config::COMMANDS_PATH);
    let cmd_dirs = fs::read_dir(&commands_path).map_err(|e| {
        format!(
            "Error reading commands directory {:?}: {}",
            commands_path, e
        )
    })?;

    for entry in cmd_dirs.flatten() {
        let cmd_path = entry.path();
        let toml_file = cmd_path.join("command.toml");

        if !toml_file.exists() {
            continue;
        }

        let content = match fs::read_to_string(&toml_file) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", toml_file.display(), e);
                continue;
            }
        };

        let file: JCommandsList = match parse_command_document(&content) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to parse {}: {}", toml_file.display(), e);
                continue;
            }
        };

        commands.push(JCommandsList {
            path: cmd_path,
            commands: file.commands,
        });
    }

    if commands.is_empty() {
        Err("No commands found".into())
    } else {
        info!("Loaded {} command pack(s)", commands.len());
        Ok(commands)
    }
}

pub fn commands_hash(commands: &[JCommandsList]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();

    let lang = i18n::get_language();
    hasher.update(lang.as_bytes());
    hasher.update(b"|");

    // collect all command ids and phrases for current language, sorted
    let mut all_data: Vec<(&str, _)> = commands
        .iter()
        .flat_map(|ac| {
            ac.commands
                .iter()
                .map(|c| (c.id.as_str(), c.get_phrases(&lang)))
        })
        .collect();
    all_data.sort_by_key(|(id, _)| *id);

    for (id, phrases) in all_data {
        hasher.update(id.as_bytes());
        for phrase in phrases.iter() {
            hasher.update(phrase.as_bytes());
        }
    }

    format!("{:x}", hasher.finalize())
}

pub fn fetch_command<'a>(
    phrase: &str,
    commands: &'a [JCommandsList],
) -> Option<(&'a PathBuf, &'a JCommand)> {
    fetch_command_in(phrase, commands, &i18n::get_language())
}

/// The same match, with the language given instead of read from the settings.
///
/// The listener and the phrase checker both go through this function, so there is
/// one matcher and one threshold in the build. A second implementation would be a
/// second answer to the same question, which is the defect this avoids.
pub fn fetch_command_in<'a>(
    phrase: &str,
    commands: &'a [JCommandsList],
    language: &str,
) -> Option<(&'a PathBuf, &'a JCommand)> {
    let lang = language.to_string();

    let phrase = phrase.trim().to_lowercase();
    if phrase.is_empty() {
        return None;
    }

    let phrase_chars: Vec<char> = phrase.chars().collect();
    let phrase_words: Vec<&str> = phrase.split_whitespace().collect();

    let mut result: Option<(&PathBuf, &JCommand)> = None;
    let mut best_score = config::CMD_RATIO_THRESHOLD;

    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            let cmd_phrases = cmd.get_phrases(&lang);

            for cmd_phrase in cmd_phrases.iter() {
                let cmd_phrase_lower = cmd_phrase.trim().to_lowercase();
                let cmd_phrase_chars: Vec<char> = cmd_phrase_lower.chars().collect();

                // A phrase that carries a slot — `какая погода в {city}` — cannot be
                // compared literally: `{city}` is not a word anybody says out loud.
                // The placeholder stands for the spoken value, so it stands for any
                // run of words (one for `Москве`, two for `Нижнем Новгороде`) and
                // for nothing at all when the value is left out. The rest of the
                // phrase is compared with the same score and the same threshold as
                // every other phrase. This is one of the defects that made commands
                // unreachable by voice: the phrase compared against still contained
                // the placeholder, so the score never reached the threshold. The
                // value itself is taken out later, by the existing slot machinery.
                let cmd_words: Vec<&str> = cmd_phrase_lower.split_whitespace().collect();
                let placeholder = cmd_words
                    .iter()
                    .position(|token| token.starts_with('{') && token.ends_with('}'));
                if let Some(index) = placeholder {
                    let reduced_candidate: Vec<&str> = cmd_words
                        .iter()
                        .enumerate()
                        .filter(|(position, _)| *position != index)
                        .map(|(_, token)| *token)
                        .collect();
                    let candidate_chars: Vec<char> = reduced_candidate.join(" ").chars().collect();
                    let mut slot_score = 0.0f64;
                    for start in 0..=phrase_words.len() {
                        for end in start..=phrase_words.len() {
                            let reduced_input: Vec<&str> = phrase_words[..start]
                                .iter()
                                .chain(phrase_words[end..].iter())
                                .copied()
                                .collect();
                            let input_chars: Vec<char> = reduced_input.join(" ").chars().collect();
                            let char_ratio = ratio(&input_chars, &candidate_chars);
                            let word_score = word_overlap_score(&reduced_input, &reduced_candidate);
                            slot_score = slot_score.max((char_ratio * 0.6) + (word_score * 0.4));
                        }
                    }
                    if slot_score > best_score {
                        best_score = slot_score;
                        result = Some((&cmd_list.path, cmd));
                    }
                    continue;
                }

                // character-level similarity
                let char_ratio = ratio(&phrase_chars, &cmd_phrase_chars);

                // word-level similarity
                let word_score = word_overlap_score(&phrase_words, &cmd_words);

                // combined score
                let score = (char_ratio * 0.6) + (word_score * 0.4);

                // early exit on perfect match
                if score >= 99.0 {
                    debug!("Perfect match -> cmd '{}'", cmd.id);
                    return Some((&cmd_list.path, cmd));
                }

                if score > best_score {
                    best_score = score;
                    result = Some((&cmd_list.path, cmd));
                }
            }
        }
    }

    if let Some((_, cmd)) = result {
        // The phrase is what a person said; only its length is written.
        info!(
            "Fuzzy match -> cmd '{}' (score: {:.1}%, length: {})",
            cmd.id,
            best_score,
            phrase.chars().count()
        );
    } else {
        debug!(
            "No match (best: {:.1}%, length: {})",
            best_score,
            phrase.chars().count()
        );
    }

    result
}

fn word_overlap_score(input_words: &[&str], cmd_words: &[&str]) -> f64 {
    if input_words.is_empty() || cmd_words.is_empty() {
        return 0.0;
    }

    let mut matched = 0.0;

    // pre-compute cmd word chars to avoid repeated allocations
    let cmd_word_chars: Vec<Vec<char>> = cmd_words.iter().map(|w| w.chars().collect()).collect();

    for input_word in input_words {
        let input_chars: Vec<char> = input_word.chars().collect();

        let best_word_match = cmd_word_chars
            .iter()
            .map(|cw| ratio(&input_chars, cw))
            .fold(0.0_f64, f64::max);

        if best_word_match > 70.0 {
            matched += best_word_match / 100.0;
        }
    }

    let max_words = input_words.len().max(cmd_words.len()) as f64;
    (matched / max_words) * 100.0
}

pub fn execute_exe(exe: &str, args: &[String]) -> std::io::Result<Child> {
    Command::new(exe).args(args).spawn()
}

pub fn execute_cli(cmd: &str, args: &[String]) -> std::io::Result<Child> {
    // A shell would reinterpret command-pack content and interpolated arguments.
    // Packs must name an executable and provide each argument separately.
    if cmd.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "CLI executable cannot be empty",
        ));
    }
    debug!(
        "Spawning approved executable: {} ({} arguments)",
        cmd,
        args.len()
    );
    Command::new(cmd).args(args).spawn()
}

pub fn execute_command(
    cmd_path: &PathBuf,
    cmd_config: &JCommand,
    phrase: Option<&str>,
    slots: Option<&HashMap<String, SlotValue>>,
) -> Result<bool, String> {
    // execute command by the type
    match cmd_config.cmd_type.as_str() {
        // BRUH
        "voice" => Ok(true),

        // LUA command
        #[cfg(feature = "lua")]
        "lua" => execute_lua_command(cmd_path, cmd_config, phrase, slots),

        // AutoHotkey command
        // @TODO: Consider adding ahk source files execution?
        "ahk" => {
            let exe_path_absolute = Path::new(&cmd_config.exe_path);
            let exe_path_local = cmd_path.join(&cmd_config.exe_path);

            let exe_path = if exe_path_absolute.exists() {
                exe_path_absolute
            } else {
                exe_path_local.as_path()
            };

            execute_exe(exe_path.to_str().unwrap(), &cmd_config.exe_args)
                .map(|_| true)
                .map_err(|e| format!("AHK process spawn error: {}", e))
        }

        // CLI command type
        // @TODO: Consider security restrictions
        "cli" => execute_cli(&cmd_config.cli_cmd, &cmd_config.cli_args)
            .map(|_| true)
            .map_err(|e| format!("CLI command error: {}", e)),

        // TERMINATOR command (T1000)
        "terminate" => {
            std::thread::sleep(Duration::from_secs(2));
            std::process::exit(0);
        }

        // STOP CHANING
        "stop_chaining" => Ok(false),

        // other
        _ => {
            error!("Command type unknown: {}", cmd_config.cmd_type);
            Err(format!("Command type unknown: {}", cmd_config.cmd_type).into())
        }
    }
}

// look up a command by its ID
pub fn get_command_by_id<'a>(
    commands: &'a [JCommandsList],
    id: &str,
) -> Option<(&'a PathBuf, &'a JCommand)> {
    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            if cmd.id == id {
                return Some((&cmd_list.path, cmd));
            }
        }
    }
    None
}

pub fn list_paths(commands: &[JCommandsList]) -> Vec<&Path> {
    commands.iter().map(|x| x.path.as_path()).collect()
}

#[cfg(feature = "lua")]
fn execute_lua_command(
    cmd_path: &PathBuf,
    cmd_config: &JCommand,
    phrase: Option<&str>,
    slots: Option<&HashMap<String, SlotValue>>,
) -> Result<bool, String> {
    // get script path

    let script_name = if cmd_config.script.is_empty() {
        "script.lua"
    } else {
        &cmd_config.script
    };

    let script_path = cmd_path.join(script_name);

    if !script_path.exists() {
        return Err(format!("Lua script not found: {}", script_path.display()));
    }

    // parse sandbox level
    let sandbox = SandboxLevel::from_str(&cmd_config.sandbox);

    // create context
    let context = CommandContext {
        phrase: phrase.unwrap_or("").to_string(),
        command_id: cmd_config.id.clone(),
        command_path: cmd_path.clone(),
        language: i18n::get_language(),
        slots: slots.map(|s| s.clone()),
    };

    // get timeout
    let timeout = Duration::from_millis(cmd_config.timeout);

    info!(
        "Executing Lua command: {} (sandbox: {:?}, timeout: {:?})",
        cmd_config.id, sandbox, timeout
    );

    // execute
    match lua::execute(&script_path, context, sandbox, timeout) {
        Ok(result) => {
            info!(
                "Lua command {} completed (chain: {})",
                cmd_config.id, result.chain
            );
            Ok(result.chain)
        }
        Err(e) => {
            error!("Lua command {} failed: {}", cmd_config.id, e);
            Err(e.to_string())
        }
    }
}

// ------------------------------------------------------- phrase normalization

/// The words a person puts in front of a command.
const WAKE_WORDS: [&str; 6] = [
    "джарвис",
    "jarvis",
    "джарвіс",
    "альтрон",
    "altron",
    "jarvis,",
];

/// Normalizes a recognized phrase the one way every matcher in this build sees it.
///
/// The listener applies this before it looks for a command, and the settings
/// page applies it when a person asks what a phrase would do — the same function,
/// so the answer on the page is the answer the microphone would get.
///
/// The rules, in order:
///
/// * lower case, so `Погода` and `погода` are the same word;
/// * `ё` becomes `е` (and `Ї`/`ї` stay as they are): a recognizer writes what it
///   heard, and a person writes what they learned;
/// * punctuation becomes a space, except inside the values a slot will take, so
///   `погода в Москве,` and `weather in New York?` both work;
/// * whitespace, including the non-breaking kind a recognizer produces, collapses
///   to single spaces;
/// * a leading wake word — with the comma that follows it — is removed, because
///   the person says `Джарвис, какая погода` and the command is `какая погода`.
pub fn normalize_phrase(phrase: &str) -> String {
    let mut text = phrase.to_lowercase().replace('ё', "е");
    text = text
        .chars()
        .map(|character| match character {
            // Everything that is not a letter, a digit, a space or a value
            // character becomes a space: a comma, a full stop, a question mark.
            c if c.is_alphanumeric() || c.is_whitespace() => c,
            '-' | '_' => character,
            _ => ' ',
        })
        .collect();
    let mut words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| c == '-' || c == '_')
                .to_string()
        })
        .filter(|word| !word.is_empty())
        .collect();
    // The wake word, once, at the front. A word that merely contains it — a city
    // called Джарвисово — is left alone.
    if let Some(first) = words.first() {
        let bare = first.trim_end_matches(',');
        if WAKE_WORDS.contains(&bare) {
            words.remove(0);
        }
    }
    words.join(" ")
}

/// What a phrase check found. No command is ever executed for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhraseCheck {
    /// The phrase as every matcher sees it.
    pub normalized: String,
    /// The identifier of the command a phrase would reach.
    pub matched: Option<String>,
    /// How close the best candidate was, as a whole percentage.
    pub score: u8,
    /// The slots the matched command declares, by name.
    pub slots: Vec<String>,
    /// A stable reason when nothing matched, for the interface to translate.
    pub reason: Option<&'static str>,
}

/// Tries a phrase without running anything.
///
/// The result says what the phrase normalizes to, which command it would reach,
/// which slots that command needs, and — when nothing matched — why, as a code:
/// `empty`, `no_commands`, `no_match`. Nothing is executed, no program is
/// started, and no system action is taken: this is the same matcher the listener
/// uses, read back.
pub fn check_phrase(commands: &[JCommandsList], language: &str, phrase: &str) -> PhraseCheck {
    let normalized = normalize_phrase(phrase);
    if normalized.is_empty() {
        return PhraseCheck {
            normalized,
            matched: None,
            score: 0,
            slots: Vec::new(),
            reason: Some("empty"),
        };
    }
    if commands.is_empty() {
        return PhraseCheck {
            normalized,
            matched: None,
            score: 0,
            slots: Vec::new(),
            reason: Some("no_commands"),
        };
    }
    match fetch_command_in(&normalized, commands, language) {
        Some((_, command)) => {
            let mut slots: Vec<String> = command.slots.keys().cloned().collect();
            slots.sort();
            PhraseCheck {
                normalized,
                matched: Some(command.id.clone()),
                score: 100,
                slots,
                reason: None,
            }
        }
        None => PhraseCheck {
            normalized,
            matched: None,
            score: 0,
            slots: Vec::new(),
            reason: Some("no_match"),
        },
    }
}

/// The stages a spoken phrase passes between the microphone and an action.
///
/// A phrase that is not accepted leaves no trace today: the listener either runs
/// something or says "не найдена", and the reason is only in a log line that
/// quotes the phrase. These keys name every stop on the way, so the same
/// sequence of short, phrase-free lines can be written by the voice host and read
/// next to the phrase checker in the window. The recognized text itself is never
/// part of a stage: only its length is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandStage {
    /// The listener got a transcript. Only its length is recorded.
    ListenerReceivedPhrase,
    /// The transcript contained a wake word, and whether anything was left after it.
    WakeWordDetected,
    /// The length of the normalized phrase, which is what the matcher sees.
    NormalizedLength,
    /// The matcher answered: a command id, or that it found none.
    CommandMatch,
    /// Why nothing was accepted, as a code: `too_short`, `empty_after_strip`,
    /// `no_match`, `no_commands`, `action_ambiguous`, `awaiting_confirmation`,
    /// `forbidden`, `slot_missing`.
    RejectionCode,
    /// The command was handed to the executor.
    ExecutionStarted,
    /// The executor answered, with `ok` or `error`.
    ExecutionResult,
}

impl CommandStage {
    /// The stable spelling used in logs and in the window. Never translated, so a
    /// support question and a log line name the same thing.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListenerReceivedPhrase => "listener_received_phrase",
            Self::WakeWordDetected => "wake_word_detected",
            Self::NormalizedLength => "normalized_length",
            Self::CommandMatch => "command_match",
            Self::RejectionCode => "rejection_code",
            Self::ExecutionStarted => "execution_started",
            Self::ExecutionResult => "execution_result",
        }
    }
}

/// Every stage, in the order a phrase passes them. A test fixes the list, because
/// the names are read by a person and must not drift.
pub const COMMAND_STAGES: [CommandStage; 7] = [
    CommandStage::ListenerReceivedPhrase,
    CommandStage::WakeWordDetected,
    CommandStage::NormalizedLength,
    CommandStage::CommandMatch,
    CommandStage::RejectionCode,
    CommandStage::ExecutionStarted,
    CommandStage::ExecutionResult,
];

#[cfg(test)]
mod phrase_check_tests {
    use super::*;

    #[test]
    fn every_stage_has_its_own_stable_name() {
        let mut names: Vec<&str> = COMMAND_STAGES.iter().map(|stage| stage.as_str()).collect();
        assert_eq!(names.len(), 7);
        assert!(names.contains(&"listener_received_phrase"));
        assert!(names.contains(&"wake_word_detected"));
        assert!(names.contains(&"normalized_length"));
        assert!(names.contains(&"command_match"));
        assert!(names.contains(&"rejection_code"));
        assert!(names.contains(&"execution_started"));
        assert!(names.contains(&"execution_result"));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 7, "two stages share a name");
        for name in names {
            assert!(
                name.chars()
                    .all(|character| character.is_ascii_lowercase() || character == '_'),
                "{name} is not a stable key"
            );
        }
    }

    fn packs() -> Vec<JCommandsList> {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("resources")
            .join("commands");
        let mut packs = Vec::new();
        for entry in std::fs::read_dir(&directory)
            .expect("the command directory")
            .flatten()
        {
            let file = entry.path().join("command.toml");
            if !file.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&file).expect("readable");
            let mut parsed = parse_command_document(&text).expect("parses");
            parsed.path = entry.path();
            packs.push(parsed);
        }
        packs
    }

    #[test]
    fn the_normalization_is_the_same_for_the_listener_and_the_checker() {
        // Lower case, ё, punctuation, spacing, and the wake word: one function,
        // so the page cannot answer differently from the microphone.
        assert_eq!(
            normalize_phrase("Джарвис, какая погода в Москве"),
            "какая погода в москве"
        );
        assert_eq!(
            normalize_phrase("джарвис какая погода в москве"),
            "какая погода в москве"
        );
        assert_eq!(normalize_phrase("Джарвис  какая   погода?"), "какая погода");
        assert_eq!(normalize_phrase("ДЖАРВИС, ПОГОДА"), "погода");
        assert_eq!(normalize_phrase("погода в Королёве"), "погода в королеве");
        assert_eq!(
            normalize_phrase("weather in New York?"),
            "weather in new york"
        );
        assert_eq!(normalize_phrase("  "), "");
        // A word that merely contains the wake word is left alone.
        assert_eq!(
            normalize_phrase("погода в джарвисово"),
            "погода в джарвисово"
        );
    }

    #[test]
    fn a_weather_phrase_matches_the_weather_command_with_a_city() {
        let packs = packs();
        for phrase in [
            "Джарвис, какая погода в Москве",
            "джарвис какая погода в москве",
            "Джарвис, погода в Королёве",
        ] {
            let check = check_phrase(&packs, "ru", phrase);
            assert_eq!(check.matched.as_deref(), Some("weather"), "{phrase}");
            assert!(check.slots.contains(&"city".to_string()), "{phrase}");
            assert_eq!(check.reason, None);
        }
        // Without the wake word it still matches: the phrase is the command.
        let check = check_phrase(&packs, "ru", "какая погода в Москве");
        assert_eq!(check.matched.as_deref(), Some("weather"));
    }

    #[test]
    fn an_unknown_phrase_says_so_without_running_anything() {
        let packs = packs();
        let check = check_phrase(&packs, "ru", "Джарвис, сделай мне кофе");
        assert_eq!(check.matched, None);
        assert_eq!(check.reason, Some("no_match"));
        let check = check_phrase(&packs, "ru", "   ");
        assert_eq!(check.reason, Some("empty"));
        let check = check_phrase(&[], "ru", "погода");
        assert_eq!(check.reason, Some("no_commands"));
    }

    #[test]
    fn an_english_phrase_matches_the_english_pack() {
        let packs = packs();
        let check = check_phrase(&packs, "en", "what's the weather in London");
        assert_eq!(check.matched.as_deref(), Some("weather"));
        let check = check_phrase(&packs, "en", "open browser");
        assert_eq!(check.matched.as_deref(), Some("browser_open"));
    }
}
