//! The two executors a command pack can name that are not a program of their own:
//! a typed native action, and a typed internal event.
//!
//! A pack is a document the user installed, and it must not be able to become a
//! launcher: there is no field that carries a path, a command line, or anything
//! this stage would hand to a shell. Instead a pack names one of a small, fixed set
//! of actions, each with the parameters that action has, and the host decides what
//! to do with it through the *existing* safe pipelines:
//!
//! * `native` becomes a [`WindowsAction`](crate::windows_actions::WindowsAction)
//!   and goes through the policy, the allowlist, the audit log and the executor the
//!   interface and the voice router already use;
//! * `internal` becomes an event of this application — ending a chain, pausing the
//!   listener — and runs no process at all.
//!
//! The host registers the two dispatchers at start-up. A core build without a host
//! (the CLI, a unit test) answers with a typed refusal instead of guessing.

use std::fmt;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

use crate::windows_actions::model::VolumeDirection;

/// The largest percentage a single volume step may move.
pub const MAX_STEP_PERCENT: u8 = 50;

/// An action a pack may ask for, by name.
///
/// The set is deliberately small and closed: an action that is not here cannot be
/// named by a pack, and every variant maps onto something this build already does.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NativeAction {
    /// Read the current volume and say it.
    GetVolume,
    /// Set the volume to an exact percentage.
    SetVolume { percent: u8 },
    /// Move the volume by a step.
    ChangeVolume {
        direction: VolumeDirection,
        step: u8,
    },
    /// Mute or unmute.
    MuteVolume { muted: bool },
    /// Start an application the user allowed, chosen by role and never by path.
    LaunchApplication { role: String },
    /// Capture the screen through the action pipeline.
    TakeScreenshot,
    /// List the visible windows.
    ListWindows,
    /// Lock the workstation through the documented API.
    LockWorkstation,
}

impl NativeAction {
    /// The stable name of the action, as it is written in a pack.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GetVolume => "get_volume",
            Self::SetVolume { .. } => "set_volume",
            Self::ChangeVolume { .. } => "change_volume",
            Self::MuteVolume { .. } => "mute_volume",
            Self::LaunchApplication { .. } => "launch_application",
            Self::TakeScreenshot => "take_screenshot",
            Self::ListWindows => "list_windows",
            Self::LockWorkstation => "lock_workstation",
        }
    }

    /// Checks every number and name the action carries, once, at load time.
    pub fn validate(&self) -> Result<(), NativeError> {
        match self {
            Self::SetVolume { percent } => {
                if *percent > 100 {
                    return Err(NativeError::code("native_volume_range"));
                }
            }
            Self::ChangeVolume { step, .. } => {
                if *step == 0 || *step > MAX_STEP_PERCENT {
                    return Err(NativeError::code("native_volume_step"));
                }
            }
            Self::LaunchApplication { role } => {
                if !is_known_role(role) {
                    return Err(NativeError::code("native_unknown_role"));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether this action needs an application the user allowed before it can run.
    pub fn needs_allowed_application(&self) -> bool {
        matches!(self, Self::LaunchApplication { .. })
    }
}

/// The applications a pack may ask for, by role.
///
/// A role is matched against the *file name* of an entry the user allowed, never
/// against a path this build chose: the pack says "the browser", and the user
/// decides which file that is. Nothing here can start a program the user did not
/// add to the allowlist.
pub const APPLICATION_ROLES: [(&str, &[&str]); 4] = [
    (
        "browser",
        &[
            "chrome.exe",
            "firefox.exe",
            "msedge.exe",
            "brave.exe",
            "opera.exe",
            "vivaldi.exe",
            "browser.exe",
        ],
    ),
    (
        "calculator",
        &["calc.exe", "calculator.exe", "calculatorapp.exe"],
    ),
    ("steam", &["steam.exe"]),
    ("task_manager", &["taskmgr.exe"]),
];

/// Whether a role names an application this build knows how to look for.
pub fn is_known_role(role: &str) -> bool {
    APPLICATION_ROLES
        .iter()
        .any(|(name, _)| *name == role.trim().to_lowercase())
}

/// Whether a file name answers a role. Comparison is case-insensitive.
pub fn role_matches(role: &str, executable_file_name: &str) -> bool {
    let role = role.trim().to_lowercase();
    let file = executable_file_name.trim().to_lowercase();
    APPLICATION_ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .map(|(_, names)| names.iter().any(|candidate| *candidate == file))
        .unwrap_or(false)
}

/// An event of this application that a pack may ask for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum InternalEvent {
    /// End the current follow-up chain: the listener goes back to the wake word.
    StopChaining,
    /// Pause recognition until it is resumed from the window or the tray.
    StopListening,
}

impl InternalEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StopChaining => "stop_chaining",
            Self::StopListening => "stop_listening",
        }
    }

    /// The Fluent key of the event, for the interface.
    pub fn label_key(self) -> String {
        format!("command-internal-{}", self.as_str())
    }
}

/// A refusal or a failure from a native action, as a code that is safe to show and
/// to log: it never carries a path, an argument or a message from the system.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeError {
    code: String,
}

impl NativeError {
    pub fn code(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.code
    }

    /// The Fluent key of the refusal.
    pub fn message_key(&self) -> String {
        format!("command-error-{}", self.code)
    }
}

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code)
    }
}

impl std::error::Error for NativeError {}

/// What the host does with a native action, and what it says about it.
///
/// The answer is a short line for the transcript ("Громкость 50%"), never a path
/// and never a message from the system.
pub type NativeDispatch = fn(&NativeAction) -> Result<String, NativeError>;

/// What the host does with an internal event. The answer is the chain flag the
/// executor returns for every command.
pub type InternalDispatch = fn(InternalEvent) -> Result<bool, String>;

static NATIVE_DISPATCH: OnceCell<NativeDispatch> = OnceCell::new();
static INTERNAL_DISPATCH: OnceCell<InternalDispatch> = OnceCell::new();

/// Registers the host's native pipeline. Returns false if one was registered
/// already, so a second host cannot silently take over the first one's work.
pub fn set_native_dispatch(dispatch: NativeDispatch) -> bool {
    NATIVE_DISPATCH.set(dispatch).is_ok()
}

/// Registers the host's internal event handler.
pub fn set_internal_dispatch(dispatch: InternalDispatch) -> bool {
    INTERNAL_DISPATCH.set(dispatch).is_ok()
}

/// Runs a native action through the registered pipeline.
pub fn dispatch_native(action: &NativeAction) -> Result<String, NativeError> {
    action.validate()?;
    let dispatch = NATIVE_DISPATCH
        .get()
        .ok_or_else(|| NativeError::code("native_not_available"))?;
    dispatch(action)
}

/// Runs an internal event through the registered handler.
pub fn dispatch_internal(event: InternalEvent) -> Result<bool, String> {
    let dispatch = INTERNAL_DISPATCH
        .get()
        .ok_or_else(|| "internal_not_available".to_string())?;
    dispatch(event)
}

/// Whether a native pipeline has been registered in this process.
pub fn native_dispatch_registered() -> bool {
    NATIVE_DISPATCH.get().is_some()
}

/// Whether an internal handler has been registered in this process.
pub fn internal_dispatch_registered() -> bool {
    INTERNAL_DISPATCH.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_is_lowercase_and_names_an_executable() {
        for (role, names) in APPLICATION_ROLES {
            assert_eq!(role, role.to_lowercase(), "a role is written in lower case");
            assert!(!names.is_empty(), "{role} has no file names");
            for name in names {
                assert!(
                    name.ends_with(".exe") && *name == name.to_lowercase(),
                    "{name} is not a lower-case executable name"
                );
            }
            assert!(is_known_role(role), "{role} must be a known role");
        }
        assert!(!is_known_role("powershell"));
        assert!(!is_known_role(""));
    }

    #[test]
    fn a_role_matches_a_file_name_and_nothing_else() {
        assert!(role_matches("browser", "chrome.exe"));
        assert!(role_matches("BROWSER", "Firefox.EXE"));
        assert!(role_matches("calculator", "calc.exe"));
        assert!(role_matches("task_manager", "taskmgr.exe"));
        assert!(!role_matches("browser", "cmd.exe"));
        assert!(!role_matches("browser", "powershell.exe"));
        assert!(!role_matches("unknown", "chrome.exe"));
        // A path that merely contains the name is not a match: only the file name is.
        assert!(role_matches("steam", "steam.exe"));
        assert!(!role_matches("steam", "steam_helper.exe"));
    }

    #[test]
    fn an_action_is_checked_before_it_is_run() {
        assert!(NativeAction::GetVolume.validate().is_ok());
        assert!(NativeAction::SetVolume { percent: 100 }.validate().is_ok());
        assert!(NativeAction::SetVolume { percent: 101 }.validate().is_err());
        assert!(NativeAction::MuteVolume { muted: true }.validate().is_ok());
        assert!(NativeAction::ChangeVolume {
            direction: VolumeDirection::Up,
            step: 10
        }
        .validate()
        .is_ok());
        assert!(NativeAction::ChangeVolume {
            direction: VolumeDirection::Up,
            step: 0
        }
        .validate()
        .is_err());
        assert!(NativeAction::LaunchApplication {
            role: "browser".to_string()
        }
        .validate()
        .is_ok());
        let unknown = NativeAction::LaunchApplication {
            role: "cmd".to_string(),
        };
        assert_eq!(
            unknown.validate().unwrap_err().as_str(),
            "native_unknown_role"
        );
        assert!(unknown.needs_allowed_application());
        assert!(!NativeAction::GetVolume.needs_allowed_application());
    }

    #[test]
    fn an_action_keeps_its_pack_name() {
        assert_eq!(NativeAction::GetVolume.as_str(), "get_volume");
        assert_eq!(NativeAction::TakeScreenshot.as_str(), "take_screenshot");
        assert_eq!(
            NativeAction::LaunchApplication {
                role: "steam".to_string()
            }
            .as_str(),
            "launch_application"
        );
        assert_eq!(InternalEvent::StopChaining.as_str(), "stop_chaining");
        assert_eq!(InternalEvent::StopListening.as_str(), "stop_listening");
        assert_eq!(
            InternalEvent::StopListening.label_key(),
            "command-internal-stop_listening"
        );
    }

    #[test]
    fn an_action_reads_from_the_document_shape_a_pack_uses() {
        // The shape a pack writes: a table with `action` and its parameters.
        #[derive(Deserialize)]
        struct Holder {
            native: NativeAction,
        }
        let holder: Holder = toml::from_str("[native]\naction = \"set_volume\"\npercent = 40\n")
            .expect("a native table");
        assert_eq!(holder.native, NativeAction::SetVolume { percent: 40 });

        let holder: Holder =
            toml::from_str("[native]\naction = \"take_screenshot\"\n").expect("a native table");
        assert_eq!(holder.native, NativeAction::TakeScreenshot);

        let holder: Holder =
            toml::from_str("[native]\naction = \"launch_application\"\nrole = \"browser\"\n")
                .expect("a native table");
        assert_eq!(
            holder.native,
            NativeAction::LaunchApplication {
                role: "browser".to_string()
            }
        );

        let holder: Holder = toml::from_str(
            "[native]\naction = \"change_volume\"\ndirection = \"down\"\nstep = 10\n",
        )
        .expect("a native table");
        assert_eq!(
            holder.native,
            NativeAction::ChangeVolume {
                direction: VolumeDirection::Down,
                step: 10
            }
        );

        // An action that is not in the closed set does not load at all.
        let failed = toml::from_str::<Holder>("[native]\naction = \"run_shell\"\n");
        assert!(failed.is_err(), "an unknown action must not parse");
    }

    #[test]
    fn an_event_reads_from_the_document_shape_a_pack_uses() {
        #[derive(Deserialize)]
        struct Holder {
            internal: InternalEvent,
        }
        let holder: Holder =
            toml::from_str("[internal]\nevent = \"stop_chaining\"\n").expect("an internal table");
        assert_eq!(holder.internal, InternalEvent::StopChaining);
        let failed = toml::from_str::<Holder>("[internal]\nevent = \"shutdown_pc\"\n");
        assert!(failed.is_err(), "an unknown event must not parse");
    }

    #[test]
    fn a_missing_pipeline_is_a_typed_refusal_and_never_a_guess() {
        // This test binary registers nothing, so both dispatchers must refuse with
        // a code instead of doing something on their own.
        assert!(!native_dispatch_registered() || native_dispatch_registered());
        let error = dispatch_native(&NativeAction::GetVolume).unwrap_err();
        assert_eq!(error.as_str(), "native_not_available");
        assert_eq!(error.message_key(), "command-error-native_not_available");
        assert_eq!(
            dispatch_internal(InternalEvent::StopChaining).unwrap_err(),
            "internal_not_available"
        );
        // A malformed action is refused before the pipeline is even asked.
        let malformed = NativeAction::SetVolume { percent: 200 };
        assert_eq!(
            dispatch_native(&malformed).unwrap_err().as_str(),
            "native_volume_range"
        );
    }
}
