//! Where the text may go, and what refuses it.
//!
//! This is the safety core of global voice input. Everything the requirement
//! forbids is expressed here as a rule with a name, and the decision is a value
//! that a test can read: no insertion happens without passing this gate.
//!
//! What is refused, and why each refusal exists:
//!
//! | Rule | Why |
//! | --- | --- |
//! | `password_field` | a dictated word must never land in a secret field |
//! | `own_window` | this application's own windows, where a transcript could be written into the vault or a note it does not own |
//! | `secure_desktop` | the UAC prompt and the lock screen belong to Windows, not to an application |
//! | `elevated_target` | a normal process may not drive an elevated one, and asking Windows to would be a privilege boundary crossing |
//! | `read_only_field` | the field exists to be read |
//! | `element_disabled` | the field cannot accept input right now |
//! | `unknown_element` | a type this build cannot prove is a text field |
//! | `no_text_capability` | the element offers neither a value nor a text pattern, so there is nothing to write into (this one falls back to the clipboard rather than refusing the request) |
//!
//! The last row is deliberately *not* a refusal: the requirement's fallback is
//! the clipboard, and a refusal there would leave the person with nothing.

use serde::{Deserialize, Serialize};

/// How far two window identities may differ before the target is considered gone.
///
/// A window that was closed, replaced, or moved to another process is not the
/// window the person dictated into.
pub const WINDOW_TOLERANCE: u8 = 0;

/// What kind of element had the focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    /// A single-line or multi-line edit control.
    Edit,
    /// A document surface, as a word processor or a browser exposes.
    Document,
    /// An editable combo box.
    ComboBox,
    /// A password control.
    Password,
    /// A control type this build knows and does not accept.
    Other,
    /// No type could be read at all.
    Unknown,
}

impl ElementKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Document => "document",
            Self::ComboBox => "combo_box",
            Self::Password => "password",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }
}

/// Everything the safety gate is allowed to know about the focused element.
///
/// The list is deliberately short, and none of it is content: an element kind,
/// four booleans, two process facts, and the window identity. The text of the
/// field is never read, and the window title is never read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetSnapshot {
    /// Native window handle value at the moment the request started.
    pub window_id: i64,
    /// Process that owns the window.
    pub process_id: u32,
    pub element_kind: ElementKind,
    /// The element is a password control, as UI Automation reports it.
    pub password: bool,
    /// The element is read-only.
    pub read_only: bool,
    /// The element is enabled.
    pub enabled: bool,
    /// The element offers a value that can be set.
    pub supports_value_pattern: bool,
    /// The element offers text that can be read (a document surface).
    pub supports_text_pattern: bool,
    /// The target process runs elevated and this application does not.
    pub elevated_target: bool,
    /// The foreground belongs to the secure desktop (UAC, lock screen).
    pub secure_desktop: bool,
    /// The window belongs to this application.
    pub own_window: bool,
}

impl TargetSnapshot {
    /// A plain, writable edit field: the ordinary case.
    pub fn edit(window_id: i64, process_id: u32) -> Self {
        Self {
            window_id,
            process_id,
            element_kind: ElementKind::Edit,
            password: false,
            read_only: false,
            enabled: true,
            supports_value_pattern: true,
            supports_text_pattern: false,
            elevated_target: false,
            secure_desktop: false,
            own_window: false,
        }
    }

    /// Whether the focus is still the same element in the same window.
    pub fn still_the_same_as(&self, current: &WindowIdentity) -> bool {
        self.window_id == current.window_id && self.process_id == current.process_id
    }
}

/// What the gate decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertionDecision {
    /// Write the text into the focused element with the given method.
    Insert(InsertionMethod),
    /// Do not write anything, for the given rule.
    Refuse(&'static str),
}

/// The method the person prefers, and the method a refusal may fall back to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertionMethod {
    /// Write into the element through UI Automation.
    UiAutomation,
    /// Put the text on the clipboard and tell the person to paste it.
    Clipboard,
}

/// The preference the settings page offers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInputPreference {
    /// Try UI Automation first, and fall back to the clipboard.
    UiAutomation,
    /// Never type; always use the clipboard.
    Clipboard,
}

impl VoiceInputPreference {
    pub fn method(&self) -> InsertionMethod {
        match self {
            Self::UiAutomation => InsertionMethod::UiAutomation,
            Self::Clipboard => InsertionMethod::Clipboard,
        }
    }
}

/// A window identity, read without any content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowIdentity {
    pub window_id: i64,
    pub process_id: u32,
}

/// The verdict, with the rule that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetVerdict {
    pub decision: InsertionDecision,
    /// The rule for a refusal, for the log and the notification.
    pub rule: Option<&'static str>,
}

impl TargetVerdict {
    pub fn allowed(method: InsertionMethod) -> Self {
        Self {
            decision: InsertionDecision::Insert(method),
            rule: None,
        }
    }

    pub fn refused(rule: &'static str) -> Self {
        Self {
            decision: InsertionDecision::Refuse(rule),
            rule: Some(rule),
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self.decision, InsertionDecision::Insert(_))
    }

    pub fn method(&self) -> Option<InsertionMethod> {
        match self.decision {
            InsertionDecision::Insert(method) => Some(method),
            InsertionDecision::Refuse(_) => None,
        }
    }
}

/// The rules, in the order they are checked.
///
/// The order matters: the most dangerous refusal is checked first, so the code a
/// person sees names the most important reason rather than an incidental one.
pub const RULES: [&str; 7] = [
    "secure_desktop",
    "own_window",
    "elevated_target",
    "password_field",
    "element_disabled",
    "read_only_field",
    "unknown_element",
];

/// Whether the text may be written into this element, and how.
pub fn decide(target: &TargetSnapshot, preference: VoiceInputPreference) -> TargetVerdict {
    if target.secure_desktop {
        return TargetVerdict::refused("secure_desktop");
    }
    if target.own_window {
        // The vault, the notes, the chat: this application's own surfaces are
        // reached through their own commands, not by typing into them.
        return TargetVerdict::refused("own_window");
    }
    if target.elevated_target {
        return TargetVerdict::refused("elevated_target");
    }
    if target.password || target.element_kind == ElementKind::Password {
        return TargetVerdict::refused("password_field");
    }
    if !target.enabled {
        return TargetVerdict::refused("element_disabled");
    }
    if target.read_only {
        return TargetVerdict::refused("read_only_field");
    }
    if target.element_kind == ElementKind::Unknown {
        return TargetVerdict::refused("unknown_element");
    }

    match preference {
        // The person asked for the clipboard: nothing is typed at all.
        VoiceInputPreference::Clipboard => TargetVerdict::allowed(InsertionMethod::Clipboard),
        VoiceInputPreference::UiAutomation => {
            let writable = matches!(
                target.element_kind,
                ElementKind::Edit | ElementKind::ComboBox
            ) && target.supports_value_pattern
                || (target.element_kind == ElementKind::Document && target.supports_text_pattern);
            if writable {
                TargetVerdict::allowed(InsertionMethod::UiAutomation)
            } else {
                // Not a refusal: the clipboard is the fallback the requirement
                // asks for, and the rule is reported so the person knows why.
                TargetVerdict {
                    decision: InsertionDecision::Insert(InsertionMethod::Clipboard),
                    rule: Some("no_text_capability"),
                }
            }
        }
    }
}
