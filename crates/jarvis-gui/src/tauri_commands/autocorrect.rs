//! Autocorrect commands exposed to the interface.
//!
//! Rules that shape this file:
//!
//! * the interface never reads SQLite, never receives a key, and never decrypts a word:
//!   it sends text and receives issues, suggestions, and previews;
//! * checking runs on a worker thread (`(async)`) and the AI improvement runs on the
//!   blocking pool, so neither blocks the window;
//! * the user's own words travel through the shared session, which hands this module the
//!   dictionary store only: **the password vault is never read, never corrected, and
//!   never sent anywhere**;
//! * nothing here logs document text: errors carry content-free messages, and a detected
//!   secret is reported by kind, never by value;
//! * an AI improvement is produced as a *preview* and applied only by a second, explicit
//!   command after the user confirmed it. A preview is never applied on the way back.
//! * the undo journal lives in memory and holds text, so locking the storage clears it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use jarvis_core::ai::Persona;
use jarvis_core::autocorrect::engine::CheckReport;
use jarvis_core::autocorrect::improvement::{
    apply_improvement, preview_improvement, LocalAiTextImprover, TextImprovementMode,
    TextImprovementPreview, TextImprovementRequest,
};
use jarvis_core::autocorrect::model::{text_version, LanguageMode};
use jarvis_core::autocorrect::replacement::{
    apply_corrections, undo_last, CorrectionJournal, MAX_JOURNAL_BATCHES,
};
use jarvis_core::autocorrect::session::AutocorrectStatus;
use jarvis_core::autocorrect::settings::{load_settings, AutocorrectSettings, CustomRule};
use jarvis_core::autocorrect::user_dictionary::{
    DictionaryImportOutcome, ImportOutcome, UserDictionaryQuery, UserDictionaryStats,
};
use jarvis_core::autocorrect::{
    dictionary_states, now, parse_word_list, resolved_dictionary_directory, unavailable_languages,
    AutocorrectError, Correction, CorrectionBatch, DictionaryManager, EncryptedUserDictionary,
    ExportedDictionaryRecord, Language, LocalSpellChecker, Suggestion, UndoOutcome,
    UserDictionaryEntry, AUTOCORRECT_SETTINGS_KEY, MAX_IMPROVE_CHARS,
};
use jarvis_core::notes::vault::{StorageStatus, VaultPaths};
use jarvis_core::SettingsManager;

use crate::AppState;

/// Longest accepted scope label from the interface.
const MAX_SCOPE_CHARS: usize = 96;

/// Which part of the interface asked for a check.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOrigin {
    Notes,
    Chat,
}

impl CheckOrigin {
    fn enabled_in(&self, settings: &AutocorrectSettings) -> bool {
        match self {
            Self::Notes => settings.checks_notes(),
            Self::Chat => settings.checks_chat(),
        }
    }
}

/// The dictionaries, the undo journals, and the settings key of this feature.
///
/// The dictionary folder follows the settings: changing it (or installing files) replaces
/// the manager, which drops every parsed dictionary and re-reads the folder on the next
/// use.
#[derive(Clone)]
pub struct AutocorrectHandle {
    settings: SettingsManager,
    dictionaries: Arc<RwLock<Arc<DictionaryManager>>>,
    journals: Arc<Mutex<HashMap<String, CorrectionJournal>>>,
}

impl AutocorrectHandle {
    /// Builds the handle from the stored settings.
    pub fn new(settings: SettingsManager) -> Self {
        let directory = default_dictionary_directory(&settings);
        Self {
            settings,
            dictionaries: Arc::new(RwLock::new(Arc::new(DictionaryManager::new(directory)))),
            journals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The settings as they are stored right now.
    pub fn autocorrect_settings(&self) -> AutocorrectSettings {
        load_settings(self.settings.read(AUTOCORRECT_SETTINGS_KEY).as_deref())
    }

    /// A checker over the current dictionaries, reloading them when the folder changed.
    pub fn checker(&self) -> LocalSpellChecker {
        let wanted = self.dictionary_directory();
        {
            let current = self.dictionaries.read();
            if current.directory() != wanted.as_path() {
                drop(current);
                self.replace_dictionaries(wanted);
            }
        }
        LocalSpellChecker::new(Arc::clone(&self.dictionaries.read()))
    }

    /// The folder the dictionaries are read from right now.
    pub fn dictionary_directory(&self) -> PathBuf {
        resolved_dictionary_directory(&data_directory(), &self.autocorrect_settings())
    }

    /// Replaces the dictionary manager, dropping every parsed dictionary.
    pub fn replace_dictionaries(&self, directory: PathBuf) {
        *self.dictionaries.write() = Arc::new(DictionaryManager::new(directory));
    }

    /// Forgets every parsed dictionary so the folder is read again.
    pub fn reload_dictionaries(&self) {
        self.dictionaries.read().reload();
    }

    /// Runs `action` against one document's undo journal.
    fn with_journal<T>(&self, scope: &str, action: impl FnOnce(&mut CorrectionJournal) -> T) -> T {
        let mut journals = self.journals.lock();
        action(journals.entry(scope.to_string()).or_default())
    }

    /// What the undo journal of one document holds, without changing it.
    pub fn journal_status(&self, scope: &str) -> UndoStatus {
        let journals = self.journals.lock();
        match journals.get(scope) {
            Some(journal) => UndoStatus {
                can_undo: journal.can_undo(),
                batches: journal.len(),
                words: journal.last_words(),
                capacity: MAX_JOURNAL_BATCHES,
            },
            None => UndoStatus {
                can_undo: false,
                batches: 0,
                words: Vec::new(),
                capacity: MAX_JOURNAL_BATCHES,
            },
        }
    }

    /// Drops every undo journal.
    ///
    /// Called on lock and at exit: a journal holds the text of the document it corrected,
    /// so it must not outlive the unlocked session.
    pub fn clear_journals(&self) {
        let mut journals = self.journals.lock();
        journals.clear();
    }

    /// How many journals are held right now, for diagnostics.
    pub fn journal_count(&self) -> usize {
        self.journals.lock().len()
    }
}

impl std::fmt::Debug for AutocorrectHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutocorrectHandle")
            .field("dictionary_dir", &self.dictionary_directory())
            .field("journals", &self.journal_count())
            .finish()
    }
}

/// The application data directory, from the same paths the storage uses.
fn data_directory() -> PathBuf {
    VaultPaths::production()
        .map(|paths| paths.data_dir)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn default_dictionary_directory(settings: &SettingsManager) -> PathBuf {
    let stored = load_settings(settings.read(AUTOCORRECT_SETTINGS_KEY).as_deref());
    resolved_dictionary_directory(&data_directory(), &stored)
}

/// Validates a scope label from the interface.
fn normalize_scope(scope: &str) -> Result<String, String> {
    let trimmed = scope.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_SCOPE_CHARS {
        return Err("the editor identifier is not usable".to_string());
    }
    Ok(trimmed.to_string())
}

/// Turns an autocorrect error into a message safe to show and to log.
fn describe(error: AutocorrectError) -> String {
    let message = error.to_string();
    // The message never carries document text, a matched secret, or a key.
    log::warn!("autocorrect: {message}");
    message
}

/// Status of the spelling layer, as the interface needs it.
#[derive(Clone, Debug, Serialize)]
pub struct AutocorrectStatusView {
    /// Shared encrypted storage state: uninitialized, locked, unlocked, key missing.
    pub storage: StorageStatus,
    pub autocorrect: AutocorrectStatus,
    /// Whether the local model is running, so the interface can explain the AI switch.
    pub ai_running: bool,
}

/// What one check found, plus what safe auto-correction would do with it.
#[derive(Clone, Debug, Serialize)]
pub struct CheckView {
    pub report: CheckReport,
    /// Corrections safe auto-correction may apply without asking; empty unless the user
    /// switched safe auto-correction on.
    pub auto_corrections: Vec<Correction>,
    /// Whether the requested origin is switched on in the settings.
    pub enabled_for_origin: bool,
}

/// How many batches one document can still undo.
#[derive(Clone, Debug, Serialize)]
pub struct UndoStatus {
    pub can_undo: bool,
    pub batches: usize,
    /// The words the next undo would put back.
    pub words: Vec<String>,
    pub capacity: usize,
}

/// A plain-text export of the user's own words.
#[derive(Clone, Debug, Serialize)]
pub struct WordExportResult {
    pub path: String,
    pub words: usize,
    /// Always true: a plain word list is not encrypted, and the interface says so.
    pub plaintext: bool,
}

/// An encrypted export of the user's own words.
#[derive(Clone, Debug, Serialize)]
pub struct DictionaryExportResult {
    pub path: String,
    pub records: usize,
}

/// Request to improve a text explicitly.
#[derive(Clone, Debug, Deserialize)]
pub struct ImproveTextRequest {
    pub text: String,
    pub mode: TextImprovementMode,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub persona: Persona,
    #[serde(default)]
    pub language: LanguageMode,
}

// ------------------------------------------------------------------- settings

/// Status of the spelling layer.
#[tauri::command(async)]
pub fn autocorrect_status(
    state: tauri::State<'_, AppState>,
) -> Result<AutocorrectStatusView, String> {
    let settings = state.autocorrect.autocorrect_settings();
    let checker = state.autocorrect.checker();
    let dictionaries = dictionary_states(checker.dictionaries());
    let unavailable = unavailable_languages(&dictionaries);
    let directory = checker.directory().to_path_buf();
    let storage = state
        .notes
        .with_session(|session| session.storage_status())?;
    let has_stored_words = state
        .notes
        .with_session(|session| Ok(session.has_autocorrect_records()))
        .unwrap_or(false);
    let ai_running = state.local_ai.gateway().state().accepts_generation();

    if !storage.is_unlocked() {
        // While locked nothing is decrypted: the user dictionary stays closed.
        return Ok(AutocorrectStatusView {
            storage,
            autocorrect: AutocorrectStatus::locked(
                settings,
                has_stored_words,
                dictionaries,
                directory,
                ai_running,
            ),
            ai_running,
        });
    }

    let stats = state
        .notes
        .with_autocorrect(|user| user.stats())
        .unwrap_or_default();
    Ok(AutocorrectStatusView {
        storage,
        autocorrect: AutocorrectStatus {
            unlocked: true,
            has_stored_words,
            settings,
            dictionaries,
            dictionary_dir: directory.display().to_string(),
            unavailable,
            user: stats,
            ai_improvement_available: ai_running,
        },
        ai_running,
    })
}

/// The stored settings.
#[tauri::command(async)]
pub fn autocorrect_get_settings(
    state: tauri::State<'_, AppState>,
) -> Result<AutocorrectSettings, String> {
    Ok(state.autocorrect.autocorrect_settings())
}

/// Stores the settings and reports the new status.
///
/// Settings are not secret, so this works while the encrypted storage is locked: that is
/// exactly when a user decides whether checking should be on at all.
#[tauri::command(async)]
pub fn autocorrect_update_settings(
    state: tauri::State<'_, AppState>,
    settings: AutocorrectSettings,
) -> Result<AutocorrectStatusView, String> {
    let normalized = settings.normalized();
    let encoded = normalized.to_json().map_err(describe)?;
    state
        .settings
        .write(AUTOCORRECT_SETTINGS_KEY, &encoded)
        .map_err(|error| {
            log::warn!("autocorrect: settings could not be saved: {error}");
            error
        })?;
    // A changed folder or a switched-off feature drops the parsed dictionaries and the
    // decrypted word list; the shared unlock state is kept, so notes, the vault, and
    // memory stay usable.
    state
        .autocorrect
        .replace_dictionaries(state.autocorrect.dictionary_directory());
    if !normalized.enabled {
        state.autocorrect.clear_journals();
        state
            .notes
            .with_session(|session| {
                session.drop_autocorrect();
                Ok(())
            })
            .ok();
    }
    autocorrect_status(state)
}

/// Re-reads the dictionary folder, for example after the user installed a pair.
#[tauri::command(async)]
pub fn autocorrect_reload_dictionaries(
    state: tauri::State<'_, AppState>,
) -> Result<AutocorrectStatusView, String> {
    state.autocorrect.reload_dictionaries();
    autocorrect_status(state)
}

// -------------------------------------------------------------------- checking

/// Checks a text and reports every word the checker cannot confirm.
///
/// The check runs against the dictionaries on disk and the user's own encrypted words.
/// While the storage is locked the words are unavailable and the check still runs against
/// the dictionaries alone; the report says so, so an incomplete result is never presented
/// as a complete one. Nothing is applied here: the interface decides, and safe
/// auto-correction only proposes what comes back in `auto_corrections`.
#[tauri::command(async)]
pub fn autocorrect_check(
    state: tauri::State<'_, AppState>,
    origin: CheckOrigin,
    text: String,
) -> Result<CheckView, String> {
    let settings = state.autocorrect.autocorrect_settings();
    if !origin.enabled_in(&settings) {
        // The user switched checks off for this part of the interface.
        return Ok(CheckView {
            report: CheckReport {
                enabled: false,
                version: text_version(&text),
                ..CheckReport::default()
            },
            auto_corrections: Vec::new(),
            enabled_for_origin: false,
        });
    }
    let checker = state.autocorrect.checker();
    let report = if state.notes.is_unlocked() {
        state
            .notes
            .with_autocorrect(|user| checker.check(Some(user), &text, &settings))?
    } else {
        // The word list is encrypted, so a locked storage is not a failure: the check
        // continues with the public dictionaries, and the report says that the user's own
        // words were left out.
        checker
            .check(None::<&mut EncryptedUserDictionary>, &text, &settings)
            .map_err(describe)?
    };
    let auto_corrections = LocalSpellChecker::safe_corrections(&report, &settings);
    Ok(CheckView {
        report,
        auto_corrections,
        enabled_for_origin: true,
    })
}

/// Suggestions for one word, for a context menu.
#[tauri::command(async)]
pub fn autocorrect_suggest(
    state: tauri::State<'_, AppState>,
    word: String,
) -> Result<Vec<Suggestion>, String> {
    let settings = state.autocorrect.autocorrect_settings();
    let checker = state.autocorrect.checker();
    if state.notes.is_unlocked() {
        state
            .notes
            .with_autocorrect(|user| checker.suggest(Some(user), &word, &settings))
    } else {
        checker
            .suggest(None::<&mut EncryptedUserDictionary>, &word, &settings)
            .map_err(describe)
    }
}

// ----------------------------------------------------------------- corrections

/// Applies the corrections the user accepted.
///
/// The text must still be the version that was checked: a stale request is refused, and a
/// correction whose text no longer matches is skipped and reported.
#[tauri::command(async)]
pub fn autocorrect_apply(
    state: tauri::State<'_, AppState>,
    scope: String,
    text: String,
    expected_version: Option<String>,
    corrections: Vec<Correction>,
) -> Result<CorrectionBatch, String> {
    let scope = normalize_scope(&scope)?;
    let batch = state.autocorrect.with_journal(&scope, |journal| {
        apply_corrections(&text, &corrections, expected_version.as_deref(), journal)
    });
    batch.map_err(describe)
}

/// Reverts the last applied batch of one document.
#[tauri::command(async)]
pub fn autocorrect_undo(
    state: tauri::State<'_, AppState>,
    scope: String,
    text: String,
) -> Result<UndoOutcome, String> {
    let scope = normalize_scope(&scope)?;
    let outcome = state
        .autocorrect
        .with_journal(&scope, |journal| undo_last(&text, journal));
    outcome.map_err(describe)
}

/// Whether the last change of one document can still be undone.
#[tauri::command(async)]
pub fn autocorrect_undo_status(
    state: tauri::State<'_, AppState>,
    scope: String,
) -> Result<UndoStatus, String> {
    let scope = normalize_scope(&scope)?;
    Ok(state.autocorrect.journal_status(&scope))
}

// -------------------------------------------------------------- user dictionary

/// Lists the user's own words. Requires an unlocked storage.
#[tauri::command(async)]
pub fn autocorrect_dictionary_list(
    state: tauri::State<'_, AppState>,
    query: UserDictionaryQuery,
) -> Result<Vec<UserDictionaryEntry>, String> {
    state.notes.with_autocorrect(|user| user.list(&query))
}

/// Adds one word to the user's dictionary.
#[tauri::command(async)]
pub fn autocorrect_dictionary_add(
    state: tauri::State<'_, AppState>,
    word: String,
    language: Language,
) -> Result<UserDictionaryEntry, String> {
    state
        .notes
        .with_autocorrect(|user| user.add(&word, language, false))
}

/// Removes one stored word.
#[tauri::command(async)]
pub fn autocorrect_dictionary_remove(
    state: tauri::State<'_, AppState>,
    id: uuid::Uuid,
) -> Result<(), String> {
    state.notes.with_autocorrect(|user| user.remove(id))
}

/// Silences a word for this unlocked session only.
///
/// Nothing is stored: "ignore once" stays a decision about the current text.
#[tauri::command(async)]
pub fn autocorrect_dictionary_ignore(
    state: tauri::State<'_, AppState>,
    word: String,
) -> Result<(), String> {
    state.notes.with_autocorrect(|user| user.ignore_word(&word))
}

/// Forgets a session ignore.
#[tauri::command(async)]
pub fn autocorrect_dictionary_unignore(
    state: tauri::State<'_, AppState>,
    word: String,
) -> Result<bool, String> {
    state
        .notes
        .with_autocorrect(|user| user.unignore_word(&word))
}

/// Words silenced for this session.
#[tauri::command(async)]
pub fn autocorrect_dictionary_ignored(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    state
        .notes
        .with_autocorrect(|user| Ok(user.ignored_words()))
}

/// Size and shape of the stored word list.
#[tauri::command(async)]
pub fn autocorrect_dictionary_stats(
    state: tauri::State<'_, AppState>,
) -> Result<UserDictionaryStats, String> {
    state.notes.with_autocorrect(|user| user.stats())
}

/// Imports words from a text file the user chooses.
#[tauri::command(async)]
pub fn autocorrect_dictionary_import_file(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    language: Language,
) -> Result<ImportOutcome, String> {
    let Some(source) = open_words_path(&app) else {
        return Ok(ImportOutcome::default());
    };
    let text = std::fs::read_to_string(&source).map_err(|_| describe(AutocorrectError::Io))?;
    let words = parse_word_list(&text);
    state
        .notes
        .with_autocorrect(|user| user.add_many(&words, language, true))
}

/// Writes the stored words to a plain text file the user chooses.
///
/// This is the one path that produces an unprotected file, it is never automatic, and the
/// interface warns that the result is not encrypted.
#[tauri::command(async)]
pub fn autocorrect_dictionary_export_file(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    confirmed: bool,
) -> Result<WordExportResult, String> {
    if !confirmed {
        return Err("an unprotected word list needs an explicit confirmation".to_string());
    }
    let Some(destination) = save_words_path(&app) else {
        return Ok(WordExportResult {
            path: String::new(),
            words: 0,
            plaintext: true,
        });
    };
    let words = state.notes.with_autocorrect(|user| user.export_words())?;
    let mut body = String::from("# JARVIS word list, one word per line, not encrypted\n");
    for word in &words {
        body.push_str(word);
        body.push('\n');
    }
    jarvis_core::fsutil::write_bytes_atomic(&destination, body.as_bytes())
        .map_err(|_| describe(AutocorrectError::Io))?;
    Ok(WordExportResult {
        path: destination.display().to_string(),
        words: words.len(),
        plaintext: true,
    })
}

/// Writes the stored words as ciphertext, for a backup the key can open.
#[tauri::command(async)]
pub fn autocorrect_dictionary_export_backup(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<DictionaryExportResult, String> {
    let Some(destination) = save_backup_path(&app) else {
        return Ok(DictionaryExportResult {
            path: String::new(),
            records: 0,
        });
    };
    let records = state.notes.with_autocorrect(|user| user.export_records())?;
    let envelope = DictionaryExportEnvelope {
        schema_version: DictionaryExportEnvelope::CURRENT_SCHEMA_VERSION,
        exported_at: now().to_rfc3339(),
        records: records.clone(),
    };
    let text =
        serde_json::to_string_pretty(&envelope).map_err(|_| describe(AutocorrectError::Io))?;
    jarvis_core::fsutil::write_bytes_atomic(&destination, text.as_bytes())
        .map_err(|_| describe(AutocorrectError::Io))?;
    Ok(DictionaryExportResult {
        path: destination.display().to_string(),
        records: records.len(),
    })
}

/// Re-applies an encrypted word-list export.
#[tauri::command(async)]
pub fn autocorrect_dictionary_import_backup(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<DictionaryImportOutcome, String> {
    let Some(source) = open_backup_path(&app) else {
        return Ok(DictionaryImportOutcome::default());
    };
    let text = std::fs::read_to_string(&source).map_err(|_| describe(AutocorrectError::Io))?;
    let envelope: DictionaryExportEnvelope =
        serde_json::from_str(&text).map_err(|_| describe(AutocorrectError::Io))?;
    if envelope.schema_version != DictionaryExportEnvelope::CURRENT_SCHEMA_VERSION {
        return Err(describe(AutocorrectError::InvalidConfiguration));
    }
    state
        .notes
        .with_autocorrect(|user| user.import_records(&envelope.records))
}

/// An encrypted export of the user's own words.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DictionaryExportEnvelope {
    pub schema_version: u32,
    pub exported_at: String,
    pub records: Vec<ExportedDictionaryRecord>,
}

impl DictionaryExportEnvelope {
    /// Version of the envelope this build writes.
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

// ------------------------------------------------------------ AI improvement

/// Produces a preview of an AI rewrite. Nothing is applied by this command.
///
/// The text is checked by the secret filter before it is sent, the model is asked for the
/// visible answer only, and the result is returned to the interface as a difference the
/// user has to confirm.
#[tauri::command]
pub async fn autocorrect_improve_text(
    state: tauri::State<'_, AppState>,
    request: ImproveTextRequest,
) -> Result<TextImprovementPreview, String> {
    let settings = state.autocorrect.autocorrect_settings();
    if !settings.offers_ai_improvement() {
        return Err("the AI text improvement is switched off in the settings".to_string());
    }
    if request.text.chars().count() > MAX_IMPROVE_CHARS {
        return Err(describe(AutocorrectError::TextTooLarge {
            limit: MAX_IMPROVE_CHARS,
        }));
    }
    let gateway = state.local_ai.shared();
    let core_request = TextImprovementRequest {
        expected_version: text_version(&request.text),
        text: request.text,
        mode: request.mode,
        instruction: request.instruction,
        persona: request.persona,
        language: request.language,
    };
    let prepared = core_request.clone();
    match tauri::async_runtime::spawn_blocking(move || {
        let provider = LocalAiTextImprover::new(gateway);
        preview_improvement(&provider, &prepared)
    })
    .await
    {
        Ok(result) => result.map_err(describe),
        Err(_) => Err(describe(AutocorrectError::AiUnavailable)),
    }
}

/// Stops a running AI improvement. Returns whether a generation was running.
#[tauri::command(async)]
pub fn autocorrect_cancel_improvement(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    // Only one generation runs at a time, and this cancels the one in flight: the
    // improvement command is waiting on it, so the cancel is exact.
    Ok(state.local_ai.gateway().cancel())
}

/// Applies an AI preview the user confirmed.
///
/// The text must still be the one the preview was computed from. The change is journalled
/// like a spelling correction, so it can be undone.
#[tauri::command(async)]
pub fn autocorrect_apply_improvement(
    state: tauri::State<'_, AppState>,
    scope: String,
    text: String,
    preview: TextImprovementPreview,
    expected_version: Option<String>,
) -> Result<CorrectionBatch, String> {
    let scope = normalize_scope(&scope)?;
    let batch = state.autocorrect.with_journal(&scope, |journal| {
        apply_improvement(&text, &preview, expected_version.as_deref(), journal)
    });
    batch.map_err(describe)
}

/// Adds a replacement rule the user wrote.
#[tauri::command(async)]
pub fn autocorrect_rule_add(
    state: tauri::State<'_, AppState>,
    pattern: String,
    replacement: String,
    auto_apply: bool,
) -> Result<AutocorrectSettings, String> {
    let mut settings = state.autocorrect.autocorrect_settings();
    let rule = CustomRule::new(&pattern, &replacement, auto_apply).map_err(describe)?;
    settings.custom_rules.push(rule);
    let normalized = settings.normalized();
    let encoded = normalized.to_json().map_err(describe)?;
    state
        .settings
        .write(AUTOCORRECT_SETTINGS_KEY, &encoded)
        .map_err(|error| {
            log::warn!("autocorrect: settings could not be saved: {error}");
            error
        })?;
    Ok(normalized)
}

/// Removes a replacement rule by its position.
#[tauri::command(async)]
pub fn autocorrect_rule_remove(
    state: tauri::State<'_, AppState>,
    index: usize,
) -> Result<AutocorrectSettings, String> {
    let mut settings = state.autocorrect.autocorrect_settings();
    if index >= settings.custom_rules.len() {
        return Err(describe(AutocorrectError::NotFound));
    }
    settings.custom_rules.remove(index);
    let normalized = settings.normalized();
    let encoded = normalized.to_json().map_err(describe)?;
    state
        .settings
        .write(AUTOCORRECT_SETTINGS_KEY, &encoded)
        .map_err(|error| {
            log::warn!("autocorrect: settings could not be saved: {error}");
            error
        })?;
    Ok(normalized)
}

// ------------------------------------------------------------------- pickers

fn open_words_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("word list", &["txt", "dic"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}

fn save_words_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name("jarvis-words.txt")
        .add_filter("text", &["txt"])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}

fn save_backup_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name("jarvis-word-list-export.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}

fn open_backup_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("JSON", &["json"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}
