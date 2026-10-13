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
    SpeechRecognized { text: String },
    CenterCommand { text: String },
    
    // Command was executed
    CommandExecuted { id: String, success: bool },
    RemoteCommandResult { request_id: String, status: String },
    RemoteCenterCommand { id: String, request_id: String },
    
    // Returned to idle state
    Idle,
    
    // Error occurred
    Error { message: String },

    Notification { title: String, primary: String, detail: Option<String> },

    MicrophoneMuted { muted: bool },
    DialogueMode { active: bool },
    
    // App started
    Started,
    
    // App is shutting down
    Stopping,
    
    // Pong response
    Pong,

    // request GUI to reveal/focus window
    RevealWindow,
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
    
    // Mute/unmute listening
    SetMuted { muted: bool },

    GetMuted,

    // Execute text command
    TextCommand { text: String },
    RemoteCommand { id: String, request_id: String, expires_at: i64 },
    CenterReply { text: String, reply_id: String, follow_up: bool },
}
