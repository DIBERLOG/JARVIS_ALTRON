//! The allowed-application registry.
//!
//! Starting a program is the one action in this feature that reaches outside the process, so
//! it is the one that is most tightly bounded:
//!
//! * an entry is created **only** in the interface, from a file the user picked natively, and
//!   the arguments are whatever the user typed at that moment — they are stored, not
//!   supplied later;
//! * the path must be absolute, canonical, a regular file, with an `.exe` extension, not a
//!   network path, and not one of the interpreters that would turn "start a program" into
//!   "run anything" ([`super::policy::FORBIDDEN_EXECUTABLE_NAMES`]);
//! * a launch request carries only the entry identifier, so the model and the voice router
//!   cannot choose a path, an argument, or a working directory even if they wanted to;
//! * the file's size, modification time, and SHA-256 are recorded when the entry is created,
//!   and a file that changed since then is refused until the user looks at it again. A hash
//!   proves the file is the same bytes; it is not a signature, and nothing here claims it is.
//!
//! The registry is a local JSON file. It is not secret — it lists programs the user already
//! installed — so it is stored in the clear, and the interface shows exactly what it holds.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::error::ActionError;
use super::model::{ApplicationId, MAX_ALLOWED_APPLICATIONS};

/// File name of the registry inside the feature's directory.
pub const ALLOWED_APPLICATIONS_FILE: &str = "allowed-applications.json";
/// Schema version of the stored document.
pub const ALLOWED_APPLICATIONS_SCHEMA_VERSION: u32 = 1;
/// Longest display name.
pub const MAX_DISPLAY_NAME_CHARS: usize = 80;
/// Longest single fixed argument.
pub const MAX_ARGUMENT_CHARS: usize = 260;
/// Most fixed arguments one entry may carry.
pub const MAX_FIXED_ARGUMENTS: usize = 12;

/// A file the user allowed, with the identity it had when they allowed it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AllowedApplication {
    pub id: String,
    pub display_name: String,
    /// Canonical absolute path, produced by the filesystem, never by a caller.
    pub canonical_executable_path: String,
    /// Arguments the user fixed when the entry was created.
    pub fixed_arguments: Vec<String>,
    pub working_directory: Option<String>,
    /// Size in bytes when the entry was created.
    pub size_bytes: u64,
    /// Modification time in milliseconds since the Unix epoch.
    pub modified_unix_ms: u64,
    /// SHA-256 of the file when the entry was created, lowercase hex.
    pub sha256: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl AllowedApplication {
    pub fn application_id(&self) -> ApplicationId {
        ApplicationId::from_stored(self.id.clone()).expect("stored identifiers are validated")
    }

    /// The file name only, for display.
    pub fn executable_file_name(&self) -> String {
        Path::new(&self.canonical_executable_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.canonical_executable_path.clone())
    }
}

/// What the user typed when adding an entry.
#[derive(Clone, Debug, Deserialize)]
pub struct AllowedApplicationDraft {
    pub display_name: String,
    /// Path chosen through the native file picker.
    pub path: String,
    #[serde(default)]
    pub fixed_arguments: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
}

/// The result of comparing a stored identity with the file on disk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityCheck {
    /// The file is the same bytes.
    Unchanged,
    /// The file changed, or cannot be read any more.
    Changed { reason: &'static str },
}

impl IdentityCheck {
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

/// Everything needed to start a program, produced only from a stored entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchSpec {
    pub application_id: String,
    pub display_name: String,
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// How many arguments the user fixed, for the confirmation dialog.
    pub argument_count: usize,
}

/// The stored document.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct StoredRegistry {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    applications: Vec<AllowedApplication>,
}

/// The registry, loaded from one directory.
#[derive(Clone, Debug)]
pub struct AllowedApplications {
    path: PathBuf,
    applications: Vec<AllowedApplication>,
}

impl AllowedApplications {
    /// Opens the registry in `directory`, creating nothing until something is added.
    pub fn open(directory: &Path) -> Self {
        let path = directory.join(ALLOWED_APPLICATIONS_FILE);
        let applications = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<StoredRegistry>(&text).ok())
            .filter(|stored| stored.schema_version == ALLOWED_APPLICATIONS_SCHEMA_VERSION)
            .map(|stored| stored.applications)
            .unwrap_or_default();
        Self { path, applications }
    }

    /// The file the registry lives in.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every entry, enabled or not.
    pub fn list(&self) -> &[AllowedApplication] {
        &self.applications
    }

    /// The entries a launch may use.
    pub fn enabled(&self) -> Vec<&AllowedApplication> {
        self.applications
            .iter()
            .filter(|application| application.enabled)
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&AllowedApplication> {
        self.applications
            .iter()
            .find(|application| application.id == id)
    }

    /// Adds an entry for a file the user picked.
    ///
    /// Every check lives here: the module never stores a path it has not validated, so a
    /// launch can trust what it reads back.
    pub fn add(
        &mut self,
        draft: &AllowedApplicationDraft,
        now: impl Into<String>,
    ) -> Result<AllowedApplication, ActionError> {
        let now = now.into();
        if self.applications.len() >= MAX_ALLOWED_APPLICATIONS {
            return Err(ActionError::InvalidArguments {
                detail: format!("at most {MAX_ALLOWED_APPLICATIONS} applications"),
            });
        }
        let display_name = draft.display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > MAX_DISPLAY_NAME_CHARS {
            return Err(ActionError::InvalidArguments {
                detail: "the name is empty or too long".to_string(),
            });
        }
        let canonical = validate_executable(Path::new(draft.path.trim()))?;
        if self.applications.iter().any(|application| {
            application
                .canonical_executable_path
                .eq_ignore_ascii_case(&canonical.display)
        }) {
            return Err(ActionError::InvalidArguments {
                detail: "that file is already allowed".to_string(),
            });
        }
        let arguments = normalize_arguments(&draft.fixed_arguments)?;
        let working_directory = match draft.working_directory.as_deref() {
            Some(directory) if !directory.trim().is_empty() => {
                Some(validate_directory(Path::new(directory.trim()))?)
            }
            _ => None,
        };
        let identity = file_identity(&canonical.path)?;
        let application = AllowedApplication {
            id: mint_application_id()?,
            display_name: display_name.to_string(),
            canonical_executable_path: canonical.display,
            fixed_arguments: arguments,
            working_directory,
            size_bytes: identity.size_bytes,
            modified_unix_ms: identity.modified_unix_ms,
            sha256: identity.sha256,
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };
        self.applications.push(application.clone());
        self.save()?;
        Ok(application)
    }

    /// Removes an entry.
    pub fn remove(&mut self, id: &str) -> Result<AllowedApplication, ActionError> {
        let index = self
            .applications
            .iter()
            .position(|application| application.id == id)
            .ok_or_else(|| ActionError::ApplicationNotAllowed {
                application_id: id.to_string(),
            })?;
        let removed = self.applications.remove(index);
        self.save()?;
        Ok(removed)
    }

    /// Enables or disables an entry without forgetting it.
    pub fn set_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        now: impl Into<String>,
    ) -> Result<AllowedApplication, ActionError> {
        let index = self
            .applications
            .iter()
            .position(|application| application.id == id)
            .ok_or_else(|| ActionError::ApplicationNotAllowed {
                application_id: id.to_string(),
            })?;
        self.applications[index].enabled = enabled;
        self.applications[index].updated_at = now.into();
        let updated = self.applications[index].clone();
        self.save()?;
        Ok(updated)
    }

    /// Re-reads the file and accepts the new identity.
    ///
    /// This is the explicit "yes, I checked the new version" step after
    /// [`IdentityCheck::Changed`]; it never happens automatically.
    pub fn reaccept_identity(
        &mut self,
        id: &str,
        now: impl Into<String>,
    ) -> Result<AllowedApplication, ActionError> {
        let index = self
            .applications
            .iter()
            .position(|application| application.id == id)
            .ok_or_else(|| ActionError::ApplicationNotAllowed {
                application_id: id.to_string(),
            })?;
        let path = PathBuf::from(&self.applications[index].canonical_executable_path);
        let canonical = validate_executable(&path)?;
        let identity = file_identity(&canonical.path)?;
        self.applications[index].size_bytes = identity.size_bytes;
        self.applications[index].modified_unix_ms = identity.modified_unix_ms;
        self.applications[index].sha256 = identity.sha256;
        self.applications[index].updated_at = now.into();
        let updated = self.applications[index].clone();
        self.save()?;
        Ok(updated)
    }

    /// Compares the stored identity with what is on disk now.
    pub fn check_identity(&self, id: &str) -> Result<IdentityCheck, ActionError> {
        let application = self
            .get(id)
            .ok_or_else(|| ActionError::ApplicationNotAllowed {
                application_id: id.to_string(),
            })?;
        let path = PathBuf::from(&application.canonical_executable_path);
        let Ok(identity) = file_identity(&path) else {
            return Ok(IdentityCheck::Changed {
                reason: "unreadable",
            });
        };
        if identity.size_bytes != application.size_bytes {
            return Ok(IdentityCheck::Changed { reason: "size" });
        }
        if identity.modified_unix_ms != application.modified_unix_ms {
            return Ok(IdentityCheck::Changed { reason: "modified" });
        }
        if identity.sha256 != application.sha256 {
            return Ok(IdentityCheck::Changed { reason: "content" });
        }
        Ok(IdentityCheck::Unchanged)
    }

    /// Builds the launch specification for an entry, refusing anything that changed.
    pub fn launch_spec(&self, id: &str) -> Result<LaunchSpec, ActionError> {
        let application = self
            .get(id)
            .ok_or_else(|| ActionError::ApplicationNotAllowed {
                application_id: id.to_string(),
            })?;
        if !application.enabled {
            return Err(ActionError::ApplicationNotAllowed {
                application_id: id.to_string(),
            });
        }
        // The path is validated again at launch time: the registry is a file on disk, and a
        // file can be edited.
        let canonical = validate_executable(Path::new(&application.canonical_executable_path))?;
        if !self.check_identity(id)?.is_unchanged() {
            return Err(ActionError::ExecutableChanged {
                application_id: id.to_string(),
            });
        }
        Ok(LaunchSpec {
            application_id: application.id.clone(),
            display_name: application.display_name.clone(),
            program: canonical.path,
            arguments: application.fixed_arguments.clone(),
            working_directory: application.working_directory.as_deref().map(PathBuf::from),
            argument_count: application.fixed_arguments.len(),
        })
    }

    fn save(&self) -> Result<(), ActionError> {
        let stored = StoredRegistry {
            schema_version: ALLOWED_APPLICATIONS_SCHEMA_VERSION,
            applications: self.applications.clone(),
        };
        let text = serde_json::to_string_pretty(&stored)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::fsutil::write_bytes_atomic(&self.path, text.as_bytes())
            .map_err(|_| ActionError::StorageError)
    }
}

/// A validated executable: the canonical path and what to show for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalExecutable {
    pub path: PathBuf,
    pub display: String,
}

/// Validates a file the user picked as something that may be started.
pub fn validate_executable(path: &Path) -> Result<CanonicalExecutable, ActionError> {
    if path.as_os_str().is_empty() {
        return Err(ActionError::InvalidArguments {
            detail: "no file was chosen".to_string(),
        });
    }
    let raw = path.to_string_lossy();
    if raw.trim().is_empty() {
        return Err(ActionError::InvalidArguments {
            detail: "no file was chosen".to_string(),
        });
    }
    if !path.is_absolute() {
        return Err(ActionError::InvalidArguments {
            detail: "the path must be absolute".to_string(),
        });
    }
    // A network path is refused by default: it can change under the application, and it is
    // not something the user verified locally. The check is on what the user picked, before
    // the filesystem rewrites the path.
    if is_network_path(&raw) {
        return Err(ActionError::InvalidArguments {
            detail: "network paths are not allowed".to_string(),
        });
    }
    if is_device_path(&raw) {
        return Err(ActionError::InvalidArguments {
            detail: "that is not a program file on this computer".to_string(),
        });
    }
    if raw.contains('%') || raw.contains('$') {
        return Err(ActionError::InvalidArguments {
            detail: "environment variables are not expanded".to_string(),
        });
    }
    let metadata = std::fs::metadata(path).map_err(|_| ActionError::InvalidArguments {
        detail: "that file cannot be read".to_string(),
    })?;
    if metadata.is_dir() {
        return Err(ActionError::InvalidArguments {
            detail: "that is a folder, not a program".to_string(),
        });
    }
    if !metadata.is_file() {
        return Err(ActionError::InvalidArguments {
            detail: "that is not a regular file".to_string(),
        });
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if !file_name.ends_with(".exe") {
        return Err(ActionError::InvalidArguments {
            detail: "only .exe files can be started".to_string(),
        });
    }
    if !super::policy::ActionPolicy::default().executable_name_is_allowed(&file_name) {
        return Err(ActionError::ForbiddenAction {
            reason: "that program can run arbitrary commands".to_string(),
        });
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| ActionError::InvalidArguments {
        detail: "that path could not be resolved".to_string(),
    })?;
    // `canonicalize` answers with a verbatim path (`\\?\C:\...`) on Windows. That prefix is
    // the local namespace, not a network share, so it is removed for storage and display; a
    // verbatim *UNC* path stays a network path and is refused.
    let verbatim = canonical.to_string_lossy().into_owned();
    if verbatim.starts_with("\\\\?\\UNC\\") {
        return Err(ActionError::InvalidArguments {
            detail: "network paths are not allowed".to_string(),
        });
    }
    let display = verbatim
        .strip_prefix("\\\\?\\")
        .map(str::to_string)
        .unwrap_or(verbatim);
    Ok(CanonicalExecutable {
        display,
        path: canonical,
    })
}

/// Whether a path as the user wrote it names a network location.
fn is_network_path(raw: &str) -> bool {
    let trimmed = raw.trim();
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.starts_with("\\\\?\\unc\\") {
        return true;
    }
    if lowered.starts_with("\\\\?\\") {
        // The verbatim namespace: `\\?\C:\...` is the local drive namespace the filesystem
        // itself answers with, and something else in it is caught by [`is_device_path`].
        return false;
    }
    trimmed.starts_with("\\\\") || trimmed.starts_with("//")
}

/// Whether a path names something other than a file on a local drive.
///
/// The only verbatim paths this feature accepts are drive paths (`\\?\C:\...`); the rest of
/// that namespace (`\\?\GLOBALROOT\...`, for example) is a device path, not a file the user
/// picked in a file dialog.
fn is_device_path(raw: &str) -> bool {
    let lowered = raw.trim().to_ascii_lowercase();
    let Some(rest) = lowered.strip_prefix("\\\\?\\") else {
        return false;
    };
    let mut characters = rest.chars();
    let drive = characters.next();
    let colon = characters.next();
    !(drive
        .map(|character| character.is_ascii_alphabetic())
        .unwrap_or(false)
        && colon == Some(':'))
}

fn validate_directory(path: &Path) -> Result<String, ActionError> {
    if !path.is_absolute() {
        return Err(ActionError::InvalidArguments {
            detail: "the working directory must be absolute".to_string(),
        });
    }
    let raw = path.to_string_lossy();
    if is_network_path(&raw) {
        return Err(ActionError::InvalidArguments {
            detail: "network paths are not allowed".to_string(),
        });
    }
    if is_device_path(&raw) {
        return Err(ActionError::InvalidArguments {
            detail: "that working directory is not a folder on this computer".to_string(),
        });
    }
    let metadata = std::fs::metadata(path).map_err(|_| ActionError::InvalidArguments {
        detail: "that working directory cannot be read".to_string(),
    })?;
    if !metadata.is_dir() {
        return Err(ActionError::InvalidArguments {
            detail: "the working directory is not a folder".to_string(),
        });
    }
    Ok(path.to_string_lossy().into_owned())
}

fn normalize_arguments(arguments: &[String]) -> Result<Vec<String>, ActionError> {
    if arguments.len() > MAX_FIXED_ARGUMENTS {
        return Err(ActionError::InvalidArguments {
            detail: format!("at most {MAX_FIXED_ARGUMENTS} arguments"),
        });
    }
    let mut normalized = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let trimmed = argument.trim();
        if trimmed.is_empty() {
            return Err(ActionError::InvalidArguments {
                detail: "an argument is empty".to_string(),
            });
        }
        if trimmed.chars().count() > MAX_ARGUMENT_CHARS {
            return Err(ActionError::InvalidArguments {
                detail: "an argument is too long".to_string(),
            });
        }
        if trimmed.chars().any(char::is_control) {
            return Err(ActionError::InvalidArguments {
                detail: "an argument contains a control character".to_string(),
            });
        }
        normalized.push(trimmed.to_string());
    }
    Ok(normalized)
}

fn mint_application_id() -> Result<String, ActionError> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|_| ActionError::UnsupportedPlatform)?;
    let mut encoded = String::from("app_");
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    Ok(encoded)
}

/// Size, modification time, and SHA-256 of a file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    pub size_bytes: u64,
    pub modified_unix_ms: u64,
    pub sha256: String,
}

/// Reads a file's identity.
pub fn file_identity(path: &Path) -> Result<FileIdentity, ActionError> {
    let metadata = std::fs::metadata(path).map_err(|_| ActionError::InvalidArguments {
        detail: "that file cannot be read".to_string(),
    })?;
    let modified_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let bytes = std::fs::read(path).map_err(|_| ActionError::InvalidArguments {
        detail: "that file cannot be read".to_string(),
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let mut sha256 = String::with_capacity(64);
    for byte in hasher.finalize() {
        sha256.push_str(&format!("{byte:02x}"));
    }
    Ok(FileIdentity {
        size_bytes: metadata.len(),
        modified_unix_ms,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Writes a file that looks like a program, with the given bytes.
    fn fake_program(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn draft(path: &Path, name: &str) -> AllowedApplicationDraft {
        AllowedApplicationDraft {
            display_name: name.to_string(),
            path: path.to_string_lossy().into_owned(),
            fixed_arguments: Vec::new(),
            working_directory: None,
        }
    }

    #[test]
    fn a_picked_file_becomes_an_entry_with_an_identity() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Notepad.exe", b"FICTIONAL_BINARY");
        let mut registry = AllowedApplications::open(directory.path());
        let entry = registry.add(&draft(&program, "Notepad"), "now").unwrap();
        assert!(entry.id.starts_with("app_"));
        assert!(entry.enabled);
        assert_eq!(entry.fixed_arguments.len(), 0);
        assert_eq!(entry.sha256.len(), 64);
        assert!(entry.canonical_executable_path.ends_with("Notepad.exe"));
        assert_eq!(registry.list().len(), 1);
        assert!(registry.check_identity(&entry.id).unwrap().is_unchanged());

        // The registry survives a reopen.
        let reopened = AllowedApplications::open(directory.path());
        assert_eq!(reopened.list().len(), 1);
        assert_eq!(reopened.list()[0].id, entry.id);
    }

    #[test]
    fn only_the_identifier_selects_a_program_and_the_rest_comes_from_the_entry() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Player.exe", b"FICTIONAL_BINARY");
        let mut registry = AllowedApplications::open(directory.path());
        let entry = registry
            .add(
                &AllowedApplicationDraft {
                    display_name: "Player".to_string(),
                    path: program.to_string_lossy().into_owned(),
                    fixed_arguments: vec!["--safe".to_string(), "--volume=10".to_string()],
                    working_directory: Some(directory.path().to_string_lossy().into_owned()),
                },
                "now",
            )
            .unwrap();

        let spec = registry.launch_spec(&entry.id).unwrap();
        assert_eq!(spec.arguments, vec!["--safe", "--volume=10"]);
        assert_eq!(spec.argument_count, 2);
        assert!(spec.program.is_absolute());
        assert_eq!(spec.display_name, "Player");

        // An unknown identifier is refused, and so is a disabled entry.
        assert_eq!(
            registry.launch_spec("app_deadbeef").unwrap_err().code(),
            "application_not_allowed"
        );
        registry.set_enabled(&entry.id, false, "later").unwrap();
        assert_eq!(
            registry.launch_spec(&entry.id).unwrap_err().code(),
            "application_not_allowed"
        );
    }

    #[test]
    fn a_changed_file_is_refused_until_the_user_looks_again() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Tool.exe", b"FICTIONAL_FIRST");
        let mut registry = AllowedApplications::open(directory.path());
        let entry = registry.add(&draft(&program, "Tool"), "now").unwrap();
        assert!(registry.check_identity(&entry.id).unwrap().is_unchanged());

        std::fs::write(&program, b"FICTIONAL_SECOND_VERSION").unwrap();
        assert!(!registry.check_identity(&entry.id).unwrap().is_unchanged());
        assert_eq!(
            registry.launch_spec(&entry.id).unwrap_err().code(),
            "executable_changed"
        );

        // The explicit re-check accepts the new bytes.
        let updated = registry.reaccept_identity(&entry.id, "later").unwrap();
        assert_ne!(updated.sha256, entry.sha256);
        assert!(registry.launch_spec(&entry.id).is_ok());
    }

    #[test]
    fn a_missing_file_is_reported_not_silently_accepted() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Gone.exe", b"FICTIONAL");
        let mut registry = AllowedApplications::open(directory.path());
        let entry = registry.add(&draft(&program, "Gone"), "now").unwrap();
        std::fs::remove_file(&program).unwrap();
        assert_eq!(
            registry.check_identity(&entry.id).unwrap(),
            IdentityCheck::Changed {
                reason: "unreadable"
            }
        );
        assert_eq!(
            registry.launch_spec(&entry.id).unwrap_err().code(),
            "invalid_arguments"
        );
    }

    #[test]
    fn a_program_that_can_run_anything_is_refused() {
        let directory = tempdir().unwrap();
        let mut registry = AllowedApplications::open(directory.path());
        for name in ["cmd.exe", "PowerShell.exe", "wscript.exe", "mshta.exe"] {
            let program = fake_program(directory.path(), name, b"FICTIONAL");
            let error = registry.add(&draft(&program, name), "now").unwrap_err();
            assert_eq!(error.code(), "forbidden_action", "{name}");
        }
        assert!(registry.list().is_empty());
    }

    #[test]
    fn relative_network_directory_and_non_exe_paths_are_refused() {
        let directory = tempdir().unwrap();
        let mut registry = AllowedApplications::open(directory.path());
        let program = fake_program(directory.path(), "App.exe", b"FICTIONAL");
        let mut relative = draft(&program, "Relative");
        relative.path = "App.exe".to_string();
        assert!(registry.add(&relative, "now").is_err());

        let mut network = draft(&program, "Network");
        network.path = r"\\server\share\App.exe".to_string();
        assert!(registry.add(&network, "now").is_err());

        let mut folder = draft(&program, "Folder");
        folder.path = directory.path().to_string_lossy().into_owned();
        assert!(registry.add(&folder, "now").is_err());

        let script = fake_program(directory.path(), "script.bat", b"FICTIONAL");
        assert!(registry.add(&draft(&script, "Script"), "now").is_err());

        let mut variable = draft(&program, "Variable");
        variable.path = "%TEMP%\\App.exe".to_string();
        assert!(registry.add(&variable, "now").is_err());
        assert!(registry.list().is_empty());
    }

    #[test]
    fn a_verbatim_local_path_is_accepted_and_a_verbatim_unc_path_is_not() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Verbatim.exe", b"FICTIONAL");
        let canonical = std::fs::canonicalize(&program).unwrap();
        let verbatim = canonical.to_string_lossy().into_owned();
        // The path the filesystem answers with is accepted and stored without its
        // namespace prefix.
        let validated = validate_executable(Path::new(&verbatim)).unwrap();
        assert!(!validated.display.starts_with("\\\\?\\"));
        assert!(validated.path.is_absolute());
        // A verbatim network path is still a network path.
        assert!(is_network_path(r"\\?\UNC\server\share\App.exe"));
        assert!(is_network_path(r"\\server\share\App.exe"));
        assert!(!is_network_path(r"C:\Program Files\App.exe"));
        assert!(!is_network_path(r"\\?\C:\Program Files\App.exe"));
    }

    #[test]
    fn the_same_file_cannot_be_added_twice() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Once.exe", b"FICTIONAL");
        let mut registry = AllowedApplications::open(directory.path());
        registry.add(&draft(&program, "Once"), "now").unwrap();
        assert_eq!(
            registry
                .add(&draft(&program, "Again"), "now")
                .unwrap_err()
                .code(),
            "invalid_arguments"
        );
    }

    #[test]
    fn names_arguments_and_working_directories_are_bounded() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Bounded.exe", b"FICTIONAL");
        let mut registry = AllowedApplications::open(directory.path());

        let mut no_name = draft(&program, "   ");
        no_name.display_name = "   ".to_string();
        assert!(registry.add(&no_name, "now").is_err());

        let mut too_many = draft(&program, "Many");
        too_many.fixed_arguments = vec!["a".to_string(); MAX_FIXED_ARGUMENTS + 1];
        assert!(registry.add(&too_many, "now").is_err());

        let mut empty_argument = draft(&program, "Empty");
        empty_argument.fixed_arguments = vec!["  ".to_string()];
        assert!(registry.add(&empty_argument, "now").is_err());

        let mut long_argument = draft(&program, "Long");
        long_argument.fixed_arguments = vec!["a".repeat(MAX_ARGUMENT_CHARS + 1)];
        assert!(registry.add(&long_argument, "now").is_err());

        let mut bad_directory = draft(&program, "BadDir");
        bad_directory.working_directory = Some(program.to_string_lossy().into_owned());
        assert!(registry.add(&bad_directory, "now").is_err());

        assert!(registry.list().is_empty());
    }

    #[test]
    fn a_path_with_spaces_and_unicode_is_handled() {
        let directory = tempdir().unwrap();
        let folder = directory.path().join("мои программы");
        std::fs::create_dir_all(&folder).unwrap();
        let program = fake_program(&folder, "Калькулятор тест.exe", b"FICTIONAL");
        let mut registry = AllowedApplications::open(directory.path());
        let entry = registry
            .add(&draft(&program, "Калькулятор"), "now")
            .unwrap();
        assert!(entry.canonical_executable_path.contains("мои программы"));
        let spec = registry.launch_spec(&entry.id).unwrap();
        assert!(spec.program.exists());
        assert_eq!(
            spec.program.file_name().unwrap().to_string_lossy(),
            "Калькулятор тест.exe"
        );
    }

    #[test]
    fn remove_and_disable_keep_the_registry_honest() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Toggle.exe", b"FICTIONAL");
        let mut registry = AllowedApplications::open(directory.path());
        let entry = registry.add(&draft(&program, "Toggle"), "now").unwrap();
        assert_eq!(registry.enabled().len(), 1);
        registry.set_enabled(&entry.id, false, "later").unwrap();
        assert_eq!(registry.enabled().len(), 0);
        assert_eq!(registry.list().len(), 1);
        let removed = registry.remove(&entry.id).unwrap();
        assert_eq!(removed.id, entry.id);
        assert!(registry.list().is_empty());
        assert!(registry.remove(&entry.id).is_err());
    }

    #[test]
    fn the_stored_file_never_holds_anything_but_the_entry() {
        let directory = tempdir().unwrap();
        let program = fake_program(directory.path(), "Public.exe", b"FICTIONAL");
        let mut registry = AllowedApplications::open(directory.path());
        registry.add(&draft(&program, "Public"), "now").unwrap();
        let text = std::fs::read_to_string(registry.path()).unwrap();
        assert!(text.contains("schema_version"));
        assert!(text.contains("canonical_executable_path"));
        assert!(!text.contains("password"));
        assert!(!text.contains("token"));
    }
}
