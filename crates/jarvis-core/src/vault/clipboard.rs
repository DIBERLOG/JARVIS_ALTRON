//! Timed clipboard handling for secrets.
//!
//! Copying happens in Rust so a password never has to travel to the interface
//! just to be placed on the clipboard. The guard remembers what it copied and
//! only clears the clipboard while that value is still there: if the user copied
//! something else in the meantime, their text is left alone.
//!
//! Secrets are held in [`Zeroizing`] buffers and never rendered or logged.
//! Clipboard failures are reported without the value that failed to be copied.

use serde::Serialize;
use std::fmt;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// Shortest accepted automatic-clear delay.
pub const MIN_CLEAR_SECONDS: u64 = 15;
/// Longest accepted automatic-clear delay.
pub const MAX_CLEAR_SECONDS: u64 = 60;
/// Default automatic-clear delay.
pub const DEFAULT_CLEAR_SECONDS: u64 = 30;

/// Clipboard failures. No variant carries the copied value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardError {
    /// The clipboard could not be opened or written.
    Unavailable,
    /// This platform has no clipboard implementation in this build.
    UnsupportedPlatform,
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "the clipboard is unavailable",
            Self::UnsupportedPlatform => {
                "the clipboard is not supported on this operating system"
            }
        })
    }
}

impl std::error::Error for ClipboardError {}

/// Platform clipboard access, kept behind a trait so the timing rules are
/// testable without touching the real clipboard.
pub trait ClipboardBackend: Send {
    /// Current text, or `None` when the clipboard holds no text or cannot be
    /// read. An unreadable clipboard must never look like a match.
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError>;
    fn write_text(&mut self, value: &str) -> Result<(), ClipboardError>;
    /// Empties the clipboard.
    fn clear(&mut self) -> Result<(), ClipboardError>;
}

/// What the watchdog did on the last tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardOutcome {
    /// Nothing was pending.
    Idle,
    /// The clipboard still held our secret and was cleared.
    Cleared,
    /// The clipboard held something else, so it was left untouched.
    LeftAlone,
    /// The pending copy was forgotten and the clipboard could not be read.
    Unreadable,
}

/// Public status of the clipboard guard; contains no secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ClipboardStatus {
    /// A timed clear is armed.
    pub armed: bool,
    /// Whole seconds left before the clear, rounded up.
    pub remaining_seconds: u64,
    /// Delay the current copy uses.
    pub timeout_seconds: u64,
}

impl ClipboardStatus {
    pub fn idle() -> Self {
        Self {
            armed: false,
            remaining_seconds: 0,
            timeout_seconds: DEFAULT_CLEAR_SECONDS,
        }
    }
}

/// Normalizes a requested delay into the supported range.
pub fn clamp_clear_seconds(seconds: u64) -> u64 {
    seconds.clamp(MIN_CLEAR_SECONDS, MAX_CLEAR_SECONDS)
}

struct PendingSecret {
    value: Zeroizing<String>,
    deadline: Instant,
    timeout: Duration,
}

/// Copies secrets and clears them again after a delay.
pub struct ClipboardGuard<B: ClipboardBackend> {
    backend: B,
    pending: Option<PendingSecret>,
}

impl<B: ClipboardBackend> ClipboardGuard<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            pending: None,
        }
    }

    /// Copies `secret` and arms the timed clear.
    ///
    /// A previous pending value is replaced, so its timer never fires later and
    /// wipes an unrelated clipboard.
    pub fn copy_secret(
        &mut self,
        secret: &str,
        clear_after_seconds: u64,
    ) -> Result<ClipboardStatus, ClipboardError> {
        self.copy_secret_at(secret, clear_after_seconds, Instant::now())
    }

    /// Same as [`ClipboardGuard::copy_secret`] with an explicit clock, so the
    /// timing rules can be tested deterministically.
    pub fn copy_secret_at(
        &mut self,
        secret: &str,
        clear_after_seconds: u64,
        now: Instant,
    ) -> Result<ClipboardStatus, ClipboardError> {
        let timeout = Duration::from_secs(clamp_clear_seconds(clear_after_seconds));
        self.backend.write_text(secret)?;
        self.pending = Some(PendingSecret {
            value: Zeroizing::new(secret.to_string()),
            deadline: now + timeout,
            timeout,
        });
        Ok(self.status_at(now))
    }

    pub fn status(&self) -> ClipboardStatus {
        self.status_at(Instant::now())
    }

    pub fn status_at(&self, now: Instant) -> ClipboardStatus {
        match &self.pending {
            None => ClipboardStatus::idle(),
            Some(pending) => ClipboardStatus {
                armed: true,
                remaining_seconds: pending
                    .deadline
                    .saturating_duration_since(now)
                    .as_secs()
                    .saturating_add(1),
                timeout_seconds: pending.timeout.as_secs(),
            },
        }
    }

    /// Clears the clipboard now, but only while it still holds our secret.
    pub fn clear_now(&mut self) -> Result<ClipboardOutcome, ClipboardError> {
        self.wipe_if_due_inner(Instant::now(), true)
    }

    /// Called by the watchdog: clears the clipboard once the deadline passed.
    pub fn wipe_if_due(&mut self, now: Instant) -> Result<ClipboardOutcome, ClipboardError> {
        self.wipe_if_due_inner(now, false)
    }

    fn wipe_if_due_inner(
        &mut self,
        now: Instant,
        force: bool,
    ) -> Result<ClipboardOutcome, ClipboardError> {
        let due = match self.pending.as_ref() {
            None => return Ok(ClipboardOutcome::Idle),
            Some(pending) => force || now >= pending.deadline,
        };
        if !due {
            return Ok(ClipboardOutcome::Idle);
        }
        let Some(pending) = self.pending.take() else {
            return Ok(ClipboardOutcome::Idle);
        };
        let current = self.backend.read_text();
        match current {
            Ok(Some(text)) if text == *pending.value => {
                self.backend.clear()?;
                Ok(ClipboardOutcome::Cleared)
            }
            // Anything else — different text, empty clipboard, unreadable
            // clipboard — must be left alone.
            Ok(_) => Ok(ClipboardOutcome::LeftAlone),
            Err(_) => Ok(ClipboardOutcome::Unreadable),
        }
    }

    /// Drops the pending value, clearing the clipboard first when it still holds
    /// our secret. Used when the storage locks.
    pub fn cancel(&mut self) -> Result<ClipboardOutcome, ClipboardError> {
        if self.pending.is_none() {
            return Ok(ClipboardOutcome::Idle);
        }
        self.clear_now()
    }

    /// Whether a timed clear is armed.
    pub fn is_armed(&self) -> bool {
        self.pending.is_some()
    }
}

impl<B: ClipboardBackend> fmt::Debug for ClipboardGuard<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClipboardGuard")
            .field("armed", &self.pending.is_some())
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Platform clipboard used by the application.
#[cfg(windows)]
pub struct SystemClipboard;

#[cfg(windows)]
impl ClipboardBackend for SystemClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        match clipboard_win::get_clipboard_string() {
            Ok(text) => Ok(Some(text)),
            // No text on the clipboard, or it is held by another process. Both
            // must be treated as "not our value" so nothing foreign is cleared.
            Err(_) => Ok(None),
        }
    }

    fn write_text(&mut self, value: &str) -> Result<(), ClipboardError> {
        clipboard_win::set_clipboard_string(value).map_err(|_| ClipboardError::Unavailable)
    }

    fn clear(&mut self) -> Result<(), ClipboardError> {
        let mut result: Result<(), ClipboardError> = Ok(());
        clipboard_win::with_clipboard(|| {
            if clipboard_win::raw::empty().is_err() {
                result = Err(ClipboardError::Unavailable);
            }
        })
        .map_err(|_| ClipboardError::Unavailable)?;
        result
    }
}

#[cfg(not(windows))]
pub struct SystemClipboard;

#[cfg(not(windows))]
impl ClipboardBackend for SystemClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        Ok(None)
    }

    fn write_text(&mut self, _value: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::UnsupportedPlatform)
    }

    fn clear(&mut self) -> Result<(), ClipboardError> {
        Err(ClipboardError::UnsupportedPlatform)
    }
}
