//! The central risk policy: one table, applied in one place.
//!
//! Everything that can ask for an action — a button, a voice phrase, a structured tool call,
//! the timer scheduler — goes through [`ActionPolicy::check`]. Nothing else in this crate,
//! and nothing in the interface, decides what is allowed: a second copy of these rules is
//! how a safe path and an unsafe path end up disagreeing.
//!
//! Three levels, and the level of an action does not depend on who asked:
//!
//! * [`ActionRisk::Safe`] — reading the volume, changing it slightly, muting, a timer, an
//!   ordinary reminder, listing windows, minimizing or restoring one of them. The user asked
//!   for it and nothing irreversible happens.
//! * [`ActionRisk::Confirm`] — starting an application, taking a screenshot, closing a
//!   window, locking the workstation, storing a reminder that looks sensitive, and changing
//!   the allowlist. These are the actions where a wrong guess costs something.
//! * [`ActionRisk::Forbidden`] — an arbitrary executable, unknown arguments, elevation,
//!   ending a process, shell interpreters, the registry, shutdown, file deletion, the vault,
//!   credentials, and another user's session. These are refused, not confirmed: no dialog
//!   makes them acceptable.
//!
//! A few actions are raised one level by the *source*: a launch asked for by the model or by
//! voice is confirmed even though the same click in the settings would be safe, because the
//! user is not looking at the thing that is being started.

use super::error::ActionError;
use super::model::{ActionRisk, ActionSource, ScreenshotTarget, WindowsAction};

/// Executables that can run arbitrary code and are therefore never allowed as a launch
/// target, whatever the user pastes into the file picker.
///
/// The list is by file name, compared case-insensitively, because the picker hands back a
/// path the user chose, and these names are the ones that turn "start an application" into
/// "run anything".
pub const FORBIDDEN_EXECUTABLE_NAMES: [&str; 18] = [
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
    "rundll32.exe",
    "regsvr32.exe",
    "conhost.exe",
    "wt.exe",
    "bash.exe",
    "sh.exe",
    "wsl.exe",
    "python.exe",
    "pythonw.exe",
    "node.exe",
    "java.exe",
    "javaw.exe",
];

/// Command-line interpreters and script hosts, by the same names but listed separately for
/// the interface, which explains *why* a file was refused.
pub const SHELL_LIKE_NAMES: [&str; 6] = [
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
];

/// The policy table: every action kind with its level and a short note.
///
/// It is data, not documentation: the tests walk it, the settings page can render it, and
/// [`ActionPolicy::risk`] is the only implementation of the rules.
pub const POLICY_TABLE: [(&str, ActionRisk, &str); 16] = [
    ("get_volume", ActionRisk::Safe, "reads the current level"),
    (
        "set_volume",
        ActionRisk::Safe,
        "changes the level to what the user said",
    ),
    (
        "change_volume",
        ActionRisk::Safe,
        "moves the level by a small step",
    ),
    ("mute_volume", ActionRisk::Safe, "mutes or unmutes"),
    (
        "create_timer",
        ActionRisk::Safe,
        "starts a countdown that only notifies",
    ),
    (
        "cancel_timer",
        ActionRisk::Safe,
        "cancels a timer this app started",
    ),
    ("list_windows", ActionRisk::Safe, "lists visible windows"),
    ("minimize_window", ActionRisk::Safe, "hides a window"),
    ("maximize_window", ActionRisk::Safe, "enlarges a window"),
    ("restore_window", ActionRisk::Safe, "restores a window"),
    (
        "launch_allowed_application",
        ActionRisk::Confirm,
        "starts an application from the allowed list",
    ),
    (
        "take_screenshot",
        ActionRisk::Confirm,
        "captures the screen to a file",
    ),
    (
        "close_window",
        ActionRisk::Confirm,
        "asks a window to close",
    ),
    (
        "lock_workstation",
        ActionRisk::Confirm,
        "locks the session immediately",
    ),
    (
        "create_reminder",
        ActionRisk::Safe,
        "stores a short message, confirmed when it looks sensitive",
    ),
    (
        "cancel_reminder",
        ActionRisk::Safe,
        "cancels a reminder this app created",
    ),
];

/// The policy itself.
#[derive(Clone, Copy, Debug)]
pub struct ActionPolicy {
    /// Largest volume change one action may request.
    pub max_volume_step: u8,
    /// Whether a launch asked for by voice or by the model needs a confirmation.
    pub confirm_remote_launch: bool,
    /// Whether a reminder whose text looks sensitive needs a confirmation.
    pub confirm_sensitive_reminder: bool,
}

impl Default for ActionPolicy {
    fn default() -> Self {
        Self {
            max_volume_step: super::model::MAX_VOLUME_STEP_PERCENT,
            confirm_remote_launch: true,
            confirm_sensitive_reminder: true,
        }
    }
}

impl ActionPolicy {
    /// The level of one action, without looking at who asked.
    pub fn risk(&self, action: &WindowsAction) -> ActionRisk {
        match action {
            WindowsAction::GetVolume
            | WindowsAction::SetVolume { .. }
            | WindowsAction::MuteVolume { .. }
            | WindowsAction::CreateTimer { .. }
            | WindowsAction::CancelTimer { .. }
            | WindowsAction::CancelReminder { .. }
            | WindowsAction::ListWindows => ActionRisk::Safe,
            // A relative change is safe because the step is bounded; a bigger step than the
            // policy allows is an argument problem, not a risk level.
            WindowsAction::ChangeVolume { step, .. } => {
                if *step <= self.max_volume_step {
                    ActionRisk::Safe
                } else {
                    ActionRisk::Confirm
                }
            }
            WindowsAction::Window { operation, .. } => match operation {
                super::model::WindowOperation::Close => ActionRisk::Confirm,
                _ => ActionRisk::Safe,
            },
            WindowsAction::LaunchAllowedApplication { .. } => ActionRisk::Confirm,
            WindowsAction::TakeScreenshot { .. } => ActionRisk::Confirm,
            WindowsAction::LockWorkstation => ActionRisk::Confirm,
            WindowsAction::CreateReminder { message, .. } => {
                if self.confirm_sensitive_reminder && reminder_looks_sensitive(message) {
                    ActionRisk::Confirm
                } else {
                    ActionRisk::Safe
                }
            }
        }
    }

    /// The level of one action raised by who asked for it.
    pub fn risk_for(&self, action: &WindowsAction, source: ActionSource) -> ActionRisk {
        let base = self.risk(action);
        if base == ActionRisk::Confirm {
            return base;
        }
        match action {
            WindowsAction::LaunchAllowedApplication { .. }
                if self.confirm_remote_launch && source.is_remote() =>
            {
                ActionRisk::Confirm
            }
            _ => base,
        }
    }

    /// Validates an action and returns what should happen to it.
    ///
    /// The single entry point every source uses. A forbidden action is an error, a safe one
    /// is answered immediately, and a `Confirm` one tells the caller to ask the user.
    pub fn check(
        &self,
        action: &WindowsAction,
        source: ActionSource,
    ) -> Result<ActionRisk, ActionError> {
        action.validate()?;
        // Arguments that are out of policy are refused before any level is considered: a
        // twenty-six point volume jump is not "risky", it is not what the user asked for.
        if let WindowsAction::ChangeVolume { step, .. } = action {
            if *step > self.max_volume_step {
                return Err(ActionError::InvalidArguments {
                    detail: format!(
                        "one volume change is at most {} points",
                        self.max_volume_step
                    ),
                });
            }
        }
        let risk = self.risk_for(action, source);
        match risk {
            ActionRisk::Forbidden => Err(ActionError::ForbiddenAction {
                reason: "the action is not in the allowed set".to_string(),
            }),
            other => Ok(other),
        }
    }

    /// Whether an executable file name may ever be added to the allowlist.
    pub fn executable_name_is_allowed(&self, file_name: &str) -> bool {
        let lowered = file_name.trim().to_ascii_lowercase();
        !FORBIDDEN_EXECUTABLE_NAMES.contains(&lowered.as_str())
    }

    /// The table, for the interface and the tests.
    pub fn table(&self) -> &'static [(&'static str, ActionRisk, &'static str)] {
        &POLICY_TABLE
    }
}

/// Whether a reminder's text looks like it could carry a secret.
///
/// The same filter the AI memory uses, so a reminder cannot become a place where a password
/// is stored without the user seeing a confirmation first. It is a heuristic, which is
/// exactly why it only raises the level instead of refusing.
pub fn reminder_looks_sensitive(message: &str) -> bool {
    !crate::memory::scan_for_secrets(message).is_clean()
}

/// A one-line description of a screenshot target, for the preview.
pub fn describe_screenshot_target(target: &ScreenshotTarget) -> String {
    match target {
        ScreenshotTarget::PrimaryMonitor => "primary_monitor".to_string(),
        ScreenshotTarget::SelectedMonitor(monitor) => format!("monitor:{monitor}"),
        ScreenshotTarget::SelectedWindow(_) => "selected_window".to_string(),
        ScreenshotTarget::AllMonitors => "all_monitors".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_actions::model::{ApplicationId, TimerId, WindowId, WindowOperation};

    fn window() -> WindowId {
        WindowId::from_stored("aabbccddeeff0011").unwrap()
    }

    #[test]
    fn the_table_covers_every_action_the_policy_knows() {
        let policy = ActionPolicy::default();
        let actions = [
            WindowsAction::GetVolume,
            WindowsAction::SetVolume { percent: 40 },
            WindowsAction::ChangeVolume {
                direction: super::super::model::VolumeDirection::Up,
                step: 10,
            },
            WindowsAction::MuteVolume { muted: true },
            WindowsAction::LaunchAllowedApplication {
                application_id: ApplicationId::from_stored("app_notepad").unwrap(),
            },
            WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::PrimaryMonitor,
            },
            WindowsAction::CreateTimer {
                duration_seconds: 600,
            },
            WindowsAction::CancelTimer {
                timer_id: TimerId::from_stored("0123456789abcdef").unwrap(),
            },
            WindowsAction::CreateReminder {
                delay_seconds: 60,
                message: "перерыв".to_string(),
            },
            WindowsAction::CancelReminder {
                timer_id: TimerId::from_stored("0123456789abcdef").unwrap(),
            },
            WindowsAction::ListWindows,
            WindowsAction::Window {
                window_id: window(),
                operation: WindowOperation::Minimize,
            },
            WindowsAction::Window {
                window_id: window(),
                operation: WindowOperation::Close,
            },
            WindowsAction::LockWorkstation,
        ];
        for action in &actions {
            let kind = action.kind();
            let listed = POLICY_TABLE
                .iter()
                .any(|(name, risk, _)| *name == kind && *risk == policy.risk(action));
            assert!(listed, "{kind} is missing from the policy table");
        }
        // The table has no entry the enum cannot produce.
        for (name, _, _) in POLICY_TABLE {
            assert!(
                actions.iter().any(|action| action.kind() == name)
                    || name == "maximize_window"
                    || name == "restore_window"
                    || name == "move_window",
                "{name} is in the table but no action produces it"
            );
        }
    }

    #[test]
    fn the_expected_levels_hold() {
        let policy = ActionPolicy::default();
        for (action, expected) in [
            (WindowsAction::GetVolume, ActionRisk::Safe),
            (WindowsAction::SetVolume { percent: 0 }, ActionRisk::Safe),
            (WindowsAction::MuteVolume { muted: false }, ActionRisk::Safe),
            (WindowsAction::ListWindows, ActionRisk::Safe),
            (
                WindowsAction::CreateTimer {
                    duration_seconds: 60,
                },
                ActionRisk::Safe,
            ),
            (
                WindowsAction::CreateReminder {
                    delay_seconds: 60,
                    message: "купить хлеб".to_string(),
                },
                ActionRisk::Safe,
            ),
            (
                WindowsAction::TakeScreenshot {
                    target: ScreenshotTarget::AllMonitors,
                },
                ActionRisk::Confirm,
            ),
            (WindowsAction::LockWorkstation, ActionRisk::Confirm),
            (
                WindowsAction::Window {
                    window_id: window(),
                    operation: WindowOperation::Close,
                },
                ActionRisk::Confirm,
            ),
            (
                WindowsAction::LaunchAllowedApplication {
                    application_id: ApplicationId::from_stored("app_notepad").unwrap(),
                },
                ActionRisk::Confirm,
            ),
        ] {
            assert_eq!(policy.risk(&action), expected, "{action:?}");
        }
    }

    #[test]
    fn a_window_move_and_a_minimize_stay_safe() {
        let policy = ActionPolicy::default();
        for operation in [
            WindowOperation::Minimize,
            WindowOperation::Maximize,
            WindowOperation::Restore,
            WindowOperation::Move {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
        ] {
            let action = WindowsAction::Window {
                window_id: window(),
                operation,
            };
            assert_eq!(policy.risk(&action), ActionRisk::Safe, "{action:?}");
        }
    }

    #[test]
    fn a_launch_from_the_model_or_voice_is_confirmed_and_a_click_is_not_the_same_question() {
        let policy = ActionPolicy::default();
        let launch = WindowsAction::LaunchAllowedApplication {
            application_id: ApplicationId::from_stored("app_notepad").unwrap(),
        };
        assert_eq!(
            policy.risk_for(&launch, ActionSource::DirectGui),
            ActionRisk::Confirm
        );
        assert_eq!(
            policy.risk_for(&launch, ActionSource::LocalAi),
            ActionRisk::Confirm
        );
        assert_eq!(
            policy.risk_for(&launch, ActionSource::Voice),
            ActionRisk::Confirm
        );
        // A safe action stays safe whatever asked for it.
        assert_eq!(
            policy.risk_for(&WindowsAction::GetVolume, ActionSource::LocalAi),
            ActionRisk::Safe
        );
    }

    #[test]
    fn a_sensitive_reminder_is_raised_to_confirmation() {
        let policy = ActionPolicy::default();
        let sensitive = WindowsAction::CreateReminder {
            delay_seconds: 60,
            message: "password: FICTIONAL_VALUE_123456".to_string(),
        };
        assert_eq!(policy.risk(&sensitive), ActionRisk::Confirm);
        let ordinary = WindowsAction::CreateReminder {
            delay_seconds: 60,
            message: "позвонить маме".to_string(),
        };
        assert_eq!(policy.risk(&ordinary), ActionRisk::Safe);
        assert!(reminder_looks_sensitive(
            "sk-FICTIONAL0000000000000000000000000000"
        ));
        assert!(!reminder_looks_sensitive("забрать посылку"));
    }

    #[test]
    fn an_oversized_volume_step_is_an_argument_error_not_a_confirmation() {
        let policy = ActionPolicy::default();
        let error = policy
            .check(
                &WindowsAction::ChangeVolume {
                    direction: super::super::model::VolumeDirection::Up,
                    step: 40,
                },
                ActionSource::DirectGui,
            )
            .unwrap_err();
        assert_eq!(error.code(), "invalid_arguments");
    }

    #[test]
    fn the_check_entry_point_validates_before_it_decides() {
        let policy = ActionPolicy::default();
        assert_eq!(
            policy
                .check(&WindowsAction::GetVolume, ActionSource::DirectGui)
                .unwrap(),
            ActionRisk::Safe
        );
        assert_eq!(
            policy
                .check(&WindowsAction::LockWorkstation, ActionSource::Voice)
                .unwrap(),
            ActionRisk::Confirm
        );
        assert_eq!(
            policy
                .check(
                    &WindowsAction::SetVolume { percent: 250 },
                    ActionSource::LocalAi
                )
                .unwrap_err()
                .code(),
            "invalid_arguments"
        );
    }

    #[test]
    fn shell_like_executables_are_never_allowed() {
        let policy = ActionPolicy::default();
        for name in FORBIDDEN_EXECUTABLE_NAMES {
            assert!(!policy.executable_name_is_allowed(name), "{name}");
            assert!(
                !policy.executable_name_is_allowed(&name.to_uppercase()),
                "{name}"
            );
        }
        assert!(policy.executable_name_is_allowed("notepad.exe"));
        assert!(policy.executable_name_is_allowed("CalculatorApp.exe"));
        // A name that merely contains one of the forbidden names is a different file.
        assert!(policy.executable_name_is_allowed("notcmd.exe"));
        for name in SHELL_LIKE_NAMES {
            assert!(FORBIDDEN_EXECUTABLE_NAMES.contains(&name));
        }
    }

    #[test]
    fn a_screenshot_target_is_described_without_a_name() {
        assert_eq!(
            describe_screenshot_target(&ScreenshotTarget::PrimaryMonitor),
            "primary_monitor"
        );
        assert_eq!(
            describe_screenshot_target(&ScreenshotTarget::SelectedMonitor(2)),
            "monitor:2"
        );
        assert_eq!(
            describe_screenshot_target(&ScreenshotTarget::SelectedWindow(window())),
            "selected_window"
        );
    }
}
