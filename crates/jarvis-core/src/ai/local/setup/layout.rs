//! Real file operations for the managed installation, and the proof that they
//! never copy a full model twice.
//!
//! `plan::plan_space` reasons about the layout; this module performs it. Every
//! promotion here is a `rename`, which is what makes the plan's arithmetic true:
//!
//! ```text
//! setup-temp/<component>/<name>.part     the only full copy that is ever written
//! setup-temp/<component>/payload/<name>  rename
//! <managed directory>/<name>             rename of the payload directory
//! ```
//!
//! The module also owns the two safety rules that keep the installer from
//! touching anything it did not create:
//!
//! * a temporary directory is only removed when it carries the application's own
//!   marker file, so a directory the user (or another tool) put there is left
//!   alone;
//! * a previous version is only retained when it is really on the disk, and the
//!   retained directory is a rename, so keeping a rollback copy costs no space
//!   beyond the copy that is already installed.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::super::managed::{managed_model_manifest, managed_runtime_manifest};

/// Application identity written into every temporary directory this app owns.
pub const OWNER_APP: &str = "com.priler.jarvis";
/// Marker file that proves a temporary directory belongs to this application.
pub const TEMP_MARKER_FILE: &str = "jarvis-managed-temp.json";
/// Marker kind, so a marker for another purpose cannot be mistaken for this one.
pub const TEMP_MARKER_KIND: &str = "local-ai-setup";

/// Why a file operation could not be completed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// The destination already exists, so promoting over it is refused.
    DestinationExists,
    /// The source is missing or is not a directory.
    SourceMissing,
    /// The rename crossed a volume boundary and would silently become a copy.
    NotSameVolume,
    /// The directory is not one this application created.
    NotOwned,
    Io,
}

/// Proof that a temporary directory was created by this application.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TempMarker {
    pub app: String,
    pub kind: String,
    /// Which component the directory is staging, for diagnostics.
    pub purpose: String,
    pub created_at: String,
}

impl TempMarker {
    /// Builds the marker this application writes.
    pub fn new(purpose: &str) -> Self {
        Self {
            app: OWNER_APP.to_string(),
            kind: TEMP_MARKER_KIND.to_string(),
            purpose: purpose.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Whether the marker belongs to this application and this purpose.
    pub fn is_ours(&self) -> bool {
        self.app == OWNER_APP && self.kind == TEMP_MARKER_KIND
    }
}

/// The directories a managed installation uses, all under one data root.
///
/// Rooting every temporary path here, rather than in the system temporary
/// directory, is what makes the preferred route possible: the download, the
/// staging copy, and the installed copy are on one volume, so every promotion is
/// a rename instead of a second full copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedRoots {
    data_dir: PathBuf,
}

impl ManagedRoots {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Root of every temporary tree this application creates.
    pub fn temp_root(&self) -> PathBuf {
        self.data_dir.join("setup-temp")
    }

    /// Stable staging directory for one component.
    ///
    /// The name is deliberately not a session id: after a restart the same
    /// directory is found again, so a `.part` file can be resumed and a validated
    /// staging copy can still be activated.
    pub fn staging_dir(&self, component: &str) -> PathBuf {
        self.temp_root().join(component)
    }

    /// Directory that is renamed into the final location once it is verified.
    pub fn payload_dir(&self, component: &str) -> PathBuf {
        self.staging_dir(component).join("payload")
    }

    /// Directory that holds an installed component retained for rollback.
    pub fn previous_dir(&self, component: &str) -> PathBuf {
        self.data_dir
            .join("setup-previous")
            .join(format!("{component}-{}", chrono::Utc::now().timestamp()))
    }

    /// Parent of the pinned runtime versions.
    pub fn runtime_parent(&self) -> PathBuf {
        self.data_dir.join("runtime").join("llama.cpp")
    }

    /// Installed directory of the pinned runtime version.
    pub fn runtime_dir(&self) -> PathBuf {
        self.runtime_parent()
            .join(managed_runtime_manifest().version)
    }

    /// Parent of the managed models.
    pub fn model_parent(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    /// Installed directory of the pinned model.
    pub fn model_dir(&self) -> PathBuf {
        self.model_parent().join("qwen3-8b-q4_k_m")
    }

    /// Installed `llama-server.exe`, when the pinned runtime is present.
    pub fn runtime_server_path(&self) -> PathBuf {
        self.runtime_server_path_named("llama-server.exe")
    }

    /// Installed server executable with an explicit name.
    pub fn runtime_server_path_named(&self, server_name: &str) -> PathBuf {
        self.runtime_dir().join(server_name)
    }

    /// Installed model file, when the pinned model is present.
    pub fn model_path(&self) -> PathBuf {
        self.model_path_named(managed_model_manifest().artifact.filename)
    }

    /// Installed model file with an explicit name.
    pub fn model_path_named(&self, filename: &str) -> PathBuf {
        self.model_dir().join(filename)
    }

    /// Whether the managed installation can be completed without a copy, because
    /// the temporary root and the final directories share a volume.
    pub fn staging_is_on_the_same_volume(&self) -> bool {
        same_volume(&self.temp_root(), &self.data_dir)
    }
}

/// Whether two paths resolve to the same volume.
///
/// A rename across volumes silently becomes a copy, which would double the space
/// an installation needs, so the check is made before every promotion. On Windows
/// the first path component is the drive or UNC prefix; on Unix it is `/`, and
/// every path in one data root shares it.
pub fn same_volume(left: &Path, right: &Path) -> bool {
    fn prefix(path: &Path) -> Option<String> {
        let anchor = existing_ancestor(path);
        let mut components = anchor.components();
        match components.next() {
            Some(Component::Prefix(prefix)) => Some(prefix.as_os_str().to_string_lossy().to_lowercase()),
            Some(Component::RootDir) => Some("/".to_string()),
            _ => None,
        }
    }
    match (prefix(left), prefix(right)) {
        (Some(left), Some(right)) => left == right,
        // An unusual path shape is treated as "cannot prove it", which the caller
        // turns into a refusal rather than a silent copy.
        _ => false,
    }
}

/// Walks up until a path that exists is found, so the volume of a directory that
/// is about to be created can still be determined.
fn existing_ancestor(path: &Path) -> PathBuf {
    let mut candidate = path.to_path_buf();
    loop {
        if candidate.exists() {
            return candidate;
        }
        match candidate.parent() {
            Some(parent) if parent != candidate => candidate = parent.to_path_buf(),
            _ => return candidate,
        }
    }
}

/// Free space on the volume that holds `path`.
///
/// Returns `None` when the volume cannot be identified, which the caller reports
/// as a warning instead of pretending the disk is empty.
pub fn available_disk_bytes(path: &Path) -> Option<u64> {
    let probe = existing_ancestor(path);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut best: Option<(usize, u64)> = None;
    for disk in disks.list() {
        let mount = disk.mount_point();
        if probe.starts_with(mount) {
            let length = mount.as_os_str().len();
            if best.is_none_or(|(current, _)| length >= current) {
                best = Some((length, disk.available_space()));
            }
        }
    }
    best.map(|(_, free)| free)
}

/// Total size of every regular file under `directory`, recursively.
pub fn directory_size(directory: &Path) -> u64 {
    let mut total = 0_u64;
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path()));
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

/// Writes the ownership marker into a temporary directory.
pub fn mark_temp_directory(directory: &Path, purpose: &str) -> Result<(), LayoutError> {
    fs::create_dir_all(directory).map_err(|_| LayoutError::Io)?;
    crate::fsutil::write_json_atomic(&directory.join(TEMP_MARKER_FILE), &TempMarker::new(purpose))
        .map_err(|_| LayoutError::Io)
}

/// Reads the ownership marker of a directory, when it has one.
pub fn read_temp_marker(directory: &Path) -> Option<TempMarker> {
    let text = fs::read_to_string(directory.join(TEMP_MARKER_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether a directory carries this application's own marker.
pub fn is_owned_temp(directory: &Path) -> bool {
    read_temp_marker(directory).is_some_and(|marker| marker.is_ours())
}

/// Removes one temporary directory, but only when it is ours.
///
/// A directory without the marker is refused, so a foreign directory that happens
/// to sit in the tree is never deleted.
pub fn remove_owned_temp(directory: &Path) -> Result<u64, LayoutError> {
    if !directory.is_dir() {
        return Ok(0);
    }
    if !is_owned_temp(directory) {
        return Err(LayoutError::NotOwned);
    }
    let freed = directory_size(directory);
    fs::remove_dir_all(directory).map_err(|_| LayoutError::Io)?;
    Ok(freed)
}

/// Result of a cleanup pass over the temporary root.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupReport {
    pub removed_bytes: u64,
    pub removed_directories: usize,
    /// Directories that were left alone because they are not ours.
    pub skipped_foreign: usize,
}

/// Removes every temporary directory this application owns.
///
/// Foreign entries — a directory without the marker, or a loose file in the
/// temporary root — are counted and left exactly as they are.
pub fn cleanup_owned_temp(root: &Path) -> Result<CleanupReport, LayoutError> {
    let mut report = CleanupReport::default();
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(report);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            report.skipped_foreign += 1;
            continue;
        }
        match remove_owned_temp(&path) {
            Ok(bytes) => {
                report.removed_bytes = report.removed_bytes.saturating_add(bytes);
                report.removed_directories += 1;
            }
            Err(LayoutError::NotOwned) => report.skipped_foreign += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

/// Promotes a verified staging directory into its final location.
///
/// This is the only way a component becomes installed. It refuses to run when the
/// destination exists, so an existing installation is never overwritten by
/// accident, and it refuses a cross-volume rename, which would be a copy.
pub fn commit_staging_dir(staging: &Path, final_dir: &Path) -> Result<(), LayoutError> {
    if !staging.is_dir() {
        return Err(LayoutError::SourceMissing);
    }
    if final_dir.exists() {
        return Err(LayoutError::DestinationExists);
    }
    if let Some(parent) = final_dir.parent() {
        fs::create_dir_all(parent).map_err(|_| LayoutError::Io)?;
    }
    if !same_volume(staging, final_dir) {
        return Err(LayoutError::NotSameVolume);
    }
    fs::rename(staging, final_dir).map_err(|_| LayoutError::Io)
}

/// Retains an installed directory as the previous version.
///
/// A rename, so keeping a rollback copy costs no additional space: the bytes were
/// already installed and are only moved out of the way.
pub fn retain_previous(final_dir: &Path, previous_dir: &Path) -> Result<u64, LayoutError> {
    if !final_dir.is_dir() {
        return Err(LayoutError::SourceMissing);
    }
    if previous_dir.exists() {
        return Err(LayoutError::DestinationExists);
    }
    let bytes = directory_size(final_dir);
    if let Some(parent) = previous_dir.parent() {
        fs::create_dir_all(parent).map_err(|_| LayoutError::Io)?;
    }
    if !same_volume(final_dir, previous_dir) {
        return Err(LayoutError::NotSameVolume);
    }
    fs::rename(final_dir, previous_dir).map_err(|_| LayoutError::Io)?;
    Ok(bytes)
}

/// Renames a file inside its own directory, which is how a verified `.part`
/// becomes the staged file.
pub fn promote_file(source: &Path, destination: &Path) -> Result<(), LayoutError> {
    if !source.is_file() {
        return Err(LayoutError::SourceMissing);
    }
    if destination.exists() {
        return Err(LayoutError::DestinationExists);
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|_| LayoutError::Io)?;
    }
    if !same_volume(source, destination) {
        return Err(LayoutError::NotSameVolume);
    }
    fs::rename(source, destination).map_err(|_| LayoutError::Io)
}

/// Creates the directory a download writes into, with its ownership marker.
pub fn prepare_staging(roots: &ManagedRoots, component: &str) -> Result<PathBuf, LayoutError> {
    let directory = roots.staging_dir(component);
    mark_temp_directory(&directory, component)?;
    Ok(directory)
}

/// Removes a staging directory, refusing when it is not ours.
pub fn discard_staging(roots: &ManagedRoots, component: &str) -> Result<u64, LayoutError> {
    remove_owned_temp(&roots.staging_dir(component))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const MIB: u64 = 1024 * 1024;

    /// Writes a file of exactly `bytes` bytes, with a fill byte so the content is
    /// deterministic without being all zeroes.
    fn write_sized(path: &Path, bytes: u64, fill: u8) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let block = vec![fill; 64 * 1024];
        let mut file = fs::File::create(path).unwrap();
        let mut written = 0_u64;
        use std::io::Write;
        while written < bytes {
            let take = (bytes - written).min(block.len() as u64) as usize;
            file.write_all(&block[..take]).unwrap();
            written += take as u64;
        }
        file.sync_all().unwrap();
    }

    /// Highest total size of the managed tree while a layout runs.
    ///
    /// This is what makes the claim "no second full copy" checkable: the tree is
    /// measured after every operation instead of asserted from the source.
    struct LayoutRecorder {
        root: PathBuf,
        measurements: Vec<u64>,
    }

    impl LayoutRecorder {
        fn new(root: &Path) -> Self {
            Self {
                root: root.to_path_buf(),
                measurements: Vec::new(),
            }
        }

        fn measure(&mut self) -> u64 {
            let size = directory_size(&self.root);
            self.measurements.push(size);
            size
        }

        fn peak(&self) -> u64 {
            self.measurements.iter().copied().max().unwrap_or(0)
        }
    }

    #[test]
    fn the_marker_proves_ownership_and_is_required_for_removal() {
        let directory = tempdir().unwrap();
        let ours = directory.path().join("ours");
        mark_temp_directory(&ours, "model").unwrap();
        assert!(is_owned_temp(&ours));
        let marker = read_temp_marker(&ours).unwrap();
        assert_eq!(marker.app, OWNER_APP);
        assert_eq!(marker.kind, TEMP_MARKER_KIND);
        assert_eq!(marker.purpose, "model");

        let foreign = directory.path().join("foreign");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("user-file.txt"), b"not ours").unwrap();
        assert!(!is_owned_temp(&foreign));
        assert_eq!(remove_owned_temp(&foreign), Err(LayoutError::NotOwned));
        // The foreign file is still exactly where the user left it.
        assert!(foreign.join("user-file.txt").is_file());

        // A marker for a different application is not ours either.
        let other = directory.path().join("other");
        fs::create_dir_all(&other).unwrap();
        crate::fsutil::write_json_atomic(
            &other.join(TEMP_MARKER_FILE),
            &TempMarker {
                app: "com.example.other".to_string(),
                kind: TEMP_MARKER_KIND.to_string(),
                purpose: "model".to_string(),
                created_at: "now".to_string(),
            },
        )
        .unwrap();
        assert_eq!(remove_owned_temp(&other), Err(LayoutError::NotOwned));
    }

    #[test]
    fn cleanup_removes_only_owned_directories() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("setup-temp");
        let ours = root.join("model");
        let foreign = root.join("someones-folder");
        mark_temp_directory(&ours, "model").unwrap();
        write_sized(&ours.join("payload").join("part.bin"), 2 * MIB, 0x11);
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("keep.txt"), b"keep me").unwrap();
        fs::write(root.join("loose-file.txt"), b"keep me too").unwrap();

        let report = cleanup_owned_temp(&root).unwrap();
        assert_eq!(report.removed_directories, 1);
        // The payload plus the small ownership marker.
        assert!(report.removed_bytes >= 2 * MIB);
        assert!(report.removed_bytes < 2 * MIB + 4096);
        assert_eq!(report.skipped_foreign, 2);
        assert!(!ours.exists());
        assert!(foreign.join("keep.txt").is_file());
        assert!(root.join("loose-file.txt").is_file());
    }

    #[test]
    fn cleanup_of_a_missing_root_is_not_an_error() {
        let directory = tempdir().unwrap();
        let report = cleanup_owned_temp(&directory.path().join("absent")).unwrap();
        assert_eq!(report, CleanupReport::default());
    }

    #[test]
    fn a_clean_install_never_holds_two_models_on_the_disk() {
        const MODEL: u64 = 3 * MIB;
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        let mut recorder = LayoutRecorder::new(directory.path());

        // The temporary tree is on the same volume as the final directories,
        // which is the precondition for every rename below.
        fs::create_dir_all(roots.temp_root()).unwrap();
        fs::create_dir_all(roots.model_parent()).unwrap();
        assert!(roots.staging_is_on_the_same_volume());

        // 1. the download writes the only full copy that is ever created.
        let part = roots.payload_dir("model").join("model.gguf.part");
        write_sized(&part, MODEL, 0x5a);
        assert_eq!(recorder.measure(), MODEL);

        // 2. the verified `.part` becomes the staged file by a rename.
        let staged = roots.payload_dir("model").join("model.gguf");
        promote_file(&part, &staged).unwrap();
        assert_eq!(recorder.measure(), MODEL, "the rename must not copy");

        // 3. the verified payload directory becomes the installed model.
        let installed = roots.model_dir();
        commit_staging_dir(&roots.payload_dir("model"), &installed).unwrap();
        assert_eq!(recorder.measure(), MODEL, "the commit must not copy");
        assert_eq!(directory_size(&installed), MODEL);
        assert!(installed.join("model.gguf").is_file());

        // No measurement ever reached two copies.
        assert_eq!(recorder.peak(), MODEL);
        assert!(recorder.peak() < 2 * MODEL);
    }

    #[test]
    fn an_update_holds_two_copies_only_because_the_old_one_is_kept() {
        const MODEL: u64 = 3 * MIB;
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        let installed = roots.model_dir();
        write_sized(&installed.join("model.gguf"), MODEL, 0x01);
        let mut recorder = LayoutRecorder::new(directory.path());
        assert_eq!(recorder.measure(), MODEL);

        // The new file is downloaded while the old one is still installed.
        let part = roots.payload_dir("model").join("model.gguf.part");
        write_sized(&part, MODEL, 0x02);
        assert_eq!(recorder.measure(), 2 * MODEL);

        // Retaining the previous version is a rename: still two copies, and not
        // one byte more than the plan reserved for them.
        let previous = roots.previous_dir("model");
        let retained = retain_previous(&installed, &previous).unwrap();
        assert_eq!(retained, MODEL);
        assert_eq!(recorder.measure(), 2 * MODEL);
        assert_eq!(directory_size(&previous), MODEL);

        promote_file(&part, &roots.payload_dir("model").join("model.gguf")).unwrap();
        commit_staging_dir(&roots.payload_dir("model"), &installed).unwrap();
        assert_eq!(recorder.measure(), 2 * MODEL);
        assert_eq!(recorder.peak(), 2 * MODEL);
    }

    #[test]
    fn promotion_refuses_to_overwrite_and_refuses_a_missing_source() {
        let directory = tempdir().unwrap();
        let stub = directory.path().join("a");
        let target = directory.path().join("b");
        fs::create_dir_all(&stub).unwrap();
        fs::create_dir_all(&target).unwrap();
        assert_eq!(
            commit_staging_dir(&stub, &target),
            Err(LayoutError::DestinationExists)
        );
        assert_eq!(
            commit_staging_dir(&directory.path().join("absent"), &directory.path().join("c")),
            Err(LayoutError::SourceMissing)
        );
        assert_eq!(
            retain_previous(&directory.path().join("absent"), &directory.path().join("d")),
            Err(LayoutError::SourceMissing)
        );
    }

    #[test]
    fn paths_inside_one_data_root_are_reported_as_one_volume() {
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        assert!(same_volume(&roots.temp_root(), &roots.model_dir()));
        assert!(same_volume(&roots.model_dir(), &roots.runtime_dir()));
        // A path without any root component cannot be proven to be local.
        assert!(!same_volume(Path::new(""), Path::new("")));
    }

    #[test]
    fn the_installed_paths_follow_the_pinned_manifests() {
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        assert!(roots
            .runtime_dir()
            .ends_with(Path::new("runtime").join("llama.cpp").join("b10964")));
        assert!(roots.runtime_server_path().ends_with("llama-server.exe"));
        assert!(roots
            .model_path()
            .ends_with(managed_model_manifest().artifact.filename));
        assert!(roots
            .model_dir()
            .ends_with(Path::new("models").join("qwen3-8b-q4_k_m")));
    }

    #[test]
    fn the_disk_probe_answers_for_a_real_directory() {
        let directory = tempdir().unwrap();
        let free = available_disk_bytes(directory.path());
        assert!(free.is_some(), "a real directory must have a volume");
        assert!(free.unwrap() > 0);
        // A path that does not exist yet is resolved through its ancestors.
        assert!(available_disk_bytes(&directory.path().join("not").join("yet")).is_some());
    }

    #[test]
    fn prepare_staging_marks_the_directory_it_creates() {
        let directory = tempdir().unwrap();
        let roots = ManagedRoots::new(directory.path());
        let staging = prepare_staging(&roots, "model").unwrap();
        assert!(is_owned_temp(&staging));
        // A second call is idempotent and keeps the marker.
        prepare_staging(&roots, "model").unwrap();
        assert!(is_owned_temp(&staging));
        // Only the marker is there, so only the marker is freed.
        let freed = discard_staging(&roots, "model").unwrap();
        assert!(freed > 0 && freed < 4096);
        assert!(!staging.exists());
    }
}
