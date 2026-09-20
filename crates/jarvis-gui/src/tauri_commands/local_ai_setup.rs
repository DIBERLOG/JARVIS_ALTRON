//! The typed setup API for the managed local AI installation.
//!
//! # What crosses this boundary
//!
//! The interface may ask for a status, for a preflight check, for a start, a
//! cancel, a retry, a cleanup, a validation, a removal, and for the managed paths
//! to become the active ones. It may not supply a URL, a hash, a destination
//! directory, a launch command, or an API key: [`StartRequest`] is two booleans,
//! and everything else comes from the catalog compiled into the backend.
//!
//! # What the DTO contains
//!
//! [`LocalAiSetupView`] is written by hand rather than returned from the core, so
//! the wire shape is a deliberate contract:
//!
//! * it carries stage and component codes, byte counts, component states, the
//!   managed-or-user-provided origin, permission flags, one error code, warning
//!   codes, the pre-release flag, and the restart-recovery state;
//! * it carries no URL, no SHA-256, no absolute staging path, no launch
//!   argument, and no installation receipt. A source is named as
//!   `github.com/ggml-org/llama.cpp`, which is a label and not a location a
//!   caller could fetch from.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::ipc::Channel;

use jarvis_core::ai::local::setup::{
    ActiveOrigin, CleanupReport, ExistingValidation, SetupCoordinator, SetupErrorCode, SetupEvent,
    SetupEventSink, SetupStatus, SetupStage, StartRequest,
};
use jarvis_core::ai::local::{LocalAiConfig, SETTINGS_KEY};
use jarvis_core::SettingsManager;

use super::local_ai::LocalAiHandle;
use crate::AppState;

/// The single setup coordinator of the application.
///
/// Cloning the handle shares one coordinator, and therefore one installation
/// slot: two windows cannot install at the same time.
#[derive(Clone)]
pub struct LocalAiSetupHandle {
    coordinator: SetupCoordinator,
}

impl LocalAiSetupHandle {
    /// Builds the coordinator for the application data root.
    pub fn new(data_directory: PathBuf) -> Self {
        Self {
            coordinator: SetupCoordinator::new(data_directory),
        }
    }

    pub fn coordinator(&self) -> &SetupCoordinator {
        &self.coordinator
    }
}

// ------------------------------------------------------------------- the DTO

/// One component as it is offered before anything is downloaded.
#[derive(Clone, Debug, Serialize)]
pub struct OfferComponentView {
    pub component: String,
    pub display_name: String,
    /// The runtime build, for the runtime. Absent for the model, whose identity
    /// is its display name and its quantisation.
    pub version: Option<String>,
    pub source_label: String,
    pub license: Option<String>,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub pre_release: bool,
    pub checksum_verified: bool,
    pub quantization: Option<String>,
    pub architecture: Option<String>,
    pub ram_recommendation_bytes: Option<u64>,
    pub context_recommendation: Option<u32>,
}

/// What the setup page shows before the user decides anything.
#[derive(Clone, Debug, Serialize)]
pub struct SetupOfferView {
    pub runtime: OfferComponentView,
    pub model: OfferComponentView,
    /// A stable code the interface localizes; never a real path.
    pub install_root: String,
    pub internet_needed_for_download_only: bool,
    pub works_offline_after_install: bool,
    pub steps: Vec<String>,
    pub stage_codes: Vec<String>,
}

/// What a restart found.
#[derive(Clone, Debug, Serialize)]
pub struct RecoveryView {
    pub interrupted: bool,
    pub interrupted_stage: Option<String>,
    pub previous_error_code: Option<String>,
    pub partial_download_bytes: u64,
    pub staging_ready: bool,
    pub runtime_installed: bool,
    pub model_installed: bool,
}

/// The whole setup state, as the interface reads it.
#[derive(Clone, Debug, Serialize)]
pub struct LocalAiSetupView {
    pub stage: String,
    pub step: String,
    pub step_index: u8,
    pub component: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub missing_bytes: u64,
    pub download_bytes: u64,
    pub temporary_bytes: u64,
    pub installed_bytes: u64,
    pub rollback_bytes: u64,
    pub safety_reserve_bytes: u64,
    pub runtime_state: String,
    pub runtime_version: Option<String>,
    pub runtime_bytes: u64,
    pub model_state: String,
    pub model_bytes: u64,
    /// `managed`, `user_provided`, `mixed`, or `unset`.
    pub active_origin: String,
    pub runtime_pre_release: bool,
    pub running: bool,
    pub can_start: bool,
    pub can_cancel: bool,
    pub can_retry: bool,
    pub can_cleanup: bool,
    pub error_code: Option<String>,
    pub warning_codes: Vec<String>,
    pub recovery: RecoveryView,
    pub offer: SetupOfferView,
}

/// One progress event, for a channel rather than a poll.
#[derive(Clone, Debug, Serialize)]
pub struct SetupEventView {
    pub stage: String,
    pub component: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

impl From<&SetupEvent> for SetupEventView {
    fn from(event: &SetupEvent) -> Self {
        Self {
            stage: event.stage.code().to_string(),
            component: event.component.map(|component| component.code().to_string()),
            downloaded_bytes: event.downloaded_bytes,
            total_bytes: event.total_bytes,
        }
    }
}

/// What a cleanup actually removed.
#[derive(Clone, Debug, Serialize)]
pub struct CleanupView {
    pub removed_bytes: u64,
    pub removed_directories: usize,
    /// Directories this application did not create and therefore left alone.
    pub skipped_foreign: usize,
}

impl From<CleanupReport> for CleanupView {
    fn from(report: CleanupReport) -> Self {
        Self {
            removed_bytes: report.removed_bytes,
            removed_directories: report.removed_directories,
            skipped_foreign: report.skipped_foreign,
        }
    }
}

/// The result of a full check of what is installed.
#[derive(Clone, Debug, Serialize)]
pub struct SetupValidationView {
    pub runtime_state: String,
    pub runtime_server_ok: bool,
    pub runtime_architecture_ok: bool,
    pub runtime_bytes: u64,
    pub model_state: String,
    pub model_size_ok: bool,
    pub model_hash_ok: bool,
    pub model_format_ok: bool,
    pub model_bytes: u64,
    pub error_code: Option<String>,
}

impl From<ExistingValidation> for SetupValidationView {
    fn from(report: ExistingValidation) -> Self {
        Self {
            runtime_state: report.runtime_state.code().to_string(),
            runtime_server_ok: report.runtime_server_ok,
            runtime_architecture_ok: report.runtime_architecture_ok,
            runtime_bytes: report.runtime_bytes,
            model_state: report.model_state.code().to_string(),
            model_size_ok: report.model_size_ok,
            model_hash_ok: report.model_hash_ok,
            model_format_ok: report.model_format_ok,
            model_bytes: report.model_bytes,
            error_code: report.error_code.map(|code| code.code().to_string()),
        }
    }
}

/// Where the configured paths came from.
///
/// Computed here rather than in the core, because only this layer knows the
/// configuration that is actually active.
pub fn active_origin(config: &LocalAiConfig, managed: Option<(&Path, &Path)>) -> ActiveOrigin {
    let server = config.server_path();
    let model = config.model_path();
    match (server, model) {
        (None, None) => ActiveOrigin::Unset,
        (Some(server), Some(model)) => match managed {
            Some((managed_server, managed_model)) => {
                let server_managed = server.as_path() == managed_server;
                let model_managed = model.as_path() == managed_model;
                match (server_managed, model_managed) {
                    (true, true) => ActiveOrigin::Managed,
                    (false, false) => ActiveOrigin::UserProvided,
                    _ => ActiveOrigin::Mixed,
                }
            }
            None => ActiveOrigin::UserProvided,
        },
        // One of the two is configured: neither purely managed nor purely manual.
        _ => ActiveOrigin::Mixed,
    }
}

/// Builds the wire view from the core status and the live configuration.
pub fn build_view(
    status: &SetupStatus,
    config: &LocalAiConfig,
    managed: Option<(&Path, &Path)>,
) -> LocalAiSetupView {
    let plan = status.plan;
    LocalAiSetupView {
        stage: status.stage.code().to_string(),
        step: status.step.code().to_string(),
        step_index: status.step.index(),
        component: status
            .component
            .map(|component| component.code().to_string()),
        downloaded_bytes: status.downloaded_bytes,
        total_bytes: status.total_bytes,
        required_bytes: plan.required_peak_bytes,
        available_bytes: plan.available_bytes,
        missing_bytes: plan.missing_bytes,
        download_bytes: plan.download_bytes,
        temporary_bytes: plan.temporary_bytes,
        installed_bytes: plan.installed_bytes,
        rollback_bytes: plan.rollback_bytes,
        safety_reserve_bytes: plan.safety_reserve_bytes,
        runtime_state: status.runtime.state.code().to_string(),
        // The runtime version is what the page must name; the model is named by
        // its display name, and its revision is deliberately not forwarded.
        runtime_version: status.runtime.version.clone(),
        runtime_bytes: status.runtime.bytes,
        model_state: status.model.state.code().to_string(),
        model_bytes: status.model.bytes,
        active_origin: active_origin(config, managed).code().to_string(),
        runtime_pre_release: status.runtime_pre_release,
        running: status.running,
        can_start: status.can_start,
        can_cancel: status.can_cancel,
        can_retry: status.can_retry,
        can_cleanup: status.can_cleanup,
        error_code: status.error_code.map(|code| code.code().to_string()),
        warning_codes: status
            .warnings
            .iter()
            .map(|code| code.code().to_string())
            .collect(),
        recovery: RecoveryView {
            interrupted: status.recovery.interrupted,
            interrupted_stage: status
                .recovery
                .interrupted_stage
                .map(|stage| stage.code().to_string()),
            previous_error_code: status
                .recovery
                .previous_error_code
                .map(|code| code.code().to_string()),
            partial_download_bytes: status.recovery.partial_download_bytes,
            staging_ready: status.recovery.staging_ready,
            runtime_installed: status.recovery.runtime_installed,
            model_installed: status.recovery.model_installed,
        },
        offer: SetupOfferView {
            // The runtime is named by its build; the model is named by its
            // display name and its quantisation. The model's pinned revision is a
            // content identity and stays in the backend.
            runtime: offer_component(&status.offer.runtime, Some(status.offer.runtime.version.clone())),
            model: offer_component(&status.offer.model, None),
            install_root: status.offer.install_root.clone(),
            internet_needed_for_download_only: status.offer.internet_needed_for_download_only,
            works_offline_after_install: status.offer.works_offline_after_install,
            steps: status
                .offer
                .steps
                .iter()
                .map(|step| step.code().to_string())
                .collect(),
            stage_codes: status.offer.stage_codes.clone(),
        },
    }
}

fn offer_component(
    offer: &jarvis_core::ai::local::setup::OfferComponent,
    version: Option<String>,
) -> OfferComponentView {
    OfferComponentView {
        component: offer.component.code().to_string(),
        display_name: offer.display_name.clone(),
        version,
        source_label: offer.source_label.clone(),
        license: offer.license.clone(),
        download_bytes: offer.download_bytes,
        installed_bytes: offer.installed_bytes,
        pre_release: offer.pre_release,
        checksum_verified: offer.checksum_verified,
        quantization: offer.quantization.clone(),
        architecture: offer.architecture.clone(),
        ram_recommendation_bytes: offer.ram_recommendation_bytes,
        context_recommendation: offer.context_recommendation,
    }
}

/// A description of one refusal, for a command that returns a typed error.
fn describe(error: SetupErrorCode) -> String {
    error.code().to_string()
}

/// The view for the paths and the configuration this application holds.
fn current_view(state: &tauri::State<'_, AppState>) -> LocalAiSetupView {
    let coordinator = state.local_ai_setup.coordinator();
    let status = coordinator.status();
    let config = state.local_ai.config();
    let managed = coordinator.managed_paths();
    let managed_pair = managed
        .as_ref()
        .map(|(server, model)| (server.as_path(), model.as_path()));
    build_view(&status, &config, managed_pair)
}

// ------------------------------------------------------------------ commands

/// The current setup state. Cheap, and safe to poll.
#[tauri::command(async)]
pub fn local_ai_setup_status(
    state: tauri::State<'_, AppState>,
) -> Result<LocalAiSetupView, String> {
    Ok(current_view(&state))
}

/// Runs the checks an installation would run first, without starting one.
///
/// The returned view carries `insufficient_space`, `not_same_volume`, or `io`
/// as the error code when an installation would be refused.
#[tauri::command(async)]
pub fn local_ai_setup_preflight(
    state: tauri::State<'_, AppState>,
) -> Result<LocalAiSetupView, String> {
    let coordinator = state.local_ai_setup.coordinator();
    match coordinator.preflight() {
        Ok(_) => Ok(current_view(&state)),
        Err(error) => Err(describe(error)),
    }
}

/// Starts the installation and streams progress through `channel`.
///
/// The request carries consent and nothing else: no URL, no hash, no path, no
/// command.
#[tauri::command(async)]
pub fn local_ai_setup_start(
    state: tauri::State<'_, AppState>,
    request: StartRequest,
    channel: Channel<SetupEventView>,
) -> Result<LocalAiSetupView, String> {
    start_with(&state, request, channel, false)
}

/// Retries a failed or cancelled run, resuming what is already on the disk.
#[tauri::command(async)]
pub fn local_ai_setup_retry(
    state: tauri::State<'_, AppState>,
    request: StartRequest,
    channel: Channel<SetupEventView>,
) -> Result<LocalAiSetupView, String> {
    start_with(&state, request, channel, true)
}

fn start_with(
    state: &tauri::State<'_, AppState>,
    request: StartRequest,
    channel: Channel<SetupEventView>,
    retry: bool,
) -> Result<LocalAiSetupView, String> {
    let coordinator = state.local_ai_setup.coordinator().clone();
    // The consent the user gave travels with the run, so the verified paths are
    // saved by the backend the moment the installation reports success — an
    // interface that is closed at that instant cannot lose them.
    let consent = request.consent_managed_paths;
    let applying = coordinator.clone();
    let local_ai = state.local_ai.clone();
    let settings = state.settings.clone();
    let sink: SetupEventSink = Arc::new(move |event: SetupEvent| {
        // Only the stage and the byte counts cross the channel.
        if let Err(error) = channel.send(SetupEventView::from(&event)) {
            log::debug!("local ai setup: the progress channel is closed: {error}");
        }
        if consent && event.stage == SetupStage::Complete {
            match applying.managed_paths() {
                Some((server, model)) => {
                    if let Err(error) = apply_managed_paths(&local_ai, &settings, &server, &model)
                    {
                        log::warn!("local ai setup: the verified paths were not applied: {error}");
                    }
                }
                // Nothing to apply: the run failed to leave both paths in place.
                None => log::warn!("local ai setup: the run completed without both paths"),
            }
        }
    });
    let result = if retry {
        coordinator.retry(request, Some(sink))
    } else {
        coordinator.start(request, Some(sink))
    };
    match result {
        Ok(_) => Ok(current_view(state)),
        Err(error) => Err(describe(error)),
    }
}

/// Asks the running installation to stop. Returns whether one was running.
#[tauri::command(async)]
pub fn local_ai_setup_cancel(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.local_ai_setup.coordinator().cancel())
}

/// Removes the temporary directories this application created.
#[tauri::command(async)]
pub fn local_ai_setup_cleanup_temp(
    state: tauri::State<'_, AppState>,
) -> Result<CleanupView, String> {
    state
        .local_ai_setup
        .coordinator()
        .cleanup_temp()
        .map(CleanupView::from)
        .map_err(describe)
}

/// Makes the verified managed paths the active ones.
///
/// `confirm` is the user's consent. Without it nothing is written, so a
/// configuration the user chose by hand is never replaced silently.
#[tauri::command(async)]
pub fn local_ai_setup_use_managed(
    state: tauri::State<'_, AppState>,
    confirm: bool,
) -> Result<LocalAiSetupView, String> {
    if !confirm {
        return Ok(current_view(&state));
    }
    let coordinator = state.local_ai_setup.coordinator();
    let Some((server, model)) = coordinator.managed_paths() else {
        return Err(describe(SetupErrorCode::NotRunning));
    };
    apply_managed_paths(&state.local_ai, &state.settings, &server, &model)?;
    Ok(current_view(&state))
}

/// Removes the managed runtime, and only when this application installed it.
#[tauri::command(async)]
pub fn local_ai_setup_remove_runtime(
    state: tauri::State<'_, AppState>,
) -> Result<LocalAiSetupView, String> {
    state
        .local_ai_setup
        .coordinator()
        .remove_runtime()
        .map_err(describe)?;
    Ok(current_view(&state))
}

/// Removes the managed model, and only when this application installed it.
#[tauri::command(async)]
pub fn local_ai_setup_remove_model(
    state: tauri::State<'_, AppState>,
) -> Result<LocalAiSetupView, String> {
    state
        .local_ai_setup
        .coordinator()
        .remove_model()
        .map_err(describe)?;
    Ok(current_view(&state))
}

/// Runs the full check: the model's SHA-256 and the server's PE header.
#[tauri::command(async)]
pub fn local_ai_setup_validate_existing(
    state: tauri::State<'_, AppState>,
) -> Result<SetupValidationView, String> {
    Ok(SetupValidationView::from(
        state.local_ai_setup.coordinator().validate_existing(),
    ))
}

/// Writes the two verified paths into the settings and applies them.
fn apply_managed_paths(
    local_ai: &LocalAiHandle,
    settings: &SettingsManager,
    server: &Path,
    model: &Path,
) -> Result<(), String> {
    let mut config = local_ai.config();
    config.server.server_path = server.to_string_lossy().into_owned();
    config.server.model_path = model.to_string_lossy().into_owned();
    let encoded = config.to_json().map_err(|_| describe(SetupErrorCode::SettingsRefused))?;
    settings
        .write(SETTINGS_KEY, &encoded)
        .map_err(|error| {
            log::warn!("local ai setup: the settings could not be saved: {error}");
            describe(SetupErrorCode::SettingsRefused)
        })?;
    local_ai
        .gateway()
        .apply_config(config)
        .map_err(|_| describe(SetupErrorCode::SettingsRefused))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jarvis_core::ai::local::setup::{ComponentState, PinnedCatalog, SetupCatalog, WizardStep};
    use std::collections::BTreeSet;

    /// A directory of its own, without touching the real profile.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("jarvis-gui-setup-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        let _ = std::fs::create_dir_all(&directory);
        directory
    }

    fn handle(name: &str) -> LocalAiSetupHandle {
        LocalAiSetupHandle::new(scratch(name))
    }

    fn view(name: &str) -> LocalAiSetupView {
        let handle = handle(name);
        build_view(&handle.coordinator().status(), &LocalAiConfig::default(), None)
    }

    /// Every key in a JSON document, at any depth.
    fn keys(value: &serde_json::Value, out: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, nested) in map {
                    out.insert(key.clone());
                    keys(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    keys(item, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn the_dto_carries_the_fields_the_interface_reads() {
        let view = view("fields");
        let json = serde_json::to_value(&view).unwrap();
        let mut present = BTreeSet::new();
        keys(&json, &mut present);
        for required in [
            "stage",
            "step",
            "step_index",
            "component",
            "downloaded_bytes",
            "total_bytes",
            "required_bytes",
            "available_bytes",
            "missing_bytes",
            "download_bytes",
            "temporary_bytes",
            "installed_bytes",
            "rollback_bytes",
            "safety_reserve_bytes",
            "runtime_state",
            "model_state",
            "active_origin",
            "running",
            "can_start",
            "can_cancel",
            "can_retry",
            "can_cleanup",
            "error_code",
            "warning_codes",
            "runtime_pre_release",
            "recovery",
            "offer",
        ] {
            assert!(
                present.contains(required),
                "the DTO must carry {required}, and it does not"
            );
        }
        // The recovery and offer sub-objects are typed, not free-form.
        for nested in [
            "interrupted",
            "interrupted_stage",
            "previous_error_code",
            "partial_download_bytes",
            "staging_ready",
            "runtime_installed",
            "model_installed",
            "display_name",
            "source_label",
            "license",
            "checksum_verified",
            "quantization",
            "ram_recommendation_bytes",
            "context_recommendation",
            "install_root",
            "internet_needed_for_download_only",
            "works_offline_after_install",
            "steps",
            "stage_codes",
        ] {
            assert!(
                present.contains(nested),
                "the DTO must carry {nested}, and it does not"
            );
        }
    }

    #[test]
    fn the_dto_never_carries_a_url_a_hash_a_path_or_a_command() {
        let view = view("forbidden");
        let json = serde_json::to_string(&view).unwrap();
        // A source is named, not located.
        assert!(json.contains("github.com/ggml-org/llama.cpp"));
        for forbidden in [
            "http://",
            "https://",
            "sha256",
            "a56061d03bd2055a8236c8a80ec2440a550a53eaecf935fb2ddf37c93995667c",
            "917f39c076402c421224824607397af20f53625a60defc20e8dd22446bf4c5d7",
            "setup-temp",
            "installation-receipt",
            "payload",
            "--model",
            "--host",
            "--port",
            "--ctx-size",
            "exe",
            "gguf",
            "C:\\\\",
            "localappdata",
        ] {
            assert!(
                !json.contains(forbidden),
                "the DTO must not contain {forbidden}"
            );
        }
        // No absolute path of any kind.
        assert!(!json.contains("\\\\?\\"));
    }

    #[test]
    fn the_dto_keeps_the_model_revision_out_of_the_wire_shape() {
        let view = view("revision");
        assert_eq!(view.model_state, "absent");
        // The pinned model revision is a 40-character hexadecimal string. A
        // removal is authorized by the receipt, not by a value the interface
        // holds, so the revision is not forwarded — for the component status or
        // for the offer.
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("7c41481f57cb95916b40956ab2f0b139b296d974"));
        assert!(view.runtime_version.is_none());
        assert!(view.offer.model.version.is_none());
        assert_eq!(view.offer.runtime.version.as_deref(), Some("b10964"));
    }

    #[test]
    fn every_key_in_the_dto_is_one_this_file_declares() {
        let view = view("allowlist");
        let json = serde_json::to_value(&view).unwrap();
        let mut present = BTreeSet::new();
        keys(&json, &mut present);
        let allowed: BTreeSet<String> = [
            // LocalAiSetupView
            "stage",
            "step",
            "step_index",
            "component",
            "downloaded_bytes",
            "total_bytes",
            "required_bytes",
            "available_bytes",
            "missing_bytes",
            "download_bytes",
            "temporary_bytes",
            "installed_bytes",
            "rollback_bytes",
            "safety_reserve_bytes",
            "runtime_state",
            "runtime_version",
            "runtime_bytes",
            "model_state",
            "model_bytes",
            "active_origin",
            "runtime_pre_release",
            "running",
            "can_start",
            "can_cancel",
            "can_retry",
            "can_cleanup",
            "error_code",
            "warning_codes",
            "recovery",
            "offer",
            // RecoveryView
            "interrupted",
            "interrupted_stage",
            "previous_error_code",
            "partial_download_bytes",
            "staging_ready",
            "runtime_installed",
            "model_installed",
            // SetupOfferView
            "runtime",
            "model",
            "install_root",
            "internet_needed_for_download_only",
            "works_offline_after_install",
            "steps",
            "stage_codes",
            // OfferComponentView
            "display_name",
            "version",
            "source_label",
            "license",
            "pre_release",
            "checksum_verified",
            "quantization",
            "architecture",
            "ram_recommendation_bytes",
            "context_recommendation",
        ]
        .into_iter()
        .map(|key| key.to_string())
        .collect();
        let unexpected: Vec<&String> = present.difference(&allowed).collect();
        assert!(
            unexpected.is_empty(),
            "the DTO gained fields nobody reviewed: {unexpected:?}"
        );
    }

    #[test]
    fn a_start_request_is_two_booleans_and_nothing_else() {
        let json = serde_json::to_string(&StartRequest {
            consent_managed_paths: true,
            resume: true,
        })
        .unwrap();
        assert_eq!(json, "{\"consent_managed_paths\":true,\"resume\":true}");
        // The default is the conservative one: no consent, no resume.
        assert!(!StartRequest::default().consent_managed_paths);
        assert!(!StartRequest::default().resume);
    }

    #[test]
    fn the_offer_states_the_facts_the_page_shows_before_downloading() {
        let view = view("offer");
        assert_eq!(view.offer.runtime.version.as_deref(), Some("b10964"));
        assert!(view.offer.runtime.pre_release);
        assert!(view.offer.runtime.checksum_verified);
        assert_eq!(view.offer.runtime.source_label, "github.com/ggml-org/llama.cpp");
        assert_eq!(view.offer.runtime.download_bytes, 18_427_629);
        assert_eq!(view.offer.model.display_name, "Qwen3-8B Q4_K_M");
        assert_eq!(view.offer.model.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(view.offer.model.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(view.offer.model.download_bytes, 5_027_783_488);
        assert_eq!(view.offer.install_root, "app_data");
        assert!(view.offer.internet_needed_for_download_only);
        assert!(view.offer.works_offline_after_install);
        // Six wizard steps and sixteen stage codes, named by the backend.
        assert_eq!(view.offer.steps.len(), 6);
        assert_eq!(
            view.offer.steps,
            vec![
                "preflight",
                "runtime",
                "model",
                "configure",
                "launch_test",
                "done"
            ]
        );
        assert_eq!(view.offer.stage_codes.len(), 16);
        assert!(view.offer.stage_codes.contains(&"readiness".to_string()));
        assert!(view.offer.stage_codes.contains(&"test_inference".to_string()));
    }

    #[test]
    fn the_requirement_the_page_shows_is_the_one_the_planner_computed() {
        let view = view("space");
        // The pinned clean-install peak: one model, one runtime bound, one reserve.
        assert_eq!(view.required_bytes, 5_665_317_696);
        assert_eq!(view.model_bytes, 0);
        assert_eq!(view.rollback_bytes, 0);
        assert_eq!(view.safety_reserve_bytes, 536_870_912);
        // The peak is the new bytes plus the reserve, and never a second model.
        assert_eq!(
            view.required_bytes,
            view.installed_bytes + view.safety_reserve_bytes
        );
        assert!(view.required_bytes < 2 * 5_027_783_488);
        assert_eq!(view.temporary_bytes, 5_027_783_488);
        // The runtime's planned footprint is the bound, and it is not a path.
        assert_eq!(
            view.download_bytes,
            18_427_629 + 5_027_783_488
        );
    }

    #[test]
    fn the_origin_distinguishes_a_managed_install_from_a_chosen_file() {
        let managed_server = PathBuf::from("C:/data/runtime/llama.cpp/b10964/llama-server.exe");
        let managed_model = PathBuf::from("C:/data/models/qwen3-8b-q4_k_m/model.gguf");

        let mut config = LocalAiConfig::default();
        assert_eq!(active_origin(&config, None), ActiveOrigin::Unset);

        config.server.server_path = managed_server.display().to_string();
        config.server.model_path = managed_model.display().to_string();
        assert_eq!(
            active_origin(&config, Some((&managed_server, &managed_model))),
            ActiveOrigin::Managed
        );
        // The same two paths without a managed installation are the user's own.
        assert_eq!(
            active_origin(&config, None),
            ActiveOrigin::UserProvided
        );

        let mut manual = LocalAiConfig::default();
        manual.server.server_path = "C:/tools/llama-server.exe".to_string();
        manual.server.model_path = "C:/models/mine.gguf".to_string();
        assert_eq!(
            active_origin(&manual, Some((&managed_server, &managed_model))),
            ActiveOrigin::UserProvided
        );

        // One of each is neither purely managed nor purely manual.
        let mut mixed = LocalAiConfig::default();
        mixed.server.server_path = managed_server.display().to_string();
        mixed.server.model_path = "C:/models/mine.gguf".to_string();
        assert_eq!(
            active_origin(&mixed, Some((&managed_server, &managed_model))),
            ActiveOrigin::Mixed
        );

        // Half-configured is reported as mixed rather than guessed at.
        let mut half = LocalAiConfig::default();
        half.server.model_path = managed_model.display().to_string();
        assert_eq!(
            active_origin(&half, Some((&managed_server, &managed_model))),
            ActiveOrigin::Mixed
        );
    }

    #[test]
    fn a_fresh_installation_reports_that_it_may_start_and_may_not_cancel() {
        let view = view("flags");
        assert_eq!(view.stage, "idle");
        assert_eq!(view.step, "preflight");
        assert!(view.can_start);
        assert!(!view.can_cancel);
        assert!(!view.can_retry);
        assert!(!view.can_cleanup);
        assert!(!view.running);
        assert_eq!(view.error_code, None);
        assert_eq!(view.active_origin, "unset");
        assert!(!view.recovery.interrupted);
    }

    #[test]
    fn a_preflight_reports_the_plan_without_starting_an_installation() {
        let handle = handle("preflight");
        let status = handle.coordinator().preflight().unwrap();
        assert_eq!(status.stage, SetupStage::Preflight);
        assert_eq!(status.step, WizardStep::Preflight);
        // Nothing runs, and nothing was downloaded.
        assert!(!status.running);
        assert!(!handle.coordinator().is_running());
        assert!(status.plan.available_bytes > 0);
        assert!(status.plan.fits());
        assert_eq!(status.error_code, None);
        assert!(!status.can_cancel);
    }

    #[test]
    fn cleanup_and_removal_report_typed_outcomes() {
        let handle = handle("cleanup");
        let report = handle.coordinator().cleanup_temp().unwrap();
        assert_eq!(report.removed_directories, 0);
        // Nothing is installed, so a removal removes nothing and refuses nothing.
        assert_eq!(handle.coordinator().remove_runtime().unwrap(), 0);
        assert_eq!(handle.coordinator().remove_model().unwrap(), 0);
        let report = handle.coordinator().validate_existing();
        assert_eq!(report.runtime_state, ComponentState::Absent);
        assert_eq!(report.model_state, ComponentState::Absent);
    }

    #[test]
    fn the_event_view_carries_only_a_stage_and_byte_counts() {
        let event = SetupEvent {
            stage: SetupStage::DownloadModel,
            component: Some(jarvis_core::ai::local::setup::SetupComponent::Model),
            downloaded_bytes: 1024,
            total_bytes: 2048,
        };
        let json = serde_json::to_string(&SetupEventView::from(&event)).unwrap();
        assert_eq!(
            json,
            "{\"stage\":\"download_model\",\"component\":\"model\",\"downloaded_bytes\":1024,\"total_bytes\":2048}"
        );
    }

    #[test]
    fn the_pinned_trust_level_is_the_only_one_the_application_builds() {
        // The catalog the application uses is the compiled-in one, and it refuses
        // anything else. This is what makes a URL from the interface impossible.
        let spec = PinnedCatalog.spec();
        assert_eq!(
            spec.trust,
            jarvis_core::ai::local::setup::ArtifactTrust::Pinned
        );
        assert!(spec.runtime_archive.url.starts_with("https://"));
        assert!(spec.model.url.starts_with("https://"));
        // And the production trust level refuses a loopback address outright.
        let fake = jarvis_core::ai::local::setup::ArtifactExpectation {
            url: "http://127.0.0.1:9/model.gguf".to_string(),
            filename: "model.gguf".to_string(),
            expected_size: 1,
            sha256: "a".repeat(64),
        };
        assert!(
            jarvis_core::ai::local::setup::download::validate_expectation(&fake, spec.trust)
                .is_err()
        );
    }
}
