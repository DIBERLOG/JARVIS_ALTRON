//! The audit log of Windows actions.
//!
//! Every request is recorded, whatever happens to it: requested, confirmed, cancelled,
//! expired, executed, rejected, failed. What is *not* recorded is as important as what is:
//! no reminder text, no window title beyond a sanitized length-bounded form, no file path of
//! a screenshot beyond the action result the user already sees, no arguments, no vault
//! content, no conversation text, and no keys.
//!
//! Two honest statements belong here rather than only in the documentation:
//!
//! * this log is **not evidence**. It is a plain local file and the owner of the computer can
//!   edit or delete it; it exists to answer "what did the assistant do", not "prove it";
//! * it is bounded. The file rotates, the in-memory view is capped, and clearing it is an
//!   explicit action.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::ActionError;
use super::model::{ActionRisk, ActionSource, ActionStatus};

/// File that holds the log.
pub const AUDIT_FILE: &str = "actions-audit.jsonl";
/// File the previous log is rotated into.
pub const AUDIT_ROTATED_FILE: &str = "actions-audit.1.jsonl";
/// Rotate once the file is larger than this.
pub const MAX_AUDIT_BYTES: u64 = 512 * 1024;
/// Most entries kept in memory for the interface.
pub const MAX_AUDIT_ENTRIES: usize = 500;
/// Longest target label kept.
pub const MAX_TARGET_CHARS: usize = 64;

/// One recorded event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditEntry {
    /// RFC 3339 timestamp.
    pub timestamp: String,
    /// Stable action name, for example `set_volume`.
    pub action_type: String,
    pub source: ActionSource,
    pub risk: ActionRisk,
    pub decision: ActionStatus,
    /// Content-free outcome label, for example `ok`, `cancelled`, `invalid_arguments`.
    pub result: String,
    pub duration_ms: u64,
    /// Error code when the event is a failure.
    pub error_category: Option<String>,
    /// What it acted on, in a bounded and sanitized form: an application id, a window
    /// identifier, a timer identifier, or a monitor number. Never a title, a path, or text.
    pub target: Option<String>,
}

/// The log: an append-only file plus a bounded in-memory view.
#[derive(Clone, Debug)]
pub struct AuditLog {
    path: PathBuf,
    entries: VecDeque<AuditEntry>,
    max_bytes: u64,
}

impl AuditLog {
    /// Opens the log in `directory`, reading at most the in-memory cap.
    pub fn open(directory: &Path) -> Self {
        let path = directory.join(AUDIT_FILE);
        let entries = read_tail(&path, MAX_AUDIT_ENTRIES);
        Self {
            path,
            entries,
            max_bytes: MAX_AUDIT_BYTES,
        }
    }

    /// Overrides the rotation threshold, for tests.
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The entries in memory, newest last.
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.iter().cloned().collect()
    }

    /// The newest `limit` entries, newest first.
    pub fn recent(&self, limit: usize) -> Vec<AuditEntry> {
        self.entries.iter().rev().take(limit).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Appends one event to memory and to the file.
    pub fn record(&mut self, mut entry: AuditEntry) -> Result<(), ActionError> {
        entry.target = entry.target.as_deref().map(sanitize_target);
        self.entries.push_back(entry.clone());
        while self.entries.len() > MAX_AUDIT_ENTRIES {
            self.entries.pop_front();
        }
        self.rotate()?;
        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        append(&self.path, line.as_bytes())
    }

    /// Records a request that started.
    pub fn record_requested(
        &mut self,
        action_type: &str,
        source: ActionSource,
        risk: ActionRisk,
        target: Option<&str>,
        timestamp: impl Into<String>,
    ) -> Result<(), ActionError> {
        self.record(AuditEntry {
            timestamp: timestamp.into(),
            action_type: action_type.to_string(),
            source,
            risk,
            decision: ActionStatus::Requested,
            result: "requested".to_string(),
            duration_ms: 0,
            error_category: None,
            target: target.map(str::to_string),
        })
    }

    /// Records a decision that is not an execution (confirmed, cancelled, expired, rejected).
    ///
    /// The arguments are the fields of one entry; grouping them into a struct would only move
    /// the same list one line up.
    #[allow(clippy::too_many_arguments)]
    pub fn record_decision(
        &mut self,
        action_type: &str,
        source: ActionSource,
        risk: ActionRisk,
        decision: ActionStatus,
        error_category: Option<&str>,
        target: Option<&str>,
        timestamp: impl Into<String>,
    ) -> Result<(), ActionError> {
        self.record(AuditEntry {
            timestamp: timestamp.into(),
            action_type: action_type.to_string(),
            source,
            risk,
            decision,
            result: decision.as_str().to_string(),
            duration_ms: 0,
            error_category: error_category.map(str::to_string),
            target: target.map(str::to_string),
        })
    }

    /// Records an execution or a failure.
    #[allow(clippy::too_many_arguments)]
    pub fn record_outcome(
        &mut self,
        action_type: &str,
        source: ActionSource,
        risk: ActionRisk,
        decision: ActionStatus,
        result: &str,
        duration_ms: u64,
        error_category: Option<&str>,
        target: Option<&str>,
        timestamp: impl Into<String>,
    ) -> Result<(), ActionError> {
        self.record(AuditEntry {
            timestamp: timestamp.into(),
            action_type: action_type.to_string(),
            source,
            risk,
            decision,
            result: result.to_string(),
            duration_ms,
            error_category: error_category.map(str::to_string),
            target: target.map(str::to_string),
        })
    }

    /// Forgets everything, in memory and on disk. Needs an explicit confirmation.
    pub fn clear(&mut self, confirmed: bool) -> Result<(), ActionError> {
        if !confirmed {
            return Err(ActionError::InvalidArguments {
                detail: "clearing the log needs a confirmation".to_string(),
            });
        }
        self.entries.clear();
        let _ = std::fs::remove_file(self.path.with_file_name(AUDIT_ROTATED_FILE));
        crate::fsutil::write_bytes_atomic(&self.path, b"").map_err(|_| ActionError::StorageError)
    }

    /// Writes the in-memory entries to `destination`, which needs a confirmation.
    ///
    /// The file is the same content-free log, so exporting it never reveals more than the
    /// interface already shows.
    pub fn export(&self, destination: &Path, confirmed: bool) -> Result<usize, ActionError> {
        if !confirmed {
            return Err(ActionError::InvalidArguments {
                detail: "exporting the log needs a confirmation".to_string(),
            });
        }
        let mut body = String::new();
        for entry in &self.entries {
            body.push_str(&serde_json::to_string(entry)?);
            body.push('\n');
        }
        crate::fsutil::write_bytes_atomic(destination, body.as_bytes())
            .map_err(|_| ActionError::StorageError)?;
        Ok(self.entries.len())
    }

    /// Rotates the file when it grew past the threshold.
    fn rotate(&self) -> Result<(), ActionError> {
        let Ok(metadata) = std::fs::metadata(&self.path) else {
            return Ok(());
        };
        if metadata.len() < self.max_bytes {
            return Ok(());
        }
        let rotated = self.path.with_file_name(AUDIT_ROTATED_FILE);
        let _ = std::fs::remove_file(&rotated);
        std::fs::rename(&self.path, &rotated).map_err(|_| ActionError::StorageError)
    }
}

fn append(path: &Path, bytes: &[u8]) -> Result<(), ActionError> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Reads the last `limit` parseable entries of a JSONL file.
fn read_tail(path: &Path, limit: usize) -> VecDeque<AuditEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return VecDeque::new();
    };
    let mut entries: VecDeque<AuditEntry> = VecDeque::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<AuditEntry>(line) {
            entries.push_back(entry);
        }
    }
    while entries.len() > limit {
        entries.pop_front();
    }
    entries
}

/// Shortens and cleans a target label.
///
/// A target is an identifier this application made, a monitor number, or an application
/// identifier. Should a caller ever pass something else, it is truncated and stripped of
/// control characters rather than stored as-is.
fn sanitize_target(target: &str) -> String {
    let cleaned: String = target
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TARGET_CHARS)
        .collect();
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entry(action_type: &str) -> AuditEntry {
        AuditEntry {
            timestamp: "2026-09-19T12:00:00+00:00".to_string(),
            action_type: action_type.to_string(),
            source: ActionSource::DirectGui,
            risk: ActionRisk::Safe,
            decision: ActionStatus::Executed,
            result: "ok".to_string(),
            duration_ms: 3,
            error_category: None,
            target: Some("app_notepad".to_string()),
        }
    }

    #[test]
    fn an_event_is_written_and_read_back() {
        let directory = tempdir().unwrap();
        let mut log = AuditLog::open(directory.path());
        assert!(log.is_empty());
        log.record_requested(
            "set_volume",
            ActionSource::Voice,
            ActionRisk::Safe,
            None,
            "t0",
        )
        .unwrap();
        log.record(entry("set_volume")).unwrap();

        let reopened = AuditLog::open(directory.path());
        assert_eq!(reopened.len(), 2);
        assert_eq!(reopened.entries()[0].decision, ActionStatus::Requested);
        assert_eq!(reopened.entries()[1].decision, ActionStatus::Executed);
        assert_eq!(reopened.recent(1)[0].action_type, "set_volume");
    }

    #[test]
    fn a_log_entry_never_carries_document_text_titles_or_paths() {
        let directory = tempdir().unwrap();
        let mut log = AuditLog::open(directory.path());
        log.record_requested(
            "create_reminder",
            ActionSource::LocalAi,
            ActionRisk::Confirm,
            Some("reminder:0123456789abcdef"),
            "t0",
        )
        .unwrap();
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(text.contains("create_reminder"));
        assert!(!text.contains("password"));
        // The struct has no field that could hold a reminder text or a title.
        let encoded = serde_json::to_string(&entry("take_screenshot")).unwrap();
        for forbidden in [
            "message",
            "title",
            "path",
            "arguments",
            "command",
            "payload",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "an audit entry must not carry {forbidden}"
            );
        }
    }

    #[test]
    fn a_long_or_control_bearing_target_is_cleaned() {
        let dirty = format!("{}\u{7}tail", "я".repeat(200));
        let cleaned = sanitize_target(&dirty);
        assert!(cleaned.chars().count() <= MAX_TARGET_CHARS);
        assert!(!cleaned.contains('\u{7}'));
    }

    #[test]
    fn the_file_rotates_once_it_grows() {
        let directory = tempdir().unwrap();
        let mut log = AuditLog::open(directory.path()).with_max_bytes(400);
        for index in 0..12 {
            log.record(entry(&format!("action_number_{index}")))
                .unwrap();
        }
        assert!(directory.path().join(AUDIT_ROTATED_FILE).is_file());
        // One older segment is kept; every rotation replaces it, so it holds a contiguous
        // block that ends just before the current file starts, and the newest entry is live.
        let rotated = std::fs::read_to_string(directory.path().join(AUDIT_ROTATED_FILE)).unwrap();
        let live = std::fs::read_to_string(log.path()).unwrap();
        assert!(!rotated.is_empty());
        assert!(live.contains("action_number_11"));
        assert!(!rotated.contains("action_number_11"));
        // Nothing is invented: every line in both files names a recorded action.
        let lines = rotated.lines().count() + live.lines().count();
        assert!(lines <= 12, "rotation must not duplicate entries: {lines}");
    }

    #[test]
    fn the_memory_view_is_bounded() {
        let directory = tempdir().unwrap();
        let mut log = AuditLog::open(directory.path());
        for index in 0..MAX_AUDIT_ENTRIES + 20 {
            log.record(entry(&format!("action_{index}"))).unwrap();
        }
        assert_eq!(log.len(), MAX_AUDIT_ENTRIES);
        // The newest entry survives, the oldest is gone from memory.
        assert_eq!(
            log.recent(1)[0].action_type,
            format!("action_{}", MAX_AUDIT_ENTRIES + 19)
        );
        assert!(AuditLog::open(directory.path()).len() <= MAX_AUDIT_ENTRIES);
    }

    #[test]
    fn clearing_and_exporting_need_a_confirmation() {
        let directory = tempdir().unwrap();
        let mut log = AuditLog::open(directory.path());
        log.record(entry("set_volume")).unwrap();
        assert!(log.clear(false).is_err());
        assert_eq!(log.len(), 1);
        let destination = directory.path().join("export.jsonl");
        assert!(log.export(&destination, false).is_err());
        let written = log.export(&destination, true).unwrap();
        assert_eq!(written, 1);
        assert!(std::fs::read_to_string(&destination)
            .unwrap()
            .contains("set_volume"));
        log.clear(true).unwrap();
        assert!(log.is_empty());
        assert!(std::fs::read_to_string(log.path()).unwrap().is_empty());
    }

    #[test]
    fn a_damaged_line_is_skipped_instead_of_failing_the_log() {
        let directory = tempdir().unwrap();
        let path = directory.path().join(AUDIT_FILE);
        std::fs::write(
            &path,
            "not json\n{\"timestamp\":\"t\",\"action_type\":\"set_volume\",\"source\":\"voice\",\
             \"risk\":\"safe\",\"decision\":\"executed\",\"result\":\"ok\",\"duration_ms\":1,\
             \"error_category\":null,\"target\":null}\n",
        )
        .unwrap();
        let log = AuditLog::open(directory.path());
        assert_eq!(log.len(), 1);
        assert_eq!(log.entries()[0].action_type, "set_volume");
    }
}
