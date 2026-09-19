/// Version of the events and actions the two processes exchange.
///
/// A mismatch is reported by the window as incompatible_version instead of a
/// listener that looks ready and answers nothing. It is bumped whenever an event
/// or an action changes shape.
/// The value lives in jarvis_core::desktop because the window process does not
/// compile the IPC module (it is behind the voice-host feature) and still has to
/// know which version it expects.
pub const IPC_PROTOCOL_VERSION: u32 = crate::desktop::VOICE_PROTOCOL_VERSION;

use serde::{Deserialize, Serialize};

// Events sent from jarvis-app to GUI
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum IpcEvent {
    // Wake word detected, starting to listen
    WakeWordDetected,

    // Actively listening for command
    Listening,

    // Speech recognized
    SpeechRecognized {
        text: String,
    },

    // Command was executed
    CommandExecuted {
        id: String,
        success: bool,
    },

    // Returned to idle state
    Idle,

    // Error occurred
    Error {
        message: String,
    },

    // App started
    Started,

    // The version the voice host speaks, sent to every client the moment it
    // connects. This is the handshake the window waits for before it calls the
    // listener ready: a process that is running but has not said hello is not a
    // listener yet, and one that says another version is not a usable one.
    Hello {
        protocol_version: u32,
    },

    // App is shutting down
    Stopping,

    // Pong response
    Pong,

    // request GUI to reveal/focus window
    RevealWindow,

    // One stop of one spoken phrase on its way through the listener, with no
    // transcript in it: a stage name, the length of the text the matcher saw, a
    // reason code, a command id and an outcome. A phrase that is not accepted is
    // answered here instead of only in a log line that quotes it.
    CommandDiagnostic {
        stage: String,
        length: usize,
        code: Option<String>,
        command_id: Option<String>,
        success: Option<bool>,
    },

    // The listener recognised the global voice input phrase and has given the
    // microphone up. The event carries no transcript: the window process runs
    // the dictation with its own session and its own target.
    GlobalDictationRequested,
}

// Actions sent from GUI to jarvis-app
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum IpcAction {
    // Request graceful shutdown
    Stop,

    // Reload commands from disk
    ReloadCommands,

    // Ping to check connection
    Ping,

    // The version the window speaks, so the host can refuse an incompatible
    // client instead of ignoring it.
    Hello { protocol_version: u32 },

    // Mute/unmute listening
    SetMuted { muted: bool },

    // Execute text command
    TextCommand { text: String },
}
