//! Confirmation gates: the one place that decides whether an action may run now, must be
//! confirmed first, or is refused outright.
//!
//! Two gates live here, and both are built from the same machinery:
//!
//! * [`ConfirmationGate`] holds one *typed, immutable* pending request. The payload is
//!   whatever the caller wants to confirm — a catalogue command id for the legacy voice
//!   path, a whole `WindowsAction` for the Windows actions. The gate hands out an opaque
//!   token, and a confirmation is accepted only for that token: a second request cannot
//!   silently confirm the first one, the payload cannot change after the token was issued,
//!   and a token is single-use.
//! * [`SafetyGate`] is the original string-payload gate. It keeps its exact public API (and
//!   its tests) because the voice path in `jarvis-app` uses it; internally it is now one use
//!   of [`ConfirmationGate`], so the rules have a single implementation.
//!
//! This module never starts a process, opens a window, or touches the platform. It is pure
//! state, which is what makes every rule here testable without a desktop session.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Default lifetime of a pending confirmation on the voice path.
pub const DEFAULT_CONFIRMATION_TTL: Duration = Duration::from_secs(15);
/// Default lifetime of a pending confirmation shown in the interface.
///
/// Long enough to read the dialog and decide, short enough that a window left open does not
/// confirm something much later by accident.
pub const GUI_CONFIRMATION_TTL: Duration = Duration::from_secs(45);

/// How risky an action is.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Safe,
    ConfirmationRequired,
    Forbidden,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::ConfirmationRequired => "confirm",
            Self::Forbidden => "forbidden",
        }
    }
}

/// What the legacy gate decided about a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GateDecision {
    Approved,
    AwaitingConfirmation { command_id: String },
    RejectedForbidden,
}

/// What happened to a confirmation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfirmationResult {
    Confirmed { command_id: String },
    Cancelled,
    Expired,
    NotPending,
}

/// An opaque, single-use confirmation token.
///
/// It is generated from the operating system's random source and carries no information
/// about the request, so it cannot be guessed, derived from the action, or reused for
/// another one.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ConfirmationToken(String);

impl ConfirmationToken {
    /// Generates a fresh 128-bit token as lowercase hex.
    pub fn random() -> Self {
        let mut bytes = [0u8; 16];
        // The same random source the key material uses. A failure here means the platform
        // cannot produce randomness at all, which is not a state to paper over: minting a
        // weak token would be worse than refusing to have one.
        if getrandom::fill(&mut bytes).is_err() {
            return Self(String::new());
        }
        let mut encoded = String::with_capacity(32);
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        Self(encoded)
    }

    /// Builds a token from a stored string, for tests and for re-reading a request.
    pub fn from_stored(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the token has the shape this type produces.
    pub fn is_well_formed(&self) -> bool {
        self.0.len() == 32 && self.0.chars().all(|character| character.is_ascii_hexdigit())
    }

    /// Constant-time comparison, so a wrong token cannot be found byte by byte.
    pub fn matches(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let given = candidate.as_bytes();
        if expected.len() != given.len() {
            return false;
        }
        let mut difference = 0u8;
        for (left, right) in expected.iter().zip(given) {
            difference |= left ^ right;
        }
        difference == 0
    }
}

/// Why a confirmation attempt failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmationFailure {
    /// No request is pending.
    NotPending,
    /// The request existed but its time ran out.
    Expired,
    /// A request is pending, but this token does not belong to it.
    Mismatch,
}

impl ConfirmationFailure {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotPending => "not_pending",
            Self::Expired => "expired",
            Self::Mismatch => "mismatch",
        }
    }
}

/// One request waiting for a decision.
///
/// The payload is stored by value and never handed out mutably, which is what makes "the
/// arguments cannot change after the token was issued" a property of the type rather than a
/// convention: execution uses this value, never anything the caller sends again.
#[derive(Clone, Debug)]
pub struct PendingConfirmation<A> {
    token: ConfirmationToken,
    payload: A,
    risk: RiskLevel,
    source: String,
    created_at: Instant,
    expires_at: Instant,
}

impl<A> PendingConfirmation<A> {
    pub fn token(&self) -> &ConfirmationToken {
        &self.token
    }

    /// The stored payload, by reference.
    pub fn payload(&self) -> &A {
        &self.payload
    }

    pub fn risk(&self) -> RiskLevel {
        self.risk
    }

    /// Free-form label of who asked (a source name, never a secret).
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Seconds left before the request expires, rounded up.
    pub fn remaining_seconds(&self, now: Instant) -> u64 {
        let left = self.expires_at.saturating_duration_since(now);
        left.as_secs() + u64::from(left.subsec_millis() > 0)
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now > self.expires_at
    }
}

/// One gate holding at most one pending confirmation.
#[derive(Debug)]
pub struct ConfirmationGate<A> {
    pending: Option<PendingConfirmation<A>>,
    ttl: Duration,
}

impl<A> Default for ConfirmationGate<A> {
    fn default() -> Self {
        Self::new(GUI_CONFIRMATION_TTL)
    }
}

impl<A> ConfirmationGate<A> {
    pub fn new(ttl: Duration) -> Self {
        Self { pending: None, ttl }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn set_ttl(&mut self, ttl: Duration) {
        self.ttl = ttl;
    }

    /// Stores a new pending request and returns its token.
    ///
    /// Any previous pending request is dropped: a new question replaces the old one instead
    /// of queueing behind it, so a token issued earlier can never confirm a later action.
    pub fn request(
        &mut self,
        payload: A,
        risk: RiskLevel,
        source: impl Into<String>,
        now: Instant,
    ) -> ConfirmationToken {
        let token = ConfirmationToken::random();
        self.pending = Some(PendingConfirmation {
            token: token.clone(),
            payload,
            risk,
            source: source.into(),
            created_at: now,
            expires_at: now + self.ttl,
        });
        token
    }

    /// The pending request, without taking it.
    pub fn peek(&self, now: Instant) -> Option<&PendingConfirmation<A>> {
        self.pending
            .as_ref()
            .filter(|pending| !pending.is_expired(now))
    }

    /// When the request was created, for diagnostics that stay content-free.
    pub fn created_at(&self) -> Option<Instant> {
        self.pending.as_ref().map(|pending| pending.created_at)
    }

    /// Takes the pending request when the token matches.
    ///
    /// A wrong token leaves the request pending, so a mistyped or forged token cannot clear
    /// a real question; an expired request is dropped and reported as such.
    pub fn confirm(
        &mut self,
        token: &str,
        now: Instant,
    ) -> Result<PendingConfirmation<A>, ConfirmationFailure> {
        let Some(pending) = self.pending.as_ref() else {
            return Err(ConfirmationFailure::NotPending);
        };
        if !pending.token.matches(token) {
            return Err(ConfirmationFailure::Mismatch);
        }
        let pending = self.pending.take().expect("checked above");
        if pending.is_expired(now) {
            return Err(ConfirmationFailure::Expired);
        }
        Ok(pending)
    }

    /// Confirms whatever is pending, without a token.
    ///
    /// Used by the voice path, where the confirmation is the word "подтверждаю" and there is
    /// no interface to carry a token.
    pub fn confirm_pending(
        &mut self,
        now: Instant,
    ) -> Result<PendingConfirmation<A>, ConfirmationFailure> {
        let Some(pending) = self.pending.take() else {
            return Err(ConfirmationFailure::NotPending);
        };
        if pending.is_expired(now) {
            return Err(ConfirmationFailure::Expired);
        }
        Ok(pending)
    }

    /// Drops the pending request. Returns whether there was one.
    pub fn cancel(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// Drops an expired request. Returns whether it removed one.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.is_expired(now))
        {
            self.pending = None;
            true
        } else {
            false
        }
    }

    pub fn has_pending(&self, now: Instant) -> bool {
        self.peek(now).is_some()
    }
}

/// The original string-payload gate used by the voice command path.
#[derive(Debug)]
pub struct SafetyGate {
    inner: ConfirmationGate<String>,
}

impl Default for SafetyGate {
    fn default() -> Self {
        Self::new(DEFAULT_CONFIRMATION_TTL)
    }
}

impl SafetyGate {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: ConfirmationGate::new(ttl),
        }
    }

    pub fn request(
        &mut self,
        command_id: impl Into<String>,
        risk: RiskLevel,
        now: Instant,
    ) -> GateDecision {
        self.inner.expire(now);
        match risk {
            RiskLevel::Safe => GateDecision::Approved,
            RiskLevel::Forbidden => GateDecision::RejectedForbidden,
            RiskLevel::ConfirmationRequired => {
                let command_id = command_id.into();
                self.inner.request(command_id.clone(), risk, "voice", now);
                GateDecision::AwaitingConfirmation { command_id }
            }
        }
    }

    pub fn confirm(&mut self, now: Instant) -> ConfirmationResult {
        match self.inner.confirm_pending(now) {
            Ok(pending) => ConfirmationResult::Confirmed {
                command_id: pending.payload().clone(),
            },
            Err(ConfirmationFailure::Expired) => ConfirmationResult::Expired,
            Err(_) => ConfirmationResult::NotPending,
        }
    }

    pub fn cancel(&mut self) -> ConfirmationResult {
        if self.inner.cancel() {
            ConfirmationResult::Cancelled
        } else {
            ConfirmationResult::NotPending
        }
    }

    pub fn expire(&mut self, now: Instant) -> bool {
        self.inner.expire(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_action_is_approved_immediately() {
        assert_eq!(
            SafetyGate::default().request("open_notepad", RiskLevel::Safe, Instant::now()),
            GateDecision::Approved
        );
    }

    #[test]
    fn risky_action_needs_a_non_expired_confirmation() {
        let now = Instant::now();
        let mut gate = SafetyGate::new(Duration::from_secs(15));
        assert!(matches!(
            gate.request("shutdown", RiskLevel::ConfirmationRequired, now),
            GateDecision::AwaitingConfirmation { .. }
        ));
        assert_eq!(
            gate.confirm(now + Duration::from_secs(14)),
            ConfirmationResult::Confirmed {
                command_id: "shutdown".into()
            }
        );
    }

    #[test]
    fn confirmation_expires_and_cannot_be_reused() {
        let now = Instant::now();
        let mut gate = SafetyGate::new(Duration::from_secs(15));
        gate.request("shutdown", RiskLevel::ConfirmationRequired, now);
        assert_eq!(
            gate.confirm(now + Duration::from_secs(16)),
            ConfirmationResult::Expired
        );
        assert_eq!(
            gate.confirm(now + Duration::from_secs(16)),
            ConfirmationResult::NotPending
        );
    }

    #[test]
    fn cancellation_clears_the_only_pending_command() {
        let now = Instant::now();
        let mut gate = SafetyGate::default();
        gate.request("shutdown", RiskLevel::ConfirmationRequired, now);
        assert_eq!(gate.cancel(), ConfirmationResult::Cancelled);
        assert_eq!(gate.confirm(now), ConfirmationResult::NotPending);
    }

    #[test]
    fn a_new_request_replaces_instead_of_accumulating_pending_commands() {
        let now = Instant::now();
        let mut gate = SafetyGate::default();
        gate.request("shutdown", RiskLevel::ConfirmationRequired, now);
        gate.request("lock_screen", RiskLevel::ConfirmationRequired, now);
        assert_eq!(
            gate.confirm(now),
            ConfirmationResult::Confirmed {
                command_id: "lock_screen".into()
            }
        );
    }

    // ------------------------------------------------------------- typed gate

    #[test]
    fn a_token_has_the_documented_shape_and_is_unique() {
        let first = ConfirmationToken::random();
        let second = ConfirmationToken::random();
        assert!(first.is_well_formed(), "{first:?}");
        assert!(second.is_well_formed(), "{second:?}");
        assert_ne!(first, second);
        assert!(first.matches(first.as_str()));
        assert!(!first.matches(second.as_str()));
        assert!(!first.matches(""));
    }

    #[test]
    fn only_the_issued_token_confirms_and_only_once() {
        let now = Instant::now();
        let mut gate = ConfirmationGate::new(Duration::from_secs(45));
        let token = gate.request("lock", RiskLevel::ConfirmationRequired, "gui", now);
        assert!(matches!(
            gate.confirm("00000000000000000000000000000000", now),
            Err(ConfirmationFailure::Mismatch)
        ));
        // A wrong token leaves the real question pending.
        assert!(gate.has_pending(now));
        assert_eq!(
            gate.confirm(token.as_str(), now).map(|pending| pending.payload),
            Ok("lock")
        );
        // The second use is refused: the request is gone.
        assert!(matches!(
            gate.confirm(token.as_str(), now),
            Err(ConfirmationFailure::NotPending)
        ));
    }

    #[test]
    fn a_new_request_never_confirms_the_previous_one() {
        let now = Instant::now();
        let mut gate: ConfirmationGate<u32> = ConfirmationGate::new(Duration::from_secs(45));
        let first = gate.request(1, RiskLevel::ConfirmationRequired, "ai", now);
        let second = gate.request(2, RiskLevel::ConfirmationRequired, "ai", now);
        assert!(matches!(
            gate.confirm(first.as_str(), now),
            Err(ConfirmationFailure::Mismatch)
        ));
        assert_eq!(
            gate.confirm(second.as_str(), now).map(|pending| pending.payload),
            Ok(2)
        );
    }

    #[test]
    fn the_stored_payload_is_what_gets_confirmed() {
        let now = Instant::now();
        let mut gate: ConfirmationGate<Vec<String>> = ConfirmationGate::default();
        let token = gate.request(
            vec!["--safe".to_string()],
            RiskLevel::ConfirmationRequired,
            "gui",
            now,
        );
        // The only way to read the payload is through the confirmation, and it comes from
        // the gate, not from the caller: there is no setter and no mutable access.
        let confirmed = gate.confirm(token.as_str(), now).unwrap();
        assert_eq!(confirmed.payload(), &vec!["--safe".to_string()]);
    }

    #[test]
    fn an_expired_request_is_dropped_and_reported() {
        let now = Instant::now();
        let mut gate = ConfirmationGate::new(Duration::from_secs(30));
        let token = gate.request("shot", RiskLevel::ConfirmationRequired, "voice", now);
        assert!(gate.peek(now + Duration::from_secs(31)).is_none());
        assert!(gate.expire(now + Duration::from_secs(31)));
        assert!(matches!(
            gate.confirm(token.as_str(), now + Duration::from_secs(31)),
            Err(ConfirmationFailure::NotPending)
        ));
        // The other order reports the expiry instead, and still drops it.
        let token = gate.request("shot", RiskLevel::ConfirmationRequired, "voice", now);
        assert!(matches!(
            gate.confirm(token.as_str(), now + Duration::from_secs(31)),
            Err(ConfirmationFailure::Expired)
        ));
        assert!(!gate.has_pending(now));
    }

    #[test]
    fn the_remaining_time_counts_down() {
        let now = Instant::now();
        let mut gate = ConfirmationGate::new(Duration::from_secs(45));
        gate.request("shot", RiskLevel::ConfirmationRequired, "gui", now);
        let pending = gate.peek(now).unwrap();
        assert_eq!(pending.remaining_seconds(now), 45);
        assert_eq!(pending.remaining_seconds(now + Duration::from_secs(40)), 5);
        assert_eq!(pending.remaining_seconds(now + Duration::from_secs(44)), 1);
        assert_eq!(pending.source(), "gui");
        assert_eq!(pending.risk(), RiskLevel::ConfirmationRequired);
        assert!(gate.peek(now + Duration::from_secs(44)).is_some());
    }

    #[test]
    fn cancelling_a_pending_request_reports_whether_there_was_one() {
        let now = Instant::now();
        let mut gate = ConfirmationGate::new(Duration::from_secs(45));
        assert!(!gate.cancel());
        gate.request("shot", RiskLevel::ConfirmationRequired, "gui", now);
        assert!(gate.cancel());
        assert!(!gate.cancel());
        assert!(!gate.has_pending(now));
    }

    #[test]
    fn the_voice_confirmation_consumes_exactly_one_request() {
        let now = Instant::now();
        let mut gate = ConfirmationGate::new(Duration::from_secs(45));
        gate.request("shot", RiskLevel::ConfirmationRequired, "voice", now);
        assert_eq!(
            gate.confirm_pending(now).map(|pending| pending.payload),
            Ok("shot")
        );
        assert!(matches!(
            gate.confirm_pending(now),
            Err(ConfirmationFailure::NotPending)
        ));
    }
}
