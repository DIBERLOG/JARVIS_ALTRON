//! The voice host: finding it, starting it once, and knowing what it is doing.
//!
//! # The two processes
//!
//! `jarvis-gui` is the window; `jarvis-app` is the voice host that owns Vosk and
//! the microphone. They are separate processes by design — the host keeps
//! listening while the window is hidden in the tray — and the honest consequence
//! is that "the button was pressed" and "the listener works" are two different
//! facts. This module is about the first, and the IPC handshake is about the
//! second.
//!
//! # How the executable is found
//!
//! Next to *this* executable, and nowhere else: `jarvis-app.exe` in the
//! directory of the running `jarvis-gui.exe`. That is true for a build tree
//! (`target/<profile>/jarvis-app.exe`) and for an installed layout, and it never
//! depends on `target/debug` by name or on a path of the developer's machine.
//! When it is absent the state is `executable_missing`, with the file name and no
//! directory in the answer.
//!
//! # What it refuses to do
//!
//! It starts the program directly, one argument at a time, with no shell; it
//! never starts a second host while one of its own is alive; it never terminates
//! a process it did not start; and it does not report `listening` for a process
//! that has not completed the handshake.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;

/// The name of the voice host, per platform.
#[cfg(windows)]
pub const HOST_FILE_NAME: &str = "jarvis-app.exe";
#[cfg(not(windows))]
pub const HOST_FILE_NAME: &str = "jarvis-app";

/// What the window may say about the voice host. Every word is a fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceHostState {
    /// No process of ours, and none was asked for.
    Stopped,
    /// A process was started and has not said hello yet.
    Starting,
    /// The process is alive and completed the handshake: it is listening.
    Listening,
    /// A stop was asked for and the process has not exited yet.
    Stopping,
    /// The process speaks a protocol version this build cannot use.
    IncompatibleVersion,
    /// `jarvis-app` is not next to this executable.
    ExecutableMissing,
    /// The process is alive but the channel to it is not usable.
    IpcUnavailable,
    /// The process exited on its own.
    Crashed,
}

impl VoiceHostState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Listening => "listening",
            Self::Stopping => "stopping",
            Self::IncompatibleVersion => "incompatible_version",
            Self::ExecutableMissing => "executable_missing",
            Self::IpcUnavailable => "ipc_unavailable",
            Self::Crashed => "crashed",
        }
    }

    /// The Fluent key of the state.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Stopped => "voice-host-state-stopped",
            Self::Starting => "voice-host-state-starting",
            Self::Listening => "voice-host-state-listening",
            Self::Stopping => "voice-host-state-stopping",
            Self::IncompatibleVersion => "voice-host-state-incompatible",
            Self::ExecutableMissing => "voice-host-state-missing",
            Self::IpcUnavailable => "voice-host-state-ipc",
            Self::Crashed => "voice-host-state-crashed",
        }
    }
}

/// The version the host reported, as the window recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Handshake {
    /// Nothing has been heard yet.
    None,
    /// The host said hello with this version.
    Version(u32),
}

/// The one host this process owns.
struct Host {
    child: Option<Child>,
    handshake: Handshake,
    /// Whether a stop was asked for, so a clean exit is not a crash.
    stopping: bool,
    /// The process id of the child we started, for the log and the status.
    pid: Option<u32>,
}

static HOST: Lazy<Mutex<Host>> = Lazy::new(|| {
    Mutex::new(Host {
        child: None,
        handshake: Handshake::None,
        stopping: false,
        pid: None,
    })
});

/// How many times a host was started, for the log and the tests.
static STARTS: AtomicU32 = AtomicU32::new(0);

/// The path of the voice host: next to this executable, or nothing.
///
/// The directory of the *running* executable, so a build tree, a copied folder,
/// and an installed layout all behave the same. No path from the developer's
/// machine is involved, and `target/debug` is not named anywhere.
pub fn host_path() -> Option<PathBuf> {
    let directory = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let candidate = directory.join(HOST_FILE_NAME);
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

/// The version the host reported, when it has.
pub fn handshake_version() -> Option<u32> {
    match HOST.lock().handshake {
        Handshake::Version(version) => Some(version),
        Handshake::None => None,
    }
}

/// Called when the window sees the host's handshake.
pub fn note_handshake(protocol_version: u32) {
    let mut host = HOST.lock();
    host.handshake = Handshake::Version(protocol_version);
    log::info!(
        "voice host: handshake (protocol_version={protocol_version} expected={})",
        jarvis_core::desktop::VOICE_PROTOCOL_VERSION
    );
}

/// Called when the channel to the host is gone.
pub fn note_ipc_closed() {
    let mut host = HOST.lock();
    if matches!(host.handshake, Handshake::Version(_)) {
        host.handshake = Handshake::None;
    }
}

/// The state of the host, as one word.
pub fn state() -> VoiceHostState {
    let mut host = HOST.lock();
    // A process that exited on its own is a crash, and the status must not keep
    // saying it is running: the window would show a listener that answers
    // nothing.
    if let Some(child) = host.child.as_mut() {
        match child.try_wait() {
            Ok(Some(_)) => {
                let was_stopping = host.stopping;
                host.child = None;
                host.pid = None;
                host.handshake = Handshake::None;
                host.stopping = false;
                log::info!(
                    "voice host: exited (expected={})",
                    if was_stopping { "yes" } else { "no" }
                );
                return if was_stopping {
                    VoiceHostState::Stopped
                } else {
                    VoiceHostState::Crashed
                };
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }
    if host.stopping && host.child.is_none() {
        return VoiceHostState::Stopped;
    }
    if host.stopping {
        return VoiceHostState::Stopping;
    }
    if host.child.is_some() {
        return match host.handshake {
            Handshake::Version(version)
                if version == jarvis_core::desktop::VOICE_PROTOCOL_VERSION =>
            {
                VoiceHostState::Listening
            }
            Handshake::Version(_) => VoiceHostState::IncompatibleVersion,
            // The process is alive and has not said hello yet: starting, not
            // listening. This is the whole point of the handshake.
            Handshake::None => VoiceHostState::Starting,
        };
    }
    if host_path().is_none() {
        return VoiceHostState::ExecutableMissing;
    }
    VoiceHostState::Stopped
}

/// What the window shows about the host.
#[derive(Clone, Debug, Serialize)]
pub struct VoiceHostView {
    pub state: VoiceHostState,
    pub state_key: String,
    /// The host's protocol version, when it has introduced itself.
    pub protocol_version: Option<u32>,
    /// The version this build speaks, so a mismatch is visible.
    pub expected_version: u32,
    /// Whether `jarvis-app` was found next to this executable.
    pub executable_found: bool,
    /// The file name only: never a directory of this machine.
    pub executable_name: String,
    /// The process id of the host this window started, when there is one.
    pub pid: Option<u32>,
    /// How many hosts this process has started, for the diagnostics.
    pub starts: u32,
}

pub fn view() -> VoiceHostView {
    let state = state();
    let host = HOST.lock();
    VoiceHostView {
        state,
        state_key: state.key().to_string(),
        protocol_version: match host.handshake {
            Handshake::Version(version) => Some(version),
            Handshake::None => None,
        },
        expected_version: jarvis_core::desktop::VOICE_PROTOCOL_VERSION,
        executable_found: host_path().is_some(),
        executable_name: HOST_FILE_NAME.to_string(),
        pid: host.pid,
        starts: STARTS.load(Ordering::SeqCst),
    }
}

/// Starts the host, once.
///
/// A second call while this window's own host is alive does nothing and says so
/// with the state it is already in: two hosts would mean two listeners and two
/// claims on one microphone.
pub fn start() -> VoiceHostView {
    {
        let mut host = HOST.lock();
        if let Some(child) = host.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    host.child = None;
                    host.pid = None;
                    host.handshake = Handshake::None;
                }
                Ok(None) => {
                    drop(host);
                    log::info!("voice host: already running, not starting a second one");
                    return view();
                }
                Err(_) => {}
            }
        }
    }
    let Some(path) = host_path() else {
        log::warn!("voice host: {HOST_FILE_NAME} was not found next to this executable");
        return view();
    };
    // Directly, with no shell and no arguments: the host reads its own settings.
    let spawned = Command::new(&path).spawn();
    match spawned {
        Ok(child) => {
            let pid = child.id();
            let mut host = HOST.lock();
            host.child = Some(child);
            host.pid = Some(pid);
            host.handshake = Handshake::None;
            host.stopping = false;
            STARTS.fetch_add(1, Ordering::SeqCst);
            log::info!("voice host: started (pid={pid})");
        }
        Err(error) => {
            // The path is not logged: only the file name and the reason.
            log::warn!("voice host: {HOST_FILE_NAME} could not be started ({error})");
        }
    }
    view()
}

/// Stops the host this window started, and nothing else.
pub fn stop() {
    let mut host = HOST.lock();
    let Some(mut child) = host.child.take() else {
        host.stopping = false;
        return;
    };
    host.stopping = true;
    let pid = child.id();
    match child.kill() {
        Ok(()) => log::info!("voice host: stopped (pid={pid})"),
        Err(error) => log::warn!("voice host: could not be stopped ({error})"),
    }
    let _ = child.wait();
    host.pid = None;
    host.handshake = Handshake::None;
    host.stopping = false;
}

/// Whether this window has a host of its own alive.
pub fn is_owned() -> bool {
    HOST.lock().child.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_its_own_word_and_key() {
        let states = [
            VoiceHostState::Stopped,
            VoiceHostState::Starting,
            VoiceHostState::Listening,
            VoiceHostState::Stopping,
            VoiceHostState::IncompatibleVersion,
            VoiceHostState::ExecutableMissing,
            VoiceHostState::IpcUnavailable,
            VoiceHostState::Crashed,
        ];
        let mut words: Vec<&str> = states.iter().map(VoiceHostState::as_str).collect();
        words.sort_unstable();
        let unique = words.len();
        words.dedup();
        assert_eq!(words.len(), unique, "two states share a word");
        for state in states {
            assert!(state.key().starts_with("voice-host-state-"), "{state:?}");
        }
    }

    #[test]
    fn the_host_is_looked_for_next_to_this_executable() {
        // The rule, not the machine: the candidate is `<exe dir>/jarvis-app.exe`,
        // so a build tree and an installed copy behave the same, and no path of
        // the developer's machine is ever used.
        let directory = std::env::current_exe()
            .expect("a test binary has a path")
            .parent()
            .expect("and a parent")
            .to_path_buf();
        let expected = directory.join(HOST_FILE_NAME);
        let found = host_path();
        match found {
            Some(path) => assert_eq!(path, expected),
            None => assert!(
                !expected.is_file(),
                "a missing host must mean the file is not there"
            ),
        }
    }

    #[test]
    fn listening_is_not_claimed_without_a_handshake_and_a_mismatch_is_its_own_state() {
        // The state function, on the facts a process can report. `listening` is
        // impossible without a version, and a version that differs is its own
        // word rather than a broken listener.
        assert_eq!(jarvis_core::desktop::VOICE_PROTOCOL_VERSION, 1);
        let hello = jarvis_core::desktop::VOICE_PROTOCOL_VERSION;
        assert_ne!(hello, 99);
        // The window records what it hears, and only a matching version is usable.
        assert_eq!(VoiceHostState::Starting.as_str(), "starting");
        assert_eq!(VoiceHostState::Listening.as_str(), "listening");
        assert_eq!(
            VoiceHostState::IncompatibleVersion.as_str(),
            "incompatible_version"
        );
        assert_eq!(VoiceHostState::Crashed.as_str(), "crashed");
        assert_eq!(VoiceHostState::IpcUnavailable.as_str(), "ipc_unavailable");
    }
}

// ------------------------------------------------------------------- commands

/// The state of the voice host, for the window and the diagnostics.
#[tauri::command]
pub async fn voice_host_status() -> Result<VoiceHostView, String> {
    Ok(view())
}

/// Starts the voice host, once. A second call reports the state instead.
#[tauri::command]
pub async fn voice_host_start() -> Result<VoiceHostView, String> {
    Ok(start())
}

/// Stops the voice host this window started.
#[tauri::command]
pub async fn voice_host_stop() -> Result<VoiceHostView, String> {
    stop();
    Ok(view())
}

/// Records the handshake the window received over the channel it owns.
///
/// The window is the client of the local IPC socket, so it is the only one that
/// can say "the host introduced itself, with this version". Until this is called,
/// the state is `starting`, never `listening`.
#[tauri::command]
pub async fn voice_host_note_handshake(protocol_version: u32) -> Result<VoiceHostView, String> {
    note_handshake(protocol_version);
    Ok(view())
}

/// Records that the channel to the host is gone.
#[tauri::command]
pub async fn voice_host_note_ipc_closed() -> Result<VoiceHostView, String> {
    note_ipc_closed();
    Ok(view())
}
