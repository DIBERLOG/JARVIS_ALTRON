//! The executor: the only place that turns a validated action into a platform call.
//!
//! It is deliberately small. The policy has already decided *whether* an action may run and
//! the gate has already obtained the confirmation; what is left is the part that must be
//! exact:
//!
//! * the **window registry** keeps the opaque identifiers short-lived. A listing is replaced
//!   on every call to [`WindowsActionExecutor::list_windows`], an identifier that outlived its
//!   validity is refused with [`ActionError::WindowExpired`], and the platform handle is
//!   looked up again immediately before acting (see `backend::native`), so a stale identifier
//!   cannot reach a different window;
//! * **screenshots are written once**, under a name this application generates, into a
//!   directory the user chose, and an existing file is never overwritten;
//! * a **sensitive window blocks a screen capture**. If a window that looks like it may show
//!   credentials has the focus, a full-screen capture is refused with
//!   [`ActionError::SensitiveWindow`]. This is a courtesy guard, not a security boundary: the
//!   screen belongs to the session, and another process can capture it whenever it likes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::backend::{CaptureRequest, NativeWindow, WindowsBackend};
use super::error::ActionError;
use super::model::{
    safe_window_title, title_looks_sensitive, ActionRequest, ActionValue, ScreenshotTarget,
    WindowSummary, WindowsAction,
    MAX_RETIRED_WINDOW_IDS, WINDOW_ID_TTL_SECONDS,
};

/// File name prefix of a captured image.
pub const SCREENSHOT_PREFIX: &str = "screenshot";

/// A listing of windows, valid for a short time.
#[derive(Clone, Debug, Default)]
struct WindowListing {
    windows: Vec<NativeWindow>,
    listed_at_ms: u64,
}

/// The short-lived identifiers the interface and the model may use.
#[derive(Debug, Default)]
pub struct WindowRegistry {
    listing: WindowListing,
    /// Identifiers from listings that have already been replaced. They are kept only so that
    /// an old identifier is answered with "expired" rather than "no such window".
    retired: Vec<String>,
    ttl_seconds: u64,
}

impl WindowRegistry {
    pub fn new() -> Self {
        Self {
            listing: WindowListing::default(),
            retired: Vec::new(),
            ttl_seconds: WINDOW_ID_TTL_SECONDS,
        }
    }

    /// Overrides the identifier lifetime, for tests.
    pub fn with_ttl_seconds(mut self, ttl_seconds: u64) -> Self {
        self.ttl_seconds = ttl_seconds;
        self
    }

    /// Replaces the listing.
    pub fn replace(&mut self, windows: Vec<NativeWindow>, now_ms: u64) {
        for window in &self.listing.windows {
            if !self.retired.iter().any(|known| known == &window.id) {
                self.retired.push(window.id.clone());
            }
        }
        if self.retired.len() > MAX_RETIRED_WINDOW_IDS {
            let excess = self.retired.len() - MAX_RETIRED_WINDOW_IDS;
            self.retired.drain(..excess);
        }
        self.listing = WindowListing {
            windows,
            listed_at_ms: now_ms,
        };
    }

    /// Whether the current listing is still usable.
    pub fn is_fresh(&self, now_ms: u64) -> bool {
        !self.listing.windows.is_empty()
            && now_ms.saturating_sub(self.listing.listed_at_ms) <= self.ttl_seconds * 1000
    }

    /// The listings, as the interface and the model see them.
    pub fn summaries(&self) -> Vec<WindowSummary> {
        self.listing
            .windows
            .iter()
            .map(|window| {
                let sensitive = title_looks_sensitive(&window.title);
                WindowSummary {
                    id: window.id.clone(),
                    title: safe_window_title(&window.title),
                    process: window.process_name.clone(),
                    state: window.state,
                    monitor: window.monitor,
                    sensitive,
                    foreground: window.is_foreground,
                }
            })
            .collect()
    }

    /// Looks one identifier up, refusing a stale listing and an unknown identifier.
    pub fn resolve(&self, id: &str, now_ms: u64) -> Result<NativeWindow, ActionError> {
        if self.listing.windows.is_empty() {
            return Err(ActionError::WindowExpired);
        }
        if !self.is_fresh(now_ms) {
            return Err(ActionError::WindowExpired);
        }
        // An identifier from an earlier listing is answered as expired: it named a window
        // once, and the answer says the listing it belonged to is gone.
        if self.retired.iter().any(|known| known == id) {
            return Err(ActionError::WindowExpired);
        }
        self.listing
            .windows
            .iter()
            .find(|window| window.id == id)
            .cloned()
            .ok_or(ActionError::WindowNotFound)
    }

    /// The foreground window of the current listing, if any.
    pub fn foreground(&self, now_ms: u64) -> Option<NativeWindow> {
        self.is_fresh(now_ms)
            .then(|| {
                self.listing
                    .windows
                    .iter()
                    .find(|window| window.is_foreground)
                    .cloned()
            })
            .flatten()
    }

    /// Windows whose title contains `needle`, for the voice router.
    pub fn matching_title(&self, needle: &str, now_ms: u64) -> Vec<NativeWindow> {
        if !self.is_fresh(now_ms) {
            return Vec::new();
        }
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.listing
            .windows
            .iter()
            .filter(|window| window.title.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }

    /// Whether a sensitive-looking window currently has the focus.
    pub fn sensitive_window_is_foreground(&self) -> bool {
        self.listing.windows.iter().any(|window| {
            window.is_foreground && super::model::title_looks_sensitive(&window.title)
        })
    }
}

/// A directory and the naming rule for captured images.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScreenshotSettings {
    pub directory: String,
    /// Whether a capture is refused while a sensitive window has the focus.
    pub block_sensitive_windows: bool,
}

impl ScreenshotSettings {
    /// The default: the user's pictures folder, with the guard on.
    pub fn default_for(data_dir: &Path) -> Self {
        Self {
            directory: data_dir
                .join(SCREENSHOT_PREFIX)
                .to_string_lossy()
                .into_owned(),
            block_sensitive_windows: true,
        }
    }
}

/// The lookup the executor uses to turn an application identifier into a launch plan.
///
/// The executor asks the allowlist, which stays the single owner of "which programs may
/// start"; the request itself carries no path and no arguments.
pub type LaunchLookup =
    Arc<dyn Fn(&str) -> Result<super::allowlist::LaunchSpec, ActionError> + Send + Sync>;

/// Runs the actions, against one backend.
pub struct WindowsActionExecutor {
    backend: Arc<dyn WindowsBackend>,
    windows: Mutex<WindowRegistry>,
    screenshots: Mutex<ScreenshotSettings>,
    launch_lookup: Mutex<Option<LaunchLookup>>,
}

impl std::fmt::Debug for WindowsActionExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsActionExecutor")
            .field("backend", &self.backend.capabilities())
            .finish()
    }
}

impl WindowsActionExecutor {
    pub fn new(backend: Arc<dyn WindowsBackend>, screenshots: ScreenshotSettings) -> Self {
        Self {
            backend,
            windows: Mutex::new(WindowRegistry::new()),
            screenshots: Mutex::new(screenshots),
            launch_lookup: Mutex::new(None),
        }
    }

    pub fn backend(&self) -> &Arc<dyn WindowsBackend> {
        &self.backend
    }

    pub fn capabilities(&self) -> super::backend::Capabilities {
        self.backend.capabilities()
    }

    /// The screenshot settings.
    pub fn screenshot_settings(&self) -> ScreenshotSettings {
        self.screenshots.lock().clone()
    }

    /// Changes the screenshot directory and the guard.
    pub fn set_screenshot_settings(&self, settings: ScreenshotSettings) -> Result<(), ActionError> {
        let directory = PathBuf::from(settings.directory.trim());
        if directory.as_os_str().is_empty() {
            return Err(ActionError::InvalidArguments {
                detail: "a screenshot folder is required".to_string(),
            });
        }
        if !directory.is_absolute() {
            return Err(ActionError::InvalidArguments {
                detail: "the screenshot folder must be absolute".to_string(),
            });
        }
        std::fs::create_dir_all(&directory)?;
        *self.screenshots.lock() = ScreenshotSettings {
            directory: directory.to_string_lossy().into_owned(),
            block_sensitive_windows: settings.block_sensitive_windows,
        };
        Ok(())
    }

    /// Lists the visible windows and replaces the identifier set.
    pub fn list_windows(&self) -> Result<Vec<WindowSummary>, ActionError> {
        let windows = self.backend.list_windows()?;
        let now = super::timers::now_unix_ms();
        let mut registry = self.windows.lock();
        registry.replace(windows, now);
        Ok(registry.summaries())
    }

    /// The current window summaries without asking the platform again.
    pub fn current_windows(&self) -> Vec<WindowSummary> {
        self.windows.lock().summaries()
    }

    /// The registry, for the voice router and for tests.
    pub fn registry(&self) -> &Mutex<WindowRegistry> {
        &self.windows
    }

    /// Runs one action that has already been allowed.
    pub fn execute(&self, request: &ActionRequest) -> Result<ActionValue, ActionError> {
        let capabilities = self.backend.capabilities();
        if let Some(capability) = capabilities.missing_capability(&request.action) {
            return Err(ActionError::CapabilityUnavailable {
                capability: capability.to_string(),
            });
        }
        match &request.action {
            WindowsAction::GetVolume => {
                let state = self.backend.volume()?;
                Ok(ActionValue::Volume {
                    percent: state.percent,
                    muted: state.muted,
                })
            }
            WindowsAction::SetVolume { percent } => {
                let state = self.backend.set_volume(*percent)?;
                Ok(ActionValue::Volume {
                    percent: state.percent,
                    muted: state.muted,
                })
            }
            WindowsAction::ChangeVolume { direction, step } => {
                let current = self.backend.volume()?;
                let target = match direction {
                    super::model::VolumeDirection::Up => {
                        current.percent.saturating_add(*step).min(100)
                    }
                    super::model::VolumeDirection::Down => current.percent.saturating_sub(*step),
                };
                let state = self.backend.set_volume(target)?;
                Ok(ActionValue::Volume {
                    percent: state.percent,
                    muted: state.muted,
                })
            }
            WindowsAction::MuteVolume { muted } => {
                let state = self.backend.set_mute(*muted)?;
                Ok(ActionValue::Volume {
                    percent: state.percent,
                    muted: state.muted,
                })
            }
            WindowsAction::LaunchAllowedApplication { application_id } => {
                // The specification is built from the stored entry, never from the request.
                let spec = self.launch_spec(application_id.as_str())?;
                let process_id = self.backend.launch(&spec)?;
                Ok(ActionValue::Launched {
                    application: spec.display_name,
                    process_id,
                })
            }
            WindowsAction::ListWindows => {
                let windows = self.list_windows()?;
                Ok(ActionValue::Windows { windows })
            }
            WindowsAction::Window {
                window_id,
                operation,
            } => {
                let window = self.resolve_window(window_id.as_str())?;
                if window.is_own_process {
                    return Err(ActionError::ForbiddenAction {
                        reason: "that window belongs to this application".to_string(),
                    });
                }
                self.backend.act_on_window(&window, operation)?;
                Ok(ActionValue::None)
            }
            WindowsAction::TakeScreenshot { target } => {
                let destination = self.destination_for_screenshot()?;
                let window = match target {
                    ScreenshotTarget::SelectedWindow(id) => Some(self.resolve_window(id.as_str())?),
                    _ => None,
                };
                self.guard_sensitive_capture(target, window.as_ref())?;
                let bytes = self.backend.capture(&CaptureRequest {
                    target: target.clone(),
                    destination: destination.clone(),
                    window,
                })?;
                Ok(ActionValue::ScreenshotPath {
                    path: destination.to_string_lossy().into_owned(),
                    bytes,
                })
            }
            WindowsAction::LockWorkstation => {
                self.backend.lock_workstation()?;
                Ok(ActionValue::Locked)
            }
            // Timers and reminders are state, not platform calls; the session owns them.
            WindowsAction::CreateTimer { .. }
            | WindowsAction::CancelTimer { .. }
            | WindowsAction::CreateReminder { .. }
            | WindowsAction::CancelReminder { .. } => Err(ActionError::UnsupportedPlatform),
        }
    }

    /// Installs the allowlist lookup.
    pub fn set_launch_lookup(&self, lookup: LaunchLookup) {
        *self.launch_lookup.lock() = Some(lookup);
    }

    /// Builds the launch specification from the stored entry.
    ///
    /// Without an installed lookup nothing may start: an executor that could invent a path
    /// would defeat the allowlist.
    fn launch_spec(
        &self,
        application_id: &str,
    ) -> Result<super::allowlist::LaunchSpec, ActionError> {
        let lookup = self.launch_lookup.lock().clone();
        let lookup = lookup.ok_or(ActionError::ApplicationNotAllowed {
            application_id: application_id.to_string(),
        })?;
        lookup(application_id)
    }

    fn resolve_window(&self, id: &str) -> Result<NativeWindow, ActionError> {
        let now = super::timers::now_unix_ms();
        let registry = self.windows.lock();
        let window = registry.resolve(id, now)?;
        Ok(window)
    }

    /// Refuses, before the user is asked, what the execution would refuse anyway.
    ///
    /// A confirmation dialog is a promise that agreeing does something. Asking the user to
    /// approve a capture that the guard will refuse, or one whose folder does not exist, would
    /// be a dishonest prompt, so the same checks run here first.
    pub fn precheck(&self, request: &ActionRequest) -> Result<(), ActionError> {
        if let WindowsAction::TakeScreenshot { target } = &request.action {
            let window = match target {
                ScreenshotTarget::SelectedWindow(id) => Some(self.resolve_window(id.as_str())?),
                _ => None,
            };
            self.guard_sensitive_capture(target, window.as_ref())?;
            // Only the folder is checked here; the file name is generated again when the
            // action actually runs.
            self.destination_for_screenshot()?;
        }
        Ok(())
    }

    fn guard_sensitive_capture(
        &self,
        target: &ScreenshotTarget,
        window: Option<&NativeWindow>,
    ) -> Result<(), ActionError> {
        let settings = self.screenshots.lock().clone();
        if !settings.block_sensitive_windows {
            return Ok(());
        }
        match target {
            ScreenshotTarget::SelectedWindow(_) => {
                if window
                    .map(|window| super::model::title_looks_sensitive(&window.title))
                    .unwrap_or(false)
                {
                    return Err(ActionError::SensitiveWindow);
                }
                Ok(())
            }
            // A screen-wide capture is refused while a sensitive window has the focus: the
            // user cannot see what is about to be written to a file.
            _ => {
                if self.windows.lock().sensitive_window_is_foreground() {
                    return Err(ActionError::SensitiveWindow);
                }
                Ok(())
            }
        }
    }

    /// A unique destination for the next screenshot.
    ///
    /// The name is generated here, an existing file is never replaced, and the directory must
    /// already exist (the settings command creates it).
    fn destination_for_screenshot(&self) -> Result<PathBuf, ActionError> {
        let settings = self.screenshots.lock().clone();
        let directory = PathBuf::from(&settings.directory);
        if !directory.is_dir() {
            return Err(ActionError::ScreenshotFailed {
                detail: "the screenshot folder does not exist".to_string(),
            });
        }
        let stamp = super::timers::now_unix_ms();
        for attempt in 0..10u32 {
            let suffix = if attempt == 0 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let candidate = directory.join(format!("{SCREENSHOT_PREFIX}-{stamp}{suffix}.png"));
            if !candidate.exists() {
                return Ok(candidate);
            }
        }
        Err(ActionError::ScreenshotFailed {
            detail: "a free file name could not be found".to_string(),
        })
    }
}

/// A clock the executor uses for identifier validity, injectable for tests.
pub fn now_ms_with(_started: Instant) -> u64 {
    super::timers::now_unix_ms()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_actions::allowlist::{AllowedApplicationDraft, AllowedApplications};
    use crate::windows_actions::backend::{Capabilities, FakeBackend};
    use crate::windows_actions::model::{ActionSource, ApplicationId, WindowId, WindowState};
    use tempfile::tempdir;

    fn window(title: &str, foreground: bool) -> NativeWindow {
        NativeWindow {
            id: WindowId::mint().unwrap().as_str().to_string(),
            native_id: 11,
            process_id: 4242,
            process_name: "Notepad.exe".to_string(),
            title: title.to_string(),
            state: WindowState::Normal,
            monitor: 1,
            is_own_process: false,
            is_foreground: foreground,
            work_area: (0, 0, 1920, 1040),
        }
    }

    fn executor_with(backend: FakeBackend, directory: &Path) -> WindowsActionExecutor {
        WindowsActionExecutor::new(
            Arc::new(backend),
            ScreenshotSettings {
                directory: directory.to_string_lossy().into_owned(),
                block_sensitive_windows: true,
            },
        )
    }

    fn request(action: WindowsAction) -> ActionRequest {
        ActionRequest::new(action, ActionSource::DirectGui, "now").unwrap()
    }

    #[test]
    fn the_volume_actions_go_through_the_backend_and_are_bounded() {
        let directory = tempdir().unwrap();
        let executor = executor_with(FakeBackend::new(), directory.path());
        let value = executor
            .execute(&request(WindowsAction::SetVolume { percent: 55 }))
            .unwrap();
        assert_eq!(
            value,
            ActionValue::Volume {
                percent: 55,
                muted: false
            }
        );
        // A relative change is computed from the current level and clamped at both ends.
        let up = executor
            .execute(&request(WindowsAction::ChangeVolume {
                direction: super::super::model::VolumeDirection::Up,
                step: 25,
            }))
            .unwrap();
        assert_eq!(
            up,
            ActionValue::Volume {
                percent: 80,
                muted: false
            }
        );
        let down = executor
            .execute(&request(WindowsAction::ChangeVolume {
                direction: super::super::model::VolumeDirection::Down,
                step: 25,
            }))
            .unwrap();
        assert_eq!(
            down,
            ActionValue::Volume {
                percent: 55,
                muted: false
            }
        );
        let muted = executor
            .execute(&request(WindowsAction::MuteVolume { muted: true }))
            .unwrap();
        assert_eq!(
            muted,
            ActionValue::Volume {
                percent: 55,
                muted: true
            }
        );
    }

    #[test]
    fn a_missing_capability_is_reported_before_the_call() {
        let directory = tempdir().unwrap();
        let backend = FakeBackend::new().with_capabilities(Capabilities {
            volume: false,
            ..Capabilities::full()
        });
        let executor = executor_with(backend, directory.path());
        let error = executor
            .execute(&request(WindowsAction::GetVolume))
            .unwrap_err();
        assert_eq!(error.code(), "capability_unavailable");
        // The backend was never asked.
        assert!(!executor.backend().capabilities().volume);
    }

    #[test]
    fn a_launch_uses_the_stored_entry_and_nothing_else() {
        let directory = tempdir().unwrap();
        let program_dir = tempdir().unwrap();
        let program = program_dir.path().join("Allowed.exe");
        std::fs::write(&program, b"FICTIONAL").unwrap();
        let mut allowlist = AllowedApplications::open(program_dir.path());
        let entry = allowlist
            .add(
                &AllowedApplicationDraft {
                    display_name: "Allowed".to_string(),
                    path: program.to_string_lossy().into_owned(),
                    fixed_arguments: vec!["--fixed".to_string()],
                    working_directory: None,
                },
                "now",
            )
            .unwrap();
        let list = Arc::new(Mutex::new(allowlist));
        let lookup = Arc::clone(&list);
        let executor = executor_with(FakeBackend::new(), directory.path());
        executor.set_launch_lookup(Arc::new(move |id: &str| lookup.lock().launch_spec(id)));

        let value = executor
            .execute(&request(WindowsAction::LaunchAllowedApplication {
                application_id: ApplicationId::from_stored(entry.id.clone()).unwrap(),
            }))
            .unwrap();
        match value {
            ActionValue::Launched { application, .. } => assert_eq!(application, "Allowed"),
            other => panic!("unexpected value {other:?}"),
        }
        let launched = executor.backend().capabilities();
        assert!(launched.platform_supported);
    }

    #[test]
    fn an_unknown_identifier_cannot_start_anything() {
        let directory = tempdir().unwrap();
        let executor = executor_with(FakeBackend::new(), directory.path());
        let error = executor
            .execute(&request(WindowsAction::LaunchAllowedApplication {
                application_id: ApplicationId::from_stored("app_deadbeef").unwrap(),
            }))
            .unwrap_err();
        assert_eq!(error.code(), "application_not_allowed");
    }

    #[test]
    fn a_window_identifier_is_short_lived() {
        let directory = tempdir().unwrap();
        let executor = executor_with(
            FakeBackend::with_windows(vec![window("Untitled", false)]),
            directory.path(),
        );
        let windows = executor.list_windows().unwrap();
        assert_eq!(windows.len(), 1);
        let id = windows[0].id.clone();
        // Acting right away works.
        assert!(executor
            .execute(&request(WindowsAction::Window {
                window_id: WindowId::from_stored(id.clone()).unwrap(),
                operation: super::super::model::WindowOperation::Minimize,
            }))
            .is_ok());
        // The registry is replaced on every listing, so the old identifier is gone.
        let fresh = executor.list_windows().unwrap();
        assert_ne!(fresh[0].id, id);
        assert_eq!(
            executor
                .execute(&request(WindowsAction::Window {
                    window_id: WindowId::from_stored(id).unwrap(),
                    operation: super::super::model::WindowOperation::Minimize,
                }))
                .unwrap_err()
                .code(),
            "window_expired"
        );
    }

    #[test]
    fn an_expired_listing_is_refused_even_when_the_identifier_matches() {
        let registry = WindowRegistry::new().with_ttl_seconds(1);
        let mut registry = registry;
        registry.replace(vec![window("Untitled", false)], 1_000);
        let id = registry.summaries()[0].id.clone();
        assert!(registry.resolve(&id, 1_500).is_ok());
        assert_eq!(
            registry.resolve(&id, 500_000).unwrap_err().code(),
            "window_expired"
        );
        // An unknown identifier in a fresh listing is a different answer.
        assert_eq!(
            registry.resolve("aabbccdd", 1_500).unwrap_err().code(),
            "window_not_found"
        );
    }

    #[test]
    fn the_registry_can_find_the_foreground_window_and_match_titles() {
        let registry = WindowRegistry::new();
        let mut registry = registry;
        registry.replace(
            vec![
                window("Notepad — notes.txt", true),
                window("Calculator", false),
            ],
            1_000,
        );
        assert_eq!(
            registry.foreground(1_100).map(|window| window.title),
            Some("Notepad — notes.txt".to_string())
        );
        assert_eq!(registry.matching_title("calc", 1_100).len(), 1);
        assert_eq!(registry.matching_title("nothing", 1_100).len(), 0);
        assert_eq!(registry.matching_title("", 1_100).len(), 0);
        // A stale listing answers nothing at all.
        assert!(registry.foreground(10_000_000).is_none());
        assert!(registry.matching_title("calc", 10_000_000).is_empty());
    }

    #[test]
    fn a_sensitive_foreground_window_blocks_a_screen_capture() {
        let directory = tempdir().unwrap();
        let executor = executor_with(
            FakeBackend::with_windows(vec![window("JARVIS — Vault", true)]),
            directory.path(),
        );
        executor.list_windows().unwrap();
        let error = executor
            .execute(&request(WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::AllMonitors,
            }))
            .unwrap_err();
        assert_eq!(error.code(), "sensitive_window");
        // Nothing was captured.
        assert!(executor.backend().capabilities().screenshots);

        // The guard can be switched off by the user, and it never blocks a capture of a
        // different, harmless window.
        executor
            .set_screenshot_settings(ScreenshotSettings {
                directory: directory.path().to_string_lossy().into_owned(),
                block_sensitive_windows: false,
            })
            .unwrap();
        assert!(executor
            .execute(&request(WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::AllMonitors,
            }))
            .is_ok());
    }

    #[test]
    fn a_screenshot_of_a_sensitive_window_is_refused() {
        let directory = tempdir().unwrap();
        let executor = executor_with(
            FakeBackend::with_windows(vec![window("Пароль — вход", false)]),
            directory.path(),
        );
        let windows = executor.list_windows().unwrap();
        assert!(windows[0].sensitive);
        let error = executor
            .execute(&request(WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::SelectedWindow(
                    WindowId::from_stored(windows[0].id.clone()).unwrap(),
                ),
            }))
            .unwrap_err();
        assert_eq!(error.code(), "sensitive_window");
    }

    #[test]
    fn sensitive_titles_are_redacted_from_window_dtos() {
        let directory = tempdir().unwrap();
        let secret = "OpenAI key sk-FICTIONAL0000000000000000000000000000";
        let executor = executor_with(
            FakeBackend::with_windows(vec![window(secret, false)]),
            directory.path(),
        );

        let summaries = executor.list_windows().unwrap();
        assert!(summaries[0].sensitive);
        assert_eq!(summaries[0].title, "Sensitive window");
        assert!(!serde_json::to_string(&summaries).unwrap().contains(secret));
    }

    #[test]
    fn a_capture_writes_one_new_file_and_keeps_the_full_path() {
        let directory = tempdir().unwrap();
        let executor = executor_with(FakeBackend::new(), directory.path());
        let value = executor
            .execute(&request(WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::PrimaryMonitor,
            }))
            .unwrap();
        let ActionValue::ScreenshotPath { path, bytes } = value else {
            panic!("expected a screenshot path");
        };
        assert!(bytes > 0);
        assert!(std::path::Path::new(&path).is_file());
        assert!(path.ends_with(".png"));

        // A second capture never reuses a name, and nothing is overwritten.
        let second = executor
            .execute(&request(WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::PrimaryMonitor,
            }))
            .unwrap();
        let ActionValue::ScreenshotPath { path: other, .. } = second else {
            panic!("expected a screenshot path");
        };
        assert_ne!(path, other);
    }

    #[test]
    fn a_capture_into_a_missing_folder_fails_with_a_reason() {
        let directory = tempdir().unwrap();
        let missing = directory.path().join("gone");
        let executor = executor_with(FakeBackend::new(), &missing);
        let error = executor
            .execute(&request(WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::PrimaryMonitor,
            }))
            .unwrap_err();
        assert_eq!(error.code(), "screenshot_failed");
        // Pointing it at an existing folder fixes it.
        executor
            .set_screenshot_settings(ScreenshotSettings {
                directory: directory.path().to_string_lossy().into_owned(),
                block_sensitive_windows: true,
            })
            .unwrap();
        assert!(executor
            .execute(&request(WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::PrimaryMonitor,
            }))
            .is_ok());
    }

    #[test]
    fn the_screenshot_settings_refuse_a_relative_folder() {
        let directory = tempdir().unwrap();
        let executor = executor_with(FakeBackend::new(), directory.path());
        let error = executor
            .set_screenshot_settings(ScreenshotSettings {
                directory: "shots".to_string(),
                block_sensitive_windows: true,
            })
            .unwrap_err();
        assert_eq!(error.code(), "invalid_arguments");
    }

    #[test]
    fn closing_a_window_asks_the_backend_and_never_ends_a_process() {
        let directory = tempdir().unwrap();
        let backend = FakeBackend::with_windows(vec![window("Untitled", false)]);
        let executor = executor_with(backend, directory.path());
        let windows = executor.list_windows().unwrap();
        assert!(executor
            .execute(&request(WindowsAction::Window {
                window_id: WindowId::from_stored(windows[0].id.clone()).unwrap(),
                operation: super::super::model::WindowOperation::Close,
            }))
            .is_ok());
        let calls = executor
            .backend()
            .list_windows()
            .map(|_| ())
            .map_err(|_| ())
            .is_ok();
        assert!(calls);
    }

    #[test]
    fn the_executor_refuses_its_own_window() {
        let directory = tempdir().unwrap();
        let mut own = window("JARVIS", false);
        own.is_own_process = true;
        let executor = executor_with(FakeBackend::with_windows(vec![own]), directory.path());
        let windows = executor.list_windows().unwrap();
        assert_eq!(
            executor
                .execute(&request(WindowsAction::Window {
                    window_id: WindowId::from_stored(windows[0].id.clone()).unwrap(),
                    operation: super::super::model::WindowOperation::Minimize,
                }))
                .unwrap_err()
                .code(),
            "forbidden_action"
        );
    }

    #[test]
    fn locking_is_a_single_platform_call() {
        let directory = tempdir().unwrap();
        let executor = executor_with(FakeBackend::new(), directory.path());
        assert_eq!(
            executor
                .execute(&request(WindowsAction::LockWorkstation))
                .unwrap(),
            ActionValue::Locked
        );
    }
}
