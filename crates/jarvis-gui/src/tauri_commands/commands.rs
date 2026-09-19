use jarvis_core::commands::{self, JCommand, JCommandsList};
use once_cell::sync::Lazy;
use serde::Serialize;

static COMMANDS: Lazy<Vec<JCommandsList>> =
    Lazy::new(|| commands::parse_commands().unwrap_or_default());

#[tauri::command]
pub fn get_commands_count() -> usize {
    COMMANDS.iter().map(|list| list.commands.len()).sum()
}

#[tauri::command]
pub fn get_commands_list() -> Vec<JCommand> {
    COMMANDS
        .iter()
        .flat_map(|list| list.commands.clone())
        .collect()
}

/// What a phrase would reach, without reaching it.
///
/// This is the same `commands::check_phrase` the voice host matches with, so the
/// answer in the window is the answer the microphone would get. Nothing is
/// executed, no program is started, and the phrase is not written to the log or
/// to a file: it is read, compared, and forgotten. The identifier of the command
/// is returned, not the command's path or its arguments.
#[derive(Clone, Debug, Serialize)]
pub struct PhraseCheckView {
    /// The phrase as the matcher sees it, so a person can see the normalization.
    pub normalized: String,
    /// The command a phrase would reach, by identifier.
    pub matched: Option<String>,
    /// How close the best candidate was, as a whole percentage.
    pub score: u8,
    /// The slots the matched command declares, by name.
    pub slots: Vec<String>,
    /// A stable reason when nothing matched, for the interface to translate.
    pub reason: Option<String>,
}

#[tauri::command]
pub fn check_phrase_without_running(phrase: String) -> PhraseCheckView {
    let language = jarvis_core::i18n::get_language();
    let check = commands::check_phrase(&COMMANDS, &language, &phrase);
    PhraseCheckView {
        normalized: check.normalized,
        matched: check.matched,
        score: check.score,
        slots: check.slots,
        reason: check.reason.map(|reason| reason.to_string()),
    }
}
