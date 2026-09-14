//! A confirmation-only gate. It deliberately never starts a process.
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const DEFAULT_CONFIRMATION_TTL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel { #[default] Safe, ConfirmationRequired, Forbidden }

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GateDecision { Approved, AwaitingConfirmation { command_id: String }, RejectedForbidden }

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfirmationResult { Confirmed { command_id: String }, Cancelled, Expired, NotPending }

#[derive(Clone, Debug)]
struct PendingAction { command_id: String, expires_at: Instant }

#[derive(Debug)]
pub struct SafetyGate { pending: Option<PendingAction>, ttl: Duration }

impl Default for SafetyGate { fn default() -> Self { Self::new(DEFAULT_CONFIRMATION_TTL) } }

impl SafetyGate {
    pub fn new(ttl: Duration) -> Self { Self { pending: None, ttl } }
    pub fn request(&mut self, command_id: impl Into<String>, risk: RiskLevel, now: Instant) -> GateDecision {
        self.expire(now);
        match risk {
            RiskLevel::Safe => GateDecision::Approved,
            RiskLevel::Forbidden => GateDecision::RejectedForbidden,
            RiskLevel::ConfirmationRequired => {
                let command_id = command_id.into();
                self.pending = Some(PendingAction { command_id: command_id.clone(), expires_at: now + self.ttl });
                GateDecision::AwaitingConfirmation { command_id }
            }
        }
    }
    pub fn confirm(&mut self, now: Instant) -> ConfirmationResult {
        let Some(pending) = self.pending.take() else { return ConfirmationResult::NotPending; };
        if now > pending.expires_at { ConfirmationResult::Expired } else { ConfirmationResult::Confirmed { command_id: pending.command_id } }
    }
    pub fn cancel(&mut self) -> ConfirmationResult {
        if self.pending.take().is_some() { ConfirmationResult::Cancelled } else { ConfirmationResult::NotPending }
    }
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.pending.as_ref().is_some_and(|pending| now > pending.expires_at) { self.pending = None; true } else { false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_action_is_approved_immediately() {
        assert_eq!(SafetyGate::default().request("open_notepad", RiskLevel::Safe, Instant::now()), GateDecision::Approved);
    }
    #[test]
    fn risky_action_needs_a_non_expired_confirmation() {
        let now = Instant::now(); let mut gate = SafetyGate::new(Duration::from_secs(15));
        assert!(matches!(gate.request("shutdown", RiskLevel::ConfirmationRequired, now), GateDecision::AwaitingConfirmation { .. }));
        assert_eq!(gate.confirm(now + Duration::from_secs(14)), ConfirmationResult::Confirmed { command_id: "shutdown".into() });
    }
    #[test]
    fn confirmation_expires_and_cannot_be_reused() {
        let now = Instant::now(); let mut gate = SafetyGate::new(Duration::from_secs(15));
        gate.request("shutdown", RiskLevel::ConfirmationRequired, now);
        assert_eq!(gate.confirm(now + Duration::from_secs(16)), ConfirmationResult::Expired);
        assert_eq!(gate.confirm(now + Duration::from_secs(16)), ConfirmationResult::NotPending);
    }
}
