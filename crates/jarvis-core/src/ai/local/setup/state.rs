//! What survives a restart.
//!
//! Two different things are needed for the setup page to be honest after the
//! application is closed and reopened:
//!
//! * a small record of the last run, so a failure can be explained again instead
//!   of silently disappearing;
//! * a scan of the disk, so a `.part` file, a validated staging directory, and an
//!   installed version are all recognized for what they are.
//!
//! The scan never hashes a model: reading five gigabytes to draw a status line
//! would make the page unusable. It checks the size and the file header, which is
//! cheap, and the explicit
//! [`validate_existing`](coordinator::SetupCoordinator::validate_existing) command
//! does the full check when the user asks for it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::super::managed::{managed_model_manifest, managed_runtime_manifest, validate_pe_x64};
use super::super::model::read_gguf_info;
use super::layout::{directory_size, is_owned_temp, ManagedRoots};
use super::{SetupComponent, SetupErrorCode, SetupStage};

/// File name of the record that describes the last run.
pub const SETUP_STATE_FILE: &str = "setup-state.json";

/// The record of the last run, written after every stage transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersistedSetup {
    /// Stage code from [`SetupStage::code`].
    pub stage: String,
    /// Error code from [`SetupErrorCode::code`], when the run failed.
    pub error_code: Option<String>,
    pub updated_at: String,
    pub runtime_version: String,
    pub model_revision: String,
    /// Whether the run reached `complete`.
    pub finished: bool,
}

impl PersistedSetup {
    /// A record for a stage that has just started.
    pub fn at_stage(stage: SetupStage) -> Self {
        let runtime = managed_runtime_manifest();
        let model = managed_model_manifest();
        Self {
            stage: stage.code().to_string(),
            error_code: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
            runtime_version: runtime.version.to_string(),
            model_revision: model.source_revision.to_string(),
            finished: false,
        }
    }

    /// The stage this record describes, when it is a stage this build knows.
    pub fn stage(&self) -> SetupStage {
        SetupStage::all()
            .into_iter()
            .find(|stage| stage.code() == self.stage)
            .unwrap_or(SetupStage::Idle)
    }

    /// The error this record describes, when it is one this build knows.
    pub fn error(&self) -> Option<SetupErrorCode> {
        let code = self.error_code.as_deref()?;
        ALL_ERROR_CODES
            .into_iter()
            .find(|candidate| candidate.code() == code)
    }
}

/// Every error code, so a stored string can be turned back into a typed value
/// without a hand-written match that could drift.
const ALL_ERROR_CODES: [SetupErrorCode; 31] = [
    SetupErrorCode::InsufficientSpace,
    SetupErrorCode::Network,
    SetupErrorCode::Timeout,
    SetupErrorCode::NoProgress,
    SetupErrorCode::HashMismatch,
    SetupErrorCode::SizeMismatch,
    SetupErrorCode::ContentLengthMismatch,
    SetupErrorCode::RangeMismatch,
    SetupErrorCode::IdentityChanged,
    SetupErrorCode::TooLarge,
    SetupErrorCode::RefusedUrl,
    SetupErrorCode::ArchiveInvalid,
    SetupErrorCode::ArchiveUnexpectedFile,
    SetupErrorCode::ArchivePathTraversal,
    SetupErrorCode::RuntimeMissing,
    SetupErrorCode::RuntimeArchitectureMismatch,
    SetupErrorCode::ModelInvalid,
    SetupErrorCode::ModelArchitectureMismatch,
    SetupErrorCode::ModelQuantizationMismatch,
    SetupErrorCode::NotSameVolume,
    SetupErrorCode::NotOwned,
    SetupErrorCode::DestinationExists,
    SetupErrorCode::Cancelled,
    SetupErrorCode::AlreadyRunning,
    SetupErrorCode::NotRunning,
    SetupErrorCode::Io,
    SetupErrorCode::Interrupted,
    SetupErrorCode::TestFailed,
    SetupErrorCode::TestTimedOut,
    SetupErrorCode::ProcessUnavailable,
    SetupErrorCode::SettingsRefused,
];

/// Path of the run record inside the application data root.
pub fn setup_state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SETUP_STATE_FILE)
}

/// Reads the run record, when there is a readable one.
pub fn read_persisted(data_dir: &Path) -> Option<PersistedSetup> {
    let text = std::fs::read_to_string(setup_state_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes the run record atomically.
pub fn write_persisted(data_dir: &Path, state: &PersistedSetup) -> Result<(), SetupErrorCode> {
    crate::fsutil::write_json_atomic(&setup_state_path(data_dir), state)
        .map_err(|_| SetupErrorCode::Io)
}

/// Removes the run record, so the setup page starts from a clean slate.
pub fn clear_persisted(data_dir: &Path) {
    let _ = std::fs::remove_file(setup_state_path(data_dir));
}

/// What is on the disk right now, as far as a cheap check can tell.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InstalledState {
    /// `llama-server.exe` is present, is the pinned size class, and has an
    /// x86-64 PE header.
    pub runtime_ready: bool,
    pub runtime_bytes: u64,
    /// The runtime directory exists but did not pass the check above.
    pub runtime_damaged: bool,
    /// A downloaded runtime archive is still in the temporary tree.
    pub runtime_archive_bytes: u64,
    /// A runtime payload directory that was extracted and verified.
    pub runtime_staging_ready: bool,

    /// The pinned model file is present with the pinned size and a GGUF header.
    pub model_ready: bool,
    pub model_bytes: u64,
    pub model_damaged: bool,
    /// A partially downloaded model.
    pub model_part_bytes: u64,
    /// A model payload that has already been verified.
    pub model_staging_ready: bool,

    /// Bytes of a runtime kept for rollback under `setup-previous`.
    pub previous_runtime_bytes: u64,
    /// Bytes of a model kept for rollback under `setup-previous`.
    pub previous_model_bytes: u64,
    /// Temporary directories this application owns.
    pub owned_temp_directories: usize,
    /// Temporary directories in the tree that are not ours.
    pub foreign_temp_directories: usize,
}

impl InstalledState {
    /// Bytes that a partial download already put on the disk.
    pub fn partial_bytes(&self) -> u64 {
        self.runtime_archive_bytes
            .saturating_add(self.model_part_bytes)
    }

    /// Whether anything from a previous run is still waiting to be finished.
    pub fn has_work_in_progress(&self) -> bool {
        self.partial_bytes() > 0 || self.runtime_staging_ready || self.model_staging_ready
    }
}

/// Whether a file looks like the pinned model: right size, right GGUF header.
fn model_looks_installed(path: &Path, expected_size: u64) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() || metadata.len() != expected_size {
        return false;
    }
    read_gguf_info(path).is_ok()
}

/// Whether a directory holds a usable server: present and an x64 PE image.
fn runtime_looks_installed(directory: &Path) -> bool {
    let server = directory.join("llama-server.exe");
    if !server.is_file() {
        return false;
    }
    validate_pe_x64(&server).is_ok()
}

/// The file names a scan looks for.
///
/// In the application these come from the compiled manifests; a test supplies
/// its own, so the same scanning code is exercised with tiny fixtures.
#[derive(Clone, Copy, Debug)]
pub struct ScanNames<'a> {
    pub runtime_archive_filename: &'a str,
    pub model_filename: &'a str,
    pub model_expected_size: u64,
}

impl ScanNames<'_> {
    /// The names and sizes of the pinned artifacts.
    pub fn pinned() -> ScanNames<'static> {
        ScanNames {
            runtime_archive_filename: managed_runtime_manifest().artifact.filename,
            model_filename: managed_model_manifest().artifact.filename,
            model_expected_size: managed_model_manifest().artifact.expected_size,
        }
    }
}

/// Walks the managed tree and reports what is there, using the pinned names.
pub fn scan(roots: &ManagedRoots) -> InstalledState {
    scan_with(roots, &ScanNames::pinned())
}

/// [`scan`] with explicit names, so a test never writes five gigabytes.
pub fn scan_with(roots: &ManagedRoots, names: &ScanNames<'_>) -> InstalledState {
    let mut state = InstalledState::default();

    let runtime_dir = roots.runtime_dir();
    if runtime_dir.is_dir() {
        state.runtime_bytes = directory_size(&runtime_dir);
        state.runtime_ready = runtime_looks_installed(&runtime_dir);
        state.runtime_damaged = !state.runtime_ready;
    }
    // Bytes of the runtime archive that are already on the disk: the promoted
    // archive, the `.part` file, or both.
    let staging_runtime = roots.staging_dir(SetupComponent::Runtime.staging_name());
    let mut archive_bytes = 0_u64;
    for name in [
        names.runtime_archive_filename.to_string(),
        format!("{}.part", names.runtime_archive_filename),
    ] {
        if let Ok(metadata) = staging_runtime.join(name).metadata() {
            archive_bytes = archive_bytes.saturating_add(metadata.len());
        }
    }
    state.runtime_archive_bytes = archive_bytes;
    let runtime_payload = roots.payload_dir(SetupComponent::Runtime.staging_name());
    state.runtime_staging_ready = runtime_looks_installed(&runtime_payload);

    let model_payload = roots.payload_dir(SetupComponent::Model.staging_name());
    let model_dir = roots.model_dir();
    if model_dir.is_dir() {
        state.model_bytes = directory_size(&model_dir);
        state.model_ready = model_looks_installed(
            &roots.model_path_named(names.model_filename),
            names.model_expected_size,
        );
        state.model_damaged = !state.model_ready;
    }
    state.model_part_bytes = model_payload
        .join(format!("{}.part", names.model_filename))
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0);
    if state.model_part_bytes == 0 {
        // A partial model may also be sitting directly in the staging directory.
        state.model_part_bytes = roots
            .staging_dir(SetupComponent::Model.staging_name())
            .join(format!("{}.part", names.model_filename))
            .metadata()
            .map(|m| m.len())
            .unwrap_or(0);
    }
    state.model_staging_ready = model_looks_installed(
        &model_payload.join(names.model_filename),
        names.model_expected_size,
    );

    if let Ok(entries) = std::fs::read_dir(roots.temp_root()) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            if is_owned_temp(&entry.path()) {
                state.owned_temp_directories += 1;
            } else {
                state.foreign_temp_directories += 1;
            }
        }
    }

    let previous_root = roots.data_dir().join("setup-previous");
    if let Ok(entries) = std::fs::read_dir(&previous_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let bytes = directory_size(&entry.path());
            if name.starts_with("runtime") {
                state.previous_runtime_bytes = state.previous_runtime_bytes.saturating_add(bytes);
            } else if name.starts_with("model") {
                state.previous_model_bytes = state.previous_model_bytes.saturating_add(bytes);
            }
        }
    }

    // A runtime kept from a different version still occupies space, even though
    // the pinned version is not installed: `previous_runtime_bytes` above.
    state
}
/// What a restart found, in the terms the interface reports.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecoveryState {
    /// A previous run stopped before it reached a terminal stage.
    pub interrupted: bool,
    /// The stage that run was in.
    pub interrupted_stage: Option<SetupStage>,
    /// The recorded failure, so the page can explain it again.
    pub previous_error_code: Option<SetupErrorCode>,
    /// Bytes of a partial download that a retry can resume.
    pub partial_download_bytes: u64,
    /// A verified staging copy is waiting to be activated.
    pub staging_ready: bool,
    pub runtime_installed: bool,
    pub model_installed: bool,
}

impl RecoveryState {
    /// Whether the interface should offer "retry".
    pub fn can_resume(&self) -> bool {
        self.interrupted && (self.partial_download_bytes > 0 || self.staging_ready)
    }
}

/// Combines the run record with the disk scan.
pub fn recovery_state(data_dir: &Path, installed: &InstalledState) -> RecoveryState {
    let persisted = read_persisted(data_dir);
    let interrupted_stage = persisted
        .as_ref()
        .filter(|state| !state.finished)
        .map(|state| state.stage())
        .filter(|stage| !stage.is_terminal());
    RecoveryState {
        interrupted: interrupted_stage.is_some(),
        interrupted_stage,
        previous_error_code: persisted.as_ref().and_then(|state| state.error()),
        partial_download_bytes: installed.partial_bytes(),
        staging_ready: installed.runtime_staging_ready || installed.model_staging_ready,
        runtime_installed: installed.runtime_ready,
        model_installed: installed.model_ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_run_record_round_trips_and_an_unknown_stage_degrades_to_idle() {
        let directory = tempdir().unwrap();
        assert!(read_persisted(directory.path()).is_none());

        let mut record = PersistedSetup::at_stage(SetupStage::DownloadModel);
        record.error_code = Some(SetupErrorCode::Network.code().to_string());
        write_persisted(directory.path(), &record).unwrap();
        let loaded = read_persisted(directory.path()).unwrap();
        assert_eq!(loaded, record);
        assert_eq!(loaded.stage(), SetupStage::DownloadModel);
        assert_eq!(loaded.error(), Some(SetupErrorCode::Network));

        // A record from a future build must not crash or lie.
        let mut future = record.clone();
        future.stage = "from_the_future".to_string();
        future.error_code = Some("not_a_code".to_string());
        write_persisted(directory.path(), &future).unwrap();
        let loaded = read_persisted(directory.path()).unwrap();
        assert_eq!(loaded.stage(), SetupStage::Idle);
        assert_eq!(loaded.error(), None);

        clear_persisted(directory.path());
        assert!(read_persisted(directory.path()).is_none());
    }

    #[test]
    fn a_damaged_record_is_ignored_rather_than_fatal() {
        let directory = tempdir().unwrap();
        std::fs::write(setup_state_path(directory.path()), b"{ truncated").unwrap();
        assert!(read_persisted(directory.path()).is_none());
        let state = scan(&ManagedRoots::new(directory.path()));
        assert!(!state.has_work_in_progress());
    }

    #[test]
    fn an_empty_data_root_reports_nothing_installed() {
        let directory = tempdir().unwrap();
        let state = scan(&ManagedRoots::new(directory.path()));
        assert!(!state.runtime_ready);
        assert!(!state.model_ready);
        assert!(!state.has_work_in_progress());
        assert_eq!(state.partial_bytes(), 0);
        assert_eq!(state.previous_model_bytes, 0);
    }

    #[test]
    fn a_restart_recognises_a_part_file_a_staging_copy_and_an_installed_file() {
        // A small stand-in for the five gigabyte model, so the test stays fast
        // while exercising exactly the same checks.
        const FIXTURE: u64 = 8192;
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        let model = managed_model_manifest();

        // 1. a `.part` file from an interrupted download.
        let payload = roots.payload_dir("model");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(
            payload.join(format!("{}.part", model.artifact.filename)),
            vec![0_u8; 4096],
        )
        .unwrap();
        let state = scan_with(&roots, &fixture_names(FIXTURE));
        assert_eq!(state.model_part_bytes, 4096);
        assert!(state.has_work_in_progress());
        assert!(!state.model_ready);

        // 2. a validated staging copy: the right size with a GGUF header.
        let staged = payload.join(model.artifact.filename);
        std::fs::write(&staged, fixture_gguf(FIXTURE)).unwrap();
        let state = scan_with(&roots, &fixture_names(FIXTURE));
        assert!(state.model_staging_ready);

        // 3. the installed model.
        let installed_dir = roots.model_dir();
        std::fs::create_dir_all(&installed_dir).unwrap();
        std::fs::rename(&staged, roots.model_path()).unwrap();
        let state = scan_with(&roots, &fixture_names(FIXTURE));
        assert!(state.model_ready);
        assert!(!state.model_damaged);
        assert_eq!(state.model_bytes, FIXTURE);
        // Against the pinned size the same file is incomplete, which is exactly
        // the check the production scan makes.
        assert!(!scan(&roots).model_ready);
    }

    #[test]
    fn a_model_of_the_wrong_size_is_reported_as_damaged() {
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        std::fs::create_dir_all(roots.model_dir()).unwrap();
        std::fs::write(roots.model_path(), b"GGUF").unwrap();
        let state = scan(&roots);
        assert!(!state.model_ready);
        assert!(state.model_damaged);
    }

    #[test]
    fn a_runtime_without_a_server_or_with_a_wrong_machine_is_damaged() {
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        std::fs::create_dir_all(roots.runtime_dir()).unwrap();
        assert!(scan(&roots).runtime_damaged);

        // A PE image for a 32-bit machine must not be accepted.
        std::fs::write(
            roots.runtime_server_path(),
            pe_stub(0x014c),
        )
        .unwrap();
        let state = scan(&roots);
        assert!(state.runtime_damaged);
        assert!(!state.runtime_ready);

        std::fs::write(roots.runtime_server_path(), pe_stub(0x8664)).unwrap();
        let state = scan(&roots);
        assert!(state.runtime_ready);
        assert!(!state.runtime_damaged);
    }

    #[test]
    fn owned_and_foreign_temporary_directories_are_counted_separately() {
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        super::super::layout::mark_temp_directory(&roots.staging_dir("model"), "model").unwrap();
        std::fs::create_dir_all(roots.staging_dir("someone-elses")).unwrap();
        let state = scan(&roots);
        assert_eq!(state.owned_temp_directories, 1);
        assert_eq!(state.foreign_temp_directories, 1);
    }

    #[test]
    fn recovery_combines_the_record_with_the_scan() {
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        let installed = scan(&roots);
        // Nothing recorded, nothing on disk.
        let state = recovery_state(directory.path(), &installed);
        assert!(!state.interrupted);
        assert!(!state.can_resume());

        // An unfinished run with a partial file is resumable.
        write_persisted(
            directory.path(),
            &PersistedSetup::at_stage(SetupStage::DownloadModel),
        )
        .unwrap();
        let mut with_part = installed.clone();
        with_part.model_part_bytes = 1024;
        let state = recovery_state(directory.path(), &with_part);
        assert!(state.interrupted);
        assert_eq!(state.interrupted_stage, Some(SetupStage::DownloadModel));
        assert_eq!(state.partial_download_bytes, 1024);
        assert!(state.can_resume());

        // A finished run is not an interruption.
        let mut finished = PersistedSetup::at_stage(SetupStage::Complete);
        finished.finished = true;
        write_persisted(directory.path(), &finished).unwrap();
        let state = recovery_state(directory.path(), &installed);
        assert!(!state.interrupted);
        assert!(!state.can_resume());
    }

    /// A GGUF file of exactly `size` bytes with the pinned metadata.
    fn fixture_gguf(size: u64) -> Vec<u8> {
        super::super::fixtures::pinned_gguf(size)
    }

    /// A minimal PE image with the given machine tag.
    fn pe_stub(machine: u16) -> Vec<u8> {
        super::super::fixtures::pe_stub(machine)
    }

    /// The pinned file names with a small stand-in size for the model.
    fn fixture_names(model_expected_size: u64) -> ScanNames<'static> {
        ScanNames {
            model_expected_size,
            ..ScanNames::pinned()
        }
    }
}

