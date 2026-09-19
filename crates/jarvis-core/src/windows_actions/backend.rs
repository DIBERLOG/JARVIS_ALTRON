//! The platform boundary: everything that actually touches Windows sits behind one trait.
//!
//! The trait exists so the rules can be tested without a desktop session. Every test in this
//! crate runs against [`FakeBackend`], which never locks a session, never starts a program,
//! and never captures a screen; the real implementation is a second, thin adapter over Win32
//! and Core Audio. Nothing above this trait knows which one it has.
//!
//! What the trait deliberately cannot do, because no variant of [`super::model::WindowsAction`]
//! can ask for it: run a command line, end a process, change the registry, shut the machine
//! down, delete a file, read the vault, or type anything.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::allowlist::LaunchSpec;
use super::error::ActionError;
use super::model::{
    ScreenshotTarget, WindowOperation, WindowState, WindowsAction, MAX_MOVE_PIXELS,
    MIN_TIMER_SECONDS,
};

#[cfg(windows)]
pub mod native;

/// The adapter for the platform this build runs on.
///
/// This is the only place that names a concrete backend; everything above it receives the
/// trait, which is why the whole feature can be tested on a fake.
pub fn platform_backend() -> Arc<dyn WindowsBackend> {
    #[cfg(windows)]
    {
        Arc::new(native::NativeWindowsBackend::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedBackend)
    }
}

/// What this machine can do right now.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Capabilities {
    /// Whether the platform adapter exists and loaded.
    pub platform_supported: bool,
    pub volume: bool,
    pub screenshots: bool,
    pub windows: bool,
    pub lock_workstation: bool,
    pub notifications: bool,
    /// Whether the local model can be given structured tools at all.
    pub ai_tools: bool,
    /// Content-free explanations of anything that is unavailable.
    pub notes: Vec<String>,
}

impl Capabilities {
    /// The capabilities of a platform with no adapter at all.
    pub fn unsupported() -> Self {
        Self {
            platform_supported: false,
            volume: false,
            screenshots: false,
            windows: false,
            lock_workstation: false,
            notifications: false,
            ai_tools: false,
            notes: vec!["windows-actions-note-unsupported-platform".to_string()],
        }
    }

    /// Everything available, as the fake backend reports.
    pub fn full() -> Self {
        Self {
            platform_supported: true,
            volume: true,
            screenshots: true,
            windows: true,
            lock_workstation: true,
            notifications: true,
            ai_tools: true,
            notes: Vec::new(),
        }
    }

    /// Whether the action can be attempted at all.
    pub fn supports(&self, action: &WindowsAction) -> bool {
        match action {
            WindowsAction::GetVolume
            | WindowsAction::SetVolume { .. }
            | WindowsAction::ChangeVolume { .. }
            | WindowsAction::MuteVolume { .. } => self.volume,
            WindowsAction::TakeScreenshot { .. } => self.screenshots,
            WindowsAction::ListWindows | WindowsAction::Window { .. } => self.windows,
            WindowsAction::LockWorkstation => self.lock_workstation,
            // Launching, timers, and reminders need no platform capability beyond the
            // process API the application already uses.
            _ => self.platform_supported,
        }
    }

    /// The capability name for an error message, or `None` when it is available.
    pub fn missing_capability(&self, action: &WindowsAction) -> Option<&'static str> {
        if self.supports(action) {
            return None;
        }
        Some(match action {
            WindowsAction::GetVolume
            | WindowsAction::SetVolume { .. }
            | WindowsAction::ChangeVolume { .. }
            | WindowsAction::MuteVolume { .. } => "windows-actions-capability-volume",
            WindowsAction::TakeScreenshot { .. } => "windows-actions-capability-screenshots",
            WindowsAction::ListWindows | WindowsAction::Window { .. } => {
                "windows-actions-capability-windows"
            }
            WindowsAction::LockWorkstation => "windows-actions-capability-lock",
            _ => "windows-actions-capability-platform",
        })
    }
}

/// The current output volume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeState {
    pub percent: u8,
    pub muted: bool,
}

/// One visible window as the platform reports it.
///
/// `id` is opaque and minted by the backend for this list; `native_id` never leaves the core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWindow {
    pub id: String,
    pub native_id: isize,
    pub process_id: u32,
    pub process_name: String,
    /// Already sanitized and truncated by the backend.
    pub title: String,
    pub state: WindowState,
    pub monitor: u32,
    /// Whether this window belongs to this application (never acted on).
    pub is_own_process: bool,
    /// Whether this window currently has the keyboard focus.
    pub is_foreground: bool,
    /// The work area of the monitor the window is on: x, y, width, height.
    pub work_area: (i32, i32, i32, i32),
}

impl NativeWindow {
    /// Clamps a requested move into the work area of the window's monitor.
    pub fn clamp_move(&self, x: i32, y: i32, width: u32, height: u32) -> (i32, i32, u32, u32) {
        let (area_x, area_y, area_width, area_height) = self.work_area;
        let width = width.min(area_width.unsigned_abs()).max(200);
        let height = height.min(area_height.unsigned_abs()).max(150);
        let max_x = area_x
            .saturating_add(area_width)
            .saturating_sub(width as i32);
        let max_y = area_y
            .saturating_add(area_height)
            .saturating_sub(height as i32);
        (
            x.clamp(area_x, max_x.max(area_x)),
            y.clamp(area_y, max_y.max(area_y)),
            width,
            height,
        )
    }
}

/// A request to capture the screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureRequest {
    pub target: ScreenshotTarget,
    /// Absolute destination chosen by the executor; never overwritten.
    pub destination: PathBuf,
    /// The window to capture, for [`ScreenshotTarget::SelectedWindow`].
    pub window: Option<NativeWindow>,
}

/// What the platform layer offers.
pub trait WindowsBackend: Send + Sync {
    /// What this machine can do.
    fn capabilities(&self) -> Capabilities;

    /// Reads the current volume.
    fn volume(&self) -> Result<VolumeState, ActionError>;

    /// Sets the volume, in percent.
    fn set_volume(&self, percent: u8) -> Result<VolumeState, ActionError>;

    /// Mutes or unmutes.
    fn set_mute(&self, muted: bool) -> Result<VolumeState, ActionError>;

    /// Starts an allowed application and returns its process identifier.
    fn launch(&self, spec: &LaunchSpec) -> Result<u32, ActionError>;

    /// Lists the visible user windows.
    fn list_windows(&self) -> Result<Vec<NativeWindow>, ActionError>;

    /// Acts on one window from the list, re-validating it first.
    fn act_on_window(
        &self,
        window: &NativeWindow,
        operation: &WindowOperation,
    ) -> Result<(), ActionError>;

    /// Captures the screen into `request.destination`.
    fn capture(&self, request: &CaptureRequest) -> Result<u64, ActionError>;

    /// Locks the workstation through the documented API.
    fn lock_workstation(&self) -> Result<(), ActionError>;

    /// Shows a local notification. A failure is not fatal: the interface can show its own.
    fn notify(&self, title: &str, message: &str) -> Result<(), ActionError>;
}

/// A backend that does nothing, for a platform without an adapter.
#[derive(Debug, Default)]
pub struct UnsupportedBackend;

impl WindowsBackend for UnsupportedBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::unsupported()
    }

    fn volume(&self) -> Result<VolumeState, ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn set_volume(&self, _: u8) -> Result<VolumeState, ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn set_mute(&self, _: bool) -> Result<VolumeState, ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn launch(&self, _: &LaunchSpec) -> Result<u32, ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn list_windows(&self) -> Result<Vec<NativeWindow>, ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn act_on_window(&self, _: &NativeWindow, _: &WindowOperation) -> Result<(), ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn capture(&self, _: &CaptureRequest) -> Result<u64, ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn lock_workstation(&self) -> Result<(), ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }

    fn notify(&self, _: &str, _: &str) -> Result<(), ActionError> {
        Err(ActionError::UnsupportedPlatform)
    }
}

/// The backend the tests use: a recording fake with injectable failures.
///
/// It never touches the operating system, so a test can exercise the whole pipeline —
/// policy, confirmation, execution, audit — without locking a session, starting a program, or
/// taking a screenshot.
#[derive(Debug)]
pub struct FakeBackend {
    state: Mutex<FakeState>,
}

#[derive(Debug)]
struct FakeState {
    capabilities: Capabilities,
    volume: VolumeState,
    windows: Vec<NativeWindow>,
    /// Capability names that should fail when used.
    failures: Vec<&'static str>,
    /// Every call, as a short description.
    calls: Vec<String>,
    launched: Vec<LaunchSpec>,
    captured: Vec<PathBuf>,
    notifications: Vec<(String, String)>,
    locks: usize,
    /// When set, `capture` writes this many bytes to the destination.
    capture_bytes: u64,
    /// When set, `capture` refuses to write anything.
    capture_fails: bool,
    /// When set, `list_windows` returns windows that are already gone.
    stale_windows: bool,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeState {
                capabilities: Capabilities::full(),
                volume: VolumeState {
                    percent: 40,
                    muted: false,
                },
                windows: Vec::new(),
                failures: Vec::new(),
                calls: Vec::new(),
                launched: Vec::new(),
                captured: Vec::new(),
                notifications: Vec::new(),
                locks: 0,
                capture_bytes: 128,
                capture_fails: false,
                stale_windows: false,
            }),
        }
    }

    /// Builds a fake with one example window.
    pub fn with_windows(windows: Vec<NativeWindow>) -> Self {
        let backend = Self::new();
        backend.state.lock().unwrap().windows = windows;
        backend
    }

    /// Makes one capability fail, to exercise the failure path.
    pub fn failing(self, capability: &'static str) -> Self {
        self.state.lock().unwrap().failures.push(capability);
        self
    }

    /// Sets the volume the fake reports.
    pub fn with_volume(self, percent: u8, muted: bool) -> Self {
        self.state.lock().unwrap().volume = VolumeState { percent, muted };
        self
    }

    /// Replaces the reported capabilities.
    pub fn with_capabilities(self, capabilities: Capabilities) -> Self {
        self.state.lock().unwrap().capabilities = capabilities;
        self
    }

    /// Makes a capture fail, for the error path.
    pub fn failing_capture(self) -> Self {
        self.state.lock().unwrap().capture_fails = true;
        self
    }

    /// Every call made so far.
    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn launched(&self) -> Vec<LaunchSpec> {
        self.state.lock().unwrap().launched.clone()
    }

    pub fn captured(&self) -> Vec<PathBuf> {
        self.state.lock().unwrap().captured.clone()
    }

    pub fn notifications(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().notifications.clone()
    }

    pub fn locks(&self) -> usize {
        self.state.lock().unwrap().locks
    }

    fn fail_if(&self, capability: &'static str) -> Result<(), ActionError> {
        if self.state.lock().unwrap().failures.contains(&capability) {
            return Err(ActionError::CapabilityUnavailable {
                capability: capability.to_string(),
            });
        }
        Ok(())
    }

    fn record(&self, call: impl Into<String>) {
        self.state.lock().unwrap().calls.push(call.into());
    }
}

impl WindowsBackend for FakeBackend {
    fn capabilities(&self) -> Capabilities {
        self.state.lock().unwrap().capabilities.clone()
    }

    fn volume(&self) -> Result<VolumeState, ActionError> {
        self.record("volume");
        self.fail_if("volume")?;
        Ok(self.state.lock().unwrap().volume)
    }

    fn set_volume(&self, percent: u8) -> Result<VolumeState, ActionError> {
        self.record(format!("set_volume:{percent}"));
        self.fail_if("volume")?;
        let mut state = self.state.lock().unwrap();
        state.volume.percent = percent.min(100);
        Ok(state.volume)
    }

    fn set_mute(&self, muted: bool) -> Result<VolumeState, ActionError> {
        self.record(format!("set_mute:{muted}"));
        self.fail_if("volume")?;
        let mut state = self.state.lock().unwrap();
        state.volume.muted = muted;
        Ok(state.volume)
    }

    fn launch(&self, spec: &LaunchSpec) -> Result<u32, ActionError> {
        self.record(format!("launch:{}", spec.application_id));
        self.fail_if("launch")?;
        self.state.lock().unwrap().launched.push(spec.clone());
        Ok(4242)
    }

    fn list_windows(&self) -> Result<Vec<NativeWindow>, ActionError> {
        self.record("list_windows");
        self.fail_if("windows")?;
        let windows = self.state.lock().unwrap().windows.clone();
        let stale = self.state.lock().unwrap().stale_windows;
        // The identifiers are minted per listing, exactly as the native backend does, so a
        // test can observe that an identifier from an earlier listing is refused.
        windows
            .into_iter()
            .map(|mut window| {
                if stale {
                    // Simulates a list that went out of date between listing and acting.
                    window.native_id = 0;
                }
                window.id = super::model::WindowId::mint()?.as_str().to_string();
                Ok(window)
            })
            .collect()
    }

    fn act_on_window(
        &self,
        window: &NativeWindow,
        operation: &WindowOperation,
    ) -> Result<(), ActionError> {
        self.record(format!("window:{}:{}", window.id, operation.as_str()));
        self.fail_if("windows")?;
        if window.native_id == 0 {
            return Err(ActionError::WindowNotFound);
        }
        if window.is_own_process {
            return Err(ActionError::ForbiddenAction {
                reason: "that window belongs to this application".to_string(),
            });
        }
        Ok(())
    }

    fn capture(&self, request: &CaptureRequest) -> Result<u64, ActionError> {
        self.record(format!("capture:{}", request.target.kind()));
        self.fail_if("screenshots")?;
        let state = self.state.lock().unwrap();
        if state.capture_fails {
            return Err(ActionError::ScreenshotFailed {
                detail: "the fake backend refuses to capture".to_string(),
            });
        }
        let bytes = state.capture_bytes;
        drop(state);
        if bytes > 0 {
            std::fs::write(&request.destination, vec![0u8; bytes as usize]).map_err(|_| {
                ActionError::ScreenshotFailed {
                    detail: "the image could not be written".to_string(),
                }
            })?;
        }
        self.state
            .lock()
            .unwrap()
            .captured
            .push(request.destination.clone());
        Ok(bytes)
    }

    fn lock_workstation(&self) -> Result<(), ActionError> {
        self.record("lock_workstation");
        self.fail_if("lock")?;
        self.state.lock().unwrap().locks += 1;
        Ok(())
    }

    fn notify(&self, title: &str, message: &str) -> Result<(), ActionError> {
        self.record("notify");
        self.fail_if("notifications")?;
        self.state
            .lock()
            .unwrap()
            .notifications
            .push((title.to_string(), message.to_string()));
        Ok(())
    }
}

/// The timers the fake backend would use, re-exported so a test can build one.
pub fn minimum_timer_seconds() -> u64 {
    MIN_TIMER_SECONDS
}

/// A destination path inside a directory, used by the executor for screenshots.
pub fn destination_in(directory: &Path, file_name: &str) -> PathBuf {
    directory.join(file_name)
}

/// Largest move the policy allows, re-exported for the interface.
pub fn max_move_pixels() -> i32 {
    MAX_MOVE_PIXELS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_actions::model::WindowId;

    fn window(native_id: isize) -> NativeWindow {
        NativeWindow {
            id: WindowId::mint().unwrap().as_str().to_string(),
            native_id,
            process_id: 100,
            process_name: "Notepad.exe".to_string(),
            title: "Untitled".to_string(),
            state: WindowState::Normal,
            monitor: 1,
            is_own_process: false,
            is_foreground: false,
            work_area: (0, 0, 1920, 1040),
        }
    }

    #[test]
    fn the_unsupported_backend_refuses_everything_without_panicking() {
        let backend = UnsupportedBackend;
        let capabilities = backend.capabilities();
        assert!(!capabilities.platform_supported);
        assert!(!capabilities.supports(&WindowsAction::GetVolume));
        assert_eq!(
            capabilities.missing_capability(&WindowsAction::LockWorkstation),
            Some("windows-actions-capability-lock")
        );
        assert!(backend.volume().is_err());
        assert!(backend.list_windows().is_err());
        assert!(backend.lock_workstation().is_err());
    }

    #[test]
    fn the_fake_backend_records_every_call_and_can_fail_on_demand() {
        let backend = FakeBackend::new();
        assert_eq!(backend.volume().unwrap().percent, 40);
        assert_eq!(backend.set_volume(70).unwrap().percent, 70);
        assert!(backend.set_mute(true).unwrap().muted);
        assert_eq!(
            backend.calls(),
            vec!["volume", "set_volume:70", "set_mute:true"]
        );

        let failing = FakeBackend::new().failing("volume");
        let error = failing.set_volume(10).unwrap_err();
        assert_eq!(error.code(), "capability_unavailable");
    }

    #[test]
    fn a_window_move_is_clamped_into_the_work_area() {
        let target = window(1);
        // A position far outside the monitor is pulled back inside.
        let (x, y, width, height) = target.clamp_move(99_999, -99_999, 800, 600);
        assert_eq!(x, 1920 - 800);
        assert_eq!(y, 0);
        assert_eq!((width, height), (800, 600));
        // A size larger than the screen is reduced to the work area.
        let (_, _, width, height) = target.clamp_move(0, 0, 9000, 9000);
        assert!(width <= 1920);
        assert!(height <= 1040);
    }

    #[test]
    fn the_fake_refuses_its_own_window_and_a_stale_handle() {
        let backend = FakeBackend::with_windows(vec![window(7)]);
        let listed = backend.list_windows().unwrap();
        assert_eq!(
            backend.act_on_window(&listed[0], &WindowOperation::Minimize),
            Ok(())
        );
        let mut own = window(9);
        own.is_own_process = true;
        assert_eq!(
            backend
                .act_on_window(&own, &WindowOperation::Close)
                .unwrap_err()
                .code(),
            "forbidden_action"
        );
        let stale = FakeBackend::with_windows(vec![NativeWindow {
            native_id: 0,
            ..window(1)
        }]);
        let listed = stale.list_windows().unwrap();
        assert_eq!(
            stale
                .act_on_window(&listed[0], &WindowOperation::Restore)
                .unwrap_err()
                .code(),
            "window_not_found"
        );
    }

    #[test]
    fn a_capture_writes_to_the_destination_and_a_failure_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let destination = destination_in(directory.path(), "shot.png");
        let backend = FakeBackend::new();
        let bytes = backend
            .capture(&CaptureRequest {
                target: ScreenshotTarget::PrimaryMonitor,
                destination: destination.clone(),
                window: None,
            })
            .unwrap();
        assert_eq!(bytes, 128);
        assert!(destination.is_file());
        assert_eq!(backend.captured(), vec![destination.clone()]);

        let failing = FakeBackend::new().failing_capture();
        let error = failing
            .capture(&CaptureRequest {
                target: ScreenshotTarget::PrimaryMonitor,
                destination: destination_in(directory.path(), "second.png"),
                window: None,
            })
            .unwrap_err();
        assert_eq!(error.code(), "screenshot_failed");
    }

    #[test]
    fn capabilities_gate_each_action_family() {
        let capabilities = Capabilities::full();
        assert!(capabilities.supports(&WindowsAction::GetVolume));
        assert!(capabilities.supports(&WindowsAction::TakeScreenshot {
            target: ScreenshotTarget::PrimaryMonitor
        }));
        assert!(capabilities.supports(&WindowsAction::LockWorkstation));
        let no_audio = Capabilities {
            volume: false,
            ..Capabilities::full()
        };
        assert!(!no_audio.supports(&WindowsAction::MuteVolume { muted: true }));
        assert_eq!(
            no_audio.missing_capability(&WindowsAction::MuteVolume { muted: true }),
            Some("windows-actions-capability-volume")
        );
    }

    #[test]
    fn the_minimum_timer_is_the_documented_one() {
        assert_eq!(minimum_timer_seconds(), 5);
        assert_eq!(max_move_pixels(), MAX_MOVE_PIXELS);
    }
}
