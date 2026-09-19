//! The typed model of a safe Windows action.
//!
//! One enum describes everything this feature can do, and every variant carries typed
//! fields only. There is deliberately no variant that takes a command line, a shell string,
//! a raw executable path, or an arbitrary argument list: an action that cannot be expressed
//! here cannot be performed, which is what keeps "AI sends a sentence, the app runs a
//! program" impossible by construction rather than by review.
//!
//! Values that arrive from outside (an AI tool call, a voice transcript, a stored file) all
//! pass through [`WindowsAction`] validation before anything else looks at them, and the
//! numbers they carry are range-checked here in one place.

use serde::{Deserialize, Serialize};

use super::error::ActionError;

/// How long a confirmation dialog stays valid.
pub const CONFIRMATION_TTL_SECONDS: u64 = 45;
/// Largest volume change one action may make, in percentage points.
pub const MAX_VOLUME_STEP_PERCENT: u8 = 25;
/// Shortest timer this feature accepts.
pub const MIN_TIMER_SECONDS: u64 = 5;
/// Longest timer this feature accepts (24 hours).
pub const MAX_TIMER_SECONDS: u64 = 24 * 60 * 60;
/// Shortest reminder delay.
pub const MIN_REMINDER_SECONDS: u64 = 30;
/// Longest reminder delay (30 days).
pub const MAX_REMINDER_SECONDS: u64 = 30 * 24 * 60 * 60;
/// Longest reminder text.
pub const MAX_REMINDER_CHARS: usize = 500;
/// How many timers, reminders, and allowed applications may exist at once.
pub const MAX_ACTIVE_TIMERS: usize = 32;
pub const MAX_ACTIVE_REMINDERS: usize = 64;
pub const MAX_ALLOWED_APPLICATIONS: usize = 64;
/// How long a window identifier stays usable.
pub const WINDOW_ID_TTL_SECONDS: u64 = 90;
/// How many identifiers from replaced listings are remembered, so that an old one is answered
/// with "expired" instead of "no such window".
pub const MAX_RETIRED_WINDOW_IDS: usize = 64;
/// Longest window title this feature keeps, after sanitizing.
pub const MAX_WINDOW_TITLE_CHARS: usize = 120;
/// Largest move this feature allows in one action, in pixels.
pub const MAX_MOVE_PIXELS: i32 = 20_000;

/// Who asked for the action.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSource {
    /// A button or a form in the interface.
    DirectGui,
    /// A recognized voice phrase.
    Voice,
    /// A structured tool call from the local model.
    LocalAi,
    /// The timer scheduler firing a stored timer or reminder.
    InternalTimer,
    /// A command pack: a document the user installed, naming one typed action.
    ///
    /// It is not a guess about what was said — the phrase is one the pack lists,
    /// and the action is one this build implements — so it is not treated as a
    /// remote source. It still passes the policy table, the allowlist and the
    /// audit log, which is more than the pack path did before.
    CommandPack,
}

impl ActionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DirectGui => "direct_gui",
            Self::Voice => "voice",
            Self::LocalAi => "local_ai",
            Self::InternalTimer => "internal_timer",
            Self::CommandPack => "command_pack",
        }
    }

    /// Whether the source is a request this application inferred rather than one
    /// the user made directly. A command pack is the user's own document.
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::LocalAi | Self::Voice)
    }

    pub fn label_key(&self) -> &'static str {
        match self {
            Self::DirectGui => "windows-source-direct-gui",
            Self::Voice => "windows-source-voice",
            Self::LocalAi => "windows-source-local-ai",
            Self::InternalTimer => "windows-source-timer",
            Self::CommandPack => "windows-source-command-pack",
        }
    }
}

/// How risky an action is. The mapping lives in [`super::policy`], not here.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRisk {
    Safe,
    Confirm,
    Forbidden,
}

/// The gate and the policy use different level types on purpose: the gate predates this
/// feature and is shared with the voice command path. The two conversions below are the only
/// place the mapping exists, so a level cannot drift between them.
impl From<ActionRisk> for crate::safety::RiskLevel {
    fn from(risk: ActionRisk) -> Self {
        match risk {
            ActionRisk::Safe => crate::safety::RiskLevel::Safe,
            ActionRisk::Confirm => crate::safety::RiskLevel::ConfirmationRequired,
            ActionRisk::Forbidden => crate::safety::RiskLevel::Forbidden,
        }
    }
}

impl From<crate::safety::RiskLevel> for ActionRisk {
    fn from(risk: crate::safety::RiskLevel) -> Self {
        match risk {
            crate::safety::RiskLevel::Safe => ActionRisk::Safe,
            crate::safety::RiskLevel::ConfirmationRequired => ActionRisk::Confirm,
            crate::safety::RiskLevel::Forbidden => ActionRisk::Forbidden,
        }
    }
}

impl ActionRisk {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Confirm => "confirm",
            Self::Forbidden => "forbidden",
        }
    }

    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Safe => "windows-risk-safe",
            Self::Confirm => "windows-risk-confirm",
            Self::Forbidden => "windows-risk-forbidden",
        }
    }
}

/// An identifier minted by this application for an action request.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ActionId(String);

impl ActionId {
    pub fn new() -> Result<Self, ActionError> {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| ActionError::UnsupportedPlatform)?;
        let mut encoded = String::with_capacity(16);
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        Ok(Self(encoded))
    }

    pub fn from_stored(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The identifier of an entry in the allowed-application list.
///
/// An AI tool call carries this identifier and nothing else: the executable, its working
/// directory, and its arguments come from the stored entry the user created.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ApplicationId(String);

impl ApplicationId {
    pub fn from_stored(value: impl Into<String>) -> Result<Self, ActionError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(ActionError::ApplicationNotAllowed {
                application_id: String::new(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A short-lived, opaque identifier for one visible window.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct WindowId(String);

impl WindowId {
    pub fn mint() -> Result<Self, ActionError> {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| ActionError::UnsupportedPlatform)?;
        let mut encoded = String::with_capacity(16);
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        Ok(Self(encoded))
    }

    pub fn from_stored(value: impl Into<String>) -> Result<Self, ActionError> {
        let value = value.into();
        if value.is_empty() || value.len() > 32 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ActionError::WindowNotFound);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A timer or reminder identifier.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct TimerId(String);

impl TimerId {
    pub fn mint() -> Result<Self, ActionError> {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| ActionError::UnsupportedPlatform)?;
        let mut encoded = String::with_capacity(16);
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        Ok(Self(encoded))
    }

    pub fn from_stored(value: impl Into<String>) -> Result<Self, ActionError> {
        let value = value.into();
        if value.is_empty() || value.len() > 32 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ActionError::TimerNotFound);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which way a relative volume change goes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeDirection {
    Up,
    Down,
}

impl VolumeDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// What a screenshot covers.
///
/// The interface and the model choose one of these variants and, for the two that need one,
/// an identifier from the list this application just produced. A monitor or window name is
/// never accepted as an argument.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "target", content = "id")]
pub enum ScreenshotTarget {
    /// The primary monitor.
    #[serde(rename = "primary_monitor")]
    PrimaryMonitor,
    /// One monitor from the current list.
    #[serde(rename = "selected_monitor")]
    SelectedMonitor(u32),
    /// One window from the current list.
    #[serde(rename = "selected_window")]
    SelectedWindow(WindowId),
    /// Every monitor, as one image.
    #[serde(rename = "all_monitors")]
    AllMonitors,
}

impl ScreenshotTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::PrimaryMonitor => "primary_monitor",
            Self::SelectedMonitor(_) => "selected_monitor",
            Self::SelectedWindow(_) => "selected_window",
            Self::AllMonitors => "all_monitors",
        }
    }
}

/// What may be done with a window.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowOperation {
    Minimize,
    Maximize,
    Restore,
    /// Move and resize inside the work area of the monitors.
    Move {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Ask the window to close with the normal message, never by ending a process.
    Close,
}

impl WindowOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Minimize => "minimize",
            Self::Maximize => "maximize",
            Self::Restore => "restore",
            Self::Move { .. } => "move",
            Self::Close => "close",
        }
    }
}

/// Everything this feature can do, with typed fields only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum WindowsAction {
    /// Read the current volume.
    GetVolume,
    /// Set the volume to an exact percentage.
    SetVolume { percent: u8 },
    /// Move the volume by a small step.
    ChangeVolume {
        direction: VolumeDirection,
        step: u8,
    },
    /// Mute or unmute.
    MuteVolume { muted: bool },
    /// Start an application the user allowed, by identifier.
    LaunchAllowedApplication { application_id: ApplicationId },
    /// Capture the screen, never automatically and never to the model.
    TakeScreenshot { target: ScreenshotTarget },
    /// Start a timer that notifies when it fires.
    CreateTimer { duration_seconds: u64 },
    /// Cancel a timer this application started.
    CancelTimer { timer_id: TimerId },
    /// Create a reminder with a short message.
    CreateReminder { delay_seconds: u64, message: String },
    /// Cancel a reminder this application created.
    CancelReminder { timer_id: TimerId },
    /// List the visible user windows.
    ListWindows,
    /// Act on one window from the current list.
    Window {
        window_id: WindowId,
        operation: WindowOperation,
    },
    /// Lock the workstation through the documented API.
    LockWorkstation,
}

impl WindowsAction {
    /// Stable name of the action, used by the policy table, the audit log, and the tools.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::GetVolume => "get_volume",
            Self::SetVolume { .. } => "set_volume",
            Self::ChangeVolume { .. } => "change_volume",
            Self::MuteVolume { .. } => "mute_volume",
            Self::LaunchAllowedApplication { .. } => "launch_allowed_application",
            Self::TakeScreenshot { .. } => "take_screenshot",
            Self::CreateTimer { .. } => "create_timer",
            Self::CancelTimer { .. } => "cancel_timer",
            Self::CreateReminder { .. } => "create_reminder",
            Self::CancelReminder { .. } => "cancel_reminder",
            Self::ListWindows => "list_windows",
            Self::Window { operation, .. } => match operation {
                WindowOperation::Minimize => "minimize_window",
                WindowOperation::Maximize => "maximize_window",
                WindowOperation::Restore => "restore_window",
                WindowOperation::Move { .. } => "move_window",
                WindowOperation::Close => "close_window",
            },
            Self::LockWorkstation => "lock_workstation",
        }
    }

    /// Validates every number and string this action carries.
    ///
    /// Called once, right after the action is decoded from an external shape, so nothing
    /// downstream has to re-check ranges.
    pub fn validate(&self) -> Result<(), ActionError> {
        match self {
            Self::SetVolume { percent } => {
                if *percent > 100 {
                    return Err(ActionError::InvalidArguments {
                        detail: "volume must be between 0 and 100".to_string(),
                    });
                }
            }
            Self::ChangeVolume { step, .. } => {
                if *step == 0 || *step > MAX_VOLUME_STEP_PERCENT {
                    return Err(ActionError::InvalidArguments {
                        detail: format!("one volume step is 1 to {MAX_VOLUME_STEP_PERCENT} points"),
                    });
                }
            }
            Self::CreateTimer { duration_seconds } => {
                if *duration_seconds < MIN_TIMER_SECONDS || *duration_seconds > MAX_TIMER_SECONDS {
                    return Err(ActionError::InvalidArguments {
                        detail: format!(
                            "a timer is {MIN_TIMER_SECONDS} seconds to {MAX_TIMER_SECONDS} seconds"
                        ),
                    });
                }
            }
            Self::CreateReminder {
                delay_seconds,
                message,
            } => {
                if *delay_seconds < MIN_REMINDER_SECONDS || *delay_seconds > MAX_REMINDER_SECONDS {
                    return Err(ActionError::InvalidArguments {
                        detail: format!(
                            "a reminder is {MIN_REMINDER_SECONDS} seconds to {MAX_REMINDER_SECONDS} seconds away"
                        ),
                    });
                }
                let trimmed = message.trim();
                if trimmed.is_empty() || trimmed.chars().count() > MAX_REMINDER_CHARS {
                    return Err(ActionError::InvalidArguments {
                        detail: format!("a reminder is 1 to {MAX_REMINDER_CHARS} characters"),
                    });
                }
                if trimmed.chars().any(char::is_control) {
                    return Err(ActionError::InvalidArguments {
                        detail: "a reminder cannot contain control characters".to_string(),
                    });
                }
            }
            Self::Window {
                operation:
                    WindowOperation::Move {
                        x,
                        y,
                        width,
                        height,
                    },
                ..
            } => {
                if x.unsigned_abs() > MAX_MOVE_PIXELS as u32
                    || y.unsigned_abs() > MAX_MOVE_PIXELS as u32
                    || *width == 0
                    || *height == 0
                    || *width > MAX_MOVE_PIXELS as u32
                    || *height > MAX_MOVE_PIXELS as u32
                {
                    return Err(ActionError::InvalidArguments {
                        detail: "that window position is outside the supported range".to_string(),
                    });
                }
            }
            Self::TakeScreenshot {
                target: ScreenshotTarget::SelectedMonitor(monitor),
            } if *monitor > 32 => {
                return Err(ActionError::InvalidArguments {
                    detail: "that monitor does not exist".to_string(),
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether this action needs the interface to ask the user first.
    pub fn requires_confirmation_by_policy(&self) -> bool {
        super::policy::ActionPolicy::default().risk(self) == ActionRisk::Confirm
    }
}

/// One request: an action, who asked, and when.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionRequest {
    pub id: ActionId,
    pub action: WindowsAction,
    pub source: ActionSource,
    /// Wall-clock timestamp, for the audit log.
    pub requested_at: String,
}

impl ActionRequest {
    pub fn new(
        action: WindowsAction,
        source: ActionSource,
        requested_at: impl Into<String>,
    ) -> Result<Self, ActionError> {
        action.validate()?;
        Ok(Self {
            id: ActionId::new()?,
            action,
            source,
            requested_at: requested_at.into(),
        })
    }

    pub fn kind(&self) -> &'static str {
        self.action.kind()
    }
}

/// One field of the preview the confirmation dialog shows.
///
/// The dialog receives only this: a label key and an already-rendered value. Arguments,
/// full paths, and any payload from the request never reach the interface.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreviewField {
    /// Fluent key of the field name.
    pub label_key: String,
    pub value: String,
}

/// What the interface shows before a confirmed action runs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionPreview {
    /// Token the confirmation has to send back.
    pub token: String,
    pub action_kind: String,
    pub risk: ActionRisk,
    pub source: ActionSource,
    /// Fluent key of the one-line title.
    pub title_key: String,
    pub fields: Vec<PreviewField>,
    /// Fluent keys of what could happen.
    pub consequences: Vec<String>,
    /// Seconds left before the request expires.
    pub expires_in_seconds: u64,
    /// Whether the action can still be cancelled (always true while it is pending).
    pub cancellable: bool,
}

/// Status of one action, from request to outcome.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Requested,
    Confirmed,
    Cancelled,
    Expired,
    Executed,
    Rejected,
    Failed,
}

impl ActionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Confirmed => "confirmed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Executed => "executed",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
        }
    }

    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Requested => "windows-status-requested",
            Self::Confirmed => "windows-status-confirmed",
            Self::Cancelled => "windows-status-cancelled",
            Self::Expired => "windows-status-expired",
            Self::Executed => "windows-status-executed",
            Self::Rejected => "windows-status-rejected",
            Self::Failed => "windows-status-failed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Requested | Self::Confirmed)
    }
}

/// What an executed action produced.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "value", rename_all = "snake_case")]
pub enum ActionValue {
    Volume {
        percent: u8,
        muted: bool,
    },
    /// The full path of the image, shown to the user and never sent to a model.
    ScreenshotPath {
        path: String,
        bytes: u64,
    },
    Launched {
        application: String,
        process_id: u32,
    },
    Windows {
        windows: Vec<WindowSummary>,
    },
    Timer {
        timer_id: String,
        fires_in_seconds: u64,
    },
    Cancelled {
        timer_id: String,
    },
    Locked,
    None,
}

/// One window as the interface and the model see it.
///
/// The title is sanitized and truncated for ordinary windows. Sensitive titles are replaced
/// before this DTO can cross an IPC or model boundary; the identifier is opaque and
/// short-lived, and the title is never used to address a window.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowSummary {
    pub id: String,
    /// A sanitized title for an ordinary window, or a content-free label for a sensitive one.
    pub title: String,
    /// Name of the executable, without a path.
    pub process: String,
    pub state: WindowState,
    pub monitor: u32,
    /// Whether this window looks like it could show credentials.
    pub sensitive: bool,
    /// Whether this window currently has the keyboard focus.
    pub foreground: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
}

impl WindowState {
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Normal => "windows-window-normal",
            Self::Minimized => "windows-window-minimized",
            Self::Maximized => "windows-window-maximized",
        }
    }
}

/// Outcome of one action.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActionResult {
    pub action_id: String,
    pub action_kind: String,
    pub status: ActionStatus,
    pub source: ActionSource,
    pub value: ActionValue,
    /// Content-free detail for a failure.
    pub detail: Option<String>,
    pub duration_ms: u64,
}

impl ActionResult {
    pub fn executed(request: &ActionRequest, value: ActionValue, duration_ms: u64) -> Self {
        Self {
            action_id: request.id.as_str().to_string(),
            action_kind: request.kind().to_string(),
            status: ActionStatus::Executed,
            source: request.source,
            value,
            detail: None,
            duration_ms,
        }
    }

    pub fn failed(request: &ActionRequest, error: &ActionError, duration_ms: u64) -> Self {
        Self {
            action_id: request.id.as_str().to_string(),
            action_kind: request.kind().to_string(),
            status: ActionStatus::Failed,
            source: request.source,
            value: ActionValue::None,
            detail: Some(error.code().to_string()),
            duration_ms,
        }
    }

    /// Whether the action ran.
    pub fn is_success(&self) -> bool {
        self.status == ActionStatus::Executed
    }
}

/// What a request produced: either an outcome, or a question the user has to answer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ActionRequestOutcome {
    /// The action ran.
    Executed { result: ActionResult },
    /// The action is waiting for a confirmation; the preview is what to show.
    AwaitingConfirmation { preview: ActionPreview },
    /// The policy refused it; the reason is content-free.
    Rejected { detail: String },
}

/// Sanitizes a window title for display and for the model.
///
/// Control characters are dropped, runs of whitespace collapse, and the result is truncated.
/// The title is never an identifier: it is shown, and nothing more.
pub fn sanitize_window_title(title: &str) -> String {
    let mut cleaned = String::with_capacity(title.len().min(MAX_WINDOW_TITLE_CHARS));
    let mut last_was_space = false;
    for character in title.chars() {
        // Whitespace is checked first: a newline or a tab separates words and becomes a single
        // space. Any other control character is removed without a replacement, so that a
        // marker word cannot be pulled apart by hiding a control character inside it.
        if character.is_whitespace() {
            if last_was_space {
                continue;
            }
            last_was_space = true;
            cleaned.push(' ');
            continue;
        }
        if character.is_control() {
            continue;
        }
        last_was_space = false;
        if cleaned.chars().count() >= MAX_WINDOW_TITLE_CHARS {
            break;
        }
        cleaned.push(character);
    }
    cleaned.trim().to_string()
}

/// Produces the only title that may leave the native window registry.
///
/// Native titles are needed briefly for voice matching and Win32 operations, but a title that
/// could contain a credential must never be serialized into an IPC response, an AI tool result,
/// or a confirmation preview.
pub fn safe_window_title(title: &str) -> String {
    if title_looks_sensitive(title) {
        "Sensitive window".to_string()
    } else {
        sanitize_window_title(title)
    }
}

/// Whether a window title suggests a place where credentials may be visible.
///
/// This is a heuristic that changes how a screenshot is handled, and it is deliberately
/// conservative: a false positive only hides a title, whereas a false negative can disclose a
/// credential in a UI, log, or model context.
pub fn title_looks_sensitive(title: &str) -> bool {
    let lowered = title.to_lowercase();
    const MARKERS: [&str; 17] = [
        "jarvis",
        "altron",
        "vault",
        "пароль",
        "password",
        "credential",
        "passphrase",
        "api key",
        "api_key",
        "access token",
        "secret",
        "private key",
        "ключ доступа",
        "токен",
        "секрет",
        "приватный ключ",
        "учетные данные",
    ];
    MARKERS.iter().any(|marker| lowered.contains(marker))
        || ["sk-", "ghp_", "github_pat_", "akia", "xoxb-", "xoxp-"]
            .iter()
            .any(|prefix| lowered.contains(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_action_set_is_closed_and_typed() {
        // A compile-time property, stated as a test: every variant is built from typed
        // fields, and there is no `RawCommand`, `Shell`, `PowerShell`, or path variant.
        let actions = [
            WindowsAction::GetVolume,
            WindowsAction::SetVolume { percent: 40 },
            WindowsAction::ChangeVolume {
                direction: VolumeDirection::Down,
                step: 10,
            },
            WindowsAction::MuteVolume { muted: true },
            WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::PrimaryMonitor,
            },
            WindowsAction::CreateTimer {
                duration_seconds: 600,
            },
            WindowsAction::CreateReminder {
                delay_seconds: 1800,
                message: "перерыв".to_string(),
            },
            WindowsAction::ListWindows,
            WindowsAction::LockWorkstation,
        ];
        for action in actions {
            assert!(action.validate().is_ok(), "{action:?}");
            assert!(!action.kind().is_empty());
        }
        let source = serde_json::to_string(&WindowsAction::SetVolume { percent: 40 }).unwrap();
        assert!(source.contains("set_volume"));
        assert!(!source.contains("command"));
    }

    #[test]
    fn numeric_arguments_are_range_checked_in_one_place() {
        assert!(WindowsAction::SetVolume { percent: 100 }.validate().is_ok());
        assert!(WindowsAction::SetVolume { percent: 101 }
            .validate()
            .is_err());
        assert!(WindowsAction::ChangeVolume {
            direction: VolumeDirection::Up,
            step: 25
        }
        .validate()
        .is_ok());
        assert!(WindowsAction::ChangeVolume {
            direction: VolumeDirection::Up,
            step: 26
        }
        .validate()
        .is_err());
        assert!(WindowsAction::ChangeVolume {
            direction: VolumeDirection::Up,
            step: 0
        }
        .validate()
        .is_err());
        assert!(WindowsAction::CreateTimer {
            duration_seconds: 5
        }
        .validate()
        .is_ok());
        assert!(WindowsAction::CreateTimer {
            duration_seconds: 4
        }
        .validate()
        .is_err());
        assert!(WindowsAction::CreateTimer {
            duration_seconds: 24 * 60 * 60 + 1
        }
        .validate()
        .is_err());
        assert!(WindowsAction::CreateReminder {
            delay_seconds: 29,
            message: "x".to_string()
        }
        .validate()
        .is_err());
        assert!(WindowsAction::CreateReminder {
            delay_seconds: 30,
            message: "  ".to_string()
        }
        .validate()
        .is_err());
        assert!(WindowsAction::CreateReminder {
            delay_seconds: 30,
            message: "x".repeat(MAX_REMINDER_CHARS + 1)
        }
        .validate()
        .is_err());
        assert!(WindowsAction::CreateReminder {
            delay_seconds: 30,
            message: "перерыв\nвторой".to_string()
        }
        .validate()
        .is_err());
        assert!(WindowsAction::TakeScreenshot {
            target: ScreenshotTarget::SelectedMonitor(200)
        }
        .validate()
        .is_err());
    }

    #[test]
    fn window_operations_carry_typed_numbers_only() {
        let window = WindowId::from_stored("aabbccddeeff0011").unwrap();
        let move_action = WindowsAction::Window {
            window_id: window.clone(),
            operation: WindowOperation::Move {
                x: 10,
                y: 20,
                width: 800,
                height: 600,
            },
        };
        assert!(move_action.validate().is_ok());
        assert_eq!(move_action.kind(), "move_window");
        let too_far = WindowsAction::Window {
            window_id: window.clone(),
            operation: WindowOperation::Move {
                x: MAX_MOVE_PIXELS + 1,
                y: 0,
                width: 10,
                height: 10,
            },
        };
        assert!(too_far.validate().is_err());
        let empty = WindowsAction::Window {
            window_id: window,
            operation: WindowOperation::Move {
                x: 0,
                y: 0,
                width: 0,
                height: 10,
            },
        };
        assert!(empty.validate().is_err());
        assert_eq!(
            WindowsAction::Window {
                window_id: WindowId::from_stored("aabbccddeeff0011").unwrap(),
                operation: WindowOperation::Close,
            }
            .kind(),
            "close_window"
        );
    }

    #[test]
    fn identifiers_are_bounded_and_strictly_shaped() {
        assert!(ApplicationId::from_stored("app_notepad").is_ok());
        assert!(ApplicationId::from_stored("").is_err());
        assert!(ApplicationId::from_stored("../../evil").is_err());
        assert!(ApplicationId::from_stored("C:/Windows/System32/cmd.exe").is_err());
        assert!(ApplicationId::from_stored("a".repeat(65)).is_err());
        assert!(WindowId::from_stored("aabbccdd").is_ok());
        assert!(WindowId::from_stored("not-hex!").is_err());
        assert!(TimerId::from_stored("0123456789abcdef").is_ok());
        assert!(TimerId::from_stored("").is_err());
        assert!(ActionId::new().unwrap().as_str().len() == 16);
        assert!(WindowId::mint().unwrap().as_str().len() == 16);
    }

    #[test]
    fn a_request_validates_its_action() {
        let request = ActionRequest::new(WindowsAction::GetVolume, ActionSource::DirectGui, "now");
        assert!(request.is_ok());
        let bad = ActionRequest::new(
            WindowsAction::SetVolume { percent: 200 },
            ActionSource::LocalAi,
            "now",
        );
        assert!(bad.is_err());
    }

    #[test]
    fn a_title_is_sanitized_and_truncated() {
        assert_eq!(sanitize_window_title("  Simple \t title  "), "Simple title");
        assert_eq!(
            // The newline separates the two words; the bell is dropped without a space, so a
            // marker word cannot be spaced apart.
            sanitize_window_title("line\nbreak\u{7}bell"),
            "line breakbell"
        );
        let long = "я".repeat(MAX_WINDOW_TITLE_CHARS + 50);
        assert_eq!(
            sanitize_window_title(&long).chars().count(),
            MAX_WINDOW_TITLE_CHARS
        );
        assert_eq!(sanitize_window_title(""), "");
    }

    #[test]
    fn a_sensitive_looking_title_is_flagged_but_only_by_a_marker() {
        assert!(title_looks_sensitive("JARVIS — Vault"));
        assert!(title_looks_sensitive("Введите пароль"));
        assert!(title_looks_sensitive("Windows Security credential prompt"));
        assert!(!title_looks_sensitive("Calculator"));
        assert!(!title_looks_sensitive("Untitled — Notepad"));
    }

    #[test]
    fn a_sensitive_title_is_replaced_before_it_can_be_exposed() {
        let title = "OpenAI key sk-FICTIONAL0000000000000000000000000000";
        assert_eq!(safe_window_title(title), "Sensitive window");
        assert_eq!(safe_window_title("Quarterly plan — Notepad"), "Quarterly plan — Notepad");
    }

    #[test]
    fn statuses_and_sources_have_stable_labels() {
        assert_eq!(ActionStatus::Requested.as_str(), "requested");
        assert!(ActionStatus::Cancelled.is_terminal());
        assert!(!ActionStatus::Requested.is_terminal());
        assert!(ActionSource::LocalAi.is_remote());
        assert!(ActionSource::Voice.is_remote());
        assert!(!ActionSource::DirectGui.is_remote());
        assert_eq!(ActionRisk::Confirm.as_str(), "confirm");
    }

    #[test]
    fn a_preview_and_an_outcome_serialize_without_payload_fields() {
        let preview = ActionPreview {
            token: "0123456789abcdef0123456789abcdef".to_string(),
            action_kind: "take_screenshot".to_string(),
            risk: ActionRisk::Confirm,
            source: ActionSource::LocalAi,
            title_key: "windows-confirm-screenshot".to_string(),
            fields: vec![PreviewField {
                label_key: "windows-field-target".to_string(),
                value: "primary_monitor".to_string(),
            }],
            consequences: vec!["windows-consequence-screenshot".to_string()],
            expires_in_seconds: 45,
            cancellable: true,
        };
        let encoded = serde_json::to_string(&preview).unwrap();
        assert!(encoded.contains("take_screenshot"));
        assert!(
            !encoded.contains("args") && !encoded.contains("path") && !encoded.contains("command")
        );
    }
}
