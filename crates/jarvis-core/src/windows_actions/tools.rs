//! Structured tool calls: the only way the local model can ask for an action.
//!
//! The model never sees a shell, a path, a file system, an environment variable, or the vault.
//! It sees a small catalogue of tools with strict JSON schemas, and it answers with a tool
//! call. Everything a tool call can say is decoded into a [`WindowsAction`], and a call that
//! does not fit its schema exactly is refused instead of being repaired:
//!
//! * every schema sets `additionalProperties: false`, and the decoder rejects unknown fields,
//!   so an argument named `command`, `path`, `args`, or `shell` is an error rather than a
//!   silently ignored extra;
//! * a name that is not in the catalogue is refused — there is no fallback that reads a
//!   command out of the model's prose, and there never will be: a phrase in an answer is text;
//! * numeric arguments are range-checked by [`WindowsAction::validate`] in one place, and the
//!   identifiers that do appear (an application id, a window id) are opaque values minted by
//!   this application;
//! * the model can ask for anything in the catalogue, and the central policy still decides
//!   whether it is safe, confirmed, or refused. A tool call is a request, not a permission.
//!
//! When the running server cannot carry structured tools, the catalogue is not offered and
//! [`ToolAvailability`] says so, so the interface can explain that AI actions are unavailable
//! instead of inventing a text protocol.

use serde::{Deserialize, Serialize};

use super::backend::Capabilities;
use super::error::ActionError;
use super::model::{
    ApplicationId, ScreenshotTarget, TimerId, VolumeDirection, WindowId, WindowOperation,
    WindowsAction,
};

/// One tool the model may call.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// Strict JSON schema of the arguments.
    pub parameters: serde_json::Value,
}

/// Whether the model can be given tools right now.
///
/// This is reported to the interface verbatim: when it is unavailable, the interface explains
/// why instead of offering an action the server cannot carry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "available", rename_all = "snake_case")]
pub enum ToolAvailability {
    /// The server's chat template accepts tools and the client can read tool calls.
    Available,
    /// Something is missing; the reason is a Fluent key the interface can show.
    Unavailable { reason: &'static str },
}

impl ToolAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Whether the catalogue may be offered, given the platform capability report.
///
/// The check is deliberately about the *server*, not about good intentions: a model that
/// cannot emit a tool call would otherwise have to be parsed from prose, which this feature
/// refuses to do.
pub fn availability(
    capabilities: &Capabilities,
    template_supports_tools: bool,
) -> ToolAvailability {
    if !capabilities.platform_supported {
        return ToolAvailability::Unavailable {
            reason: "windows-ai-tools-unavailable-platform",
        };
    }
    if !template_supports_tools {
        return ToolAvailability::Unavailable {
            reason: "windows-ai-tools-unavailable-template",
        };
    }
    ToolAvailability::Available
}

/// The catalogue offered to the model.
pub fn tool_catalogue() -> Vec<ToolDefinition> {
    vec![
        tool("get_volume", "Read the current output volume.", empty()),
        tool(
            "set_volume",
            "Set the output volume to an exact percentage between 0 and 100.",
            object(
                &[("percent", integer(0, 100, "Volume in percent."))],
                &["percent"],
            ),
        ),
        tool(
            "change_volume",
            "Move the output volume up or down by a small step.",
            object(
                &[
                    (
                        "direction",
                        enumerated(&["up", "down"], "Which way to move."),
                    ),
                    ("step", integer(1, 25, "How many percentage points.")),
                ],
                &["direction", "step"],
            ),
        ),
        tool(
            "mute_volume",
            "Mute or unmute the output volume.",
            object(
                &[("muted", boolean("True to mute, false to unmute."))],
                &["muted"],
            ),
        ),
        tool(
            "launch_allowed_application",
            "Start an application from the user's allowed list, by its identifier.",
            object(
                &[(
                    "application_id",
                    string_value("Identifier from the allowed list."),
                )],
                &["application_id"],
            ),
        ),
        tool(
            "take_screenshot",
            "Capture the screen or one window into a file. Always confirmed by the user.",
            object(
                &[
                    (
                        "target",
                        enumerated(
                            &[
                                "primary_monitor",
                                "selected_monitor",
                                "selected_window",
                                "all_monitors",
                            ],
                            "What to capture.",
                        ),
                    ),
                    (
                        "monitor",
                        integer(1, 32, "Monitor number, for selected_monitor."),
                    ),
                    (
                        "window_id",
                        string_value("Window identifier, for selected_window."),
                    ),
                ],
                &["target"],
            ),
        ),
        tool(
            "create_timer",
            "Start a timer that notifies when it fires.",
            object(
                &[(
                    "duration_seconds",
                    integer(5, 86_400, "How long, in seconds."),
                )],
                &["duration_seconds"],
            ),
        ),
        tool(
            "cancel_timer",
            "Cancel a timer.",
            object(
                &[("timer_id", string_value("Timer identifier."))],
                &["timer_id"],
            ),
        ),
        tool(
            "create_reminder",
            "Create a reminder with a short message.",
            object(
                &[
                    (
                        "delay_seconds",
                        integer(30, 2_592_000, "How far away, in seconds."),
                    ),
                    ("message", string_value("Short reminder text.")),
                ],
                &["delay_seconds", "message"],
            ),
        ),
        tool(
            "cancel_reminder",
            "Cancel a reminder.",
            object(
                &[("timer_id", string_value("Reminder identifier."))],
                &["timer_id"],
            ),
        ),
        tool("list_windows", "List the visible windows.", empty()),
        tool(
            "minimize_window",
            "Minimize one window from the last listing.",
            window_argument(),
        ),
        tool(
            "maximize_window",
            "Maximize one window from the last listing.",
            window_argument(),
        ),
        tool(
            "restore_window",
            "Restore one window from the last listing.",
            window_argument(),
        ),
        tool(
            "move_window",
            "Move and resize one window inside its monitor.",
            object(
                &[
                    ("window_id", string_value("Window identifier.")),
                    ("x", integer(-20_000, 20_000, "Left edge.")),
                    ("y", integer(-20_000, 20_000, "Top edge.")),
                    ("width", integer(200, 20_000, "Width in pixels.")),
                    ("height", integer(150, 20_000, "Height in pixels.")),
                ],
                &["window_id", "x", "y", "width", "height"],
            ),
        ),
        tool(
            "close_window",
            "Ask one window to close. Always confirmed by the user.",
            window_argument(),
        ),
        tool(
            "lock_workstation",
            "Lock the workstation immediately. Always confirmed by the user.",
            empty(),
        ),
    ]
}

/// Decodes one tool call into an action.
///
/// `arguments` is the object the model produced. An unknown field, a missing field, a wrong
/// type, a value outside its range, or an unknown tool name is an error; nothing is guessed
/// and nothing is taken from prose.
pub fn decode_tool_call(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<WindowsAction, ActionError> {
    if !arguments.is_object() && !matches!(name, "get_volume" | "list_windows" | "lock_workstation")
    {
        return Err(ActionError::InvalidArguments {
            detail: "the arguments must be an object".to_string(),
        });
    }
    match name {
        "get_volume" => {
            expect_object(arguments, &[])?;
            Ok(WindowsAction::GetVolume)
        }
        "set_volume" => {
            expect_object(arguments, &["percent"])?;
            Ok(WindowsAction::SetVolume {
                percent: number(arguments, "percent", 0, 100)? as u8,
            })
        }
        "change_volume" => {
            expect_object(arguments, &["direction", "step"])?;
            let direction = match string(arguments, "direction")?.as_str() {
                "up" => VolumeDirection::Up,
                "down" => VolumeDirection::Down,
                other => {
                    return Err(ActionError::InvalidArguments {
                        detail: format!("unknown direction: {other}"),
                    })
                }
            };
            Ok(WindowsAction::ChangeVolume {
                direction,
                step: number(arguments, "step", 1, 25)? as u8,
            })
        }
        "mute_volume" => {
            expect_object(arguments, &["muted"])?;
            Ok(WindowsAction::MuteVolume {
                muted: boolean_value(arguments, "muted")?,
            })
        }
        "launch_allowed_application" => {
            expect_object(arguments, &["application_id"])?;
            let application_id = ApplicationId::from_stored(string(arguments, "application_id")?)?;
            Ok(WindowsAction::LaunchAllowedApplication { application_id })
        }
        "take_screenshot" => {
            expect_object(arguments, &["target", "monitor", "window_id"])?;
            let target = match string(arguments, "target")?.as_str() {
                "primary_monitor" => ScreenshotTarget::PrimaryMonitor,
                "all_monitors" => ScreenshotTarget::AllMonitors,
                "selected_monitor" => {
                    let monitor = number(arguments, "monitor", 1, 32)? as u32;
                    ScreenshotTarget::SelectedMonitor(monitor)
                }
                "selected_window" => {
                    let id = WindowId::from_stored(string(arguments, "window_id")?)?;
                    ScreenshotTarget::SelectedWindow(id)
                }
                other => {
                    return Err(ActionError::InvalidArguments {
                        detail: format!("unknown screenshot target: {other}"),
                    })
                }
            };
            Ok(WindowsAction::TakeScreenshot { target })
        }
        "create_timer" => {
            expect_object(arguments, &["duration_seconds"])?;
            Ok(WindowsAction::CreateTimer {
                duration_seconds: number(arguments, "duration_seconds", 5, 86_400)?,
            })
        }
        "cancel_timer" => {
            expect_object(arguments, &["timer_id"])?;
            Ok(WindowsAction::CancelTimer {
                timer_id: TimerId::from_stored(string(arguments, "timer_id")?)?,
            })
        }
        "create_reminder" => {
            expect_object(arguments, &["delay_seconds", "message"])?;
            let message = string(arguments, "message")?;
            Ok(WindowsAction::CreateReminder {
                delay_seconds: number(arguments, "delay_seconds", 30, 2_592_000)?,
                message,
            })
        }
        "cancel_reminder" => {
            expect_object(arguments, &["timer_id"])?;
            Ok(WindowsAction::CancelReminder {
                timer_id: TimerId::from_stored(string(arguments, "timer_id")?)?,
            })
        }
        "list_windows" => {
            expect_object(arguments, &[])?;
            Ok(WindowsAction::ListWindows)
        }
        "minimize_window" | "maximize_window" | "restore_window" | "close_window" => {
            expect_object(arguments, &["window_id"])?;
            let window_id = WindowId::from_stored(string(arguments, "window_id")?)?;
            let operation = match name {
                "minimize_window" => WindowOperation::Minimize,
                "maximize_window" => WindowOperation::Maximize,
                "restore_window" => WindowOperation::Restore,
                _ => WindowOperation::Close,
            };
            Ok(WindowsAction::Window {
                window_id,
                operation,
            })
        }
        "move_window" => {
            expect_object(arguments, &["window_id", "x", "y", "width", "height"])?;
            Ok(WindowsAction::Window {
                window_id: WindowId::from_stored(string(arguments, "window_id")?)?,
                operation: WindowOperation::Move {
                    x: number(arguments, "x", -20_000, 20_000)? as i32,
                    y: number(arguments, "y", -20_000, 20_000)? as i32,
                    width: number(arguments, "width", 200, 20_000)? as u32,
                    height: number(arguments, "height", 150, 20_000)? as u32,
                },
            })
        }
        "lock_workstation" => {
            expect_object(arguments, &[])?;
            Ok(WindowsAction::LockWorkstation)
        }
        other => Err(ActionError::InvalidArguments {
            detail: format!("unknown tool: {other}"),
        }),
    }
}

/// Fields that must never appear in a tool call.
///
/// The decoder already refuses unknown fields, so this list exists to give a *specific*
/// refusal — and a test asserts that a call carrying one of these names is rejected for the
/// right reason.
pub const FORBIDDEN_ARGUMENT_NAMES: [&str; 12] = [
    "command",
    "cmd",
    "shell",
    "powershell",
    "exec",
    "execute",
    "path",
    "executable",
    "args",
    "arguments",
    "script",
    "environment",
];

fn expect_object(arguments: &serde_json::Value, allowed: &[&str]) -> Result<(), ActionError> {
    let object = match arguments.as_object() {
        Some(object) => object,
        None => {
            // The empty-argument tools accept a missing object as well.
            return if allowed.is_empty() {
                Ok(())
            } else {
                Err(ActionError::InvalidArguments {
                    detail: "the arguments must be an object".to_string(),
                })
            };
        }
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            if FORBIDDEN_ARGUMENT_NAMES.contains(&key.as_str()) {
                return Err(ActionError::ForbiddenAction {
                    reason: format!("the tool does not accept {key}"),
                });
            }
            return Err(ActionError::InvalidArguments {
                detail: format!("unexpected argument: {key}"),
            });
        }
    }
    Ok(())
}

fn number(
    arguments: &serde_json::Value,
    key: &str,
    min: i64,
    max: i64,
) -> Result<u64, ActionError> {
    let value = arguments
        .get(key)
        .and_then(|value| value.as_i64())
        .ok_or_else(|| ActionError::InvalidArguments {
            detail: format!("{key} must be a number"),
        })?;
    if value < min || value > max {
        return Err(ActionError::InvalidArguments {
            detail: format!("{key} must be between {min} and {max}"),
        });
    }
    Ok(value.unsigned_abs())
}

fn boolean_value(arguments: &serde_json::Value, key: &str) -> Result<bool, ActionError> {
    arguments
        .get(key)
        .and_then(|value| value.as_bool())
        .ok_or_else(|| ActionError::InvalidArguments {
            detail: format!("{key} must be true or false"),
        })
}

fn string(arguments: &serde_json::Value, key: &str) -> Result<String, ActionError> {
    let value = arguments
        .get(key)
        .and_then(|value| value.as_str())
        .ok_or_else(|| ActionError::InvalidArguments {
            detail: format!("{key} must be a string"),
        })?;
    if value.len() > 512 {
        return Err(ActionError::InvalidArguments {
            detail: format!("{key} is too long"),
        });
    }
    Ok(value.to_string())
}

/// Builds one catalogue entry.
fn tool(name: &str, description: &str, parameters: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

fn empty() -> serde_json::Value {
    object(&[], &[])
}

fn window_argument() -> serde_json::Value {
    object(
        &[("window_id", string_value("Window identifier."))],
        &["window_id"],
    )
}

fn object(properties: &[(&str, serde_json::Value)], required: &[&str]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, schema) in properties {
        map.insert((*name).to_string(), schema.clone());
    }
    serde_json::json!({
        "type": "object",
        "properties": map,
        "required": required,
        "additionalProperties": false,
    })
}

fn integer(min: i64, max: i64, description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "integer",
        "minimum": min,
        "maximum": max,
        "description": description,
    })
}

fn string_value(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "description": description,
    })
}

fn boolean(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "boolean",
        "description": description,
    })
}

fn enumerated(values: &[&str], description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "enum": values,
        "description": description,
    })
}

/// The system-prompt addition that tells the model how to use the catalogue.
///
/// It is short on purpose: the schemas carry the detail, and a long prompt is a place where
/// instructions about *not* doing something quietly become suggestions.
pub const TOOLS_HINT: &str = "\
You may use the provided tools to control this computer. Only the tools listed are \
available. Never write a command line, a program path, or an argument list in your answer: \
if an action is needed, call a tool with its identifier. Every action is validated and some \
are confirmed by the user before they run.";

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        tool_catalogue().into_iter().map(|tool| tool.name).collect()
    }

    #[test]
    fn the_catalogue_has_exactly_the_documented_tools() {
        let names = names();
        assert_eq!(
            names,
            vec![
                "get_volume",
                "set_volume",
                "change_volume",
                "mute_volume",
                "launch_allowed_application",
                "take_screenshot",
                "create_timer",
                "cancel_timer",
                "create_reminder",
                "cancel_reminder",
                "list_windows",
                "minimize_window",
                "maximize_window",
                "restore_window",
                "move_window",
                "close_window",
                "lock_workstation",
            ]
        );
        assert_eq!(names.len(), 17);
    }

    #[test]
    fn every_schema_is_strict_and_names_no_path_or_command() {
        for tool in tool_catalogue() {
            let parameters = &tool.parameters;
            assert_eq!(
                parameters.get("additionalProperties"),
                Some(&serde_json::Value::Bool(false)),
                "{} must reject extra fields",
                tool.name
            );
            assert_eq!(
                parameters.get("type").and_then(|value| value.as_str()),
                Some("object")
            );
            let properties = parameters
                .get("properties")
                .and_then(|value| value.as_object())
                .cloned()
                .unwrap_or_default();
            for key in properties.keys() {
                assert!(
                    !FORBIDDEN_ARGUMENT_NAMES.contains(&key.as_str()),
                    "{} exposes {key}",
                    tool.name
                );
            }
            let text = parameters.to_string();
            assert!(!text.contains("command"));
            assert!(!text.contains("path"));
            assert!(!text.contains("shell"));
        }
    }

    #[test]
    fn every_tool_decodes_its_own_valid_call() {
        let cases: Vec<(&str, serde_json::Value)> = vec![
            ("get_volume", serde_json::json!({})),
            ("set_volume", serde_json::json!({"percent": 40})),
            (
                "change_volume",
                serde_json::json!({"direction": "up", "step": 10}),
            ),
            ("mute_volume", serde_json::json!({"muted": true})),
            (
                "launch_allowed_application",
                serde_json::json!({"application_id": "app_abc123"}),
            ),
            (
                "take_screenshot",
                serde_json::json!({"target": "primary_monitor"}),
            ),
            (
                "take_screenshot",
                serde_json::json!({"target": "selected_monitor", "monitor": 2}),
            ),
            ("create_timer", serde_json::json!({"duration_seconds": 600})),
            (
                "cancel_timer",
                serde_json::json!({"timer_id": "0123456789abcdef"}),
            ),
            (
                "create_reminder",
                serde_json::json!({"delay_seconds": 60, "message": "перерыв"}),
            ),
            (
                "cancel_reminder",
                serde_json::json!({"timer_id": "0123456789abcdef"}),
            ),
            ("list_windows", serde_json::json!({})),
            (
                "minimize_window",
                serde_json::json!({"window_id": "aabbccddeeff0011"}),
            ),
            (
                "move_window",
                serde_json::json!({"window_id": "aabbccddeeff0011", "x": 10, "y": 20, "width": 800, "height": 600}),
            ),
            (
                "close_window",
                serde_json::json!({"window_id": "aabbccddeeff0011"}),
            ),
            ("lock_workstation", serde_json::json!({})),
        ];
        for (name, arguments) in cases {
            let action = decode_tool_call(name, &arguments)
                .unwrap_or_else(|error| panic!("{name} failed: {error}"));
            assert!(action.validate().is_ok(), "{name}");
        }
    }

    #[test]
    fn an_extra_field_is_refused() {
        let error = decode_tool_call(
            "set_volume",
            &serde_json::json!({"percent": 40, "extra": 1}),
        )
        .unwrap_err();
        assert_eq!(error.code(), "invalid_arguments");
    }

    #[test]
    fn a_command_like_field_is_refused_as_forbidden() {
        for field in ["command", "shell", "path", "executable", "args", "script"] {
            let mut object = serde_json::Map::new();
            object.insert("percent".to_string(), serde_json::json!(40));
            object.insert(field.to_string(), serde_json::json!("calc.exe"));
            let error =
                decode_tool_call("set_volume", &serde_json::Value::Object(object)).unwrap_err();
            assert_eq!(error.code(), "forbidden_action", "{field}");
            assert!(error.is_policy_refusal());
        }
    }

    #[test]
    fn a_wrong_enum_a_wrong_type_and_a_missing_field_are_refused() {
        assert!(decode_tool_call(
            "change_volume",
            &serde_json::json!({"direction": "sideways", "step": 5})
        )
        .is_err());
        assert!(decode_tool_call("set_volume", &serde_json::json!({"percent": "40"})).is_err());
        assert!(decode_tool_call("set_volume", &serde_json::json!({})).is_err());
        assert!(
            decode_tool_call("take_screenshot", &serde_json::json!({"target": "desktop"})).is_err()
        );
        assert!(decode_tool_call(
            "take_screenshot",
            &serde_json::json!({"target": "selected_window"})
        )
        .is_err());
    }

    #[test]
    fn a_number_outside_its_range_is_refused() {
        assert!(decode_tool_call("set_volume", &serde_json::json!({"percent": 140})).is_err());
        assert!(decode_tool_call("set_volume", &serde_json::json!({"percent": -1})).is_err());
        assert!(decode_tool_call(
            "change_volume",
            &serde_json::json!({"direction": "up", "step": 90})
        )
        .is_err());
        assert!(
            decode_tool_call("create_timer", &serde_json::json!({"duration_seconds": 1})).is_err()
        );
        assert!(decode_tool_call(
            "create_reminder",
            &serde_json::json!({"delay_seconds": 5, "message": "x"})
        )
        .is_err());
    }

    #[test]
    fn an_unknown_tool_is_refused_and_never_guessed() {
        let error =
            decode_tool_call("run_program", &serde_json::json!({"path": "calc.exe"})).unwrap_err();
        assert_eq!(error.code(), "invalid_arguments");
        let error = decode_tool_call("", &serde_json::json!({})).unwrap_err();
        assert_eq!(error.code(), "invalid_arguments");
    }

    #[test]
    fn a_prose_answer_is_not_a_tool_call() {
        // There is no function that reads an action out of text; the decoder only accepts a
        // name from the catalogue and an object of arguments.
        let text = "Sure, I ran cmd.exe /c format C:";
        assert!(decode_tool_call(text, &serde_json::json!({})).is_err());
    }

    #[test]
    fn an_identifier_that_is_not_shaped_like_one_is_refused() {
        assert!(decode_tool_call(
            "launch_allowed_application",
            &serde_json::json!({"application_id": "../../evil"})
        )
        .is_err());
        assert!(decode_tool_call(
            "launch_allowed_application",
            &serde_json::json!({"application_id": "C:/Windows/System32/cmd.exe"})
        )
        .is_err());
        assert!(
            decode_tool_call("close_window", &serde_json::json!({"window_id": "not-hex"})).is_err()
        );
    }

    #[test]
    fn availability_follows_the_server_and_the_platform() {
        assert_eq!(
            availability(&Capabilities::full(), true),
            ToolAvailability::Available
        );
        assert!(!availability(&Capabilities::full(), false).is_available());
        assert!(!availability(&Capabilities::unsupported(), true).is_available());
        match availability(&Capabilities::full(), false) {
            ToolAvailability::Unavailable { reason } => {
                assert_eq!(reason, "windows-ai-tools-unavailable-template")
            }
            ToolAvailability::Available => panic!("expected unavailable"),
        }
    }

    #[test]
    fn both_profiles_get_the_same_catalogue() {
        // The catalogue is a pure function: nothing about JARVIS or ALTRON changes it, so the
        // two profiles cannot end up with different powers.
        let first = tool_catalogue();
        let second = tool_catalogue();
        assert_eq!(first.len(), second.len());
        for (left, right) in first.iter().zip(second.iter()) {
            assert_eq!(left.name, right.name);
            assert_eq!(left.parameters, right.parameters);
            assert!(!left.description.is_empty());
        }
        assert!(TOOLS_HINT.contains("Never write a command line"));
    }
}
