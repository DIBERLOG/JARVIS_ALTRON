//! Chat and local-runtime failures.
//!
//! Variants never carry a prompt, a completion, a secret, or a raw server body,
//! so an error can be shown to the user and written to a log without leaking
//! conversation content or credentials.

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatError {
    Disabled,
    MissingApiKey,
    NetworkUnavailable,
    Unauthorized,
    RateLimited,
    TimedOut,
    InvalidResponse,
    Provider(String),

    // Local runtime (llama-server) failures.
    /// No usable configuration yet: the server or the model path is missing.
    NotConfigured,
    /// The configuration was rejected before anything was started.
    InvalidConfiguration(String),
    /// The managed server is not running.
    ServerNotRunning,
    /// The managed server is starting and cannot accept requests yet.
    ServerNotReady,
    /// The managed server exited unexpectedly.
    ServerStopped,
    /// Another instance of the local server is already managed.
    AlreadyRunning,
    /// The server did not become ready within the startup timeout.
    StartupTimedOut,
    /// The server process could not be spawned, for example because the path is
    /// not an executable.
    ProcessUnavailable,
    /// The model file was rejected.
    ModelUnavailable(String),
    /// The machine does not obviously have enough memory for the configuration.
    InsufficientResources(String),
    /// The endpoint answered with a non-success status. The body is never kept.
    HttpStatus(u16),
    /// The streaming body was malformed or ended unexpectedly.
    InvalidStream,
    /// The user cancelled the generation.
    Cancelled,
    /// A generation is already running in this session.
    GenerationInProgress,
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => f.write_str("AI chat is disabled"),
            Self::MissingApiKey => f.write_str("AI API key is not configured"),
            Self::NetworkUnavailable => {
                f.write_str("AI service is unavailable; check your internet connection")
            }
            Self::Unauthorized => f.write_str("AI API key was rejected"),
            Self::RateLimited => f.write_str("AI service limit reached; try again later"),
            Self::TimedOut => f.write_str("AI request timed out"),
            Self::InvalidResponse => f.write_str("AI service returned an invalid response"),
            Self::Provider(message) => f.write_str(message),

            Self::NotConfigured => {
                f.write_str("the local AI is not configured: select llama-server and a GGUF model")
            }
            Self::InvalidConfiguration(detail) => write!(f, "invalid local AI settings: {detail}"),
            Self::ServerNotRunning => f.write_str("the local AI server is not running"),
            Self::ServerNotReady => f.write_str("the local AI server is still starting"),
            Self::ServerStopped => f.write_str("the local AI server stopped unexpectedly"),
            Self::AlreadyRunning => f.write_str("a local AI server is already managed"),
            Self::StartupTimedOut => {
                f.write_str("the local AI server did not become ready in time")
            }
            Self::ProcessUnavailable => {
                f.write_str("the local AI server process could not be started")
            }
            Self::ModelUnavailable(detail) => write!(f, "the model file cannot be used: {detail}"),
            Self::InsufficientResources(detail) => {
                write!(f, "not enough memory for this configuration: {detail}")
            }
            Self::HttpStatus(status) => {
                write!(f, "the local AI server answered with HTTP {status}")
            }
            Self::InvalidStream => f.write_str("the local AI response stream was malformed"),
            Self::Cancelled => f.write_str("generation was cancelled"),
            Self::GenerationInProgress => f.write_str("a generation is already running"),
        }
    }
}

impl std::error::Error for ChatError {}

impl From<std::io::Error> for ChatError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Self::TimedOut,
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::AddrNotAvailable => Self::ServerNotRunning,
            _ => Self::NetworkUnavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_is_safe_to_show_and_to_log() {
        // No variant can carry a prompt or a response body, so an error is always
        // safe to display and to log.
        let errors = [
            ChatError::Disabled,
            ChatError::MissingApiKey,
            ChatError::NetworkUnavailable,
            ChatError::Unauthorized,
            ChatError::RateLimited,
            ChatError::TimedOut,
            ChatError::InvalidResponse,
            ChatError::NotConfigured,
            ChatError::InvalidConfiguration("missing port".into()),
            ChatError::ServerNotRunning,
            ChatError::ServerNotReady,
            ChatError::ServerStopped,
            ChatError::AlreadyRunning,
            ChatError::StartupTimedOut,
            ChatError::ProcessUnavailable,
            ChatError::ModelUnavailable("not a GGUF file".into()),
            ChatError::InsufficientResources("needs more RAM".into()),
            ChatError::HttpStatus(500),
            ChatError::InvalidStream,
            ChatError::Cancelled,
            ChatError::GenerationInProgress,
        ];
        for error in errors {
            let rendered = format!("{error} {error:?}");
            assert!(!rendered.is_empty());
            assert!(!rendered.contains("FICTIONAL_PROMPT"));
        }
    }

    #[test]
    fn io_errors_map_onto_controlled_variants() {
        use std::io::{Error, ErrorKind};
        assert_eq!(
            ChatError::from(Error::new(ErrorKind::TimedOut, "timed out")),
            ChatError::TimedOut
        );
        assert_eq!(
            ChatError::from(Error::new(ErrorKind::ConnectionRefused, "refused")),
            ChatError::ServerNotRunning
        );
        assert_eq!(
            ChatError::from(Error::other("mystery")),
            ChatError::NetworkUnavailable
        );
    }
}
