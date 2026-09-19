//! The desktop shell: what the window does when it is closed, what is written
//! into autostart, what the tray says, and where the first-run wizard got to.
//!
//! Everything here is state and decisions — no window, no tray handle, no
//! registry call. The platform parts live behind one trait
//! ([`AutostartBackend`]) so every rule below is tested without a desktop, and
//! the Tauri layer only renders what this module decides.
//!
//! The rules this module exists to keep:
//!
//! * **autostart is off, and stays off until a person turns it on.** Nothing
//!   writes a Run entry because the application was launched;
//! * **autostart never starts a microphone.** `start_local_ai` and `start_vosk`
//!   are separate switches, both off; Whisper has no switch at all, because a
//!   dictation session is something a person starts;
//! * **the close button asks by default.** Hiding a window is a decision the
//!   user makes once and understands, not a surprise;
//! * **there is one microphone state, and stale events are ignored.** The tray,
//!   the window, and the exit all read the same value, and an event from a
//!   session that has already ended cannot put the tray into "recording";
//! * **the wizard's progress is versioned.** A single boolean cannot be
//!   migrated; `setup_version` plus the completed and skipped steps can.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// File name of the desktop settings document inside the feature directory.
pub const DESKTOP_SETTINGS_FILE: &str = "desktop.json";
/// Schema version of the desktop settings document.
pub const DESKTOP_SCHEMA_VERSION: u32 = 1;
/// File name of the first-run state inside the feature directory.
pub const SETUP_STATE_FILE: &str = "setup.json";
/// Version of the wizard. Bumping it makes the wizard run again on the next
/// start, which is how a new step is introduced without a boolean migration.
pub const SETUP_VERSION: u32 = 1;

/// What the close button does.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    /// Hide the window and keep running in the tray.
    Tray,
    /// Run the full exit.
    Exit,
    /// Ask, every time, until the user says "remember".
    #[default]
    Ask,
}

impl CloseBehavior {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tray => "tray",
            Self::Exit => "exit",
            Self::Ask => "ask",
        }
    }

    /// The Fluent key of the choice, for the dialog and the settings page.
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Tray => "desktop-close-tray",
            Self::Exit => "desktop-close-exit",
            Self::Ask => "desktop-close-ask",
        }
    }

    /// Whether a close event can be answered without asking.
    pub fn decides_by_itself(&self) -> bool {
        !matches!(self, Self::Ask)
    }
}

/// What is written into autostart, and what the tray shows afterwards.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopSettings {
    pub close_behavior: CloseBehavior,
    /// Whether the application may start with Windows. Off until asked for.
    pub autostart_enabled: bool,
    /// Whether an autostart launch begins hidden in the tray.
    pub start_minimized: bool,
    /// Whether an autostart launch starts the managed model server.
    pub start_local_ai: bool,
    /// Whether an autostart launch starts Vosk.
    pub start_vosk: bool,
    /// Whether the user has been told what hiding the window means.
    pub tray_explained: bool,
    pub schema_version: u32,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            close_behavior: CloseBehavior::Ask,
            autostart_enabled: false,
            start_minimized: false,
            // Nothing heavy starts by itself: a login is not a request to load a
            // model or open a microphone.
            start_local_ai: false,
            start_vosk: false,
            tray_explained: false,
            schema_version: DESKTOP_SCHEMA_VERSION,
        }
    }
}

impl DesktopSettings {
    /// Repairs a damaged document instead of refusing to load.
    pub fn normalized(mut self) -> Self {
        self.schema_version = DESKTOP_SCHEMA_VERSION;
        self
    }

    /// What an autostart launch is allowed to start.
    ///
    /// Whisper is not in this list, and there is no switch that would put it
    /// there: a dictation session is started by a person, never by a login.
    pub fn autostart_actions(&self) -> AutostartActions {
        AutostartActions {
            start_minimized: self.start_minimized,
            start_local_ai: self.autostart_enabled && self.start_local_ai,
            start_vosk: self.autostart_enabled && self.start_vosk,
            start_whisper: false,
        }
    }

    pub fn to_json(&self) -> Result<String, DesktopError> {
        serde_json::to_string(self).map_err(|_| DesktopError::Storage)
    }

    pub fn from_json(text: &str) -> Result<Self, DesktopError> {
        serde_json::from_str(text).map_err(|_| DesktopError::DamagedSettings)
    }

    /// Reads the stored document, falling back to the safe defaults.
    pub fn load_or_default(text: Option<&str>) -> Self {
        text.and_then(|text| Self::from_json(text).ok())
            .unwrap_or_default()
            .normalized()
    }
}

/// What a launch with Windows is allowed to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutostartActions {
    pub start_minimized: bool,
    pub start_local_ai: bool,
    pub start_vosk: bool,
    /// Always false. The field exists so that a caller cannot forget the rule.
    pub start_whisper: bool,
}

impl AutostartActions {
    /// Whether anything other than the tray icon is started.
    pub fn starts_anything(&self) -> bool {
        self.start_local_ai || self.start_vosk
    }
}

/// The state of the autostart entry as the platform reports it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AutostartStatus {
    /// Whether an entry exists at all.
    pub entry_present: bool,
    /// Whether the entry points at *this* executable.
    pub path_matches: bool,
    /// The command that is written, when the platform can report it.
    pub command: Option<String>,
    /// Why the platform refused, in one short sentence.
    pub error: Option<String>,
}

impl AutostartStatus {
    /// Whether the setting and the machine agree.
    pub fn is_enabled(&self) -> bool {
        self.entry_present && self.path_matches
    }

    /// What the interface should say.
    pub fn state(&self) -> AutostartState {
        if self.error.is_some() {
            return AutostartState::Unavailable;
        }
        match (self.entry_present, self.path_matches) {
            (false, _) => AutostartState::Disabled,
            (true, true) => AutostartState::Enabled,
            // The entry exists and points somewhere else: the application was
            // moved, or another copy wrote it.
            (true, false) => AutostartState::NeedsAttention,
        }
    }

    /// Repairs an entry that points at a path the application no longer uses.
    pub fn needs_repair(&self) -> bool {
        self.entry_present && !self.path_matches && self.error.is_none()
    }
}

/// The three answers the interface shows for autostart.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutostartState {
    Enabled,
    #[default]
    Disabled,
    /// An entry exists and does not match this executable.
    NeedsAttention,
    /// The platform refused, usually because a policy forbids the Run key.
    Unavailable,
}

impl AutostartState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::NeedsAttention => "needs_attention",
            Self::Unavailable => "unavailable",
        }
    }
}

/// The platform part of autostart: one entry, per user, no shell.
pub trait AutostartBackend: Send + Sync {
    /// Writes the entry so that `program` starts with `arguments`.
    fn enable(&self, program: &Path, arguments: &[String]) -> Result<(), String>;
    /// Removes the entry. Removing what is not there is not an error.
    fn disable(&self) -> Result<(), String>;
    /// Reads the entry back from the platform.
    fn status(&self) -> Result<AutostartStatus, String>;
}

/// An autostart backend that does nothing but remember, for tests and for a
/// platform that has no supported mechanism.
#[derive(Default)]
pub struct FakeAutostart {
    /// What is written, as `(program, arguments)`.
    entry: Mutex<Option<(PathBuf, Vec<String>)>>,
    /// What `status` reports, for a test that wants a mismatch or a refusal.
    forced_status: Mutex<Option<AutostartStatus>>,
    /// When set, every call fails with this message.
    failure: Mutex<Option<String>>,
}

impl FakeAutostart {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the platform refuse, the way a group policy does.
    pub fn failing(self, message: &str) -> Self {
        *self.failure.lock() = Some(message.to_string());
        self
    }

    /// Makes `status` report something other than what was written.
    pub fn with_status(self, status: AutostartStatus) -> Self {
        *self.forced_status.lock() = Some(status);
        self
    }

    pub fn written(&self) -> Option<(PathBuf, Vec<String>)> {
        self.entry.lock().clone()
    }
}

impl AutostartBackend for FakeAutostart {
    fn enable(&self, program: &Path, arguments: &[String]) -> Result<(), String> {
        if let Some(message) = self.failure.lock().clone() {
            return Err(message);
        }
        *self.entry.lock() = Some((program.to_path_buf(), arguments.to_vec()));
        // A real platform now reports what was written, so a forced status is
        // cleared: otherwise a test could not tell a repair from a refusal.
        *self.forced_status.lock() = None;
        Ok(())
    }

    fn disable(&self) -> Result<(), String> {
        if let Some(message) = self.failure.lock().clone() {
            return Err(message);
        }
        *self.entry.lock() = None;
        *self.forced_status.lock() = None;
        Ok(())
    }

    fn status(&self) -> Result<AutostartStatus, String> {
        if let Some(message) = self.failure.lock().clone() {
            return Err(message);
        }
        if let Some(status) = self.forced_status.lock().clone() {
            return Ok(status);
        }
        match self.entry.lock().clone() {
            Some((program, _)) => Ok(AutostartStatus {
                entry_present: true,
                path_matches: true,
                command: Some(program.to_string_lossy().into_owned()),
                error: None,
            }),
            None => Ok(AutostartStatus::default()),
        }
    }
}

/// The arguments an autostart entry carries.
///
/// They say "start hidden" and nothing else: which services start is decided by
/// the settings document, so a change there does not need the entry rewritten.
pub const START_MINIMIZED_FLAG: &str = "--start-minimized";

/// The autostart setting, kept in step with the platform.
pub struct AutostartController {
    backend: Box<dyn AutostartBackend>,
    program: PathBuf,
}

impl AutostartController {
    pub fn new(backend: Box<dyn AutostartBackend>, program: impl Into<PathBuf>) -> Self {
        Self {
            backend,
            program: program.into(),
        }
    }

    /// What the platform says, with a failure turned into a status rather than
    /// an error, because a settings page has to render something.
    pub fn status(&self) -> AutostartStatus {
        match self.backend.status() {
            Ok(status) => status,
            Err(message) => AutostartStatus {
                error: Some(crate::text::shorten(&message, 120)),
                ..AutostartStatus::default()
            },
        }
    }

    /// Turns autostart on for this executable.
    pub fn enable(&self) -> Result<AutostartStatus, DesktopError> {
        self.backend
            .enable(&self.program, &[START_MINIMIZED_FLAG.to_string()])
            .map_err(|message| {
                DesktopError::AutostartRefused(crate::text::shorten(&message, 120))
            })?;
        Ok(self.status())
    }

    pub fn disable(&self) -> Result<AutostartStatus, DesktopError> {
        self.backend.disable().map_err(|message| {
            DesktopError::AutostartRefused(crate::text::shorten(&message, 120))
        })?;
        Ok(self.status())
    }

    /// Rewrites the entry when it points at a path this copy no longer uses,
    /// which is what an update or a move leaves behind.
    pub fn repair_if_needed(&self) -> Result<Option<AutostartStatus>, DesktopError> {
        let status = self.status();
        if !status.needs_repair() {
            return Ok(None);
        }
        self.enable().map(Some)
    }
}

/// Whether the microphone is in use, and by what.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MicrophoneState {
    #[default]
    Idle,
    /// The wake-word listener is running.
    VoskListening,
    /// A dictation recording is in progress.
    WhisperDictation,
    /// An audio file is being transcribed; the microphone is not open.
    TranscribingFile,
    /// Something is stopping; the microphone may still be open for a moment.
    Stopping,
    /// The last attempt failed.
    Failed,
}

impl MicrophoneState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::VoskListening => "vosk_listening",
            Self::WhisperDictation => "whisper_dictation",
            Self::TranscribingFile => "transcribing_file",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
        }
    }

    /// Whether the microphone itself is open in this state.
    pub fn holds_microphone(&self) -> bool {
        matches!(
            self,
            Self::VoskListening | Self::WhisperDictation | Self::Stopping
        )
    }

    /// Whether a person looking at the tray should see an indicator.
    pub fn needs_indicator(&self) -> bool {
        self.holds_microphone()
    }

    /// The Fluent key the tray tooltip and the panel use.
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Idle => "desktop-mic-idle",
            Self::VoskListening => "desktop-mic-vosk",
            Self::WhisperDictation => "desktop-mic-whisper",
            Self::TranscribingFile => "desktop-mic-file",
            Self::Stopping => "desktop-mic-stopping",
            Self::Failed => "desktop-mic-failed",
        }
    }

    /// Whether the user can stop what is happening.
    pub fn can_stop(&self) -> bool {
        matches!(
            self,
            Self::VoskListening | Self::WhisperDictation | Self::TranscribingFile
        )
    }
}

/// The single source of truth for the microphone.
///
/// Every session takes a ticket when it starts and has to present it to change
/// the state, so an event from a session that has already ended cannot make the
/// tray claim that a microphone is open.
#[derive(Default)]
pub struct AudioSession {
    state: Mutex<MicrophoneState>,
    generation: Mutex<u64>,
}

impl AudioSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> MicrophoneState {
        *self.state.lock()
    }

    /// Starts a session and returns its ticket.
    ///
    /// A second session is refused while one is holding the microphone: two
    /// consumers of one device is a bug, not a feature.
    pub fn begin(&self, intended: MicrophoneState) -> Result<SessionTicket, DesktopError> {
        let mut state = self.state.lock();
        if state.holds_microphone() && *state != MicrophoneState::Failed {
            return Err(DesktopError::MicrophoneBusy);
        }
        let mut generation = self.generation.lock();
        *generation += 1;
        let ticket = SessionTicket(*generation);
        *state = intended;
        Ok(ticket)
    }

    /// The states a running session may move to, and only with its own ticket.
    pub fn update(&self, ticket: SessionTicket, next: MicrophoneState) -> Result<(), DesktopError> {
        if !self.is_current(ticket) {
            return Err(DesktopError::StaleSession);
        }
        *self.state.lock() = next;
        Ok(())
    }

    /// Ends a session. A stale ticket changes nothing and is not an error the
    /// user has to see: the session it belonged to is already over.
    pub fn end(&self, ticket: SessionTicket) {
        if !self.is_current(ticket) {
            return;
        }
        *self.state.lock() = MicrophoneState::Idle;
    }

    /// Whether the ticket still belongs to the running session.
    pub fn is_current(&self, ticket: SessionTicket) -> bool {
        *self.generation.lock() == ticket.0
    }

    /// Releases the microphone whatever state it is in, for a full exit.
    pub fn release_for_exit(&self) {
        let mut generation = self.generation.lock();
        *generation += 1;
        *self.state.lock() = MicrophoneState::Idle;
    }
}

/// A ticket that identifies one audio session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionTicket(u64);

impl SessionTicket {
    pub fn generation(&self) -> u64 {
        self.0
    }
}

/// Everything the tray menu is built from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesktopSnapshot {
    /// Whether the window is currently visible.
    pub window_visible: bool,
    /// Local AI: not configured, stopped, ready, or busy.
    pub ai_state: AiSnapshot,
    pub microphone: MicrophoneState,
    pub vault_unlocked: bool,
    /// How many timers and reminders are waiting.
    pub pending_timers: usize,
    pub whisper_configured: bool,
    pub vosk_running: bool,
    /// Whether autostart is on, as the platform reports it.
    pub autostart: AutostartState,
}

/// The local AI, as far as the tray is concerned.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AiSnapshot {
    #[default]
    NotConfigured,
    Stopped,
    Ready,
    Busy,
}

impl AiSnapshot {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Stopped => "stopped",
            Self::Ready => "ready",
            Self::Busy => "busy",
        }
    }

    /// The Fluent key of the one-line status the menu shows.
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::NotConfigured => "desktop-ai-not-configured",
            Self::Stopped => "desktop-ai-stopped",
            Self::Ready => "desktop-ai-ready",
            Self::Busy => "desktop-ai-busy",
        }
    }
}

/// One row of the tray menu.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrayMenuRow {
    /// Stable identifier the Tauri layer dispatches on.
    pub id: &'static str,
    /// Fluent key of the label. A status row carries its value separately.
    pub label_key: &'static str,
    pub enabled: bool,
    /// A literal value shown next to the label: a count, or a state word.
    pub value: Option<String>,
    /// Whether the row is a separator instead of an item.
    pub separator: bool,
}

impl TrayMenuRow {
    fn item(id: &'static str, label_key: &'static str, enabled: bool) -> Self {
        Self {
            id,
            label_key,
            enabled,
            value: None,
            separator: false,
        }
    }

    fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    fn separator() -> Self {
        Self {
            id: "separator",
            label_key: "",
            enabled: false,
            value: None,
            separator: true,
        }
    }
}

/// Builds the tray menu from a snapshot.
///
/// The order is the contract: the tests and the Tauri layer both read it, so a
/// row cannot appear in one place and not the other.
pub fn tray_menu(snapshot: &DesktopSnapshot) -> Vec<TrayMenuRow> {
    use MicrophoneState as Mic;

    let mut rows = Vec::with_capacity(14);
    rows.push(TrayMenuRow::item(
        "open",
        if snapshot.window_visible {
            "desktop-menu-focus"
        } else {
            "desktop-menu-open"
        },
        true,
    ));
    rows.push(TrayMenuRow::item(
        "hide",
        "desktop-menu-hide",
        snapshot.window_visible,
    ));
    rows.push(TrayMenuRow::separator());

    // The AI row is a status, not a button: it says what is true right now.
    rows.push(
        TrayMenuRow::item("ai_status", "desktop-menu-ai", false)
            .with_value(snapshot.ai_state.label_key()),
    );

    // Vosk: one row that starts or stops, and says which it will do.
    let vosk_label = if snapshot.vosk_running {
        "desktop-menu-vosk-stop"
    } else {
        "desktop-menu-vosk-start"
    };
    rows.push(TrayMenuRow::item(
        "vosk_toggle",
        vosk_label,
        // Starting is refused while dictation holds the microphone.
        snapshot.vosk_running || !matches!(snapshot.microphone, Mic::WhisperDictation),
    ));

    // Dictation: two explicit rows, so the state is never guessed from one
    // label. Starting needs a configured Whisper and a free microphone.
    let can_start = snapshot.whisper_configured
        && matches!(snapshot.microphone, Mic::Idle | Mic::Failed)
        && !snapshot.vosk_running;
    rows.push(TrayMenuRow::item(
        "dictate_start",
        "desktop-menu-dictate-start",
        can_start,
    ));
    rows.push(TrayMenuRow::item(
        "dictate_stop",
        "desktop-menu-dictate-stop",
        snapshot.microphone.can_stop(),
    ));

    // The microphone row is the visible indicator: it is a status row, and it
    // is what makes a hidden window honest about holding the device.
    rows.push(
        TrayMenuRow::item("microphone", "desktop-menu-microphone", false)
            .with_value(snapshot.microphone.label_key()),
    );
    rows.push(TrayMenuRow::separator());

    // Timers: a count, so "0" is visible rather than an empty row.
    rows.push(
        TrayMenuRow::item("timers", "desktop-menu-timers", false)
            .with_value(snapshot.pending_timers.to_string()),
    );

    // Locking the stores is only offered when there is something to lock.
    rows.push(TrayMenuRow::item(
        "lock_storage",
        "desktop-menu-lock",
        snapshot.vault_unlocked,
    ));
    rows.push(TrayMenuRow::separator());

    rows.push(TrayMenuRow::item("settings", "desktop-menu-settings", true));
    // Exit is always available, whatever else is going on: a person must always
    // be able to stop the application from the tray.
    rows.push(TrayMenuRow::item("exit", "desktop-menu-exit", true));
    rows
}

/// The tray tooltip, which is where a hidden window says what it is doing.
pub fn tray_tooltip(snapshot: &DesktopSnapshot) -> String {
    // The tooltip is a plain string, so it uses short words rather than Fluent
    // keys: the Tauri layer cannot translate it without the interface running.
    let microphone = match snapshot.microphone {
        MicrophoneState::Idle => "microphone idle",
        MicrophoneState::VoskListening => "listening",
        MicrophoneState::WhisperDictation => "recording",
        MicrophoneState::TranscribingFile => "transcribing",
        MicrophoneState::Stopping => "stopping",
        MicrophoneState::Failed => "microphone failed",
    };
    let ai = match snapshot.ai_state {
        AiSnapshot::NotConfigured => "AI not configured",
        AiSnapshot::Stopped => "AI stopped",
        AiSnapshot::Ready => "AI ready",
        AiSnapshot::Busy => "AI working",
    };
    let timers = if snapshot.pending_timers == 0 {
        "no timers".to_string()
    } else {
        format!("{} timer(s)", snapshot.pending_timers)
    };
    format!("JARVIS — {ai}, {microphone}, {timers}")
}

/// Where the first-run wizard got to.
///
/// Versioned on purpose: `setup_version` is compared against [`SETUP_VERSION`],
/// so adding a step to the wizard asks every existing profile to see it once,
/// while a single boolean would have to be migrated by hand.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetupState {
    pub setup_version: u32,
    /// RFC 3339, when the wizard was finished.
    pub completed_at: Option<String>,
    pub completed_steps: Vec<String>,
    pub skipped_steps: Vec<String>,
    /// The language chosen in the first step, if it was chosen.
    pub language: Option<String>,
}

/// The steps of the wizard, in order. The list is the contract the interface
/// renders and the tests check.
pub const SETUP_STEPS: [&str; 10] = [
    "language",
    "storage",
    "local_ai",
    "whisper",
    "microphone",
    "vosk",
    "dictionaries",
    "windows_actions",
    "autostart",
    "diagnostics",
];

impl SetupState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the wizard has to run on this start.
    pub fn needs_wizard(&self) -> bool {
        self.setup_version < SETUP_VERSION || self.completed_at.is_none()
    }

    /// Whether every step is either done or skipped.
    pub fn is_complete(&self) -> bool {
        SETUP_STEPS.iter().all(|step| self.is_step_handled(step))
    }

    pub fn is_step_handled(&self, step: &str) -> bool {
        self.completed_steps.iter().any(|done| done == step)
            || self.skipped_steps.iter().any(|skipped| skipped == step)
    }

    pub fn is_step_skipped(&self, step: &str) -> bool {
        self.skipped_steps.iter().any(|skipped| skipped == step)
    }

    /// Marks a step done. A step that was skipped and is then done is no longer
    /// skipped: the later answer is the one that counts.
    pub fn complete_step(&mut self, step: &str) -> Result<(), DesktopError> {
        self.check_step(step)?;
        self.skipped_steps.retain(|skipped| skipped != step);
        if !self.completed_steps.iter().any(|done| done == step) {
            self.completed_steps.push(step.to_string());
        }
        Ok(())
    }

    /// Marks a step skipped, which is always allowed.
    pub fn skip_step(&mut self, step: &str) -> Result<(), DesktopError> {
        self.check_step(step)?;
        self.completed_steps.retain(|done| done != step);
        if !self.skipped_steps.iter().any(|skipped| skipped == step) {
            self.skipped_steps.push(step.to_string());
        }
        Ok(())
    }

    /// Finishes the wizard, whatever is left unfinished.
    ///
    /// A step the user never saw is recorded as skipped rather than left
    /// unhandled: the summary page has to be able to say "not configured", and
    /// a feature that was never visited must not look pending forever.
    pub fn finish(&mut self, completed_at: impl Into<String>) {
        for step in SETUP_STEPS {
            if !self.is_step_handled(step) {
                self.skipped_steps.push(step.to_string());
            }
        }
        self.setup_version = SETUP_VERSION;
        self.completed_at = Some(completed_at.into());
    }

    /// The step a returning user should be shown: the first that is not handled.
    pub fn next_step(&self) -> Option<&'static str> {
        SETUP_STEPS
            .iter()
            .find(|step| !self.is_step_handled(step))
            .copied()
    }

    /// The steps that were skipped, for the summary page.
    pub fn skipped(&self) -> Vec<&'static str> {
        SETUP_STEPS
            .iter()
            .filter(|step| self.is_step_skipped(step))
            .copied()
            .collect()
    }

    /// The steps that are done.
    pub fn completed(&self) -> Vec<&'static str> {
        SETUP_STEPS
            .iter()
            .filter(|step| self.completed_steps.iter().any(|done| done == *step))
            .copied()
            .collect()
    }

    fn check_step(&self, step: &str) -> Result<(), DesktopError> {
        if SETUP_STEPS.contains(&step) {
            Ok(())
        } else {
            Err(DesktopError::UnknownSetupStep)
        }
    }

    /// Reading the state again is what makes the wizard re-runnable: the stored
    /// document is reset, and the settings themselves are untouched.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn to_json(&self) -> Result<String, DesktopError> {
        serde_json::to_string(self).map_err(|_| DesktopError::Storage)
    }

    pub fn from_json(text: &str) -> Result<Self, DesktopError> {
        serde_json::from_str(text).map_err(|_| DesktopError::DamagedSetupState)
    }

    /// Reads the stored state, falling back to "the wizard has not run".
    pub fn load_or_default(text: Option<&str>) -> Self {
        text.and_then(|text| Self::from_json(text).ok())
            .unwrap_or_default()
    }
}

/// Reading and writing the two documents the shell owns.
pub struct DesktopStore {
    directory: PathBuf,
}

impl DesktopStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn settings(&self) -> DesktopSettings {
        let text = std::fs::read_to_string(self.directory.join(DESKTOP_SETTINGS_FILE)).ok();
        DesktopSettings::load_or_default(text.as_deref())
    }

    /// Writes the settings atomically: a temporary file, then a rename.
    pub fn save_settings(&self, settings: &DesktopSettings) -> Result<(), DesktopError> {
        let normalized = settings.clone().normalized();
        write_atomic(
            &self.directory,
            DESKTOP_SETTINGS_FILE,
            &normalized.to_json()?,
        )
    }

    pub fn setup(&self) -> SetupState {
        let text = std::fs::read_to_string(self.directory.join(SETUP_STATE_FILE)).ok();
        SetupState::load_or_default(text.as_deref())
    }

    pub fn save_setup(&self, state: &SetupState) -> Result<(), DesktopError> {
        write_atomic(&self.directory, SETUP_STATE_FILE, &state.to_json()?)
    }
}

/// Writes `contents` to `directory/name` through a temporary file.
fn write_atomic(directory: &Path, name: &str, contents: &str) -> Result<(), DesktopError> {
    std::fs::create_dir_all(directory).map_err(|_| DesktopError::Storage)?;
    let temporary = directory.join(format!("{name}.tmp"));
    std::fs::write(&temporary, contents).map_err(|_| DesktopError::Storage)?;
    std::fs::rename(&temporary, directory.join(name)).map_err(|_| DesktopError::Storage)
}

/// What can go wrong in the shell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesktopError {
    /// The settings document could not be parsed.
    DamagedSettings,
    /// The first-run state could not be parsed.
    DamagedSetupState,
    /// A step name that is not one of [`SETUP_STEPS`].
    UnknownSetupStep,
    /// The platform refused to write the autostart entry.
    AutostartRefused(String),
    /// Something is already using the microphone.
    MicrophoneBusy,
    /// The event came from a session that has already ended.
    StaleSession,
    /// A file the shell owns could not be written.
    Storage,
}

impl DesktopError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DamagedSettings => "damaged_settings",
            Self::DamagedSetupState => "damaged_setup_state",
            Self::UnknownSetupStep => "unknown_setup_step",
            Self::AutostartRefused(_) => "autostart_refused",
            Self::MicrophoneBusy => "microphone_busy",
            Self::StaleSession => "stale_session",
            Self::Storage => "storage",
        }
    }
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DamagedSettings => {
                formatter.write_str("the desktop settings were damaged and the defaults were used")
            }
            Self::DamagedSetupState => formatter.write_str("the first-run state was damaged"),
            Self::UnknownSetupStep => formatter.write_str("that is not a step of the wizard"),
            Self::AutostartRefused(detail) => {
                write!(formatter, "Windows refused the autostart entry: {detail}")
            }
            Self::MicrophoneBusy => formatter.write_str("the microphone is already in use"),
            Self::StaleSession => formatter.write_str("that session has already ended"),
            Self::Storage => formatter.write_str("a desktop file could not be written"),
        }
    }
}

impl std::error::Error for DesktopError {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn snapshot() -> DesktopSnapshot {
        DesktopSnapshot {
            window_visible: true,
            ai_state: AiSnapshot::Stopped,
            microphone: MicrophoneState::Idle,
            vault_unlocked: true,
            pending_timers: 0,
            whisper_configured: true,
            vosk_running: false,
            autostart: AutostartState::Disabled,
        }
    }

    fn row<'a>(rows: &'a [TrayMenuRow], id: &str) -> &'a TrayMenuRow {
        rows.iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("the menu must have a {id} row"))
    }

    // ---------------------------------------------------------------- close behaviour

    #[test]
    fn the_close_button_asks_until_the_user_decides() {
        assert_eq!(CloseBehavior::default(), CloseBehavior::Ask);
        assert!(!CloseBehavior::Ask.decides_by_itself());
        assert!(CloseBehavior::Tray.decides_by_itself());
        assert!(CloseBehavior::Exit.decides_by_itself());
        assert_eq!(CloseBehavior::Tray.as_str(), "tray");
        assert_eq!(CloseBehavior::Exit.label_key(), "desktop-close-exit");
    }

    // ------------------------------------------------------------------- autostart

    #[test]
    fn autostart_is_off_and_starts_nothing_by_default() {
        let settings = DesktopSettings::default();
        assert!(!settings.autostart_enabled, "autostart must be opt-in");
        let actions = settings.autostart_actions();
        assert!(!actions.start_local_ai);
        assert!(!actions.start_vosk);
        assert!(!actions.start_whisper, "a login never starts dictation");
        assert!(!actions.start_minimized);
        assert!(!actions.starts_anything());
    }

    #[test]
    fn enabling_autostart_does_not_start_services_unless_each_one_is_asked_for() {
        let settings = DesktopSettings {
            autostart_enabled: true,
            ..DesktopSettings::default()
        };
        let actions = settings.autostart_actions();
        assert!(
            !actions.starts_anything(),
            "the tray icon alone is not a service"
        );
        // Whisper has no switch, so it cannot be started by a login.
        assert!(!actions.start_whisper);
        let with_ai = DesktopSettings {
            start_local_ai: true,
            ..settings.clone()
        };
        assert!(with_ai.autostart_actions().start_local_ai);
        assert!(!with_ai.autostart_actions().start_vosk);
        // Turning autostart off turns the services off with it.
        let disabled = DesktopSettings {
            autostart_enabled: false,
            ..with_ai
        };
        assert!(!disabled.autostart_actions().starts_anything());
    }

    #[test]
    fn a_damaged_settings_document_falls_back_to_the_safe_defaults() {
        let repaired = DesktopSettings::load_or_default(Some("{not json"));
        assert_eq!(repaired, DesktopSettings::default());
        assert!(!repaired.autostart_enabled);
        assert_eq!(repaired.close_behavior, CloseBehavior::Ask);
        let round_trip = DesktopSettings {
            close_behavior: CloseBehavior::Tray,
            autostart_enabled: true,
            start_minimized: true,
            ..DesktopSettings::default()
        };
        let text = round_trip.to_json().unwrap();
        assert_eq!(DesktopSettings::from_json(&text).unwrap(), round_trip);
    }

    #[test]
    fn the_controller_writes_reads_back_and_removes_the_entry() {
        let backend = std::sync::Arc::new(FakeAutostart::new());
        let controller = AutostartController::new(
            Box::new(SharedBackend(backend.clone())),
            "C:/apps/jarvis.exe",
        );
        assert_eq!(controller.status().state(), AutostartState::Disabled);
        assert!(!controller.status().is_enabled());

        let status = controller.enable().unwrap();
        assert_eq!(status.state(), AutostartState::Enabled);
        assert!(status.is_enabled());
        let (program, arguments) = backend.written().unwrap();
        assert_eq!(program.to_string_lossy(), "C:/apps/jarvis.exe");
        assert_eq!(arguments, vec![START_MINIMIZED_FLAG.to_string()]);

        let status = controller.disable().unwrap();
        assert_eq!(status.state(), AutostartState::Disabled);
        assert!(backend.written().is_none());
    }

    #[test]
    fn moving_the_application_is_detected_and_repaired() {
        // The entry exists and points at the old location: this is what an
        // update or a move leaves behind.
        let backend = std::sync::Arc::new(FakeAutostart::new().with_status(AutostartStatus {
            entry_present: true,
            path_matches: false,
            command: Some("C:/old/place/jarvis.exe".to_string()),
            error: None,
        }));
        let controller = AutostartController::new(
            Box::new(SharedBackend(backend.clone())),
            "C:/new/place/jarvis.exe",
        );
        let status = controller.status();
        assert_eq!(status.state(), AutostartState::NeedsAttention);
        assert!(status.needs_repair());
        // The repair writes the current path, and the fake then reports a match.
        let repaired = controller.repair_if_needed().unwrap();
        assert_eq!(repaired.unwrap().state(), AutostartState::Enabled);
        // An entry that already matches is left alone.
        assert!(controller.repair_if_needed().unwrap().is_none());
    }

    #[test]
    fn a_policy_that_forbids_the_run_key_is_reported_as_unavailable() {
        let backend =
            std::sync::Arc::new(FakeAutostart::new().failing("access denied by group policy"));
        let controller =
            AutostartController::new(Box::new(SharedBackend(backend)), "C:/apps/jarvis.exe");
        let status = controller.status();
        assert_eq!(status.state(), AutostartState::Unavailable);
        assert!(status.error.is_some());
        assert!(!status.is_enabled());
        let error = controller.enable().unwrap_err();
        assert_eq!(error.code(), "autostart_refused");
        assert!(error.to_string().contains("group policy"));
        // The message is shortened and carries no path.
        assert!(!error.to_string().contains("C:/"));
    }

    /// Shares one fake between the test and the controller.
    struct SharedBackend(std::sync::Arc<FakeAutostart>);

    impl AutostartBackend for SharedBackend {
        fn enable(&self, program: &Path, arguments: &[String]) -> Result<(), String> {
            self.0.enable(program, arguments)
        }
        fn disable(&self) -> Result<(), String> {
            self.0.disable()
        }
        fn status(&self) -> Result<AutostartStatus, String> {
            self.0.status()
        }
    }

    // ------------------------------------------------------------ audio session

    #[test]
    fn one_session_holds_the_microphone_and_a_second_is_refused() {
        let session = AudioSession::new();
        assert_eq!(session.state(), MicrophoneState::Idle);
        let ticket = session.begin(MicrophoneState::WhisperDictation).unwrap();
        assert_eq!(session.state(), MicrophoneState::WhisperDictation);
        assert!(session.state().holds_microphone());
        assert_eq!(
            session.begin(MicrophoneState::VoskListening).unwrap_err(),
            DesktopError::MicrophoneBusy
        );
        session.end(ticket);
        assert_eq!(session.state(), MicrophoneState::Idle);
        // With the device free again, a session can start.
        assert!(session.begin(MicrophoneState::VoskListening).is_ok());
    }

    #[test]
    fn an_event_from_an_old_session_cannot_change_the_state() {
        let session = AudioSession::new();
        let first = session.begin(MicrophoneState::WhisperDictation).unwrap();
        session.end(first);
        let second = session.begin(MicrophoneState::VoskListening).unwrap();
        // The old session reports that it stopped transcribing: it must not
        // clear or overwrite the state of the running one.
        assert_eq!(
            session.update(first, MicrophoneState::Idle).unwrap_err(),
            DesktopError::StaleSession
        );
        assert_eq!(session.state(), MicrophoneState::VoskListening);
        session.end(first);
        assert_eq!(
            session.state(),
            MicrophoneState::VoskListening,
            "a stale end must not release the device"
        );
        // The current session can still move and end normally.
        session
            .update(second, MicrophoneState::TranscribingFile)
            .unwrap();
        assert_eq!(session.state(), MicrophoneState::TranscribingFile);
        assert!(
            !session.state().holds_microphone(),
            "a file needs no microphone"
        );
        assert_eq!(second.generation(), 2);
    }

    #[test]
    fn a_failed_state_does_not_block_a_new_attempt() {
        let session = AudioSession::new();
        let ticket = session.begin(MicrophoneState::WhisperDictation).unwrap();
        session.update(ticket, MicrophoneState::Failed).unwrap();
        assert!(!session.state().holds_microphone());
        // After a failure the user can try again without a restart.
        let retry = session.begin(MicrophoneState::WhisperDictation).unwrap();
        assert_eq!(session.state(), MicrophoneState::WhisperDictation);
        session.end(retry);
    }

    #[test]
    fn the_exit_releases_the_microphone_and_invalidates_every_ticket() {
        let session = AudioSession::new();
        let ticket = session.begin(MicrophoneState::WhisperDictation).unwrap();
        session.release_for_exit();
        assert_eq!(session.state(), MicrophoneState::Idle);
        assert!(!session.is_current(ticket));
        // A late event from the session that was running is ignored.
        assert_eq!(
            session
                .update(ticket, MicrophoneState::WhisperDictation)
                .unwrap_err(),
            DesktopError::StaleSession
        );
        assert_eq!(session.state(), MicrophoneState::Idle);
    }

    #[test]
    fn every_microphone_state_says_whether_it_holds_the_device() {
        assert!(!MicrophoneState::Idle.holds_microphone());
        assert!(MicrophoneState::VoskListening.holds_microphone());
        assert!(MicrophoneState::WhisperDictation.holds_microphone());
        assert!(!MicrophoneState::TranscribingFile.holds_microphone());
        assert!(MicrophoneState::Stopping.holds_microphone());
        assert!(!MicrophoneState::Failed.holds_microphone());
        // Only a state that holds the device puts an indicator in the tray.
        assert!(MicrophoneState::WhisperDictation.needs_indicator());
        assert!(!MicrophoneState::TranscribingFile.needs_indicator());
        assert!(MicrophoneState::WhisperDictation.can_stop());
        assert!(!MicrophoneState::Failed.can_stop());
        assert_eq!(
            MicrophoneState::VoskListening.label_key(),
            "desktop-mic-vosk"
        );
    }

    // ------------------------------------------------------------------ tray menu

    #[test]
    fn the_menu_always_offers_a_way_back_to_the_window_and_out() {
        for visible in [true, false] {
            let rows = tray_menu(&DesktopSnapshot {
                window_visible: visible,
                ..snapshot()
            });
            assert!(row(&rows, "open").enabled);
            assert_eq!(
                row(&rows, "open").label_key,
                if visible {
                    "desktop-menu-focus"
                } else {
                    "desktop-menu-open"
                }
            );
            assert_eq!(row(&rows, "hide").enabled, visible);
            // Exit is never disabled, whatever else is happening.
            assert!(row(&rows, "exit").enabled);
            assert!(row(&rows, "settings").enabled);
        }
    }

    #[test]
    fn the_menu_reports_the_ai_state_instead_of_offering_a_button() {
        for (state, key) in [
            (AiSnapshot::NotConfigured, "desktop-ai-not-configured"),
            (AiSnapshot::Stopped, "desktop-ai-stopped"),
            (AiSnapshot::Ready, "desktop-ai-ready"),
            (AiSnapshot::Busy, "desktop-ai-busy"),
        ] {
            let rows = tray_menu(&DesktopSnapshot {
                ai_state: state,
                ..snapshot()
            });
            let status = row(&rows, "ai_status");
            assert!(!status.enabled, "a status row is not a button");
            assert_eq!(status.value.as_deref(), Some(key));
            assert!(!state.as_str().is_empty());
        }
    }

    #[test]
    fn dictation_cannot_start_without_whisper_or_with_the_microphone_taken() {
        // Not configured: the row is there and disabled.
        let rows = tray_menu(&DesktopSnapshot {
            whisper_configured: false,
            ..snapshot()
        });
        assert!(!row(&rows, "dictate_start").enabled);
        assert!(!row(&rows, "dictate_stop").enabled);

        // Configured and idle: both rows reflect what is possible.
        let rows = tray_menu(&snapshot());
        assert!(row(&rows, "dictate_start").enabled);
        assert!(!row(&rows, "dictate_stop").enabled);

        // Recording: starting is refused, stopping is offered, and the
        // microphone row shows what is happening.
        let rows = tray_menu(&DesktopSnapshot {
            microphone: MicrophoneState::WhisperDictation,
            ..snapshot()
        });
        assert!(!row(&rows, "dictate_start").enabled);
        assert!(row(&rows, "dictate_stop").enabled);
        assert_eq!(
            row(&rows, "microphone").value.as_deref(),
            Some("desktop-mic-whisper")
        );

        // Vosk listening: starting dictation would need the same device.
        let rows = tray_menu(&DesktopSnapshot {
            vosk_running: true,
            microphone: MicrophoneState::VoskListening,
            ..snapshot()
        });
        assert!(!row(&rows, "dictate_start").enabled);
        assert_eq!(
            row(&rows, "vosk_toggle").label_key,
            "desktop-menu-vosk-stop"
        );
        assert!(row(&rows, "vosk_toggle").enabled);

        // Transcribing a file: the microphone is free, but the session that
        // reads the audio is busy, so a second dictation is not offered. It can
        // still be stopped.
        let rows = tray_menu(&DesktopSnapshot {
            microphone: MicrophoneState::TranscribingFile,
            ..snapshot()
        });
        assert!(
            !row(&rows, "dictate_start").enabled,
            "one transcription at a time"
        );
        assert!(row(&rows, "dictate_stop").enabled);
        assert_eq!(
            row(&rows, "microphone").value.as_deref(),
            Some("desktop-mic-file")
        );
    }

    #[test]
    fn locking_the_storage_is_offered_only_when_it_is_unlocked() {
        let unlocked = tray_menu(&DesktopSnapshot {
            vault_unlocked: true,
            ..snapshot()
        });
        assert!(row(&unlocked, "lock_storage").enabled);
        let locked = tray_menu(&DesktopSnapshot {
            vault_unlocked: false,
            ..snapshot()
        });
        assert!(
            !row(&locked, "lock_storage").enabled,
            "locking what is already locked is not an action"
        );
    }

    #[test]
    fn the_timer_row_shows_a_count_including_zero() {
        let none = tray_menu(&snapshot());
        assert_eq!(
            none.iter()
                .find(|row| row.id == "timers")
                .unwrap()
                .value
                .as_deref(),
            Some("0")
        );
        let three = tray_menu(&DesktopSnapshot {
            pending_timers: 3,
            ..snapshot()
        });
        assert_eq!(
            three
                .iter()
                .find(|row| row.id == "timers")
                .unwrap()
                .value
                .as_deref(),
            Some("3")
        );
    }

    #[test]
    fn the_menu_lists_every_row_it_is_expected_to_have_and_nothing_is_a_secret() {
        let rows = tray_menu(&snapshot());
        for id in [
            "open",
            "hide",
            "ai_status",
            "vosk_toggle",
            "dictate_start",
            "dictate_stop",
            "microphone",
            "timers",
            "lock_storage",
            "settings",
            "exit",
        ] {
            assert!(rows.iter().any(|row| row.id == id), "{id} is missing");
        }
        // A label is a key or a state word, never a value from the user's data.
        for row in &rows {
            if let Some(value) = &row.value {
                assert!(
                    value.len() < 40 && !value.contains('/') && !value.contains('\\'),
                    "a menu value carries something it should not: {value}"
                );
            }
            assert!(!row.label_key.contains(' '));
        }
    }

    #[test]
    fn the_tooltip_says_what_a_hidden_window_is_doing() {
        let idle = tray_tooltip(&snapshot());
        assert!(idle.contains("AI stopped"));
        assert!(idle.contains("microphone idle"));
        assert!(idle.contains("no timers"));
        let busy = tray_tooltip(&DesktopSnapshot {
            ai_state: AiSnapshot::Busy,
            microphone: MicrophoneState::WhisperDictation,
            pending_timers: 2,
            ..snapshot()
        });
        assert!(busy.contains("AI working"));
        assert!(busy.contains("recording"));
        assert!(busy.contains("2 timer(s)"));
    }

    // ---------------------------------------------------------------- first run

    #[test]
    fn a_new_profile_needs_the_wizard_and_every_step_can_be_skipped() {
        let mut state = SetupState::new();
        assert!(state.needs_wizard());
        assert_eq!(state.setup_version, 0);
        assert_eq!(state.next_step(), Some("language"));
        assert!(!state.is_complete());

        // Skipping every step leaves the wizard finishable and the features
        // unconfigured rather than broken.
        for step in SETUP_STEPS {
            state.skip_step(step).unwrap();
        }
        assert!(state.is_complete());
        assert_eq!(state.skipped().len(), SETUP_STEPS.len());
        assert!(state.completed().is_empty());
        assert_eq!(state.next_step(), None);
        // Nothing was marked as done, so nothing claims to be configured.
        assert!(state.skipped_steps.contains(&"local_ai".to_string()));
    }

    #[test]
    fn a_step_that_is_done_after_being_skipped_is_done() {
        let mut state = SetupState::new();
        state.skip_step("local_ai").unwrap();
        assert!(state.is_step_skipped("local_ai"));
        state.complete_step("local_ai").unwrap();
        assert!(!state.is_step_skipped("local_ai"));
        assert!(state.completed().contains(&"local_ai"));
        // Marking it done twice does not duplicate it.
        state.complete_step("local_ai").unwrap();
        assert_eq!(
            state
                .completed_steps
                .iter()
                .filter(|step| *step == "local_ai")
                .count(),
            1
        );
        // And skipping it again takes it back out of "done".
        state.skip_step("local_ai").unwrap();
        assert!(!state.completed().contains(&"local_ai"));
    }

    #[test]
    fn the_wizard_resumes_where_it_stopped_and_can_be_run_again() {
        let mut state = SetupState::new();
        state.complete_step("language").unwrap();
        state.skip_step("storage").unwrap();
        assert_eq!(state.next_step(), Some("local_ai"));
        assert_eq!(state.completed(), vec!["language"]);
        assert_eq!(state.skipped(), vec!["storage"]);

        state.finish("2026-01-01T00:00:00Z");
        assert!(!state.needs_wizard());
        assert!(state.is_complete());

        // Running it again is a reset of this state only: the settings and the
        // features are untouched.
        state.reset();
        assert!(state.needs_wizard());
        assert_eq!(state.next_step(), Some("language"));
        assert!(state.completed_steps.is_empty());
    }

    #[test]
    fn an_unknown_step_is_refused_instead_of_being_stored() {
        let mut state = SetupState::new();
        assert_eq!(
            state.complete_step("teleport").unwrap_err(),
            DesktopError::UnknownSetupStep
        );
        assert_eq!(
            state.skip_step("").unwrap_err().code(),
            "unknown_setup_step"
        );
        assert!(state.completed_steps.is_empty());
        assert!(state.skipped_steps.is_empty());
    }

    #[test]
    fn a_damaged_first_run_state_runs_the_wizard_again() {
        let state = SetupState::load_or_default(Some("{oops"));
        assert!(state.needs_wizard());
        assert_eq!(state.setup_version, 0);
        let round_trip = {
            let mut state = SetupState::new();
            state.complete_step("language").unwrap();
            state.language = Some("ru".to_string());
            state.finish("2026-01-01T00:00:00Z");
            state
        };
        let text = round_trip.to_json().unwrap();
        assert_eq!(SetupState::from_json(&text).unwrap(), round_trip);
        // The state carries no secret: the password never reaches this document.
        for secret in ["password", "passphrase", "secret", "master_key", "token"] {
            assert!(!text.contains(secret), "{secret} must not be in the state");
        }
    }

    #[test]
    fn a_newer_wizard_version_asks_an_existing_profile_to_see_it_once() {
        // A profile that finished an older version of the wizard.
        let older = SetupState {
            setup_version: SETUP_VERSION - 1,
            completed_at: Some("2025-01-01T00:00:00Z".to_string()),
            ..SetupState::new()
        };
        assert!(
            older.needs_wizard(),
            "a new step has to reach a profile that already finished"
        );
        // Finishing it again records the current version.
        let mut upgraded = older;
        upgraded.finish("2026-01-01T00:00:00Z");
        assert_eq!(upgraded.setup_version, SETUP_VERSION);
        assert!(!upgraded.needs_wizard());
    }

    #[test]
    fn the_store_round_trips_both_documents_atomically() {
        let directory = tempdir().unwrap();
        let store = DesktopStore::new(directory.path());
        assert_eq!(store.settings(), DesktopSettings::default());
        assert!(store.setup().needs_wizard());

        let settings = DesktopSettings {
            close_behavior: CloseBehavior::Tray,
            autostart_enabled: true,
            start_minimized: true,
            tray_explained: true,
            ..DesktopSettings::default()
        };
        store.save_settings(&settings).unwrap();
        let mut setup = SetupState::new();
        setup.complete_step("language").unwrap();
        store.save_setup(&setup).unwrap();

        assert_eq!(store.settings(), settings);
        assert_eq!(store.setup(), setup);
        // No temporary file is left behind by the atomic write.
        assert!(!directory
            .path()
            .join(format!("{DESKTOP_SETTINGS_FILE}.tmp"))
            .exists());
        assert!(!directory
            .path()
            .join(format!("{SETUP_STATE_FILE}.tmp"))
            .exists());

        // A damaged document on disk is repaired rather than fatal.
        std::fs::write(directory.path().join(DESKTOP_SETTINGS_FILE), "{broken").unwrap();
        assert_eq!(store.settings(), DesktopSettings::default());
    }
}
