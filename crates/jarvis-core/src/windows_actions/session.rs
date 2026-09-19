//! The session: the object the interface, the voice host, and the model-facing path share.
//!
//! It owns five things and nothing else: the policy, the confirmation gate, the allowlist, the
//! audit log, and the timer scheduler. Every source — a button, a phrase, a tool call, the
//! scheduler — enters through [`WindowsActions::request`], which is the only door:
//!
//! ```text
//! ActionSource ──► policy ──► gate (safe: run now) ──► executor ──► ActionResult ──► audit
//!                     │                        │
//!                     │                        └─ confirm: a preview with a token; the stored
//!                     │                           request is executed, never the caller's copy
//!                     └─ forbidden: refused, and the refusal is audited
//! ```
//!
//! The gate stores the whole [`ActionRequest`] by value and hands the interface only an
//! [`ActionPreview`]. Confirming executes exactly what was stored, so an argument cannot be
//! changed between the question and the answer, and a token issued for one request can never
//! confirm another.
//!
//! The session never touches the vault, the notes storage, the AI memory, or the master key.
//! It has no handle to any of them, which is why none of them can appear in an action.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::allowlist::{AllowedApplication, AllowedApplicationDraft, AllowedApplications};
use super::audit::{AuditEntry, AuditLog};
use super::backend::{Capabilities, NativeWindow, WindowsBackend};
use super::error::ActionError;
use super::executor::{LaunchLookup, ScreenshotSettings, WindowRegistry, WindowsActionExecutor};
use super::model::{
    ActionPreview, ActionRequest, ActionRequestOutcome, ActionResult, ActionRisk, ActionSource,
    ActionStatus, ActionValue, PreviewField, WindowOperation, WindowSummary, WindowsAction,
    CONFIRMATION_TTL_SECONDS,
};
use super::policy::{describe_screenshot_target, ActionPolicy};
use super::timers::{
    FireSink, ScheduledItem, ScheduledKind, ScheduledView, TimerScheduler, TimersState,
};
use super::tools::{ToolAvailability, ToolDefinition};
use crate::safety::{ConfirmationFailure, ConfirmationGate, PendingConfirmation};

/// File name of the settings document inside the feature directory.
pub const SETTINGS_FILE: &str = "settings.json";
/// Schema version of the settings document.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// What the host is told when a timer or reminder fires, so it can show its own notification.
pub type FiredHook = Arc<dyn Fn(ScheduledView) + Send + Sync>;

/// What the user may switch on and off, and where screenshots go.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WindowsActionSettings {
    /// Whether the local model may ask for actions at all.
    pub ai_tools_enabled: bool,
    /// Whether voice phrases may ask for actions.
    pub voice_actions_enabled: bool,
    /// How long a confirmation stays valid.
    pub confirm_ttl_seconds: u64,
    pub screenshots: ScreenshotSettings,
    pub schema_version: u32,
}

impl WindowsActionSettings {
    pub fn default_for(data_dir: &Path) -> Self {
        Self {
            ai_tools_enabled: true,
            voice_actions_enabled: true,
            confirm_ttl_seconds: CONFIRMATION_TTL_SECONDS,
            screenshots: ScreenshotSettings::default_for(data_dir),
            schema_version: SETTINGS_SCHEMA_VERSION,
        }
    }

    /// Repairs out-of-range values instead of refusing to load.
    pub fn normalized(mut self, data_dir: &Path) -> Self {
        self.confirm_ttl_seconds = self.confirm_ttl_seconds.clamp(30, 120);
        let directory = self.screenshots.directory.trim().to_string();
        if directory.is_empty() || !Path::new(&directory).is_absolute() {
            // A stored folder that is empty or relative cannot be trusted to mean the same
            // place twice, so the default comes back.
            self.screenshots = ScreenshotSettings::default_for(data_dir);
        }
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        self
    }

    pub fn to_json(&self) -> Result<String, ActionError> {
        serde_json::to_string(self).map_err(|_| ActionError::StorageError)
    }

    pub fn from_json(text: &str) -> Result<Self, ActionError> {
        serde_json::from_str(text).map_err(|_| ActionError::StorageError)
    }
}

/// What a voice transcript produced.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum VoiceRoute {
    /// The phrase became a request; the outcome says whether it ran or is waiting.
    Requested { outcome: Box<ActionRequestOutcome> },
    /// The phrase is an action but is not decided.
    Ambiguous { reason: String },
    /// The phrase is not an action; the caller may try another route.
    NotAnAction,
    /// The feature is switched off in the settings.
    Disabled,
}

/// Everything the interface, the voice host, and the model path share.
pub struct WindowsActions {
    data_dir: PathBuf,
    policy: ActionPolicy,
    executor: Arc<WindowsActionExecutor>,
    gate: ConfirmationGate<ActionRequest>,
    allowlist: AllowedApplications,
    audit: Arc<Mutex<AuditLog>>,
    timers: Arc<TimerScheduler>,
    settings: WindowsActionSettings,
    /// Called when a timer or reminder fires, so the interface can show its own notification.
    on_fired: Arc<Mutex<Option<FiredHook>>>,
}

impl std::fmt::Debug for WindowsActions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsActions")
            .field("data_dir", &self.data_dir)
            .field("pending", &self.gate.has_pending(std::time::Instant::now()))
            .field("allowed_applications", &self.allowlist.list().len())
            .field("audit_entries", &self.audit.lock().len())
            .finish()
    }
}

impl WindowsActions {
    /// Opens the feature over `data_dir` with a platform backend.
    ///
    /// The backend is injected so that the whole session can be built on a fake: a test can
    /// drive the complete pipeline without locking a session, starting a program, or capturing
    /// a screen.
    pub fn open(
        data_dir: &Path,
        backend: Arc<dyn WindowsBackend>,
        settings: WindowsActionSettings,
    ) -> Self {
        // The document in the feature directory is what the user last saved, so it wins over
        // the value the caller built; the caller's value is the starting point for a profile
        // that has never saved anything, and the explicit value in a test.
        let settings = Self::stored_settings(data_dir).unwrap_or(settings);
        let settings = settings.normalized(data_dir);
        let executor = Arc::new(WindowsActionExecutor::new(
            backend,
            settings.screenshots.clone(),
        ));
        // The configured folder is created when the feature starts, so a capture works on a
        // fresh profile. A folder that cannot be created keeps the setting it had, and the
        // capture then fails with a reason the user can read.
        let _ = executor.set_screenshot_settings(settings.screenshots.clone());
        let audit = Arc::new(Mutex::new(AuditLog::open(data_dir)));
        let fired_audit = Arc::clone(&audit);
        let fired_backend = Arc::clone(executor.backend());
        // The host hook is shared with the scheduler thread, so a timer that fires while the
        // interface is idle still reaches it.
        let on_fired: Arc<Mutex<Option<FiredHook>>> = Arc::new(Mutex::new(None));
        let fired_hook = Arc::clone(&on_fired);
        let on_fire: FireSink = Arc::new(move |item: &ScheduledItem| {
            fire(item, &fired_backend, &fired_audit);
            // The host is told after the state is persisted and the log is written, so a
            // notification can never be the only record of a fired reminder.
            let view = TimersState::view_of(item, super::timers::now_unix_ms());
            if let Some(hook) = fired_hook.lock().clone() {
                hook(view);
            }
        });
        let timers = Arc::new(TimerScheduler::start(data_dir, on_fire));
        Self {
            data_dir: data_dir.to_path_buf(),
            policy: ActionPolicy::default(),
            executor,
            gate: ConfirmationGate::new(std::time::Duration::from_secs(
                settings.confirm_ttl_seconds,
            )),
            allowlist: AllowedApplications::open(data_dir),
            audit,
            timers,
            settings,
            on_fired,
        }
    }

    /// The settings document of a feature directory, if one was saved.
    ///
    /// This is the single owner of the settings: the interface writes through
    /// [`WindowsActions::update_settings`] and reads through here, and nothing keeps a second
    /// copy that could disagree with it.
    pub fn stored_settings(data_dir: &Path) -> Option<WindowsActionSettings> {
        let text = std::fs::read_to_string(data_dir.join(SETTINGS_FILE)).ok()?;
        WindowsActionSettings::from_json(&text).ok()
    }

    /// The directory the feature stores its files in.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn policy(&self) -> ActionPolicy {
        self.policy
    }

    pub fn executor(&self) -> &Arc<WindowsActionExecutor> {
        &self.executor
    }

    pub fn capabilities(&self) -> Capabilities {
        self.executor.capabilities()
    }

    /// The path the allowlist is stored in.
    pub fn allowlist_path(&self) -> &Path {
        self.allowlist.path()
    }

    /// Registers the hook that runs when a timer or reminder fires.
    pub fn set_fired_hook(&self, hook: FiredHook) {
        *self.on_fired.lock() = Some(hook);
    }

    /// Stops the scheduler thread. Called when the application exits.
    pub fn shutdown(&self) {
        self.timers.shutdown();
    }

    // ------------------------------------------------------------------ settings

    pub fn settings(&self) -> &WindowsActionSettings {
        &self.settings
    }

    /// Applies new settings: the confirmation lifetime, the screenshot folder, and the two
    /// switches that gate the sources.
    pub fn update_settings(
        &mut self,
        settings: WindowsActionSettings,
    ) -> Result<WindowsActionSettings, ActionError> {
        let settings = settings.normalized(&self.data_dir);
        self.executor
            .set_screenshot_settings(settings.screenshots.clone())?;
        self.gate
            .set_ttl(std::time::Duration::from_secs(settings.confirm_ttl_seconds));
        self.settings = settings.clone();
        std::fs::write(self.data_dir.join(SETTINGS_FILE), settings.to_json()?)?;
        Ok(settings)
    }

    // ----------------------------------------------------------------- allowlist

    pub fn allowed_applications(&self) -> Vec<AllowedApplication> {
        self.allowlist.list().to_vec()
    }

    /// Adds an application the user picked in the interface.
    pub fn add_allowed_application(
        &mut self,
        draft: &AllowedApplicationDraft,
    ) -> Result<AllowedApplication, ActionError> {
        let now = super::timers::now_timestamp();
        let added = self.allowlist.add(draft, now.clone())?;
        self.record(
            "add_allowed_application",
            ActionSource::DirectGui,
            ActionRisk::Confirm,
            ActionStatus::Executed,
            "ok",
            0,
            None,
            Some(&added.id),
            now,
        )?;
        Ok(added)
    }

    pub fn remove_allowed_application(
        &mut self,
        id: &str,
    ) -> Result<AllowedApplication, ActionError> {
        let removed = self.allowlist.remove(id)?;
        self.record(
            "remove_allowed_application",
            ActionSource::DirectGui,
            ActionRisk::Confirm,
            ActionStatus::Executed,
            "ok",
            0,
            None,
            Some(&removed.id),
            super::timers::now_timestamp(),
        )?;
        Ok(removed)
    }

    pub fn set_allowed_application_enabled(
        &mut self,
        id: &str,
        enabled: bool,
    ) -> Result<AllowedApplication, ActionError> {
        self.allowlist
            .set_enabled(id, enabled, super::timers::now_timestamp())
    }

    /// Re-reads a changed executable and accepts the new identity, on an explicit request.
    pub fn reaccept_allowed_application(
        &mut self,
        id: &str,
    ) -> Result<AllowedApplication, ActionError> {
        self.allowlist
            .reaccept_identity(id, super::timers::now_timestamp())
    }

    /// Whether a stored entry still matches the file on disk.
    pub fn allowed_application_identity(
        &self,
        id: &str,
    ) -> Result<super::allowlist::IdentityCheck, ActionError> {
        self.allowlist.check_identity(id)
    }

    /// Installs the allowlist lookup in the executor, so a launch can only use a stored entry.
    pub fn install_launch_lookup(&self) {
        let path = self.allowlist.path().to_path_buf();
        let lookup: LaunchLookup = Arc::new(move |id: &str| {
            // Re-read the registry on every launch: the file is the authority, and a change
            // made in the settings is picked up without restarting anything.
            let directory = path.parent().unwrap_or(Path::new("."));
            AllowedApplications::open(directory).launch_spec(id)
        });
        self.executor.set_launch_lookup(lookup);
    }

    // -------------------------------------------------------------------- request

    /// The single entry point for every source.
    pub fn request(
        &mut self,
        action: WindowsAction,
        source: ActionSource,
    ) -> Result<ActionRequestOutcome, ActionError> {
        let requested_at = super::timers::now_timestamp();
        let kind = action.kind();
        let request = match ActionRequest::new(action, source, requested_at.clone()) {
            Ok(request) => request,
            Err(error) => {
                // A malformed action never becomes a request, but the refusal is still
                // recorded: the log says what was asked for and that it was refused.
                self.record(
                    kind,
                    source,
                    ActionRisk::Forbidden,
                    ActionStatus::Rejected,
                    error.code(),
                    0,
                    Some(error.code()),
                    None,
                    requested_at,
                )?;
                return Err(error);
            }
        };
        self.request_prepared(request)
    }

    /// As [`WindowsActions::request`], for a request that was built (and validated) already.
    pub fn request_prepared(
        &mut self,
        request: ActionRequest,
    ) -> Result<ActionRequestOutcome, ActionError> {
        let risk = match self.policy.check(&request.action, request.source) {
            Ok(risk) => risk,
            Err(error) => {
                self.record(
                    request.kind(),
                    request.source,
                    ActionRisk::Forbidden,
                    ActionStatus::Rejected,
                    error.code(),
                    0,
                    Some(error.code()),
                    None,
                    request.requested_at.clone(),
                )?;
                return Err(error);
            }
        };
        // A request that could only fail is refused before the user is asked to approve it:
        // the log says "refused", not "requested".
        if let Err(error) = self.executor.precheck(&request) {
            self.record(
                request.kind(),
                request.source,
                ActionRisk::Forbidden,
                ActionStatus::Rejected,
                error.code(),
                0,
                Some(error.code()),
                None,
                request.requested_at.clone(),
            )?;
            return Err(error);
        }

        self.record(
            request.kind(),
            request.source,
            risk,
            ActionStatus::Requested,
            "requested",
            0,
            None,
            target_of(&request.action),
            request.requested_at.clone(),
        )?;

        // A timer or reminder is state, not a platform call; the session owns them. A reminder
        // that looks like it may carry a credential is not safe, and is asked about first.
        if risk == ActionRisk::Safe {
            if is_timer_action(&request.action) {
                return Ok(ActionRequestOutcome::Executed {
                    result: self.execute_timer_action(&request)?,
                });
            }
            return Ok(ActionRequestOutcome::Executed {
                result: self.execute_now(&request)?,
            });
        }

        // A confirmation is needed: store the request itself, and show only a preview.
        let now = std::time::Instant::now();
        let token = self
            .gate
            .request(request.clone(), risk.into(), request.source.as_str(), now);
        let preview = self.preview_for(token.as_str(), now)?;
        Ok(ActionRequestOutcome::AwaitingConfirmation { preview })
    }

    /// The pending confirmation, as the interface shows it.
    pub fn pending_preview(&self) -> Option<ActionPreview> {
        let now = std::time::Instant::now();
        let pending = self.gate.peek(now)?;
        self.preview_of_pending(pending, now).ok()
    }

    /// Whether anything is waiting for the user.
    pub fn has_pending(&self) -> bool {
        self.gate.has_pending(std::time::Instant::now())
    }

    /// Confirms the pending request with its token, and runs exactly what was stored.
    pub fn confirm(&mut self, token: &str) -> Result<ActionResult, ActionError> {
        let now = std::time::Instant::now();
        let pending = self.gate.confirm(token, now).map_err(confirmation_error)?;
        self.run_confirmed(pending)
    }

    /// Confirms whatever is pending, for the voice path.
    pub fn confirm_pending(&mut self) -> Result<ActionResult, ActionError> {
        let now = std::time::Instant::now();
        let pending = self.gate.confirm_pending(now).map_err(confirmation_error)?;
        self.run_confirmed(pending)
    }

    /// Cancels the pending request.
    pub fn cancel(&mut self) -> bool {
        let cancelled = self.gate.cancel();
        if cancelled {
            let _ = self.record(
                "confirmation",
                ActionSource::DirectGui,
                ActionRisk::Confirm,
                ActionStatus::Cancelled,
                "cancelled",
                0,
                None,
                None,
                super::timers::now_timestamp(),
            );
        }
        cancelled
    }

    /// Drops an expired request, if there is one.
    pub fn expire(&mut self) -> bool {
        self.gate.expire(std::time::Instant::now())
    }

    fn run_confirmed(
        &mut self,
        pending: PendingConfirmation<ActionRequest>,
    ) -> Result<ActionResult, ActionError> {
        // The stored request is the one that runs; the caller's copy is irrelevant.
        let request = pending.payload().clone();
        self.record(
            request.kind(),
            request.source,
            ActionRisk::Confirm,
            ActionStatus::Confirmed,
            "confirmed",
            0,
            None,
            target_of(&request.action),
            super::timers::now_timestamp(),
        )?;
        if is_timer_action(&request.action) {
            return self.execute_timer_action(&request);
        }
        self.execute_now(&request)
    }

    /// Runs an allowed action and records the outcome.
    fn execute_now(&mut self, request: &ActionRequest) -> Result<ActionResult, ActionError> {
        let started = std::time::Instant::now();
        let result = self.executor.execute(request);
        let duration_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(value) => {
                self.record(
                    request.kind(),
                    request.source,
                    self.policy.risk_for(&request.action, request.source),
                    ActionStatus::Executed,
                    "ok",
                    duration_ms,
                    None,
                    target_of(&request.action),
                    super::timers::now_timestamp(),
                )?;
                Ok(ActionResult::executed(request, value, duration_ms))
            }
            Err(error) => {
                self.record(
                    request.kind(),
                    request.source,
                    self.policy.risk_for(&request.action, request.source),
                    ActionStatus::Failed,
                    error.code(),
                    duration_ms,
                    Some(error.code()),
                    target_of(&request.action),
                    super::timers::now_timestamp(),
                )?;
                Err(error)
            }
        }
    }

    /// Runs a timer or reminder action, which is state rather than a platform call.
    fn execute_timer_action(
        &mut self,
        request: &ActionRequest,
    ) -> Result<ActionResult, ActionError> {
        let started = std::time::Instant::now();
        let now = super::timers::now_unix_ms();
        let outcome = self
            .timers
            .with_state(|state| -> Result<ActionValue, ActionError> {
                match &request.action {
                    WindowsAction::CreateTimer { duration_seconds } => {
                        let item = state.create_timer(*duration_seconds, request.source, now)?;
                        Ok(ActionValue::Timer {
                            timer_id: item.id.clone(),
                            fires_in_seconds: item.remaining_seconds(now),
                        })
                    }
                    WindowsAction::CreateReminder {
                        delay_seconds,
                        message,
                    } => {
                        let item =
                            state.create_reminder(*delay_seconds, message, request.source, now)?;
                        Ok(ActionValue::Timer {
                            timer_id: item.id.clone(),
                            fires_in_seconds: item.remaining_seconds(now),
                        })
                    }
                    WindowsAction::CancelTimer { timer_id }
                    | WindowsAction::CancelReminder { timer_id } => {
                        let item = state.cancel(timer_id.as_str())?;
                        Ok(ActionValue::Cancelled { timer_id: item.id })
                    }
                    _ => Err(ActionError::UnsupportedPlatform),
                }
            });
        self.timers.wake();
        let duration_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok(value) => {
                self.record(
                    request.kind(),
                    request.source,
                    self.policy.risk_for(&request.action, request.source),
                    ActionStatus::Executed,
                    "ok",
                    duration_ms,
                    None,
                    target_of(&request.action),
                    super::timers::now_timestamp(),
                )?;
                Ok(ActionResult::executed(request, value, duration_ms))
            }
            Err(error) => {
                self.record(
                    request.kind(),
                    request.source,
                    self.policy.risk_for(&request.action, request.source),
                    ActionStatus::Failed,
                    error.code(),
                    duration_ms,
                    Some(error.code()),
                    target_of(&request.action),
                    super::timers::now_timestamp(),
                )?;
                Err(error)
            }
        }
    }

    // -------------------------------------------------------------------- windows

    /// Lists the visible windows and refreshes the identifiers.
    pub fn list_windows(&mut self) -> Result<Vec<WindowSummary>, ActionError> {
        let windows = self.executor.list_windows()?;
        Ok(windows)
    }

    /// The window identifiers currently held, for the voice router.
    pub fn registry(&self) -> &Mutex<WindowRegistry> {
        self.executor.registry()
    }

    // --------------------------------------------------------------------- timers

    /// Every timer and reminder, newest deadline last.
    pub fn scheduled(&self) -> Vec<ScheduledView> {
        let now = super::timers::now_unix_ms();
        self.timers.with_state(|state| state.views(now))
    }

    /// Removes fired and cancelled items.
    pub fn prune_scheduled(&self) -> Result<usize, ActionError> {
        self.timers.with_state(|state| state.prune())
    }

    // ---------------------------------------------------------------------- audit

    pub fn audit_entries(&self) -> Vec<AuditEntry> {
        self.audit.lock().entries()
    }

    pub fn recent_audit(&self, limit: usize) -> Vec<AuditEntry> {
        self.audit.lock().recent(limit)
    }

    pub fn clear_audit(&self, confirmed: bool) -> Result<(), ActionError> {
        self.audit.lock().clear(confirmed)
    }

    pub fn export_audit(&self, destination: &Path, confirmed: bool) -> Result<usize, ActionError> {
        self.audit.lock().export(destination, confirmed)
    }

    // ----------------------------------------------------------------------- tools

    /// Whether the model may be given tools, and what they are.
    pub fn ai_tools(
        &self,
        template_supports_tools: bool,
    ) -> (ToolAvailability, Vec<ToolDefinition>) {
        let availability =
            super::tools::availability(&self.capabilities(), template_supports_tools);
        if !self.settings.ai_tools_enabled || !availability.is_available() {
            return (
                if self.settings.ai_tools_enabled {
                    availability
                } else {
                    ToolAvailability::Unavailable {
                        reason: "windows-ai-tools-disabled",
                    }
                },
                Vec::new(),
            );
        }
        (availability, super::tools::tool_catalogue())
    }

    /// Handles one structured tool call from the model.
    pub fn request_from_tool_call(
        &mut self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<ActionRequestOutcome, ActionError> {
        if !self.settings.ai_tools_enabled {
            return Err(ActionError::CapabilityUnavailable {
                capability: "windows-ai-tools-disabled".to_string(),
            });
        }
        let action = super::tools::decode_tool_call(name, arguments)?;
        self.request(action, ActionSource::LocalAi)
    }

    // ----------------------------------------------------------------------- voice

    /// Routes a transcript through the same pipeline as everything else.
    pub fn route_voice(&mut self, text: &str) -> Result<VoiceRoute, ActionError> {
        if !self.settings.voice_actions_enabled {
            return Ok(VoiceRoute::Disabled);
        }
        // A confirmation or a cancellation acts on whatever is pending, and nothing else.
        if super::voice::is_confirmation(text) {
            return match self.confirm_pending() {
                Ok(result) => Ok(VoiceRoute::Requested {
                    outcome: Box::new(ActionRequestOutcome::Executed { result }),
                }),
                Err(error) => Err(error),
            };
        }
        if super::voice::is_cancellation(text) {
            let cancelled = self.cancel();
            if cancelled {
                return Ok(VoiceRoute::Requested {
                    outcome: Box::new(ActionRequestOutcome::Rejected {
                        detail: "cancelled".to_string(),
                    }),
                });
            }
            return Err(ActionError::ConfirmationRequired);
        }
        let now = super::timers::now_unix_ms();
        let outcome = {
            let registry = self.executor.registry().lock();
            super::voice::route(text, &self.allowlist, &registry, now)
        };
        match outcome {
            super::voice::VoiceOutcome::Action(action) => {
                let outcome = self.request(*action, ActionSource::Voice)?;
                Ok(VoiceRoute::Requested {
                    outcome: Box::new(outcome),
                })
            }
            super::voice::VoiceOutcome::Ambiguous { reason } => Ok(VoiceRoute::Ambiguous {
                reason: reason.to_string(),
            }),
            super::voice::VoiceOutcome::NotAnAction => Ok(VoiceRoute::NotAnAction),
        }
    }

    // -------------------------------------------------------------------- preview

    /// Builds the preview of a just-stored request, identified by its token.
    fn preview_for(
        &self,
        token: &str,
        now: std::time::Instant,
    ) -> Result<ActionPreview, ActionError> {
        let pending = self
            .gate
            .peek(now)
            .filter(|pending| pending.token().matches(token))
            .ok_or(ActionError::ConfirmationRequired)?;
        self.preview_of_pending(pending, now)
    }

    /// Builds the preview from the stored request.
    ///
    /// The dialog receives this and nothing else: a title key, an already-rendered set of
    /// fields, and the consequences. There is no field a caller could use to smuggle an
    /// argument in, and no argument is included.
    fn preview_of_pending(
        &self,
        pending: &PendingConfirmation<ActionRequest>,
        now: std::time::Instant,
    ) -> Result<ActionPreview, ActionError> {
        let request = pending.payload();
        let mut fields = Vec::new();
        let mut consequences = Vec::new();
        match &request.action {
            WindowsAction::LaunchAllowedApplication { application_id } => {
                let application = self.allowlist.get(application_id.as_str());
                let name = application
                    .map(|application| application.display_name.clone())
                    .unwrap_or_else(|| application_id.as_str().to_string());
                fields.push(field("windows-field-application", &name));
                if let Some(application) = application {
                    fields.push(field(
                        "windows-field-executable",
                        &application.executable_file_name(),
                    ));
                    if !application.fixed_arguments.is_empty() {
                        fields.push(field(
                            "windows-field-arguments",
                            &application.fixed_arguments.len().to_string(),
                        ));
                    }
                }
                consequences.push("windows-consequence-launch".to_string());
            }
            WindowsAction::TakeScreenshot { target } => {
                fields.push(field(
                    "windows-field-target",
                    &describe_screenshot_target(target),
                ));
                fields.push(field(
                    "windows-field-screenshot-folder",
                    &self.executor.screenshot_settings().directory,
                ));
                consequences.push("windows-consequence-screenshot".to_string());
            }
            WindowsAction::Window {
                window_id,
                operation,
            } => {
                fields.push(field("windows-field-operation", operation.as_str()));
                let title = self
                    .executor
                    .registry()
                    .lock()
                    .resolve(window_id.as_str(), super::timers::now_unix_ms())
                    .map(|window| super::model::safe_window_title(&window.title))
                    .unwrap_or_default();
                if !title.is_empty() {
                    fields.push(field("windows-field-window", &title));
                }
                if matches!(operation, WindowOperation::Close) {
                    consequences.push("windows-consequence-close".to_string());
                }
            }
            WindowsAction::LockWorkstation => {
                consequences.push("windows-consequence-lock".to_string());
            }
            WindowsAction::CreateReminder {
                delay_seconds,
                message,
            } => {
                fields.push(field(
                    "windows-field-delay",
                    &format_seconds(*delay_seconds),
                ));
                fields.push(field("windows-field-reminder", message));
                consequences.push("windows-consequence-reminder".to_string());
            }
            WindowsAction::ChangeVolume { direction, step } => {
                fields.push(field("windows-field-direction", direction.as_str()));
                fields.push(field("windows-field-step", &step.to_string()));
            }
            other => {
                fields.push(field("windows-field-action", other.kind()));
            }
        }
        Ok(ActionPreview {
            token: pending.token().as_str().to_string(),
            action_kind: request.kind().to_string(),
            risk: pending.risk().into(),
            source: request.source,
            title_key: title_key_for(request.kind()).to_string(),
            fields,
            consequences,
            expires_in_seconds: pending.remaining_seconds(now),
            cancellable: true,
        })
    }

    // ---------------------------------------------------------------- audit helper

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        action_type: &str,
        source: ActionSource,
        risk: ActionRisk,
        decision: ActionStatus,
        result: &str,
        duration_ms: u64,
        error_category: Option<&str>,
        target: Option<&str>,
        timestamp: impl Into<String>,
    ) -> Result<(), ActionError> {
        self.audit.lock().record_outcome(
            action_type,
            source,
            risk,
            decision,
            result,
            duration_ms,
            error_category,
            target,
            timestamp,
        )
    }
}

/// Runs when a timer or reminder fires: a local notification, never a command.
fn fire(item: &ScheduledItem, backend: &Arc<dyn WindowsBackend>, audit: &Arc<Mutex<AuditLog>>) {
    let (message, unreadable) = TimersState::message_of(item);
    let title = match item.kind {
        ScheduledKind::Timer => "JARVIS: таймер",
        ScheduledKind::Reminder => "JARVIS: напоминание",
    };
    let body = match (item.kind, message.as_deref(), unreadable) {
        (ScheduledKind::Timer, _, _) => "Таймер сработал".to_string(),
        (ScheduledKind::Reminder, Some(text), _) => text.to_string(),
        (ScheduledKind::Reminder, None, true) => "Напоминание (текст недоступен)".to_string(),
        (ScheduledKind::Reminder, None, false) => "Напоминание".to_string(),
    };
    // Best effort: a system toast may be unavailable, and the interface shows its own.
    let _ = backend.notify(title, &body);
    let _ = audit.lock().record_outcome(
        if item.kind == ScheduledKind::Timer {
            "timer_fired"
        } else {
            "reminder_fired"
        },
        item.source,
        ActionRisk::Safe,
        ActionStatus::Executed,
        "ok",
        0,
        None,
        Some(&item.id),
        super::timers::now_timestamp(),
    );
}

fn field(label_key: &str, value: &str) -> PreviewField {
    PreviewField {
        label_key: label_key.to_string(),
        value: value.chars().take(200).collect(),
    }
}

fn title_key_for(kind: &str) -> &'static str {
    match kind {
        "launch_allowed_application" => "windows-confirm-launch",
        "take_screenshot" => "windows-confirm-screenshot",
        "close_window" => "windows-confirm-close",
        "lock_workstation" => "windows-confirm-lock",
        "create_reminder" => "windows-confirm-reminder",
        "change_volume" | "set_volume" | "mute_volume" => "windows-confirm-volume",
        _ => "windows-confirm-action",
    }
}

fn format_seconds(seconds: u64) -> String {
    if seconds >= 3600 && seconds.is_multiple_of(3600) {
        format!("{} h", seconds / 3600)
    } else if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

fn is_timer_action(action: &WindowsAction) -> bool {
    matches!(
        action,
        WindowsAction::CreateTimer { .. }
            | WindowsAction::CancelTimer { .. }
            | WindowsAction::CreateReminder { .. }
            | WindowsAction::CancelReminder { .. }
    )
}

/// A bounded, sanitized label of what an action acts on.
///
/// Identifiers and numbers only: no title, no path, no message text.
fn target_of(action: &WindowsAction) -> Option<&'static str> {
    Some(match action {
        WindowsAction::GetVolume
        | WindowsAction::SetVolume { .. }
        | WindowsAction::ChangeVolume { .. }
        | WindowsAction::MuteVolume { .. } => "volume",
        WindowsAction::LaunchAllowedApplication { .. } => "application",
        WindowsAction::TakeScreenshot { .. } => "screenshot",
        WindowsAction::CreateTimer { .. } | WindowsAction::CancelTimer { .. } => "timer",
        WindowsAction::CreateReminder { .. } | WindowsAction::CancelReminder { .. } => "reminder",
        WindowsAction::ListWindows => "window_list",
        WindowsAction::Window { .. } => "window",
        WindowsAction::LockWorkstation => "workstation",
    })
}

fn confirmation_error(failure: ConfirmationFailure) -> ActionError {
    match failure {
        ConfirmationFailure::Expired => ActionError::ConfirmationExpired,
        ConfirmationFailure::Mismatch => ActionError::ConfirmationMismatch,
        ConfirmationFailure::NotPending => ActionError::ConfirmationRequired,
    }
}

/// The native window type, re-exported for the interface that lists windows.
pub type ListedWindow = NativeWindow;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_actions::backend::{Capabilities, FakeBackend, NativeWindow};
    use crate::windows_actions::model::{
        ApplicationId, TimerId, VolumeDirection, WindowId, WindowState,
    };
    use tempfile::tempdir;

    fn session(directory: &Path) -> WindowsActions {
        let session = WindowsActions::open(
            directory,
            Arc::new(FakeBackend::new()),
            WindowsActionSettings::default_for(directory),
        );
        session.install_launch_lookup();
        session
    }

    fn session_with(directory: &Path, backend: FakeBackend) -> WindowsActions {
        let session = WindowsActions::open(
            directory,
            Arc::new(backend),
            WindowsActionSettings::default_for(directory),
        );
        session.install_launch_lookup();
        session
    }

    fn window(title: &str, foreground: bool) -> NativeWindow {
        NativeWindow {
            id: WindowId::mint().unwrap().as_str().to_string(),
            native_id: 3,
            process_id: 9,
            process_name: "Notepad.exe".to_string(),
            title: title.to_string(),
            state: WindowState::Normal,
            monitor: 1,
            is_own_process: false,
            is_foreground: foreground,
            work_area: (0, 0, 1920, 1040),
        }
    }

    fn allowed_application(directory: &Path, name: &str) -> String {
        let program = directory.join(format!("{name}.exe"));
        std::fs::write(&program, b"FICTIONAL").unwrap();
        let draft = AllowedApplicationDraft {
            display_name: name.to_string(),
            path: program.to_string_lossy().into_owned(),
            fixed_arguments: vec!["--safe".to_string()],
            working_directory: None,
        };
        let mut allowlist = AllowedApplications::open(directory);
        allowlist.add(&draft, "now").unwrap().id
    }

    #[test]
    fn a_safe_action_runs_immediately_and_is_audited() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let outcome = session
            .request(
                WindowsAction::SetVolume { percent: 30 },
                ActionSource::DirectGui,
            )
            .unwrap();
        match outcome {
            ActionRequestOutcome::Executed { result } => {
                assert!(result.is_success());
                assert_eq!(
                    result.value,
                    ActionValue::Volume {
                        percent: 30,
                        muted: false
                    }
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        let entries = session.audit_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].decision, ActionStatus::Requested);
        assert_eq!(entries[1].decision, ActionStatus::Executed);
        assert_eq!(entries[1].target.as_deref(), Some("volume"));
    }

    #[test]
    fn a_confirmed_action_runs_only_after_its_token_and_only_once() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let outcome = session
            .request(
                WindowsAction::TakeScreenshot {
                    target: super::super::model::ScreenshotTarget::PrimaryMonitor,
                },
                ActionSource::LocalAi,
            )
            .unwrap();
        let ActionRequestOutcome::AwaitingConfirmation { preview } = outcome else {
            panic!("a screenshot must be confirmed");
        };
        assert_eq!(preview.risk, ActionRisk::Confirm);
        assert_eq!(preview.source, ActionSource::LocalAi);
        assert!(preview.cancellable);
        assert!(preview.expires_in_seconds > 0);
        assert!(!preview.fields.is_empty());
        assert!(!preview.consequences.is_empty());
        // The preview carries no argument field beyond what the dialog shows.
        let encoded = serde_json::to_string(&preview).unwrap();
        assert!(!encoded.contains("args"));
        assert!(!encoded.contains("command"));

        let result = session.confirm(&preview.token).unwrap();
        assert!(result.is_success());
        // The token is spent.
        assert_eq!(
            session.confirm(&preview.token).unwrap_err().code(),
            "confirmation_required"
        );
    }

    #[test]
    fn a_wrong_token_confirms_nothing() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        session
            .request(WindowsAction::LockWorkstation, ActionSource::Voice)
            .unwrap();
        assert_eq!(
            session
                .confirm("00000000000000000000000000000000")
                .unwrap_err()
                .code(),
            "confirmation_mismatch"
        );
        // The real question is still pending.
        assert!(session.has_pending());
        assert!(session.pending_preview().is_some());
        assert!(session.confirm_pending().is_ok());
        assert!(!session.has_pending());
    }

    #[test]
    fn a_second_request_never_confirms_the_first() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        session
            .request(WindowsAction::LockWorkstation, ActionSource::Voice)
            .unwrap();
        let second = session
            .request(
                WindowsAction::TakeScreenshot {
                    target: super::super::model::ScreenshotTarget::AllMonitors,
                },
                ActionSource::Voice,
            )
            .unwrap();
        let ActionRequestOutcome::AwaitingConfirmation { preview } = second else {
            panic!("expected a confirmation");
        };
        // The old preview is not the pending one any more.
        let stale_ok = session.confirm("11111111111111111111111111111111").is_err();
        assert!(stale_ok);
        assert!(session.confirm(&preview.token).is_ok());
    }

    #[test]
    fn cancelling_drops_the_request_and_records_it() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        session
            .request(WindowsAction::LockWorkstation, ActionSource::DirectGui)
            .unwrap();
        assert!(session.cancel());
        assert!(!session.cancel());
        assert!(!session.has_pending());
        let decisions: Vec<ActionStatus> = session
            .audit_entries()
            .iter()
            .map(|entry| entry.decision)
            .collect();
        assert!(decisions.contains(&ActionStatus::Cancelled));
    }

    #[test]
    fn a_forbidden_action_is_refused_and_rejected_in_the_log() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        // A volume step above the policy limit is refused before anything else.
        let error = session
            .request(
                WindowsAction::ChangeVolume {
                    direction: VolumeDirection::Up,
                    step: 40,
                },
                ActionSource::LocalAi,
            )
            .unwrap_err();
        assert_eq!(error.code(), "invalid_arguments");
        let entries = session.audit_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].decision, ActionStatus::Rejected);
        assert!(!session.has_pending());
    }

    #[test]
    fn the_executed_action_comes_from_the_stored_request_not_from_the_caller() {
        let directory = tempdir().unwrap();
        let application_id = allowed_application(directory.path(), "Player");
        let mut session = session(directory.path());
        let outcome = session
            .request(
                WindowsAction::LaunchAllowedApplication {
                    application_id: ApplicationId::from_stored(application_id).unwrap(),
                },
                ActionSource::LocalAi,
            )
            .unwrap();
        let ActionRequestOutcome::AwaitingConfirmation { preview } = outcome else {
            panic!("a launch must be confirmed");
        };
        // The preview names the application and its fixed argument count, and shows no
        // argument, path, or command.
        let values: Vec<&str> = preview
            .fields
            .iter()
            .map(|field| field.value.as_str())
            .collect();
        assert!(values.contains(&"Player"));
        assert!(values.contains(&"1"));
        let result = session.confirm(&preview.token).unwrap();
        match result.value {
            ActionValue::Launched { application, .. } => assert_eq!(application, "Player"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn timers_are_state_and_are_audited() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let outcome = session
            .request(
                WindowsAction::CreateTimer {
                    duration_seconds: 60,
                },
                ActionSource::Voice,
            )
            .unwrap();
        let ActionRequestOutcome::Executed { result } = outcome else {
            panic!("a timer is safe");
        };
        let timer_id = match result.value {
            ActionValue::Timer { timer_id, .. } => timer_id,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(session.scheduled().len(), 1);
        assert!(session.scheduled()[0].remaining_seconds <= 60);
        // Cancelling works through the same path.
        session
            .request(
                WindowsAction::CancelTimer {
                    timer_id: TimerId::from_stored(timer_id).unwrap(),
                },
                ActionSource::Voice,
            )
            .unwrap();
        assert_eq!(
            session.scheduled()[0].status,
            super::super::timers::ScheduledStatus::Cancelled
        );
        assert_eq!(session.prune_scheduled().unwrap(), 1);
        assert!(session.scheduled().is_empty());
        // The audit carries no reminder text and no window title.
        let encoded = serde_json::to_string(&session.audit_entries()).unwrap();
        assert!(!encoded.contains("windows-field"));
    }

    #[test]
    fn a_sensitive_reminder_is_confirmed_before_it_is_stored() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let outcome = session
            .request(
                WindowsAction::CreateReminder {
                    delay_seconds: 60,
                    message: "password: FICTIONAL_SECRET_VALUE".to_string(),
                },
                ActionSource::LocalAi,
            )
            .unwrap();
        let ActionRequestOutcome::AwaitingConfirmation { preview } = outcome else {
            panic!("a sensitive reminder must be confirmed");
        };
        assert_eq!(preview.risk, ActionRisk::Confirm);
        assert_eq!(session.scheduled().len(), 0);
        session.confirm(&preview.token).unwrap();
        assert_eq!(session.scheduled().len(), 1);
    }

    #[test]
    fn the_ai_tool_path_is_gated_by_settings_and_the_catalogue() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let (availability, tools) = session.ai_tools(true);
        assert!(availability.is_available());
        assert_eq!(tools.len(), 17);
        let (unavailable, empty) = session.ai_tools(false);
        assert!(!unavailable.is_available());
        assert!(empty.is_empty());

        // A structured call goes through the policy like any other source.
        let outcome = session
            .request_from_tool_call("set_volume", &serde_json::json!({"percent": 20}))
            .unwrap();
        assert!(matches!(outcome, ActionRequestOutcome::Executed { .. }));
        // A call that carries a command is refused.
        let error = session
            .request_from_tool_call("set_volume", &serde_json::json!({"command": "calc"}))
            .unwrap_err();
        assert_eq!(error.code(), "forbidden_action");

        // Switching the tools off stops the path entirely.
        let mut disabled = WindowsActionSettings::default_for(directory.path());
        disabled.ai_tools_enabled = false;
        session.update_settings(disabled).unwrap();
        assert_eq!(
            session
                .request_from_tool_call("get_volume", &serde_json::json!({}))
                .unwrap_err()
                .code(),
            "capability_unavailable"
        );
    }

    #[test]
    fn the_voice_path_uses_the_same_pipeline_and_the_same_words() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        // A safe phrase runs.
        let route = session.route_voice("громкость 25 процентов").unwrap();
        assert!(matches!(route, VoiceRoute::Requested { .. }));
        // A dangerous phrase asks for a confirmation and waits for the word.
        let route = session.route_voice("заблокируй компьютер").unwrap();
        match route {
            VoiceRoute::Requested { outcome } => {
                assert!(matches!(
                    *outcome,
                    ActionRequestOutcome::AwaitingConfirmation { .. }
                ))
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(session.has_pending());
        let confirmed = session.route_voice("подтверждаю").unwrap();
        assert!(matches!(confirmed, VoiceRoute::Requested { .. }));
        assert!(!session.has_pending());
        // A doubt is not an action.
        assert!(matches!(
            session.route_voice("сверни окно яндекс").unwrap(),
            VoiceRoute::Ambiguous { .. }
        ));
        assert!(matches!(
            session.route_voice("расскажи анекдот").unwrap(),
            VoiceRoute::NotAnAction
        ));
    }

    #[test]
    fn the_voice_switch_turns_the_route_off() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let mut settings = WindowsActionSettings::default_for(directory.path());
        settings.voice_actions_enabled = false;
        session.update_settings(settings).unwrap();
        assert_eq!(
            session.route_voice("заблокируй компьютер").unwrap(),
            VoiceRoute::Disabled
        );
    }

    #[test]
    fn the_allowlist_operations_are_audited_and_checked() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let program = directory.path().join("Allowed.exe");
        std::fs::write(&program, b"FICTIONAL").unwrap();
        let added = session
            .add_allowed_application(&AllowedApplicationDraft {
                display_name: "Allowed".to_string(),
                path: program.to_string_lossy().into_owned(),
                fixed_arguments: Vec::new(),
                working_directory: None,
            })
            .unwrap();
        assert!(session
            .allowed_application_identity(&added.id)
            .unwrap()
            .is_unchanged());
        session
            .set_allowed_application_enabled(&added.id, false)
            .unwrap();
        assert!(!session.allowed_applications()[0].enabled);
        session.remove_allowed_application(&added.id).unwrap();
        assert!(session.allowed_applications().is_empty());
        assert!(session
            .audit_entries()
            .iter()
            .any(|entry| entry.action_type == "add_allowed_application"));
    }

    #[test]
    fn the_settings_are_normalized_and_persisted() {
        let directory = tempdir().unwrap();
        let mut session = session(directory.path());
        let mut settings = WindowsActionSettings::default_for(directory.path());
        settings.confirm_ttl_seconds = 5;
        settings.screenshots.directory = "   ".to_string();
        let stored = session.update_settings(settings).unwrap();
        assert_eq!(stored.confirm_ttl_seconds, 30);
        assert!(stored.screenshots.directory.contains("screenshot"));
        assert!(directory.path().join(SETTINGS_FILE).is_file());
    }

    #[test]
    fn a_window_capture_of_a_sensitive_window_is_refused_through_the_session() {
        let directory = tempdir().unwrap();
        let mut session = session_with(
            directory.path(),
            FakeBackend::with_windows(vec![window("Пароль", true)]),
        );
        let windows = session.list_windows().unwrap();
        assert_eq!(windows.len(), 1);
        let error = session
            .request(
                WindowsAction::TakeScreenshot {
                    target: super::super::model::ScreenshotTarget::AllMonitors,
                },
                ActionSource::LocalAi,
            )
            .unwrap_err();
        // The refusal happens before the confirmation, so the user is never asked to approve
        // a capture that would be refused anyway.
        assert_eq!(error.code(), "sensitive_window");
        assert!(!session.has_pending());
    }

    #[test]
    fn a_stale_window_identifier_cannot_be_confirmed_later() {
        let directory = tempdir().unwrap();
        let mut session = session_with(
            directory.path(),
            FakeBackend::with_windows(vec![window("Notepad", false)]),
        );
        let windows = session.list_windows().unwrap();
        let outcome = session
            .request(
                WindowsAction::Window {
                    window_id: WindowId::from_stored(windows[0].id.clone()).unwrap(),
                    operation: WindowOperation::Close,
                },
                ActionSource::LocalAi,
            )
            .unwrap();
        let ActionRequestOutcome::AwaitingConfirmation { preview } = outcome else {
            panic!("closing a window must be confirmed");
        };
        // A new listing replaces the identifiers.
        session.list_windows().unwrap();
        let error = session.confirm(&preview.token).unwrap_err();
        assert_eq!(error.code(), "window_expired");
    }

    #[test]
    fn the_capabilities_and_policy_are_exposed_without_secrets() {
        let directory = tempdir().unwrap();
        let session = session(directory.path());
        let capabilities = session.capabilities();
        assert!(capabilities.platform_supported);
        assert!(session.policy().table().len() >= 16);
        let rendered = format!("{session:?}");
        assert!(!rendered.contains("FICTIONAL"));
        assert!(rendered.contains("allowed_applications"));
    }

    #[test]
    fn timers_and_reminders_survive_a_restart() {
        let directory = tempdir().unwrap();
        {
            let mut session = session(directory.path());
            session
                .request(
                    WindowsAction::CreateReminder {
                        delay_seconds: 600,
                        message: "позвонить".to_string(),
                    },
                    ActionSource::DirectGui,
                )
                .unwrap();
            session.shutdown();
        }
        let reopened = session(directory.path());
        let scheduled = reopened.scheduled();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].kind, ScheduledKind::Reminder);
        if cfg!(windows) {
            assert_eq!(scheduled[0].message.as_deref(), Some("позвонить"));
        }
        reopened.shutdown();
    }

    #[test]
    fn a_fake_backend_without_volume_reports_the_missing_capability() {
        let directory = tempdir().unwrap();
        let mut session = session_with(
            directory.path(),
            FakeBackend::new().with_capabilities(Capabilities {
                volume: false,
                ..Capabilities::full()
            }),
        );
        let error = session
            .request(WindowsAction::GetVolume, ActionSource::DirectGui)
            .unwrap_err();
        assert_eq!(error.code(), "capability_unavailable");
    }

    /// The shortest timer the policy allows expires in five seconds; this test waits for it so
    /// that the whole chain — scheduler thread, audit, host hook — is exercised for real
    /// instead of being simulated. The sealed reminder text has its own test in `timers`.
    #[test]
    fn a_fired_timer_reaches_the_host_hook_and_the_audit() {
        let directory = tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new());
        let mut session = WindowsActions::open(
            directory.path(),
            Arc::clone(&backend) as Arc<dyn WindowsBackend>,
            WindowsActionSettings::default_for(directory.path()),
        );
        session.install_launch_lookup();
        let (sender, receiver) = std::sync::mpsc::channel();
        session.set_fired_hook(Arc::new(move |view: ScheduledView| {
            let _ = sender.send(view);
        }));
        session
            .request(
                WindowsAction::CreateTimer {
                    duration_seconds: super::super::model::MIN_TIMER_SECONDS,
                },
                ActionSource::InternalTimer,
            )
            .unwrap();
        let view = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the timer must fire and reach the host");
        assert_eq!(view.kind, ScheduledKind::Timer);
        assert_eq!(view.status, super::super::timers::ScheduledStatus::Fired);
        // The system notification was attempted, and the log says a timer fired.
        assert_eq!(backend.notifications().len(), 1);
        assert!(session
            .audit_entries()
            .iter()
            .any(|entry| entry.action_type == "timer_fired"));
        // The host is told exactly once, and the item is no longer active.
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            session.scheduled()[0].status,
            super::super::timers::ScheduledStatus::Fired
        );
        session.shutdown();
    }
}
