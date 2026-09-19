//! The portable, encrypted backup: one container, one password, one sequence.
//!
//! # What this module is
//!
//! A full backup of everything this application stores that belongs to the user,
//! in one file that can be carried to another Windows machine:
//!
//! * four SQLite databases — notes, the password vault, AI memory, and the
//!   spelling dictionary — each snapshotted consistently ([`snapshot`]);
//! * the settings documents, without any model, runtime, log, cache, or
//!   temporary file;
//! * the portable master-key envelope, which is what makes the data openable on
//!   another machine.
//!
//! It is deliberately *not* a copy of the application directory: the files to
//! include are a list, not a directory walk, so a cache directory, a webview
//! profile, a log, a downloaded model, or a temporary WAV cannot end up in a
//! backup by accident.
//!
//! # The cryptography, in one paragraph
//!
//! The container carries the existing portable key envelope — the same
//! Argon2id-derived, XChaCha20-Poly1305-wrapped master key the application
//! already uses for its own key backup — and its payload is encrypted with a key
//! derived from that same master key through HKDF-SHA256 under a purpose label
//! unique to backups (`JARVIS/backup/v1`). The master password is what opens it
//! on any Windows machine; a DPAPI blob is **not** in the container, because a
//! blob belongs to one account on one computer. Every chunk has its own random
//! 24-byte nonce, its own AEAD tag, and associated data that binds it to the
//! manifest, the entry name, the chunk index, and its length.
//!
//! # What it is not
//!
//! It is not a claim that a backup cannot be broken. Anyone who has the file and
//! the password has the data; anyone who has the file and can run code on the
//! machine while the storage is unlocked can read it there; malware running as
//! the user can read what the user can read. The password is the only thing
//! protecting the container, and its strength is the strength of the backup.

pub mod container;
pub mod error;
pub mod restore;
pub mod snapshot;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub use container::{
    BackupManifest, BackupPreview, ComponentKind, ContainerHeader, ContainerSummary, EntrySource,
    Limits, ManifestEntry, FORMAT_NAME, FORMAT_VERSION, LIMITS, MAGIC,
};
pub use error::BackupError;
pub use restore::{
    describe_current, NoKeyBinding, RestoreJournal, RestorePlan, RestoreReport, RestoreStage,
    JOURNAL_FILE, PREVIOUS_DIR, SAFETY_DIR, STAGING_DIR,
};
pub use snapshot::{
    checkpoint_databases, BackupRoots, Component, SnapshotReport, COMPONENTS, KEY_ENVELOPE,
};

use crate::sync::crypto::{derive_purpose_key, import_backup, KeyPurpose, PortableKeyBackup};

/// File name the application uses for its portable key envelope.
pub const KEY_ENVELOPE_FILE: &str = "key.backup.json";

/// Version of the application, as written into a container.
pub fn app_version() -> &'static str {
    crate::config::APP_VERSION.unwrap_or("unknown")
}

/// RFC 3339, UTC, for a container header and a journal.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// A compact UTC stamp for a generated file name.
pub fn now_compact() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// What an export produced.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ExportReport {
    pub format_version: u16,
    pub created_at: String,
    pub app_version: String,
    /// Logical names, in container order.
    pub components: Vec<String>,
    pub total_bytes: u64,
    /// File name only: the interface never keeps a full path.
    pub file: String,
}

impl ExportReport {
    pub fn preview(&self) -> BackupPreview {
        BackupPreview {
            format_version: self.format_version,
            created_at: self.created_at.clone(),
            app_version: self.app_version.clone(),
            total_bytes: self.total_bytes,
            entries: self
                .components
                .iter()
                .map(|name| container::PreviewEntry {
                    name: name.clone(),
                    kind: ComponentKind::Document,
                    bytes: 0,
                })
                .collect(),
            warnings: Vec::new(),
        }
    }
}

/// Everything one export needs.
pub struct ExportPlan {
    pub roots: BackupRoots,
    /// The components to include, in the order they are written.
    pub components: Vec<Component>,
    /// Whether an existing destination may be replaced. Never assumed.
    pub overwrite: bool,
    pub limits: Limits,
}

impl ExportPlan {
    /// A full export over the production components.
    pub fn full(roots: BackupRoots) -> Self {
        Self {
            roots,
            components: COMPONENTS.to_vec(),
            overwrite: false,
            limits: LIMITS,
        }
    }
}

/// Writes a full backup to `destination`.
///
/// The file is written as a temporary neighbour, flushed and synced, and only
/// then renamed over the destination, so an interrupted export never leaves a
/// half-written container where the user expects a backup. A failure removes the
/// temporary file. An existing destination is refused unless the caller passed
/// [`ExportPlan::overwrite`], which is what the interface sets after asking.
pub fn export(
    plan: &ExportPlan,
    destination: &Path,
    password: &[u8],
) -> Result<ExportReport, BackupError> {
    if password.len() < crate::sync::crypto::MIN_PASSWORD_BYTES {
        return Err(BackupError::PasswordTooShort);
    }
    if destination.is_file() && !plan.overwrite {
        return Err(BackupError::DestinationUnavailable);
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // The key envelope is what makes the container usable elsewhere, and the
    // password is checked against it before a byte is written.
    let envelope_path = plan.roots.data_dir.join(KEY_ENVELOPE_FILE);
    if !envelope_path.is_file() {
        return Err(BackupError::Storage);
    }
    let envelope: PortableKeyBackup = serde_json::from_slice(&std::fs::read(&envelope_path)?)?;
    let master = import_backup(&envelope, password)?;
    let key = derive_purpose_key(&master, KeyPurpose::Backup)?;

    let staging = plan.roots.data_dir.join("export-staging");
    let _ = std::fs::remove_dir_all(&staging);
    let entries = match snapshot::snapshot_all(&plan.roots, &staging, &plan.components) {
        Ok(entries) => entries,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

    let created_at = now_rfc3339();
    let temporary = destination.with_extension(format!("part-{}", std::process::id()));
    let written = container::write_container(
        &temporary,
        &entries,
        &envelope,
        &key,
        app_version(),
        &created_at,
        &plan.limits,
    );
    let _ = std::fs::remove_dir_all(&staging);
    let summary = match written {
        Ok(summary) => summary,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = std::fs::rename(&temporary, destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(BackupError::from(error));
    }

    Ok(ExportReport {
        format_version: summary.format_version,
        created_at: summary.created_at,
        app_version: summary.app_version,
        components: summary
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect(),
        total_bytes: summary.total_bytes,
        file: destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    })
}

/// Checks a container completely and reports what it holds, writing nothing.
///
/// This is what the interface shows before a restore is confirmed: version,
/// date, components, sizes, and content-free warnings. A wrong password, a
/// damaged byte, or a tampered manifest is refused here, before anything on this
/// machine is touched.
pub fn inspect(
    path: &Path,
    password: &[u8],
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<BackupPreview, BackupError> {
    let summary = container::verify(path, password, cancel, limits)?;
    let (header, _) = container::read_header(path, limits)?;
    let mut preview = header.preview();
    preview.total_bytes = summary.total_bytes;
    Ok(preview)
}

/// What diagnostics may say about the backup feature, and nothing more.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct BackupStatus {
    /// Whether the feature can run: a key envelope exists to protect a container.
    pub available: bool,
    pub format_version: u16,
    /// Whether a portable key envelope is present.
    pub key_envelope_present: bool,
    /// Whether a restore was interrupted and has to be finished or undone.
    pub interrupted_restore: Option<String>,
    /// Whether the state a previous restore replaced is still on disk.
    pub previous_state_present: bool,
    /// The newest safety backup, by file name only.
    pub newest_safety_backup: Option<String>,
    /// The last operation this process ran, and its outcome code.
    pub last_operation: Option<String>,
    pub last_error_code: Option<String>,
}

impl BackupStatus {
    /// Reads the status from disk. Nothing decrypted, nothing named.
    pub fn read(roots: &BackupRoots) -> Self {
        let interrupted = RestoreJournal::read(roots).and_then(|journal| match journal.stage {
            RestoreStage::Committed => None,
            stage => Some(stage_name(stage).to_string()),
        });
        let newest = std::fs::read_dir(roots.data_dir.join(SAFETY_DIR))
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".jarvisbak"))
            .max();
        Self {
            available: roots.data_dir.join(KEY_ENVELOPE_FILE).is_file(),
            format_version: FORMAT_VERSION,
            key_envelope_present: roots.data_dir.join(KEY_ENVELOPE_FILE).is_file(),
            interrupted_restore: interrupted,
            previous_state_present: roots.data_dir.join(PREVIOUS_DIR).is_dir(),
            newest_safety_backup: newest,
            last_operation: None,
            last_error_code: None,
        }
    }

    /// The same status with the last operation of this process attached.
    pub fn with_last(mut self, operation: Option<&str>, code: Option<&str>) -> Self {
        self.last_operation = operation.map(str::to_string);
        self.last_error_code = code.map(str::to_string);
        self
    }
}

/// The stable name of a restore stage.
pub fn stage_name(stage: RestoreStage) -> &'static str {
    match stage {
        RestoreStage::Prepared => "prepared",
        RestoreStage::OldMoved => "old_moved",
        RestoreStage::NewInstalled => "new_installed",
        RestoreStage::Verified => "verified",
        RestoreStage::Committed => "committed",
    }
}

/// The staging directory of an export, for a caller that has to clean up.
pub fn export_staging(roots: &BackupRoots) -> PathBuf {
    roots.data_dir.join("export-staging")
}
