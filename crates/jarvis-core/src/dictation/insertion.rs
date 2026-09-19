//! Getting the text into the field: UI Automation first, the clipboard when
//! nothing else is safe.
//!
//! # Why the clipboard is the fallback and not the primary
//!
//! The clipboard is where a person's text goes when an application cannot write
//! into a field directly. It is *visible*: it sits in the clipboard until it is
//! replaced or wiped, and any process on the machine can read it. UI Automation
//! writes into the field itself, so the text never sits anywhere.
//!
//! # Why there is no keyboard synthesis here
//!
//! This project forbids keyboard synthesis and its relatives by construction.
//! Typing a transcript by synthesizing keystrokes means driving whatever has the
//! focus, keystroke by keystroke, with no way to prove afterwards where the text
//! went — and it is the same primitive that makes a keylogger indistinguishable
//! from an assistant. If UI Automation cannot do it, the clipboard does, and the
//! person pastes. Narrowing that prohibition needs a separate decision and a
//! threat model; it is not taken here.
//!
//! # What the probes are
//!
//! Three traits, so the whole route can be tested without a Windows desktop:
//! [`ForegroundProbe`] reads the focused element, [`TextInserter`] writes into it,
//! and [`ClipboardWriter`] puts the text where a person can paste it.

use serde::Serialize;

use super::error::DictationError;
use super::target::{
    decide, InsertionMethod, TargetSnapshot, VoiceInputPreference, WindowIdentity,
};

/// Reads the focused element and the foreground window.
pub trait ForegroundProbe: Send + Sync {
    /// What has the focus right now, and what it is.
    ///
    /// Returning an error means "this build cannot tell", which is treated as a
    /// refusal: an element that cannot be inspected cannot be proven writable.
    fn focused_target(&self) -> Result<TargetSnapshot, DictationError>;

    /// The foreground window a moment later, for the change check.
    fn foreground_identity(&self) -> Option<WindowIdentity>;

    /// Whether this machine can inspect the focus at all.
    fn is_available(&self) -> bool;
}

/// Writes text into the focused element through UI Automation.
pub trait TextInserter: Send + Sync {
    /// Writes `text` into the element it was asked about.
    ///
    /// The implementation must re-check what it is writing into: the check that
    /// happened before the transcription is a moment old by the time the text
    /// arrives.
    fn insert(&self, text: &str) -> Result<(), DictationError>;
}

/// Puts text where a person can paste it.
pub trait ClipboardWriter: Send + Sync {
    /// Copies `text`, and arms whatever cleanup the application uses for it.
    fn write(&self, text: &str) -> Result<(), DictationError>;
}

/// A probe for a platform without UI Automation: it never says anything is
/// writable, so the route falls back to the clipboard and stays safe.
pub struct UnavailableProbe;

impl ForegroundProbe for UnavailableProbe {
    fn focused_target(&self) -> Result<TargetSnapshot, DictationError> {
        Err(DictationError::Unavailable("ui_automation"))
    }

    fn foreground_identity(&self) -> Option<WindowIdentity> {
        None
    }

    fn is_available(&self) -> bool {
        false
    }
}

/// Alias kept for the callers that name the platform probe by its role.
pub type UiAutomationProbe = UnavailableProbe;

/// How the text arrived.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMethod {
    /// Written into the element, without touching the clipboard.
    UiAutomation,
    /// Copied, for the person to paste.
    Clipboard,
}

/// What the delivery did.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Delivery {
    pub method: DeliveryMethod,
    /// The rule that sent it to the clipboard, when a rule did.
    pub rule: Option<&'static str>,
    /// Characters delivered. Never the characters themselves.
    pub characters: usize,
}

/// Delivers `text` to the element the request was made about.
///
/// The order is the requirement's order:
///
/// 1. the window must still be the same one, or nothing is written anywhere;
/// 2. the gate must allow it;
/// 3. UI Automation is tried first when the person prefers it, and a failure
///    there is not the end of the attempt — the clipboard takes over;
/// 4. the clipboard is used when it was chosen, when the element has no way to
///    accept a value, or when UI Automation refused.
pub fn deliver(
    target: &TargetSnapshot,
    preference: VoiceInputPreference,
    text: &str,
    probe: &dyn ForegroundProbe,
    inserter: &dyn TextInserter,
    clipboard: &dyn ClipboardWriter,
) -> Result<Delivery, DictationError> {
    if text.trim().is_empty() {
        return Err(DictationError::EmptyRecording);
    }
    // The clipboard mode is the whole delivery: no focused element is asked
    // about, no window is compared, and no field is written into. The person
    // pastes where they want the text, so a target that is unknown, read-only,
    // disabled, in another window, or not a text field at all is simply not
    // this route's business — refusing for one of those reasons would throw a
    // recognized transcript away for nothing.
    if preference == VoiceInputPreference::Clipboard {
        return copy(text, clipboard, None);
    }
    // The window is checked again here, immediately before anything is written.
    let current = probe
        .foreground_identity()
        .ok_or(DictationError::WindowChanged)?;
    if !target.still_the_same_as(&current) {
        return Err(DictationError::WindowChanged);
    }

    let verdict = decide(target, preference);
    let method = match verdict.decision {
        super::target::InsertionDecision::Refuse(rule) => {
            return Err(DictationError::TargetRefused(rule));
        }
        super::target::InsertionDecision::Insert(method) => method,
    };

    match method {
        InsertionMethod::UiAutomation => match inserter.insert(text) {
            Ok(()) => Ok(Delivery {
                method: DeliveryMethod::UiAutomation,
                rule: None,
                characters: text.chars().count(),
            }),
            Err(error) => {
                // The element was there a moment ago and is not writable now.
                // The text is not lost: it goes to the clipboard.
                log::warn!(
                    "dictation: ui automation could not write (error_code={})",
                    error.code()
                );
                copy(text, clipboard, Some("ui_automation_failed"))
            }
        },
        InsertionMethod::Clipboard => copy(text, clipboard, verdict.rule),
    }
}

fn copy(
    text: &str,
    clipboard: &dyn ClipboardWriter,
    rule: Option<&'static str>,
) -> Result<Delivery, DictationError> {
    clipboard.write(text)?;
    Ok(Delivery {
        method: DeliveryMethod::Clipboard,
        rule,
        characters: text.chars().count(),
    })
}
