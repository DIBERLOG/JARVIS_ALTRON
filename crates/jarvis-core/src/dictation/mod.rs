//! Global voice input: say the phrase, speak, and the text appears where the
//! cursor was.
//!
//! # The route
//!
//! ```text
//! Vosk intent ─┐
//!              ├─► GlobalDictationEngine ─► VoiceHost   (Vosk stops, microphone released)
//! confirmation ┘                          ─► Dictation   (the existing Whisper session)
//!                                         ─► Corrector   (the local spelling layer)
//!                                         ─► punctuation (voice punctuation, local)
//!                                         ─► probe       (is the window still the same?)
//!                                         ─► deliver     (UI Automation, or the clipboard)
//!                                         ─► VoiceHost   (Vosk starts again)
//! ```
//!
//! # What this module refuses to do
//!
//! * it does not open a second microphone, a second recorder, or a second Whisper
//!   session: the host that owns the microphone is a parameter, and the voice
//!   path is one object;
//! * it does not type into anything it has not checked. Every insertion goes
//!   through [`target::decide`], which refuses password fields, read-only fields,
//!   disabled and unknown elements, a secure desktop, an elevated target the
//!   application cannot reach, and this application's own windows;
//! * it does not use a shell, a command line, or another process. There is no
//!   path here that could run anything;
//! * it does not send the text to the local model. An improvement pass is a
//!   separate, explicit action with a preview, and it is not part of this route;
//! * it does not log the text, the field, the window title, the clipboard, or a
//!   file name. The log gets stages, lengths, the delivery method, and a code.

pub mod error;
pub mod insertion;
pub mod punctuation;
pub mod session;
pub mod target;

#[cfg(test)]
mod tests;

pub use error::DictationError;
pub use insertion::{
    deliver, ClipboardWriter, Delivery, DeliveryMethod, ForegroundProbe, TextInserter,
    UnavailableProbe,
};
pub use punctuation::{apply_voice_punctuation, PunctuationReport};
pub use session::{
    status_of, DictationEngine, DictationOutcome, DictationRequest, DictationStage,
    DictationStatusView, EngineDeps, TextCorrector, TranscribedText, VoiceHost, VoiceTranscriber,
};
pub use target::{
    decide, ElementKind, InsertionDecision, TargetSnapshot, TargetVerdict, VoiceInputPreference,
    WindowIdentity, WINDOW_TOLERANCE,
};

/// The phrases that start global dictation, and the answer to them.
///
/// Kept here as data so the voice layer, the settings page and the tests all
/// read the same list. The phrases are matched after normalization: lower case,
/// no punctuation, collapsed whitespace.
pub const START_PHRASES: [&str; 7] = [
    "джарвис голосовой ввод",
    "джарвис начни голосовой ввод",
    "джарвис включи диктовку",
    "джарвис продиктую текст",
    "jarvis voice input",
    "jarvis start voice input",
    "jarvis диктовка",
];

/// What a recognized phrase means, as a type.
///
/// The voice layer produces one of these instead of a string that something
/// further down might hand to a command runner. A phrase that is not one of the
/// two intents is `None`, and nothing else can be expressed — there is no variant
/// that carries a command, a path, or an argument, so a misunderstood phrase
/// cannot become an action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoiceIntent {
    /// Start global dictation.
    StartGlobalDictation,
    /// Stop the recording and take the text.
    StopDictation,
    /// Anything else: the phrase is left to the rest of the assistant.
    None,
}

impl VoiceIntent {
    /// A stable name, for the log and the tray. Content-free by construction.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StartGlobalDictation => "start_global_dictation",
            Self::StopDictation => "stop_dictation",
            Self::None => "none",
        }
    }
}

/// Turns a recognized phrase into a typed intent.
///
/// This is the whole trigger: data in, an enum out. The voice host calls it with
/// what Vosk recognized and acts on the variant; there is no string command, no
/// shell and no path anywhere in the route, and the phrase never reaches the
/// local model.
pub fn intent_of(text: &str) -> VoiceIntent {
    if is_start_request(text) {
        VoiceIntent::StartGlobalDictation
    } else if is_stop_request(text) {
        VoiceIntent::StopDictation
    } else {
        VoiceIntent::None
    }
}

/// The same match, with the settings of the feature applied.
///
/// A phrase the person configured is an intent for that configuration; the
/// built-in list is used when the feature is switched on.
pub fn intent_with_settings(settings: &GlobalDictationSettings, text: &str) -> VoiceIntent {
    if !settings.enabled {
        return VoiceIntent::None;
    }
    if settings.matches(text) {
        VoiceIntent::StartGlobalDictation
    } else if is_stop_request(text) {
        VoiceIntent::StopDictation
    } else {
        VoiceIntent::None
    }
}

/// The confirmation the assistant says, per language.
pub fn confirmation(language: &str) -> &'static str {
    match language {
        "ru" => "Да, сэр. Начинаю голосовой ввод",
        "ua" => "Так, сер. Починаю голосове введення",
        _ => "Yes, sir. Starting voice input",
    }
}

/// Normalizes one phrase the way the intent match needs it.
pub fn normalize_phrase(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character.to_lowercase().next().unwrap_or(character)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a recognized phrase is a request to start global dictation.
pub fn is_start_request(text: &str) -> bool {
    let normalized = normalize_phrase(text);
    START_PHRASES
        .iter()
        .any(|phrase| normalize_phrase(phrase) == normalized)
}

/// Whether a recognized phrase is a request to stop and take the text.
pub fn is_stop_request(text: &str) -> bool {
    let normalized = normalize_phrase(text);
    [
        "стоп",
        "остановись",
        "хватит",
        "stop",
        "stop dictation",
        "готово",
    ]
    .iter()
    .any(|phrase| normalize_phrase(phrase) == normalized)
}

/// The settings of the feature, as stored.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GlobalDictationSettings {
    /// Off until the person asks for it. Nothing listens for a phrase otherwise.
    pub enabled: bool,
    /// The phrase that starts it, as the person wrote it.
    pub phrase: String,
    /// Whether the confirmation is spoken.
    pub speak_confirmation: bool,
    /// Language hint for the model, `auto` for the model's own choice.
    pub language: String,
    /// Whether the local spelling layer touches the result.
    pub autocorrect: bool,
    /// Whether the spoken marks become marks.
    pub punctuation: bool,
    /// Where the text goes.
    pub preference: VoiceInputPreference,
    /// How long a copied text stays on the clipboard before it is wiped.
    pub clipboard_seconds: u64,
    /// Whether the text is shown for confirmation before it is inserted.
    pub preview_before_insert: bool,
}

impl Default for GlobalDictationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            phrase: START_PHRASES[0].to_string(),
            speak_confirmation: true,
            language: "auto".to_string(),
            autocorrect: true,
            punctuation: true,
            // The production mode of this build: the clipboard, and the person
            // pastes. UI Automation is the experimental extension.
            preference: VoiceInputPreference::Clipboard,
            clipboard_seconds: crate::vault::clipboard::DEFAULT_CLEAR_SECONDS,
            preview_before_insert: false,
        }
    }
}

impl GlobalDictationSettings {
    /// Repairs what a settings document could get wrong.
    pub fn normalized(mut self) -> Self {
        if self.phrase.trim().is_empty() {
            self.phrase = START_PHRASES[0].to_string();
        }
        self.phrase = self.phrase.trim().to_string();
        self.clipboard_seconds =
            crate::vault::clipboard::clamp_clear_seconds(self.clipboard_seconds);
        if !crate::whisper::LANGUAGES.contains(&self.language.as_str()) {
            self.language = "auto".to_string();
        }
        self
    }

    /// The phrase this configuration answers to, as data.
    pub fn phrases(&self) -> Vec<String> {
        let mut phrases: Vec<String> = START_PHRASES.iter().map(|p| p.to_string()).collect();
        let custom = normalize_phrase(&self.phrase);
        if !custom.is_empty() && !phrases.iter().any(|p| normalize_phrase(p) == custom) {
            phrases.push(self.phrase.clone());
        }
        phrases
    }

    /// Whether a recognized phrase starts the feature for this configuration.
    pub fn matches(&self, text: &str) -> bool {
        self.enabled
            && self
                .phrases()
                .iter()
                .any(|phrase| normalize_phrase(phrase) == normalize_phrase(text))
    }
}
