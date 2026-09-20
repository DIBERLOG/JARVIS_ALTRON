//! Backend orchestration for installing the managed local AI runtime and model.
//!
//! The interface asks; the backend installs. Everything that must not be chosen
//! by the frontend lives here:
//!
//! * the pinned URLs, sizes, and SHA-256 values, which come from
//!   [`managed`](super::managed) and are never sent to the interface;
//! * the destination paths, which are derived from the application data root;
//! * the launch command for the readiness test, which is built from separate
//!   arguments and never from a shell string;
//! * the stage sequence, which is a stable vocabulary of string codes.
//!
//! The frontend sends a request with no URL, no hash, no path, and no command —
//! see [`StartRequest`] and the DTO in the Tauri layer.
//!
//! ```text
//! preflight -> download runtime -> validate runtime -> extract runtime
//!           -> activate runtime -> download model -> validate model
//!           -> activate model -> configure -> launch test -> readiness
//!           -> test inference -> complete | cancelled | failed
//! ```
//!
//! Exactly one installation runs at a time. The state lives here, not in the
//! interface, so closing and reopening the setup page shows the same progress,
//! and a restart of the application recognizes a `.part` file, a validated
//! staging directory, and an installed version.

pub mod coordinator;
pub mod download;
pub mod firstrun;
pub mod layout;
pub mod plan;
pub mod state;

#[cfg(test)]
pub mod fake_http;
#[cfg(test)]
pub mod fixtures;

use serde::{Deserialize, Serialize};

pub use coordinator::{
    plan_for, source_label, CatalogSpec, ComponentStatus, ExistingValidation, OfferComponent,
    PinnedCatalog, SetupCatalog, SetupCoordinator, SetupEvent, SetupEventSink, SetupOffer,
    SetupStatus, INSTALL_ROOT_CODE, RUNTIME_IS_PRE_RELEASE,
};
pub use download::{
    download_artifact, identity_path, sha256_file, ArtifactExpectation, ArtifactTrust,
    DownloadError, DownloadOutcome, DownloadRequest, DownloadTransport, PartIdentity,
    ReqwestTransport, MAX_DOWNLOAD_ATTEMPTS,
};
pub use firstrun::{
    first_run_arguments, free_loopback_port, run_first_run, FirstRunOptions, FirstRunReport,
    TEST_CONTEXT_SIZE, TEST_GPU_LAYERS, TEST_HOST, TEST_MAX_TOKENS, TEST_PROMPT,
    TEST_READINESS_TIMEOUT,
};
pub use layout::{
    available_disk_bytes, cleanup_owned_temp, commit_staging_dir, directory_size, discard_staging,
    is_owned_temp, mark_temp_directory, prepare_staging, promote_file, read_temp_marker,
    remove_owned_temp, retain_previous, same_volume, CleanupReport, LayoutError, ManagedRoots,
    TempMarker, OWNER_APP, TEMP_MARKER_FILE,
};
pub use plan::{
    clean_install_plan, clean_install_required_bytes, plan_space, update_plan,
    update_required_bytes, ComponentPlan, ComponentPlanKind, ModelInstallKind, PlanInputs,
    SpacePlan, RUNTIME_PLAN_BYTES, SAFETY_RESERVE_BYTES,
};
pub use state::{
    clear_persisted, read_persisted, recovery_state, scan, scan_with, write_persisted,
    InstalledState, PersistedSetup, RecoveryState, ScanNames, SETUP_STATE_FILE,
};

/// The one model, its licence, and its pinned identity, as the interface may
/// describe them without receiving a URL.
pub const MODEL_LICENSE: &str = "Apache-2.0";

/// Stable stage codes. The interface switches on these strings, so they are part
/// of the wire contract and must not be renamed casually.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStage {
    /// Nothing has been started, or the previous run finished and was cleared.
    Idle,
    /// Space, directories, and existing state are checked.
    Preflight,
    DownloadRuntime,
    ValidateRuntime,
    ExtractRuntime,
    ActivateRuntime,
    DownloadModel,
    ValidateModel,
    ActivateModel,
    Configure,
    LaunchTest,
    Readiness,
    TestInference,
    Complete,
    Cancelled,
    Failed,
}

impl SetupStage {
    /// Every stage in the order the coordinator visits them.
    pub fn all() -> [Self; 16] {
        [
            Self::Idle,
            Self::Preflight,
            Self::DownloadRuntime,
            Self::ValidateRuntime,
            Self::ExtractRuntime,
            Self::ActivateRuntime,
            Self::DownloadModel,
            Self::ValidateModel,
            Self::ActivateModel,
            Self::Configure,
            Self::LaunchTest,
            Self::Readiness,
            Self::TestInference,
            Self::Complete,
            Self::Cancelled,
            Self::Failed,
        ]
    }

    /// The stable string the interface receives.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Preflight => "preflight",
            Self::DownloadRuntime => "download_runtime",
            Self::ValidateRuntime => "validate_runtime",
            Self::ExtractRuntime => "extract_runtime",
            Self::ActivateRuntime => "activate_runtime",
            Self::DownloadModel => "download_model",
            Self::ValidateModel => "validate_model",
            Self::ActivateModel => "activate_model",
            Self::Configure => "configure",
            Self::LaunchTest => "launch_test",
            Self::Readiness => "readiness",
            Self::TestInference => "test_inference",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// Whether the run has stopped for good.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Cancelled | Self::Failed)
    }

    /// Which component the stage is working on.
    pub fn component(&self) -> Option<SetupComponent> {
        match self {
            Self::DownloadRuntime
            | Self::ValidateRuntime
            | Self::ExtractRuntime
            | Self::ActivateRuntime => Some(SetupComponent::Runtime),
            Self::DownloadModel | Self::ValidateModel | Self::ActivateModel => {
                Some(SetupComponent::Model)
            }
            _ => None,
        }
    }

    /// The wizard step this stage belongs to.
    pub fn step(&self) -> WizardStep {
        match self {
            Self::Idle | Self::Preflight | Self::Failed | Self::Cancelled => WizardStep::Preflight,
            Self::DownloadRuntime
            | Self::ValidateRuntime
            | Self::ExtractRuntime
            | Self::ActivateRuntime => WizardStep::Runtime,
            Self::DownloadModel | Self::ValidateModel | Self::ActivateModel => WizardStep::Model,
            Self::Configure => WizardStep::Configure,
            Self::LaunchTest | Self::Readiness | Self::TestInference => WizardStep::LaunchTest,
            Self::Complete => WizardStep::Done,
        }
    }
}

/// The two components the installer manages.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupComponent {
    /// `llama-server.exe` and the libraries it needs.
    Runtime,
    /// The pinned Qwen3 GGUF.
    Model,
}

impl SetupComponent {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Model => "model",
        }
    }

    /// Directory name used inside the temporary root.
    pub fn staging_name(&self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Model => "model",
        }
    }
}

/// The six steps the setup page shows.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WizardStep {
    Preflight,
    Runtime,
    Model,
    Configure,
    LaunchTest,
    Done,
}

impl WizardStep {
    /// Every step, in order, so the interface does not hard-code the list.
    pub fn all() -> [Self; 6] {
        [
            Self::Preflight,
            Self::Runtime,
            Self::Model,
            Self::Configure,
            Self::LaunchTest,
            Self::Done,
        ]
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Runtime => "runtime",
            Self::Model => "model",
            Self::Configure => "configure",
            Self::LaunchTest => "launch_test",
            Self::Done => "done",
        }
    }

    /// Position in the sequence, starting at one.
    pub fn index(&self) -> u8 {
        match self {
            Self::Preflight => 1,
            Self::Runtime => 2,
            Self::Model => 3,
            Self::Configure => 4,
            Self::LaunchTest => 5,
            Self::Done => 6,
        }
    }
}

/// What a component's files on the disk look like right now.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    /// Nothing installed and nothing staged.
    Absent,
    /// A `.part` file, a staging directory, or both exist.
    Partial,
    /// Installed and usable at the pinned version.
    Ready,
    /// Installed but it no longer matches what was verified.
    Damaged,
    /// Being written by the run that is in progress.
    Updating,
}

impl ComponentState {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Partial => "partial",
            Self::Ready => "ready",
            Self::Damaged => "damaged",
            Self::Updating => "updating",
        }
    }
}

/// Where the paths the runtime is using came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveOrigin {
    /// The application installed them itself.
    Managed,
    /// The user chose them with the file picker.
    UserProvided,
    /// No server path or model path is configured.
    Unset,
    /// Both are configured and they differ in origin.
    Mixed,
}

impl ActiveOrigin {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::UserProvided => "user_provided",
            Self::Unset => "unset",
            Self::Mixed => "mixed",
        }
    }
}

/// Stable error codes the interface translates. Keys, not sentences, and never a
/// path, a URL, or a command.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupErrorCode {
    InsufficientSpace,
    Network,
    Timeout,
    NoProgress,
    HashMismatch,
    SizeMismatch,
    ContentLengthMismatch,
    RangeMismatch,
    IdentityChanged,
    TooLarge,
    RefusedUrl,
    ArchiveInvalid,
    ArchiveUnexpectedFile,
    ArchivePathTraversal,
    RuntimeMissing,
    RuntimeArchitectureMismatch,
    ModelInvalid,
    ModelArchitectureMismatch,
    ModelQuantizationMismatch,
    NotSameVolume,
    NotOwned,
    DestinationExists,
    Cancelled,
    AlreadyRunning,
    NotRunning,
    Io,
    Interrupted,
    TestFailed,
    TestTimedOut,
    ProcessUnavailable,
    SettingsRefused,
}

impl SetupErrorCode {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InsufficientSpace => "insufficient_space",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::NoProgress => "no_progress",
            Self::HashMismatch => "hash_mismatch",
            Self::SizeMismatch => "size_mismatch",
            Self::ContentLengthMismatch => "content_length_mismatch",
            Self::RangeMismatch => "range_mismatch",
            Self::IdentityChanged => "identity_changed",
            Self::TooLarge => "too_large",
            Self::RefusedUrl => "refused_url",
            Self::ArchiveInvalid => "archive_invalid",
            Self::ArchiveUnexpectedFile => "archive_unexpected_file",
            Self::ArchivePathTraversal => "archive_path_traversal",
            Self::RuntimeMissing => "runtime_missing",
            Self::RuntimeArchitectureMismatch => "runtime_architecture_mismatch",
            Self::ModelInvalid => "model_invalid",
            Self::ModelArchitectureMismatch => "model_architecture_mismatch",
            Self::ModelQuantizationMismatch => "model_quantization_mismatch",
            Self::NotSameVolume => "not_same_volume",
            Self::NotOwned => "not_owned",
            Self::DestinationExists => "destination_exists",
            Self::Cancelled => "cancelled",
            Self::AlreadyRunning => "already_running",
            Self::NotRunning => "not_running",
            Self::Io => "io",
            Self::Interrupted => "interrupted",
            Self::TestFailed => "test_failed",
            Self::TestTimedOut => "test_timed_out",
            Self::ProcessUnavailable => "process_unavailable",
            Self::SettingsRefused => "settings_refused",
        }
    }
}

/// Stable warning codes, for notices that do not stop the installation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupWarningCode {
    /// The runtime build is an official pre-release.
    RuntimePreRelease,
    /// The first download needs a connection; the local mode does not.
    InternetRequiredForDownload,
    /// A path the user chose by hand was left untouched.
    ManualSettingsPreserved,
    /// A partially downloaded file was found and is being resumed.
    DownloadResumed,
    /// A validated staging copy was found and is being activated.
    StagingReused,
    /// A previous version is being kept so it can be restored.
    PreviousVersionRetained,
    /// The runtime was already installed and was not downloaded again.
    RuntimeAlreadyInstalled,
    /// The model was already installed and was not downloaded again.
    ModelAlreadyInstalled,
    /// The free space is above the requirement but below twice it.
    SpaceIsTight,
    /// The volume could not be identified, so the free space is unknown.
    SpaceUnknown,
    /// The model remains the user's own file; it was not copied or moved.
    UserModelUntouched,
}

impl SetupWarningCode {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RuntimePreRelease => "runtime_pre_release",
            Self::InternetRequiredForDownload => "internet_required_for_download",
            Self::ManualSettingsPreserved => "manual_settings_preserved",
            Self::DownloadResumed => "download_resumed",
            Self::StagingReused => "staging_reused",
            Self::PreviousVersionRetained => "previous_version_retained",
            Self::RuntimeAlreadyInstalled => "runtime_already_installed",
            Self::ModelAlreadyInstalled => "model_already_installed",
            Self::SpaceIsTight => "space_is_tight",
            Self::SpaceUnknown => "space_unknown",
            Self::UserModelUntouched => "user_model_untouched",
        }
    }
}

/// The outcome of the technical first-run test.
///
/// The prompt and the answer are deliberately absent: the report says whether a
/// usable answer arrived, how long it took, and how many tokens it had.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TestOutcome {
    pub passed: bool,
    /// The server answered on `/health` inside the timeout.
    pub server_ready: bool,
    /// A non-empty answer arrived.
    pub answer_received: bool,
    pub elapsed_ms: u64,
    /// Number of answer tokens, when the server reported it.
    pub answer_tokens: Option<u32>,
}

/// A typed request from the interface.
///
/// There is nothing here that could redirect the installation: no URL, no hash,
/// no path, no command, no port, no argument.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct StartRequest {
    /// Whether the verified paths may replace the current configuration.
    ///
    /// Sent by the interface only after the user chose the automatic
    /// installation. Without it a manual configuration is left exactly as it is.
    pub consent_managed_paths: bool,
    /// Whether a failure should be retried from where it stopped.
    pub resume: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_codes_are_stable_and_serde_agrees_with_them() {
        for stage in SetupStage::all() {
            let json = serde_json::to_string(&stage).unwrap();
            assert_eq!(json, format!("\"{}\"", stage.code()));
        }
        // The exact strings the interface switches on.
        assert_eq!(SetupStage::Preflight.code(), "preflight");
        assert_eq!(SetupStage::DownloadRuntime.code(), "download_runtime");
        assert_eq!(SetupStage::ValidateRuntime.code(), "validate_runtime");
        assert_eq!(SetupStage::ExtractRuntime.code(), "extract_runtime");
        assert_eq!(SetupStage::ActivateRuntime.code(), "activate_runtime");
        assert_eq!(SetupStage::DownloadModel.code(), "download_model");
        assert_eq!(SetupStage::ValidateModel.code(), "validate_model");
        assert_eq!(SetupStage::ActivateModel.code(), "activate_model");
        assert_eq!(SetupStage::Configure.code(), "configure");
        assert_eq!(SetupStage::LaunchTest.code(), "launch_test");
        assert_eq!(SetupStage::Readiness.code(), "readiness");
        assert_eq!(SetupStage::TestInference.code(), "test_inference");
        assert_eq!(SetupStage::Complete.code(), "complete");
        assert_eq!(SetupStage::Cancelled.code(), "cancelled");
        assert_eq!(SetupStage::Failed.code(), "failed");
    }

    #[test]
    fn every_stage_belongs_to_a_wizard_step_and_the_steps_are_complete() {
        let steps: Vec<WizardStep> = SetupStage::all().iter().map(|stage| stage.step()).collect();
        for step in WizardStep::all() {
            assert!(
                steps.contains(&step),
                "no stage reaches the step {}",
                step.code()
            );
        }
        assert_eq!(WizardStep::all().len(), 6);
        assert_eq!(WizardStep::Preflight.index(), 1);
        assert_eq!(WizardStep::Done.index(), 6);
        for step in WizardStep::all() {
            let json = serde_json::to_string(&step).unwrap();
            assert_eq!(json, format!("\"{}\"", step.code()));
        }
    }

    #[test]
    fn the_ordered_stage_list_is_the_documented_sequence() {
        let codes: Vec<&str> = SetupStage::all().iter().map(|stage| stage.code()).collect();
        assert_eq!(
            codes,
            vec![
                "idle",
                "preflight",
                "download_runtime",
                "validate_runtime",
                "extract_runtime",
                "activate_runtime",
                "download_model",
                "validate_model",
                "activate_model",
                "configure",
                "launch_test",
                "readiness",
                "test_inference",
                "complete",
                "cancelled",
                "failed",
            ]
        );
        assert!(SetupStage::Complete.is_terminal());
        assert!(SetupStage::Cancelled.is_terminal());
        assert!(SetupStage::Failed.is_terminal());
        assert!(!SetupStage::Configure.is_terminal());
    }

    #[test]
    fn terminal_stages_are_never_mistaken_for_progress() {
        for stage in SetupStage::all() {
            if stage.is_terminal() {
                assert_eq!(stage.component(), None);
            }
        }
        assert_eq!(
            SetupStage::DownloadRuntime.component(),
            Some(SetupComponent::Runtime)
        );
        assert_eq!(
            SetupStage::ActivateModel.component(),
            Some(SetupComponent::Model)
        );
    }

    #[test]
    fn error_and_warning_codes_are_stable_strings() {
        assert_eq!(SetupErrorCode::InsufficientSpace.code(), "insufficient_space");
        assert_eq!(SetupErrorCode::HashMismatch.code(), "hash_mismatch");
        assert_eq!(SetupErrorCode::Cancelled.code(), "cancelled");
        assert_eq!(
            SetupWarningCode::RuntimePreRelease.code(),
            "runtime_pre_release"
        );
        assert_eq!(
            SetupWarningCode::ManualSettingsPreserved.code(),
            "manual_settings_preserved"
        );
        assert_eq!(ComponentState::Ready.code(), "ready");
        assert_eq!(ActiveOrigin::UserProvided.code(), "user_provided");
    }

    #[test]
    fn a_start_request_carries_no_way_to_redirect_the_installation() {
        let request = StartRequest {
            consent_managed_paths: true,
            resume: true,
        };
        let json = serde_json::to_string(&request).unwrap();
        // The whole request is two booleans.
        assert_eq!(json, "{\"consent_managed_paths\":true,\"resume\":true}");
        // No URL, no hash, no command, no executable, no argument, and nothing
        // shaped like a path.
        for forbidden in [
            "http", "sha256", "command", "url", "exe", "gguf", "port", "arg", "\\\\", ":/", "..",
        ] {
            assert!(
                !json.contains(forbidden),
                "the request must not mention {forbidden}"
            );
        }
        // Deserializing an empty object gives the conservative default.
        let default: StartRequest = serde_json::from_str("{}").unwrap();
        assert!(!default.consent_managed_paths);
        assert!(!default.resume);
    }
}
