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

/// The catalogue the «Команды» page is built from.
///
/// It is read from the installed packs with the loader's own parser, in the
/// language the window is shown in, and it carries no path, no executable and no
/// argument: a pack is named by its logical name. Packs the loader cannot read —
/// the ones that ship `command.yaml`, which the loader does not open — are listed
/// with a reason code instead of being left out, and the global voice input is
/// listed from the settings, where it really lives.
#[tauri::command]
pub fn command_catalog(state: tauri::State<'_, crate::AppState>) -> commands::CommandCatalog {
    let language = jarvis_core::i18n::get_language();
    let mut catalog = commands::load_catalog(&language);
    let settings = state.voice_input.settings();
    catalog.entries.push(commands::global_voice_input_entry(
        &settings.phrase,
        settings.enabled,
    ));
    catalog.entries.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then(left.pack.cmp(&right.pack))
            .then(left.id.cmp(&right.id))
    });
    catalog
}
