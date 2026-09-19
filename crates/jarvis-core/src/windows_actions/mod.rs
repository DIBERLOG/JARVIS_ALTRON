//! Safe Windows actions: a closed set of typed actions, one central policy, one confirmation
//! gate, one executor, and one audit log.
//!
//! ```text
//! DirectGui ─┐
//! Voice ─────┤
//! LocalAi ───┼─► ActionRequest ─► ActionPolicy ─► ConfirmationGate ─► WindowsActionExecutor
//! Timer ─────┘                       (Safe/Confirm/Forbidden)   │              │
//!                                                               │              ├─ Win32 / Core Audio / GDI
//!                                                               │              └─ AuditLog
//!                                                               └─ ActionPreview ─► the dialog
//! ```
//!
//! What this feature deliberately does **not** have, by construction rather than by review:
//!
//! * no variant of [`WindowsAction`] carries a command line, a shell string, an executable
//!   path, or an argument list, so there is nothing for a caller to smuggle in. The four
//!   shapes the task forbids — `RawCommand`, `Shell`, `PowerShell`,
//!   `ExecutablePathFromAi`, `ArbitraryArguments` — cannot be expressed;
//! * no function turns prose into an action. The model path accepts a structured tool call and
//!   nothing else, and an unknown tool name is an error;
//! * no path to the vault, the notes storage, the AI memory, the master key, the registry,
//!   file deletion, shutdown, process termination, elevation, or another user's session. A
//!   structural test in `crates/jarvis-core/tests/windows_actions_isolation.rs` scans these
//!   modules and the command module for each of them.
//!
//! Wake-on-LAN is not implemented here, is not planned, and is excluded from the project.
//!
//! The user-facing description, the risk table, and the honest limitations live in
//! `docs/WINDOWS_ACTIONS.md`, `docs/ADR_WINDOWS_ACTIONS.md`, and
//! `docs/THREAT_MODEL_WINDOWS_ACTIONS.md`.

pub mod allowlist;
pub mod audit;
pub mod backend;
pub mod error;
pub mod executor;
pub mod model;
pub mod policy;
pub mod session;
pub mod timers;
pub mod tools;
pub mod voice;

pub use allowlist::{
    file_identity, validate_executable, AllowedApplication, AllowedApplicationDraft,
    AllowedApplications, CanonicalExecutable, FileIdentity, IdentityCheck, LaunchSpec,
    ALLOWED_APPLICATIONS_FILE,
};
pub use audit::{AuditEntry, AuditLog, AUDIT_FILE, AUDIT_ROTATED_FILE};
pub use backend::{
    platform_backend, Capabilities, CaptureRequest, FakeBackend, NativeWindow, UnsupportedBackend,
    VolumeState, WindowsBackend,
};
pub use error::ActionError;
pub use executor::{ScreenshotSettings, WindowRegistry, WindowsActionExecutor, SCREENSHOT_PREFIX};
pub use model::{
    sanitize_window_title, title_looks_sensitive, ActionId, ActionPreview, ActionRequest,
    ActionRequestOutcome, ActionResult, ActionRisk, ActionSource, ActionStatus, ActionValue,
    ApplicationId, PreviewField, ScreenshotTarget, TimerId, VolumeDirection, WindowId,
    WindowOperation, WindowState, WindowSummary, WindowsAction, CONFIRMATION_TTL_SECONDS,
    MAX_ACTIVE_REMINDERS, MAX_ACTIVE_TIMERS, MAX_ALLOWED_APPLICATIONS, MAX_MOVE_PIXELS,
    MAX_REMINDER_CHARS, MAX_RETIRED_WINDOW_IDS, MAX_VOLUME_STEP_PERCENT, MAX_WINDOW_TITLE_CHARS,
    WINDOW_ID_TTL_SECONDS,
};
pub use policy::{
    reminder_looks_sensitive, ActionPolicy, FORBIDDEN_EXECUTABLE_NAMES, POLICY_TABLE,
    SHELL_LIKE_NAMES,
};
pub use session::{FiredHook, VoiceRoute, WindowsActionSettings, WindowsActions, SETTINGS_FILE};
pub use timers::{
    FireSink, ScheduledItem, ScheduledKind, ScheduledStatus, ScheduledView, TimerScheduler,
    TimersState, TIMERS_FILE,
};
pub use tools::{
    availability as tool_availability, decode_tool_call, tool_catalogue, ToolAvailability,
    ToolDefinition, FORBIDDEN_ARGUMENT_NAMES, TOOLS_HINT,
};
pub use voice::{VoiceOutcome, VOICE_CANCEL_WORDS, VOICE_CONFIRM_WORDS};
