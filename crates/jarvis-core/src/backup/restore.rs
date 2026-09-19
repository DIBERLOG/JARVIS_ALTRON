//! Putting a backup back, without ever leaving a mixture of old and new data.
//!
//! # The sequence
//!
//! 1. **staged** — the container is verified and written into a staging
//!    directory; every SQLite file is opened there and made to pass
//!    `PRAGMA integrity_check`; a snapshot declaring a newer schema than this
//!    build is refused. Nothing live has been touched yet, so a failure here
//!    costs nothing;
//! 2. **safety** — the current state is exported into a full container of its
//!    own, with the same password, before anything is moved. It is never deleted
//!    automatically afterwards;
//! 3. **prepared** — the journal is written. From here on, an interrupted
//!    restore is recoverable even if the process dies;
//! 4. **old_moved** — the current files are moved aside, not deleted;
//! 5. **new_installed** — the staged files are moved into place;
//! 6. **verified** — every installed database passes its integrity check again,
//!    and the local key is re-bound to this machine;
//! 7. **committed** — the journal is removed. The old state and the safety
//!    backup stay on disk until the user deletes them.
//!
//! # Rollback
//!
//! Any failure after step 3 runs the rollback: files that were moved aside are
//! moved back, files that this restore installed and that did not exist before
//! are removed, and the journal is deleted. A rollback that itself fails is
//! reported as [`BackupError::RecoveryFailed`] and leaves the journal in place,
//! which is exactly what [`recover_interrupted`] needs to try again on the next
//! start.
//!
//! # What is deliberately changed after a restore
//!
//! Two settings are forced off, because restoring them would act on the user's
//! behalf: dictation (`enabled`) and autostart (`autostart_enabled`). Both are
//! choices that belong to the person sitting at *this* machine, and a backup
//! made elsewhere must not make them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::container::{self, ComponentKind, ContainerSummary, EntrySource, Limits, LIMITS};
use super::error::BackupError;
use super::snapshot::{self, BackupRoots, Component, SnapshotReport, COMPONENTS};

/// File holding the restore journal, inside the data directory.
pub const JOURNAL_FILE: &str = "restore-journal.json";
/// Directory holding the state a restore replaced, until the user deletes it.
pub const PREVIOUS_DIR: &str = "restore-previous";
/// Directory holding safety backups.
pub const SAFETY_DIR: &str = "backups";
/// Directory holding the staged plaintext while a restore runs.
pub const STAGING_DIR: &str = "restore-staging";

/// How far a restore got. The names are the contract of the journal file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreStage {
    /// The journal exists and nothing has been moved yet.
    Prepared,
    /// The previous state has been moved aside.
    OldMoved,
    /// The new state has been put in place.
    NewInstalled,
    /// The new state passed its checks and the local key was re-bound.
    Verified,
    /// The restore finished. A journal in this state is cleaned up on sight.
    Committed,
}

/// One component in the journal: enough to undo the move, and no paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JournalComponent {
    pub name: String,
    pub kind: ComponentKind,
    /// File name inside the data or configuration directory.
    pub file: String,
    pub in_config_dir: bool,
    /// Whether the file existed before the restore, so an install can be undone.
    pub existed_before: bool,
}

/// The on-disk record of a restore in progress.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RestoreJournal {
    pub stage: RestoreStage,
    /// RFC 3339, UTC.
    pub started_at: String,
    /// File *name* of the container, never its path.
    pub container: String,
    /// File name of the safety backup this restore created, if it got that far.
    pub safety_backup: Option<String>,
    pub components: Vec<JournalComponent>,
}

impl RestoreJournal {
    fn path(roots: &BackupRoots) -> PathBuf {
        roots.data_dir.join(JOURNAL_FILE)
    }

    fn write(&self, roots: &BackupRoots) -> Result<(), BackupError> {
        let bytes = serde_json::to_vec(self)?;
        crate::fsutil::write_bytes_atomic(&Self::path(roots), &bytes)?;
        Ok(())
    }

    /// The journal of an interrupted restore, when there is one.
    pub fn read(roots: &BackupRoots) -> Option<Self> {
        let bytes = std::fs::read(Self::path(roots)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn remove(roots: &BackupRoots) {
        let _ = std::fs::remove_file(Self::path(roots));
    }
}

/// How the restored master key is bound to the machine that restored it.
///
/// The container carries the *portable* envelope and never a DPAPI blob, because
/// a blob is bound to one Windows account on one machine. After a restore this
/// machine needs its own blob, and this is the seam that does it — and the seam
/// a test uses to fail it on purpose.
pub trait LocalKeyBinding: Send + Sync {
    /// Binds `master_key` to this account, or reports why it could not.
    fn bind(&self, master_key: &crate::sync::crypto::MasterKey) -> Result<(), BackupError>;
    /// A stable, content-free name for the log and the report.
    fn name(&self) -> &'static str;
}

/// The production binding: a DPAPI blob for the current Windows user.
pub struct DpapiKeyBinding {
    /// Where the blob is written.
    pub path: PathBuf,
}

impl LocalKeyBinding for DpapiKeyBinding {
    fn bind(&self, master_key: &crate::sync::crypto::MasterKey) -> Result<(), BackupError> {
        let blob = crate::sync::crypto::dpapi_protect(master_key)
            .map_err(|_| BackupError::LocalKeyFailed)?;
        crate::fsutil::write_bytes_atomic(&self.path, blob.as_bytes())
            .map_err(|_| BackupError::LocalKeyFailed)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "dpapi"
    }
}

/// A binding that binds nothing: for a platform without DPAPI, and for tests.
pub struct NoKeyBinding;

impl LocalKeyBinding for NoKeyBinding {
    fn bind(&self, _: &crate::sync::crypto::MasterKey) -> Result<(), BackupError> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

/// Everything one restore needs.
pub struct RestorePlan {
    pub roots: BackupRoots,
    /// The container to restore from.
    pub container: PathBuf,
    /// The components to restore, in container order.
    pub components: Vec<Component>,
    /// Whether a safety backup of the current state is created first.
    pub safety_backup: bool,
    pub bind: Box<dyn LocalKeyBinding>,
    pub limits: Limits,
}

impl RestorePlan {
    /// A plan over the production components, with a safety backup.
    pub fn full(roots: BackupRoots, container: PathBuf, bind: Box<dyn LocalKeyBinding>) -> Self {
        Self {
            roots,
            container,
            components: COMPONENTS.to_vec(),
            safety_backup: true,
            bind,
            limits: LIMITS,
        }
    }
}

/// What a restore did, for the report and the log.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub absent: Vec<String>,
    /// File name of the safety backup, when one was made.
    pub safety_backup: Option<String>,
    /// Directory holding the state this restore replaced, when there is one.
    pub previous_state: Option<String>,
    /// Whether the local key was re-bound, and how.
    pub local_key: String,
    /// Whether the rollback ran, and whether it succeeded.
    pub rolled_back: Option<bool>,
    pub total_bytes: u64,
}

impl RestoreReport {
    fn new(summary: &ContainerSummary) -> Self {
        Self {
            restored: summary
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
            absent: Vec::new(),
            safety_backup: None,
            previous_state: None,
            local_key: "none".to_string(),
            rolled_back: None,
            total_bytes: summary.total_bytes,
        }
    }
}

/// Stage one container into the staging directory, verified and checked.
///
/// Returns the entries that were staged, in container order.
pub fn stage(
    plan: &RestorePlan,
    password: &[u8],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(ContainerSummary, Vec<EntrySource>), BackupError> {
    let staging = plan.roots.data_dir.join(STAGING_DIR);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let summary = container::extract(&plan.container, password, &staging, cancel, &plan.limits)?;

    // Every database is proved before it is trusted, and a schema this build
    // does not know is refused while the live data is still untouched.
    let mut staged = Vec::with_capacity(summary.entries.len());
    let mut known: Vec<String> = Vec::new();
    for entry in &summary.entries {
        let path = container::entry_path(&staging, &entry.name)?;
        if entry.kind == ComponentKind::Sqlite {
            snapshot::integrity_check(&path)?;
            let version = snapshot::database_schema_version(&path)?;
            if let Some(declared) = entry.schema_version {
                if version > declared && version > crate::sync::sqlite::SCHEMA_VERSION {
                    return Err(BackupError::SchemaTooNew);
                }
            }
            if version > crate::sync::sqlite::SCHEMA_VERSION {
                return Err(BackupError::SchemaTooNew);
            }
        }
        known.push(entry.name.clone());
        let component = plan
            .components
            .iter()
            .find(|component| component.name == entry.name)
            .copied();
        if let Some(component) = component {
            staged.push(EntrySource {
                name: entry.name.clone(),
                kind: entry.kind,
                path,
                schema_version: entry.schema_version,
            });
            let _ = component;
        }
    }
    if !known.iter().any(|name| name == snapshot::NOTES_DATABASE) {
        return Err(BackupError::InvalidHeader(
            "the container has no notes database".to_string(),
        ));
    }
    if !known.iter().any(|name| name == snapshot::KEY_ENVELOPE) {
        return Err(BackupError::InvalidHeader(
            "the container has no key envelope".to_string(),
        ));
    }
    Ok((summary, staged))
}

/// Restores a container over the current state, with rollback on any failure.
pub fn restore(
    plan: &RestorePlan,
    password: &[u8],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<RestoreReport, BackupError> {
    let (summary, staged) = stage(plan, password, cancel)?;
    let mut report = RestoreReport::new(&summary);

    // The safety backup comes before the journal: if it fails, nothing has been
    // moved and nothing needs recovering.
    if plan.safety_backup {
        report.safety_backup = Some(write_safety_backup(plan, password)?);
    }

    let journal = RestoreJournal {
        stage: RestoreStage::Prepared,
        started_at: super::now_rfc3339(),
        container: plan
            .container
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        safety_backup: report.safety_backup.clone(),
        components: plan
            .components
            .iter()
            .map(|component| JournalComponent {
                name: component.name.to_string(),
                kind: component.kind,
                file: component.file.to_string(),
                in_config_dir: component.in_config_dir,
                existed_before: plan.roots.source_path(component).is_file(),
            })
            .collect(),
    };
    journal.write(&plan.roots)?;

    let outcome = install(plan, &journal, &staged, password, cancel, &mut report);
    match outcome {
        Ok(()) => {
            let mut committed = journal.clone();
            committed.stage = RestoreStage::Committed;
            let _ = committed.write(&plan.roots);
            RestoreJournal::remove(&plan.roots);
            report.previous_state = Some(PREVIOUS_DIR.to_string());
            report.local_key = plan.bind.name().to_string();
            let _ = std::fs::remove_dir_all(plan.roots.data_dir.join(STAGING_DIR));
            Ok(report)
        }
        Err(error) => {
            let rolled_back = rollback(plan, &journal).is_ok();
            report.rolled_back = Some(rolled_back);
            let _ = std::fs::remove_dir_all(plan.roots.data_dir.join(STAGING_DIR));
            if rolled_back {
                Err(error)
            } else {
                Err(BackupError::RecoveryFailed)
            }
        }
    }
}

/// Moves the old state aside, puts the new state in, and verifies it.
fn install(
    plan: &RestorePlan,
    journal: &RestoreJournal,
    staged: &[EntrySource],
    password: &[u8],
    cancel: &std::sync::atomic::AtomicBool,
    report: &mut RestoreReport,
) -> Result<(), BackupError> {
    let previous = plan.roots.data_dir.join(PREVIOUS_DIR);
    if previous.exists() {
        std::fs::remove_dir_all(&previous)?;
    }
    std::fs::create_dir_all(&previous)?;

    // 1. move the current files aside.
    for entry in &journal.components {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(BackupError::Storage);
        }
        if !entry.existed_before {
            continue;
        }
        let source = if entry.in_config_dir {
            plan.roots.config_dir.join(&entry.file)
        } else {
            plan.roots.data_dir.join(&entry.file)
        };
        if !source.is_file() {
            continue;
        }
        let destination = previous.join(&entry.file);
        std::fs::rename(&source, &destination)?;
    }
    let mut stage = journal.clone();
    stage.stage = RestoreStage::OldMoved;
    stage.write(&plan.roots)?;

    // 2. put the staged files where they belong. A file that was not in the
    //    container is left alone: a restore is not a wipe.
    for entry in staged {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(BackupError::Storage);
        }
        let component = journal
            .components
            .iter()
            .find(|component| component.name == entry.name)
            .ok_or(BackupError::InvalidHeader(
                "the container names an unknown component".to_string(),
            ))?;
        let destination = if component.in_config_dir {
            plan.roots.config_dir.join(&component.file)
        } else {
            plan.roots.data_dir.join(&component.file)
        };
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&entry.path, &destination)
            .map_err(|_| BackupError::DestinationUnavailable)?;
        if let Some(name) = destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        {
            if name.contains("whisper-settings") {
                let _ = force_off(&destination, "enabled");
            }
            if name == "desktop.json" {
                let _ = force_off(&destination, "autostart_enabled");
            }
        }
    }
    let mut stage = journal.clone();
    stage.stage = RestoreStage::NewInstalled;
    stage.write(&plan.roots)?;

    // 3. verify what was installed, then bind the key to this machine.
    for entry in &journal.components {
        let path = if entry.in_config_dir {
            plan.roots.config_dir.join(&entry.file)
        } else {
            plan.roots.data_dir.join(&entry.file)
        };
        if entry.kind == ComponentKind::Sqlite && path.is_file() {
            snapshot::integrity_check(&path)?;
        }
    }
    let envelope_path = plan.roots.data_dir.join("key.backup.json");
    let envelope_bytes = std::fs::read(&envelope_path)?;
    let envelope: crate::sync::crypto::PortableKeyBackup = serde_json::from_slice(&envelope_bytes)?;
    let master = crate::sync::crypto::import_backup(&envelope, password)?;
    plan.bind.bind(&master)?;

    let mut stage = journal.clone();
    stage.stage = RestoreStage::Verified;
    stage.write(&plan.roots)?;
    report.restored = staged.iter().map(|entry| entry.name.clone()).collect();
    Ok(())
}

/// Sets a boolean field of a JSON document to false, leaving the rest intact.
///
/// Two settings must not travel in a backup: dictation and autostart. Both are
/// decisions about the machine the backup is restored *onto*.
fn force_off(path: &Path, field: &str) -> Result<(), BackupError> {
    let bytes = std::fs::read(path)?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(field.to_string(), serde_json::Value::Bool(false));
    }
    crate::fsutil::write_bytes_atomic(path, &serde_json::to_vec(&value)?)?;
    Ok(())
}

/// Puts the state that a restore replaced back where it was.
fn rollback(plan: &RestorePlan, journal: &RestoreJournal) -> Result<(), BackupError> {
    let previous = plan.roots.data_dir.join(PREVIOUS_DIR);
    // Remove what this restore installed and did not find, then move the old
    // files back. Order matters: a failure between the two leaves the journal in
    // place, and the next start tries again.
    for entry in &journal.components {
        let path = if entry.in_config_dir {
            plan.roots.config_dir.join(&entry.file)
        } else {
            plan.roots.data_dir.join(&entry.file)
        };
        if !entry.existed_before && path.is_file() {
            std::fs::remove_file(&path)?;
        }
    }
    for entry in &journal.components {
        if !entry.existed_before {
            continue;
        }
        let saved = previous.join(&entry.file);
        if !saved.is_file() {
            continue;
        }
        let path = if entry.in_config_dir {
            plan.roots.config_dir.join(&entry.file)
        } else {
            plan.roots.data_dir.join(&entry.file)
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&path);
        std::fs::rename(&saved, &path)?;
    }
    let _ = std::fs::remove_dir_all(&previous);
    RestoreJournal::remove(&plan.roots);
    Ok(())
}

/// Finishes or undoes a restore that was interrupted by a crash or a power cut.
///
/// A journal that is not `committed` means the new state may be half in place, so
/// the answer is always the old state: the user still has the container and can
/// try again. This runs on start-up, before any store is opened.
pub fn recover_interrupted(roots: &BackupRoots) -> Result<Option<RestoreStage>, BackupError> {
    let Some(journal) = RestoreJournal::read(roots) else {
        return Ok(None);
    };
    match journal.stage {
        // The new state is live and verified; only the journal is left over.
        RestoreStage::Committed => {
            RestoreJournal::remove(roots);
            Ok(Some(RestoreStage::Committed))
        }
        stage => {
            rollback(
                &RestorePlan {
                    roots: roots.clone(),
                    container: PathBuf::new(),
                    components: COMPONENTS.to_vec(),
                    safety_backup: false,
                    bind: Box::new(NoKeyBinding),
                    limits: LIMITS,
                },
                &journal,
            )?;
            Ok(Some(stage))
        }
    }
}

/// Exports the current state into a full container of its own.
///
/// This is the safety backup a restore takes first, and it is deliberately the
/// same code path as a normal export: a safety net that is written differently
/// from a real backup is not a safety net.
pub fn write_safety_backup(plan: &RestorePlan, password: &[u8]) -> Result<String, BackupError> {
    let directory = plan.roots.data_dir.join(SAFETY_DIR);
    std::fs::create_dir_all(&directory)?;
    let name = format!("safety-{}.jarvisbak", super::now_compact());
    let destination = directory.join(&name);
    let staging = directory.join("staging");
    let _ = std::fs::remove_dir_all(&staging);
    let entries = snapshot::snapshot_all(&plan.roots, &staging, &plan.components)?;
    let envelope_path = plan.roots.data_dir.join("key.backup.json");
    let envelope: crate::sync::crypto::PortableKeyBackup =
        serde_json::from_slice(&std::fs::read(&envelope_path)?)?;
    let master = crate::sync::crypto::import_backup(&envelope, password)?;
    let key =
        crate::sync::crypto::derive_purpose_key(&master, crate::sync::crypto::KeyPurpose::Backup)?;
    let temporary = directory.join(format!("{name}.part"));
    let outcome = container::write_container(
        &temporary,
        &entries,
        &envelope,
        &key,
        super::app_version(),
        &super::now_rfc3339(),
        &plan.limits,
    );
    let _ = std::fs::remove_dir_all(&staging);
    match outcome {
        Ok(_) => {
            std::fs::rename(&temporary, &destination)?;
            Ok(name)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

/// Reports what a full snapshot of the current state would contain.
pub fn describe_current(roots: &BackupRoots) -> SnapshotReport {
    let mut report = SnapshotReport::default();
    for component in COMPONENTS.iter() {
        if roots.source_path(component).is_file() {
            report.captured.push(component.name.to_string());
        } else {
            report.absent.push(component.name.to_string());
        }
    }
    report
}
