//! The single coordinator for one managed installation.
//!
//! # One operation at a time
//!
//! [`SetupCoordinator::start`] claims a slot before it does anything. A second
//! call while a run is in flight returns [`SetupErrorCode::AlreadyRunning`] and
//! changes nothing, so two windows cannot download the same model twice.
//!
//! # The interface sends nothing but consent
//!
//! Everything the installation depends on Р Р†Р вЂљРІР‚Сњ the URL, the size, the SHA-256, the
//! destination directory, the launch arguments Р Р†Р вЂљРІР‚Сњ is chosen by
//! [`SetupCatalog`], which in the application is always [`PinnedCatalog`]
//! reading the manifests compiled into the build. The frontend sends a
//! [`StartRequest`], which is two booleans.
//!
//! # Long work runs off the window thread
//!
//! [`SetupCoordinator::start`] returns as soon as the work is scheduled, and the
//! stage, the byte counts, and the warnings are published in a shared status that
//! the interface polls. Progress is also pushed to an optional sink, so a speed
//! readout needs no polling loop of its own. The state is the backend's, so
//! closing and reopening the page changes nothing.
//!
//! # Cancellation
//!
//! Every long loop Р Р†Р вЂљРІР‚Сњ the download, the hash, the extraction, the first-run test Р Р†Р вЂљРІР‚Сњ
//! checks one cancellation flag. A cancelled run keeps its `.part` file and its
//! staging directory, which is what makes a later retry a resume rather than a
//! restart.
//!
//! # What a failure never destroys
//!
//! A component is only removed from the disk by an explicit removal command, and
//! only when it carries this application's installation receipt. A failure while
//! installing the model leaves the runtime exactly as it was; a failure while
//! writing a `.part` file leaves the previous installation untouched.

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::super::managed::{
    extract_runtime_archive_with, has_installation_receipt, managed_model_manifest,
    managed_runtime_manifest, validate_pe_x64, validate_runtime_archive,
    write_installation_receipt, InstallationFacts, RuntimeArchiveSpec, RUNTIME_ARCHIVE_FILES,
    RUNTIME_INSTALLED_FILES, MAX_RUNTIME_ARCHIVE_FILES, MAX_RUNTIME_COMPRESSION_RATIO,
    MAX_RUNTIME_UNPACKED_BYTES,
};
use super::super::model::read_gguf_info;
use super::download::{
    download_artifact, sha256_file, ArtifactExpectation, ArtifactTrust, DownloadError,
    DownloadRequest, DownloadTransport, ReqwestTransport,
};
use super::layout::{
    available_disk_bytes, cleanup_owned_temp, commit_staging_dir, directory_size, discard_staging,
    is_owned_temp, prepare_staging, promote_file, retain_previous, CleanupReport, LayoutError,
    ManagedRoots,
};
use super::plan::{
    plan_space, ComponentPlanKind, ModelInstallKind, PlanInputs, SpacePlan, RUNTIME_PLAN_BYTES,
};
use super::state::{
    recovery_state, scan_with, write_persisted, InstalledState, PersistedSetup, RecoveryState,
    ScanNames,
};
use super::{
    ComponentState, SetupComponent, SetupErrorCode, SetupStage, SetupWarningCode, StartRequest,
    TestOutcome, WizardStep,
};

/// Where a managed installation puts its files, as a stable code the interface
/// turns into a localized label. It is never an absolute path.
pub const INSTALL_ROOT_CODE: &str = "app_data";

/// The pinned runtime is an official pre-release build of `llama.cpp`.
pub const RUNTIME_IS_PRE_RELEASE: bool = true;

/// One published progress event.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SetupEvent {
    pub stage: SetupStage,
    pub component: Option<SetupComponent>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

/// Where progress events are delivered.
pub type SetupEventSink = Arc<dyn Fn(SetupEvent) + Send + Sync>;

/// Everything about a component that the interface may show.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ComponentStatus {
    pub component: SetupComponent,
    pub state: ComponentState,
    pub version: Option<String>,
    pub bytes: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

/// One component as it is offered before anything is downloaded.
#[derive(Clone, Debug, serde::Serialize)]
pub struct OfferComponent {
    pub component: SetupComponent,
    pub display_name: String,
    pub version: String,
    /// Host and repository of the pinned source. Not a URL the interface may
    /// change, and not something the interface supplies.
    pub source_label: String,
    pub license: Option<String>,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub pre_release: bool,
    /// The hash is checked by the backend on every download.
    pub checksum_verified: bool,
    pub quantization: Option<String>,
    pub architecture: Option<String>,
    pub ram_recommendation_bytes: Option<u64>,
    pub context_recommendation: Option<u32>,
}

/// What the setup page shows before the user decides anything.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SetupOffer {
    pub runtime: OfferComponent,
    pub model: OfferComponent,
    /// A stable code; the interface localizes it. Never a real path.
    pub install_root: String,
    /// The network is used for the download and for nothing else.
    pub internet_needed_for_download_only: bool,
    /// After the installation the local mode works without a network.
    pub works_offline_after_install: bool,
    /// Steps in order, so the interface does not hard-code them.
    pub steps: Vec<WizardStep>,
    /// Stable stage codes, in order, for a progress list.
    pub stage_codes: Vec<String>,
}

/// Everything the interface may read about the setup.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SetupStatus {
    pub stage: SetupStage,
    pub step: WizardStep,
    pub component: Option<SetupComponent>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub runtime: ComponentStatus,
    pub model: ComponentStatus,
    pub plan: SpacePlan,
    pub offer: SetupOffer,
    pub recovery: RecoveryState,
    pub error_code: Option<SetupErrorCode>,
    pub warnings: Vec<SetupWarningCode>,
    pub runtime_pre_release: bool,
    /// Whether both installed paths exist and can be used.
    pub managed_paths_ready: bool,
    pub running: bool,
    pub can_start: bool,
    pub can_cancel: bool,
    pub can_retry: bool,
    pub can_cleanup: bool,
    /// Present after the first-run test has run in this session.
    pub test: Option<TestOutcome>,
}

/// The result of a full check of what is already installed.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ExistingValidation {
    pub runtime_state: ComponentState,
    pub runtime_server_ok: bool,
    pub runtime_architecture_ok: bool,
    pub runtime_bytes: u64,
    pub model_state: ComponentState,
    pub model_size_ok: bool,
    pub model_hash_ok: bool,
    pub model_format_ok: bool,
    pub model_bytes: u64,
    pub error_code: Option<SetupErrorCode>,
}

/// The pinned artifacts, as the setup code needs them.
///
/// A data structure rather than a dozen trait methods, so a test can describe a
/// fixture with a few small values and the production catalog with the manifests.
#[derive(Clone, Debug)]
pub struct CatalogSpec {
    pub trust: ArtifactTrust,
    pub runtime_version: String,
    pub runtime_pre_release: bool,
    pub runtime_source_label: String,
    pub runtime_archive: ArtifactExpectation,
    pub runtime_allowed_entries: Vec<&'static str>,
    pub runtime_installed_entries: Vec<&'static str>,
    pub runtime_server_name: &'static str,
    /// Planned footprint of the unpacked runtime.
    pub runtime_plan_bytes: u64,
    pub model_id: String,
    pub model_display_name: String,
    pub model_revision: String,
    pub model_source_label: String,
    pub model_license: String,
    pub model_quantization: String,
    pub model_architecture: String,
    pub model_context_recommendation: u32,
    pub model_ram_recommendation_bytes: u64,
    pub model: ArtifactExpectation,
}

/// Chooses the artifacts an installation may use.
pub trait SetupCatalog: Send + Sync {
    fn spec(&self) -> CatalogSpec;
}

/// The production catalog: only the manifests compiled into this build.
pub struct PinnedCatalog;

impl SetupCatalog for PinnedCatalog {
    fn spec(&self) -> CatalogSpec {
        let runtime = managed_runtime_manifest();
        let model = managed_model_manifest();
        CatalogSpec {
            trust: ArtifactTrust::Pinned,
            runtime_version: runtime.version.to_string(),
            runtime_pre_release: RUNTIME_IS_PRE_RELEASE,
            runtime_source_label: source_label(runtime.artifact.source_url),
            runtime_archive: ArtifactExpectation::from(runtime.artifact),
            runtime_allowed_entries: RUNTIME_ARCHIVE_FILES.to_vec(),
            runtime_installed_entries: RUNTIME_INSTALLED_FILES.to_vec(),
            runtime_server_name: "llama-server.exe",
            runtime_plan_bytes: RUNTIME_PLAN_BYTES,
            model_id: model.model_id.to_string(),
            model_display_name: model.display_name.to_string(),
            model_revision: model.source_revision.to_string(),
            model_source_label: source_label(model.artifact.source_url),
            model_license: model.license_identifier.to_string(),
            model_quantization: model.quantization.to_string(),
            model_architecture: "qwen3".to_string(),
            model_context_recommendation: model.context_recommendation,
            model_ram_recommendation_bytes: model.ram_recommendation_bytes,
            model: ArtifactExpectation::from(model.artifact),
        }
    }
}

/// `github.com/ggml-org/llama.cpp` from the pinned URL: a source the interface
/// may name, without handing it a URL it could rewrite.
pub fn source_label(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    rest.split('/').take(3).collect::<Vec<_>>().join("/")
}

/// Internal state, shared with the worker thread.
struct Inner {
    roots: ManagedRoots,
    catalog: Arc<dyn SetupCatalog>,
    transport: Arc<dyn DownloadTransport>,
    status: Mutex<SetupStatus>,
    sink: Mutex<Option<SetupEventSink>>,
    last_scan: Mutex<InstalledState>,
}

/// The single coordinator for one data root.
#[derive(Clone)]
pub struct SetupCoordinator {
    inner: Arc<Inner>,
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl SetupCoordinator {
    /// Builds the coordinator the application uses: pinned artifacts and a real
    /// HTTP transport.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self::with_parts(
            data_dir,
            Arc::new(PinnedCatalog),
            Arc::new(ReqwestTransport::default()),
        )
    }

    /// Builds a coordinator with explicit parts, for tests.
    pub fn with_parts(
        data_dir: impl Into<PathBuf>,
        catalog: Arc<dyn SetupCatalog>,
        transport: Arc<dyn DownloadTransport>,
    ) -> Self {
        let spec = catalog.spec();
        let coordinator = Self {
            inner: Arc::new(Inner {
                roots: ManagedRoots::new(data_dir),
                catalog,
                transport,
                status: Mutex::new(SetupStatus::default()),
                sink: Mutex::new(None),
                last_scan: Mutex::new(InstalledState::default()),
            }),
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        coordinator.inner.install_offer(&spec);
        coordinator.refresh_now();
        coordinator
    }

    /// The data root this coordinator installs into.
    pub fn data_dir(&self) -> &Path {
        self.inner.roots.data_dir()
    }

    /// Whether an operation is in flight.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// The current status. Cheap to call: it rescans the tree, never a file.
    pub fn status(&self) -> SetupStatus {
        if self.is_running() {
            return self.inner.decorate(true);
        }
        self.refresh_now()
    }

    /// Whether both managed paths are installed and usable.
    pub fn managed_ready(&self) -> bool {
        self.managed_paths().is_some()
    }

    /// The installed paths, when both exist. Used only after a validated pass.
    pub fn managed_paths(&self) -> Option<(PathBuf, PathBuf)> {
        let spec = self.inner.catalog.spec();
        let server = self
            .inner
            .roots
            .runtime_server_path_named(spec.runtime_server_name);
        let model = self.inner.roots.model_path_named(&spec.model.filename);
        if server.is_file() && model.is_file() {
            Some((server, model))
        } else {
            None
        }
    }

    fn refresh_now(&self) -> SetupStatus {
        self.inner.refresh()
    }

    /// Starts one installation. Returns immediately with the initial status.
    ///
    /// Refuses with [`SetupErrorCode::AlreadyRunning`] when another run holds the
    /// slot, so only one installation is ever in flight.
    pub fn start(
        &self,
        request: StartRequest,
        sink: Option<SetupEventSink>,
    ) -> Result<SetupStatus, SetupErrorCode> {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(SetupErrorCode::AlreadyRunning);
        }
        self.cancel.store(false, Ordering::SeqCst);
        if let Ok(mut slot) = self.inner.sink.lock() {
            *slot = sink;
        }
        self.inner.begin_run(request.resume);
        let inner = Arc::clone(&self.inner);
        let cancel = Arc::clone(&self.cancel);
        let running = Arc::clone(&self.running);
        let spawned = std::thread::Builder::new()
            .name("jarvis-local-ai-setup".to_string())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| execute(&inner, &cancel, request)));
                match outcome {
                    Ok(Ok(())) => inner.finish(SetupStage::Complete, None),
                    Ok(Err(SetupErrorCode::Cancelled)) => inner.finish(SetupStage::Cancelled, None),
                    Ok(Err(code)) => inner.finish(SetupStage::Failed, Some(code)),
                    // A panic in this thread must not leave the slot claimed or
                    // the page showing a stage that will never advance.
                    Err(_) => inner.finish(SetupStage::Failed, Some(SetupErrorCode::Io)),
                }
                running.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.running.store(false, Ordering::SeqCst);
            return Err(SetupErrorCode::Io);
        }
        Ok(self.status())
    }

    /// Retries a failed run, resuming anything that is already on the disk.
    pub fn retry(
        &self,
        request: StartRequest,
        sink: Option<SetupEventSink>,
    ) -> Result<SetupStatus, SetupErrorCode> {
        self.start(
            StartRequest {
                resume: true,
                ..request
            },
            sink,
        )
    }

    /// Asks the running operation to stop. Returns whether one was running.
    pub fn cancel(&self) -> bool {
        if !self.is_running() {
            return false;
        }
        self.cancel.store(true, Ordering::SeqCst);
        true
    }

    /// Removes every temporary directory this application owns.
    ///
    /// A directory without the ownership marker Р Р†Р вЂљРІР‚Сњ including one a user created Р Р†Р вЂљРІР‚Сњ
    /// is counted and left alone.
    pub fn cleanup_temp(&self) -> Result<CleanupReport, SetupErrorCode> {
        if self.is_running() {
            return Err(SetupErrorCode::AlreadyRunning);
        }
        let report =
            cleanup_owned_temp(&self.inner.roots.temp_root()).map_err(map_layout_error)?;
        self.refresh_now();
        Ok(report)
    }

    /// Removes the managed runtime, and only when this application installed it.
    pub fn remove_runtime(&self) -> Result<u64, SetupErrorCode> {
        self.remove_managed_component(&self.inner.roots.runtime_dir(), "llama.cpp", "runtime")
    }

    /// Removes the managed model, and only when this application installed it.
    pub fn remove_model(&self) -> Result<u64, SetupErrorCode> {
        self.remove_managed_component(&self.inner.roots.model_dir(), "model", "model")
    }

    fn remove_managed_component(
        &self,
        directory: &Path,
        receipt_kind: &str,
        previous_prefix: &str,
    ) -> Result<u64, SetupErrorCode> {
        if self.is_running() {
            return Err(SetupErrorCode::AlreadyRunning);
        }
        let mut removed = 0_u64;
        if directory.is_dir() {
            if !has_installation_receipt(directory, receipt_kind) {
                // Not ours: a directory that appeared here without our receipt is
                // never deleted.
                return Err(SetupErrorCode::NotOwned);
            }
            removed = directory_size(directory);
            fs::remove_dir_all(directory).map_err(|_| SetupErrorCode::Io)?;
        }
        // Rollback copies this application moved aside go with the component.
        let previous_root = self.inner.roots.data_dir().join("setup-previous");
        if let Ok(entries) = fs::read_dir(&previous_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with(previous_prefix) {
                    continue;
                }
                removed = removed.saturating_add(directory_size(&entry.path()));
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        self.refresh_now();
        Ok(removed)
    }

    /// Runs the full check: the model's SHA-256 and the server's PE header.
    ///
    /// Deliberately separate from [`SetupStatus`], which must stay cheap.
    pub fn validate_existing(&self) -> ExistingValidation {
        let spec = self.inner.catalog.spec();
        let mut report = ExistingValidation {
            runtime_state: ComponentState::Absent,
            runtime_server_ok: false,
            runtime_architecture_ok: false,
            runtime_bytes: 0,
            model_state: ComponentState::Absent,
            model_size_ok: false,
            model_hash_ok: false,
            model_format_ok: false,
            model_bytes: 0,
            error_code: None,
        };

        let server = self.inner.roots.runtime_server_path();
        if server.is_file() {
            report.runtime_bytes = directory_size(&self.inner.roots.runtime_dir());
            report.runtime_server_ok = true;
            report.runtime_architecture_ok = validate_pe_x64(&server).is_ok();
            report.runtime_state = if report.runtime_architecture_ok {
                ComponentState::Ready
            } else {
                ComponentState::Damaged
            };
            if !report.runtime_architecture_ok {
                report.error_code = Some(SetupErrorCode::RuntimeArchitectureMismatch);
            }
        }

        let model = self.inner.roots.model_path_named(&spec.model.filename);
        if model.is_file() {
            report.model_bytes = model.metadata().map(|m| m.len()).unwrap_or(0);
            report.model_size_ok = report.model_bytes == spec.model.expected_size;
            if report.model_size_ok {
                report.model_hash_ok = sha256_file(&model)
                    .map(|actual| actual.eq_ignore_ascii_case(&spec.model.sha256))
                    .unwrap_or(false);
            }
            report.model_format_ok = read_gguf_info(&model).is_ok();
            report.model_state =
                if report.model_size_ok && report.model_hash_ok && report.model_format_ok {
                    ComponentState::Ready
                } else {
                    ComponentState::Damaged
                };
            if report.model_state == ComponentState::Damaged && report.error_code.is_none() {
                report.error_code = Some(if !report.model_size_ok {
                    SetupErrorCode::SizeMismatch
                } else if !report.model_hash_ok {
                    SetupErrorCode::HashMismatch
                } else {
                    SetupErrorCode::ModelInvalid
                });
            }
        }
        report
    }
}

impl Default for SetupStatus {
    fn default() -> Self {
        Self {
            stage: SetupStage::Idle,
            step: WizardStep::Preflight,
            component: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            runtime: placeholder_component(SetupComponent::Runtime),
            model: placeholder_component(SetupComponent::Model),
            plan: plan_space(PlanInputs {
                model_bytes: 0,
                runtime_archive_bytes: 0,
                runtime_unpacked_bytes: 0,
                runtime: ComponentPlanKind::Keep,
                model: ModelInstallKind::Keep,
                runtime_retained_bytes: 0,
                model_retained_bytes: 0,
                available_bytes: 0,
            }),
            offer: SetupOffer {
                runtime: placeholder_offer(SetupComponent::Runtime),
                model: placeholder_offer(SetupComponent::Model),
                install_root: INSTALL_ROOT_CODE.to_string(),
                internet_needed_for_download_only: true,
                works_offline_after_install: true,
                steps: WizardStep::all().to_vec(),
                stage_codes: SetupStage::all()
                    .iter()
                    .map(|stage| stage.code().to_string())
                    .collect(),
            },
            recovery: RecoveryState::default(),
            error_code: None,
            warnings: Vec::new(),
            runtime_pre_release: false,
            managed_paths_ready: false,
            running: false,
            can_start: true,
            can_cancel: false,
            can_retry: false,
            can_cleanup: false,
            test: None,
        }
    }
}

fn placeholder_component(component: SetupComponent) -> ComponentStatus {
    ComponentStatus {
        component,
        state: ComponentState::Absent,
        version: None,
        bytes: 0,
        downloaded_bytes: 0,
        total_bytes: 0,
    }
}

fn placeholder_offer(component: SetupComponent) -> OfferComponent {
    OfferComponent {
        component,
        display_name: String::new(),
        version: String::new(),
        source_label: String::new(),
        license: None,
        download_bytes: 0,
        installed_bytes: 0,
        pre_release: false,
        checksum_verified: true,
        quantization: None,
        architecture: None,
        ram_recommendation_bytes: None,
        context_recommendation: None,
    }
}

fn build_offer(spec: &CatalogSpec) -> SetupOffer {
    SetupOffer {
        runtime: OfferComponent {
            component: SetupComponent::Runtime,
            display_name: format!("llama.cpp {}", spec.runtime_version),
            version: spec.runtime_version.clone(),
            source_label: spec.runtime_source_label.clone(),
            license: None,
            download_bytes: spec.runtime_archive.expected_size,
            installed_bytes: spec.runtime_plan_bytes,
            pre_release: spec.runtime_pre_release,
            checksum_verified: true,
            quantization: None,
            architecture: Some("x86-64".to_string()),
            ram_recommendation_bytes: None,
            context_recommendation: None,
        },
        model: OfferComponent {
            component: SetupComponent::Model,
            display_name: spec.model_display_name.clone(),
            version: spec.model_revision.clone(),
            source_label: spec.model_source_label.clone(),
            license: Some(spec.model_license.clone()),
            download_bytes: spec.model.expected_size,
            installed_bytes: spec.model.expected_size,
            pre_release: false,
            checksum_verified: true,
            quantization: Some(spec.model_quantization.clone()),
            architecture: Some(spec.model_architecture.clone()),
            ram_recommendation_bytes: Some(spec.model_ram_recommendation_bytes),
            context_recommendation: Some(spec.model_context_recommendation),
        },
        install_root: INSTALL_ROOT_CODE.to_string(),
        internet_needed_for_download_only: true,
        works_offline_after_install: true,
        steps: WizardStep::all().to_vec(),
        stage_codes: SetupStage::all()
            .iter()
            .map(|stage| stage.code().to_string())
            .collect(),
    }
}

impl Inner {
    /// Fills in the static part of the offer from the catalog.
    fn install_offer(&self, spec: &CatalogSpec) {
        let mut status = self.lock();
        status.offer = build_offer(spec);
        status.runtime_pre_release = spec.runtime_pre_release;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SetupStatus> {
        self.status.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// Resets the run-scoped fields and records the intent to install.
    fn begin_run(&self, resume: bool) {
        {
            let mut status = self.lock();
            status.stage = SetupStage::Preflight;
            status.step = WizardStep::Preflight;
            status.component = None;
            status.downloaded_bytes = 0;
            status.total_bytes = 0;
            status.error_code = None;
            status.warnings.clear();
            status.test = None;
            status.running = true;
            status.can_start = false;
            status.can_cancel = true;
            if resume {
                status.warnings.push(SetupWarningCode::DownloadResumed);
            }
        }
        self.persist(SetupStage::Preflight, None);
        self.emit();
    }

    fn enter_stage(&self, stage: SetupStage) {
        {
            let mut status = self.lock();
            status.stage = stage;
            status.step = stage.step();
            status.component = stage.component();
        }
        self.persist(stage, None);
        self.emit();
    }

    fn note_progress(&self, downloaded: u64, total: u64, component: SetupComponent) {
        {
            let mut status = self.lock();
            status.component = Some(component);
            status.downloaded_bytes = downloaded;
            status.total_bytes = total;
        }
        self.emit();
    }

    fn warn(&self, code: SetupWarningCode) {
        let mut status = self.lock();
        if !status.warnings.contains(&code) {
            status.warnings.push(code);
        }
    }

    fn set_scan(&self, installed: &InstalledState) {
        if let Ok(mut slot) = self.last_scan.lock() {
            *slot = installed.clone();
        }
    }

    fn set_plan(&self, plan: SpacePlan) {
        self.lock().plan = plan;
    }

    fn scan_now(&self) -> InstalledState {
        self.last_scan
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    fn persist(&self, stage: SetupStage, error: Option<SetupErrorCode>) {
        let mut record = PersistedSetup::at_stage(stage);
        record.error_code = error.map(|code| code.code().to_string());
        record.finished = stage == SetupStage::Complete;
        let _ = write_persisted(self.roots.data_dir(), &record);
    }

    fn finish(&self, stage: SetupStage, error: Option<SetupErrorCode>) {
        self.persist(stage, error);
        {
            let mut status = self.lock();
            status.stage = stage;
            status.step = stage.step();
            status.component = None;
            status.running = false;
            // A cancellation is an outcome the user asked for, not a failure to
            // explain; the retry flag still offers to continue.
            status.error_code = match error {
                Some(SetupErrorCode::Cancelled) => None,
                other => other,
            };
        }
        self.refresh();
        self.emit();
        if let Ok(mut slot) = self.sink.lock() {
            *slot = None;
        }
    }

    /// Rebuilds the disk-derived part of the status without touching the stage.
    fn refresh(&self) -> SetupStatus {
        let spec = self.catalog.spec();
        let installed = scan_with(&self.roots, &scan_names(&spec));
        let available = available_disk_bytes(self.roots.data_dir());
        self.set_scan(&installed);
        let recovery = recovery_state(self.roots.data_dir(), &installed);
        let plan = plan_for(&installed, available, &spec);
        let runtime = component_status(SetupComponent::Runtime, &installed, &spec);
        let model = component_status(SetupComponent::Model, &installed, &spec);
        let managed_paths_ready = self
            .roots
            .runtime_server_path_named(spec.runtime_server_name)
            .is_file()
            && self.roots.model_path_named(&spec.model.filename).is_file();

        {
            let mut status = self.lock();
            status.runtime = runtime;
            status.model = model;
            status.plan = plan;
            status.recovery = recovery;
            status.managed_paths_ready = managed_paths_ready;
            if !status.stage.is_terminal() && status.stage == SetupStage::Idle {
                // A restart that found an unfinished run explains it again.
                if let Some(stage) = status.recovery.interrupted_stage {
                    status.stage = stage;
                    status.step = stage.step();
                    status.error_code = status.recovery.previous_error_code;
                }
            }
            if available.is_none() && !status.warnings.contains(&SetupWarningCode::SpaceUnknown) {
                status.warnings.push(SetupWarningCode::SpaceUnknown);
            }
            if let Some(free) = available {
                if plan.fits()
                    && plan.required_peak_bytes.saturating_mul(2) > free
                    && !status.warnings.contains(&SetupWarningCode::SpaceIsTight)
                {
                    status.warnings.push(SetupWarningCode::SpaceIsTight);
                }
            }
            status.runtime_pre_release = spec.runtime_pre_release;
        }
        self.decorate(false)
    }

    /// Applies the permission flags, which depend on whether a run is in flight.
    fn decorate(&self, running: bool) -> SetupStatus {
        let installed = self.scan_now();
        let has_temp = installed.partial_bytes() > 0
            || installed.owned_temp_directories > 0
            || installed.runtime_staging_ready
            || installed.model_staging_ready;
        let mut status = self.lock().clone();
        status.running = running;
        status.can_start = !running;
        status.can_cancel = running && !status.stage.is_terminal();
        status.can_retry = !running
            && (status.stage == SetupStage::Cancelled
                || status.error_code.is_some()
                || status.recovery.partial_download_bytes > 0
                || status.recovery.staging_ready);
        status.can_cleanup = !running && has_temp;
        status.step = status.stage.step();
        status
    }

    fn emit(&self) {
        let sink = match self.sink.lock() {
            Ok(slot) => slot.clone(),
            Err(poison) => poison.into_inner().clone(),
        };
        let Some(sink) = sink else {
            return;
        };
        let event = {
            let status = self.lock();
            SetupEvent {
                stage: status.stage,
                component: status.component,
                downloaded_bytes: status.downloaded_bytes,
                total_bytes: status.total_bytes,
            }
        };
        sink(event);
    }
}

/// The file names a catalog pins, as the scanner needs them.
fn scan_names(spec: &CatalogSpec) -> ScanNames<'_> {
    ScanNames {
        runtime_archive_filename: &spec.runtime_archive.filename,
        model_filename: &spec.model.filename,
        model_expected_size: spec.model.expected_size,
    }
}

/// Builds the plan for what is actually on the disk.
pub fn plan_for(
    installed: &InstalledState,
    available: Option<u64>,
    spec: &CatalogSpec,
) -> SpacePlan {
    let runtime = if installed.runtime_ready {
        ComponentPlanKind::Keep
    } else {
        ComponentPlanKind::Install
    };
    // A managed model that is present Р Р†Р вЂљРІР‚Сњ usable or damaged Р Р†Р вЂљРІР‚Сњ is replaced in
    // place, and the previous directory is kept until the swap. Only a machine
    // with no managed model at all gets the single-copy "clean" plan.
    let has_managed_model = installed.model_bytes > 0 || installed.previous_model_bytes > 0;
    let model = if installed.model_ready {
        ModelInstallKind::Keep
    } else if has_managed_model {
        ModelInstallKind::Update
    } else {
        ModelInstallKind::Clean
    };
    let retained = if model == ModelInstallKind::Update {
        installed.model_bytes.max(installed.previous_model_bytes)
    } else {
        0
    };
    plan_space(PlanInputs {
        model_bytes: spec.model.expected_size,
        runtime_archive_bytes: spec.runtime_archive.expected_size,
        runtime_unpacked_bytes: spec.runtime_plan_bytes,
        runtime,
        model,
        runtime_retained_bytes: installed.previous_runtime_bytes,
        model_retained_bytes: retained,
        available_bytes: available.unwrap_or(0),
    })
}

/// Builds the status of one component from the scan.
fn component_status(
    component: SetupComponent,
    installed: &InstalledState,
    spec: &CatalogSpec,
) -> ComponentStatus {
    match component {
        SetupComponent::Runtime => {
            let state = if installed.runtime_ready {
                ComponentState::Ready
            } else if installed.runtime_damaged {
                ComponentState::Damaged
            } else if installed.runtime_staging_ready
                || installed.runtime_archive_bytes > 0
                || installed.owned_temp_directories > 0
            {
                ComponentState::Partial
            } else {
                ComponentState::Absent
            };
            ComponentStatus {
                component,
                state,
                version: (installed.runtime_ready || installed.runtime_damaged)
                    .then(|| spec.runtime_version.clone()),
                bytes: installed.runtime_bytes,
                downloaded_bytes: installed.runtime_archive_bytes,
                total_bytes: spec.runtime_archive.expected_size,
            }
        }
        SetupComponent::Model => {
            let state = if installed.model_ready {
                ComponentState::Ready
            } else if installed.model_damaged {
                ComponentState::Damaged
            } else if installed.model_staging_ready || installed.model_part_bytes > 0 {
                ComponentState::Partial
            } else {
                ComponentState::Absent
            };
            ComponentStatus {
                component,
                state,
                version: (installed.model_ready || installed.model_damaged)
                    .then(|| spec.model_revision.clone()),
                bytes: installed.model_bytes,
                downloaded_bytes: installed.model_part_bytes,
                total_bytes: spec.model.expected_size,
            }
        }
    }
}

/// The archive rules for a catalog.
fn archive_spec(spec: &CatalogSpec) -> RuntimeArchiveSpec<'_> {
    RuntimeArchiveSpec {
        allowed: &spec.runtime_allowed_entries,
        keep: &spec.runtime_installed_entries,
        max_files: MAX_RUNTIME_ARCHIVE_FILES.min(spec.runtime_allowed_entries.len() + 8),
        max_unpacked_bytes: spec.runtime_plan_bytes.max(MAX_RUNTIME_UNPACKED_BYTES),
        max_compression_ratio: MAX_RUNTIME_COMPRESSION_RATIO,
        require_pe_x64: true,
        server_name: spec.runtime_server_name,
    }
}

fn map_layout_error(error: LayoutError) -> SetupErrorCode {
    match error {
        LayoutError::DestinationExists => SetupErrorCode::DestinationExists,
        LayoutError::SourceMissing => SetupErrorCode::RuntimeMissing,
        LayoutError::NotSameVolume => SetupErrorCode::NotSameVolume,
        LayoutError::NotOwned => SetupErrorCode::NotOwned,
        LayoutError::Io => SetupErrorCode::Io,
    }
}

fn map_download_error(error: DownloadError) -> SetupErrorCode {
    match error {
        DownloadError::RefusedUrl => SetupErrorCode::RefusedUrl,
        DownloadError::Destination => SetupErrorCode::Io,
        DownloadError::TooLarge => SetupErrorCode::TooLarge,
        DownloadError::SizeMismatch => SetupErrorCode::SizeMismatch,
        DownloadError::HashMismatch => SetupErrorCode::HashMismatch,
        DownloadError::ContentLengthMismatch => SetupErrorCode::ContentLengthMismatch,
        DownloadError::RangeMismatch => SetupErrorCode::RangeMismatch,
        DownloadError::IdentityChanged => SetupErrorCode::IdentityChanged,
        DownloadError::Cancelled => SetupErrorCode::Cancelled,
        DownloadError::Network => SetupErrorCode::Network,
        DownloadError::Timeout => SetupErrorCode::Timeout,
        DownloadError::NoProgress => SetupErrorCode::NoProgress,
        DownloadError::Io => SetupErrorCode::Io,
    }
}

fn map_runtime_error(error: super::super::managed::RuntimeInstallError) -> SetupErrorCode {
    use super::super::managed::RuntimeInstallError as E;
    match error {
        E::ArchiveInvalid => SetupErrorCode::ArchiveInvalid,
        E::ArchivePathTraversal => SetupErrorCode::ArchivePathTraversal,
        E::ArchiveTooLarge => SetupErrorCode::TooLarge,
        E::ArchiveUnexpectedFile => SetupErrorCode::ArchiveUnexpectedFile,
        E::RuntimeMissing => SetupErrorCode::RuntimeMissing,
        E::RuntimeArchitectureMismatch => SetupErrorCode::RuntimeArchitectureMismatch,
        E::Io => SetupErrorCode::Io,
    }
}

/// Checks a model file. `hash_verified` is set when the caller has just hashed
/// exactly these bytes.
fn check_model(path: &Path, spec: &CatalogSpec, hash_verified: bool) -> Result<(), SetupErrorCode> {
    let metadata = path.metadata().map_err(|_| SetupErrorCode::Io)?;
    if !metadata.is_file() {
        return Err(SetupErrorCode::ModelInvalid);
    }
    if metadata.len() != spec.model.expected_size {
        return Err(SetupErrorCode::SizeMismatch);
    }
    if !hash_verified {
        let actual = sha256_file(path).map_err(|_| SetupErrorCode::Io)?;
        if !actual.eq_ignore_ascii_case(&spec.model.sha256) {
            return Err(SetupErrorCode::HashMismatch);
        }
    }
    let info = read_gguf_info(path).map_err(|_| SetupErrorCode::ModelInvalid)?;
    if !info
        .architecture
        .as_deref()
        .is_some_and(|architecture| architecture.eq_ignore_ascii_case(&spec.model_architecture))
    {
        return Err(SetupErrorCode::ModelArchitectureMismatch);
    }
    if info.quantisation.as_deref() != Some(spec.model_quantization.as_str()) {
        return Err(SetupErrorCode::ModelQuantizationMismatch);
    }
    Ok(())
}

fn directory_is_empty(directory: &Path) -> bool {
    fs::read_dir(directory)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

/// One run, with the cancellation flag and the pinned artifacts.
struct Run<'a> {
    inner: &'a Inner,
    cancel: &'a AtomicBool,
    spec: CatalogSpec,
}

impl Run<'_> {
    fn check_cancelled(&self) -> Result<(), SetupErrorCode> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(SetupErrorCode::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Enters a stage, refusing to continue once cancellation was asked for.
    fn enter(&self, stage: SetupStage) -> Result<(), SetupErrorCode> {
        self.check_cancelled()?;
        self.inner.enter_stage(stage);
        Ok(())
    }

    /// Downloads a pinned artifact into its `.part` file, with progress.
    fn download(
        &self,
        expectation: &ArtifactExpectation,
        part: &Path,
        component: SetupComponent,
    ) -> Result<(), SetupErrorCode> {
        let inner = self.inner;
        let cancel = self.cancel;
        download_artifact(
            inner.transport.as_ref(),
            DownloadRequest {
                expectation,
                part,
                trust: self.spec.trust,
            },
            |progress| inner.note_progress(progress.downloaded, progress.total, component),
            || cancel.load(Ordering::Relaxed),
        )
        .map(|_| ())
        .map_err(map_download_error)
    }

    /// Promotes the verified runtime payload into its final directory.
    fn activate_runtime(&self) -> Result<PathBuf, SetupErrorCode> {
        let roots = &self.inner.roots;
        let final_dir = roots.runtime_dir();
        let server = final_dir.join(self.spec.runtime_server_name);
        if server.is_file() {
            validate_pe_x64(&server).map_err(map_runtime_error)?;
            return Ok(server);
        }
        let payload = roots.payload_dir(SetupComponent::Runtime.staging_name());
        if !payload.join(self.spec.runtime_server_name).is_file() {
            return Err(SetupErrorCode::RuntimeMissing);
        }
        if final_dir.exists() {
            // A directory at the pinned version path that has no server is either
            // this application's own leftover or empty. Anything else is refused
            // rather than deleted.
            if !has_installation_receipt(&final_dir, "llama.cpp") && !directory_is_empty(&final_dir)
            {
                return Err(SetupErrorCode::DestinationExists);
            }
            fs::remove_dir_all(&final_dir).map_err(|_| SetupErrorCode::Io)?;
        }
        commit_staging_dir(&payload, &final_dir).map_err(map_layout_error)?;
        let server = final_dir.join(self.spec.runtime_server_name);
        if let Err(error) = validate_pe_x64(&server) {
            let _ = fs::remove_dir_all(&final_dir);
            return Err(map_runtime_error(error));
        }
        let facts = InstallationFacts {
            kind: "llama.cpp".to_string(),
            version: self.spec.runtime_version.clone(),
            artifact_name: self.spec.runtime_archive.filename.clone(),
            sha256: self.spec.runtime_archive.sha256.clone(),
            size_bytes: self.spec.runtime_archive.expected_size,
            files: self
                .spec
                .runtime_installed_entries
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        };
        write_installation_receipt(&final_dir, &facts).map_err(|_| SetupErrorCode::Io)?;
        Ok(server)
    }

    /// Promotes the verified model payload into its final directory, keeping a
    /// previously installed model until the swap.
    fn activate_model(&self) -> Result<PathBuf, SetupErrorCode> {
        let roots = &self.inner.roots;
        let final_dir = roots.model_dir();
        let final_model = final_dir.join(&self.spec.model.filename);
        if final_model.is_file() {
            check_model(&final_model, &self.spec, false)?;
            return Ok(final_model);
        }
        let payload = roots.payload_dir(SetupComponent::Model.staging_name());
        let staged = payload.join(&self.spec.model.filename);
        if !staged.is_file() {
            return Err(SetupErrorCode::ModelInvalid);
        }
        if final_dir.exists() {
            if has_installation_receipt(&final_dir, "model") {
                // The previous version stays on the disk, renamed, until the new
                // one is in place: that is what makes a rollback possible.
                let previous = roots.previous_dir(SetupComponent::Model.staging_name());
                retain_previous(&final_dir, &previous).map_err(map_layout_error)?;
                self.inner.warn(SetupWarningCode::PreviousVersionRetained);
            } else if directory_is_empty(&final_dir) {
                fs::remove_dir_all(&final_dir).map_err(|_| SetupErrorCode::Io)?;
            } else {
                return Err(SetupErrorCode::DestinationExists);
            }
        }
        commit_staging_dir(&payload, &final_dir).map_err(map_layout_error)?;
        // The SHA-256 of exactly these bytes was verified before the promotion,
        // and the promotion was a rename, so only the metadata is re-checked.
        check_model(&final_model, &self.spec, true)?;
        let facts = InstallationFacts {
            kind: "model".to_string(),
            version: self.spec.model_revision.clone(),
            artifact_name: self.spec.model.filename.clone(),
            sha256: self.spec.model.sha256.clone(),
            size_bytes: self.spec.model.expected_size,
            files: vec![self.spec.model.filename.clone()],
        };
        write_installation_receipt(&final_dir, &facts).map_err(|_| SetupErrorCode::Io)?;
        Ok(final_model)
    }
}

/// The whole installation, in the documented order.
fn execute(inner: &Inner, cancel: &AtomicBool, request: StartRequest) -> Result<(), SetupErrorCode> {
    let spec = inner.catalog.spec();
    let run = Run {
        inner,
        cancel,
        spec,
    };
    let roots = &inner.roots;

    // ------------------------------------------------------------- preflight
    run.enter(SetupStage::Preflight)?;
    fs::create_dir_all(roots.data_dir()).map_err(|_| SetupErrorCode::Io)?;
    if !roots.staging_is_on_the_same_volume() {
        // Without one volume every promotion would be a copy, and the space the
        // user was promised would be wrong.
        return Err(SetupErrorCode::NotSameVolume);
    }
    let installed = scan_with(roots, &scan_names(&run.spec));
    let available = available_disk_bytes(roots.data_dir());
    inner.set_scan(&installed);
    let plan = plan_for(&installed, available, &run.spec);
    inner.set_plan(plan);
    // An unreadable volume size is a warning, not a refusal: the plan is still
    // reported and the interface says the free space is unknown.
    if available.is_some() && !plan.fits() {
        return Err(SetupErrorCode::InsufficientSpace);
    }
    {
        let mut status = inner.lock();
        status.recovery = recovery_state(roots.data_dir(), &installed);
    }
    inner.emit();

    // --------------------------------------------------------------- runtime
    if installed.runtime_ready {
        inner.warn(SetupWarningCode::RuntimeAlreadyInstalled);
    } else {
        let staging = prepare_staging(roots, SetupComponent::Runtime.staging_name())
            .map_err(map_layout_error)?;
        let archive = staging.join(&run.spec.runtime_archive.filename);
        let part = staging.join(format!("{}.part", run.spec.runtime_archive.filename));

        run.enter(SetupStage::DownloadRuntime)?;
        if archive.is_file() {
            // A previous run already fetched and verified it.
            inner.warn(SetupWarningCode::DownloadResumed);
        } else {
            run.download(&run.spec.runtime_archive, &part, SetupComponent::Runtime)?;
            promote_file(&part, &archive).map_err(map_layout_error)?;
        }

        run.enter(SetupStage::ValidateRuntime)?;
        let archive_rules = archive_spec(&run.spec);
        let unpacked = validate_runtime_archive(&archive, &archive_rules).map_err(map_runtime_error)?;
        inner.lock().runtime.total_bytes = unpacked;

        run.enter(SetupStage::ExtractRuntime)?;
        let payload = roots.payload_dir(SetupComponent::Runtime.staging_name());
        extract_runtime_archive_with(&archive, &payload, &archive_rules)
            .map_err(map_runtime_error)?;
        // Only now, with a verified payload on the disk, is the archive dropped.
        let _ = fs::remove_file(&archive);

        run.enter(SetupStage::ActivateRuntime)?;
        run.activate_runtime()?;
        // The staging directory holds only this component's own files.
        if is_owned_temp(&staging) {
            let _ = discard_staging(roots, SetupComponent::Runtime.staging_name());
        }
    }

    // ----------------------------------------------------------------- model
    let installed = scan_with(roots, &scan_names(&run.spec));
    if installed.model_ready {
        inner.warn(SetupWarningCode::ModelAlreadyInstalled);
    } else {
        let staging = prepare_staging(roots, SetupComponent::Model.staging_name())
            .map_err(map_layout_error)?;
        let payload = roots.payload_dir(SetupComponent::Model.staging_name());
        fs::create_dir_all(&payload).map_err(|_| SetupErrorCode::Io)?;
        let part = payload.join(format!("{}.part", run.spec.model.filename));
        let staged = payload.join(&run.spec.model.filename);

        run.enter(SetupStage::DownloadModel)?;
        if staged.is_file() {
            inner.warn(SetupWarningCode::StagingReused);
        } else {
            run.download(&run.spec.model, &part, SetupComponent::Model)?;
        }

        run.enter(SetupStage::ValidateModel)?;
        if staged.is_file() {
            // A verified copy from an interrupted run: it has not been hashed in
            // this session, so it is hashed before it may be activated.
            check_model(&staged, &run.spec, false)?;
        } else {
            // The download verified exactly these bytes, and the promotion below
            // is a rename, so the hash is not repeated.
            check_model(&part, &run.spec, true)?;
            promote_file(&part, &staged).map_err(map_layout_error)?;
        }

        run.enter(SetupStage::ActivateModel)?;
        run.activate_model()?;
        if is_owned_temp(&staging) {
            let _ = discard_staging(roots, SetupComponent::Model.staging_name());
        }
    }

    // ------------------------------------------------------------- configure
    run.enter(SetupStage::Configure)?;
    let runtime_path = roots.runtime_server_path_named(run.spec.runtime_server_name);
    let model_path = roots.model_path_named(&run.spec.model.filename);
    if !runtime_path.is_file() || !model_path.is_file() {
        return Err(SetupErrorCode::Interrupted);
    }
    inner.lock().managed_paths_ready = true;
    if !request.consent_managed_paths {
        // Without consent the two verified paths are reported, not applied: the
        // configuration the user chose by hand is left exactly as it is.
        inner.warn(SetupWarningCode::ManualSettingsPreserved);
    }
    inner.warn(SetupWarningCode::InternetRequiredForDownload);
    if run.spec.runtime_pre_release {
        inner.warn(SetupWarningCode::RuntimePreRelease);
    }

    run.check_cancelled()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::local::setup::download::{identity_path, PartIdentity};
    use crate::ai::local::setup::fake_http::{FakeResponse, FakeServer};
    use crate::ai::local::setup::fixtures;
    use tempfile::tempdir;

    /// Shaped like the real thing: the model is much larger than the runtime, so
    /// the plan's peak is one model and not the extraction window.
    const MODEL_BYTES: u64 = 8 * 1024 * 1024;
    /// Big enough that a cancelled transfer is unmistakably partial.
    const RUNTIME_FILLER: usize = 512 * 1024;
    /// Planned footprint of the fixture runtime. Smaller than the model, as the
    /// pinned runtime is smaller than the pinned model.
    const RUNTIME_PLAN_BYTES_FIXTURE: u64 = 2 * 1024 * 1024;
    /// Bytes after which a test asks the run to stop.
    const CANCEL_AFTER_BYTES: u64 = 4096;

    struct FixtureCatalog {
        spec: CatalogSpec,
    }

    impl SetupCatalog for FixtureCatalog {
        fn spec(&self) -> CatalogSpec {
            self.spec.clone()
        }
    }

    /// The standard runtime archive: a real PE stub and two libraries.
    fn runtime_archive() -> Vec<u8> {
        fixtures::zip_archive(&[
            ("llama-server.exe", fixtures::pe_x64()),
            ("llama.dll", fixtures::bytes(RUNTIME_FILLER, 3)),
            ("ggml.dll", fixtures::bytes(1024, 4)),
        ])
    }

    /// A catalog whose pinned identities match exactly these bytes.
    fn catalog_for(server: &FakeServer, archive: &[u8], model: &[u8]) -> Arc<dyn SetupCatalog> {
        Arc::new(FixtureCatalog {
            spec: CatalogSpec {
                trust: ArtifactTrust::LoopbackTesting,
                runtime_version: "b-test".to_string(),
                runtime_pre_release: true,
                runtime_source_label: "example.test/ggml".to_string(),
                runtime_archive: ArtifactExpectation {
                    url: server.url("runtime.zip"),
                    filename: "runtime.zip".to_string(),
                    expected_size: archive.len() as u64,
                    sha256: fixtures::sha256_hex(archive),
                },
                runtime_allowed_entries: vec!["llama-server.exe", "llama.dll", "ggml.dll"],
                runtime_installed_entries: vec!["llama-server.exe", "llama.dll", "ggml.dll"],
                runtime_server_name: "llama-server.exe",
                runtime_plan_bytes: RUNTIME_PLAN_BYTES_FIXTURE,
                model_id: "test/model".to_string(),
                model_display_name: "Test Model".to_string(),
                model_revision: "rev-test".to_string(),
                model_source_label: "example.test/model".to_string(),
                model_license: "Apache-2.0".to_string(),
                model_quantization: "Q4_K_M".to_string(),
                model_architecture: "qwen3".to_string(),
                model_context_recommendation: 8192,
                model_ram_recommendation_bytes: 14 * 1024 * 1024 * 1024,
                model: ArtifactExpectation {
                    url: server.url("model.gguf"),
                    filename: "model.gguf".to_string(),
                    expected_size: model.len() as u64,
                    sha256: fixtures::sha256_hex(model),
                },
            },
        })
    }

    fn build(directory: &Path, server: &FakeServer, archive: &[u8], model: &[u8]) -> SetupCoordinator {
        SetupCoordinator::with_parts(
            directory,
            catalog_for(server, archive, model),
            Arc::new(ReqwestTransport::new().unwrap()),
        )
    }

    fn coordinator(
        directory: &Path,
        server: &FakeServer,
    ) -> (SetupCoordinator, Vec<u8>, Vec<u8>) {
        let archive = runtime_archive();
        let model = fixtures::pinned_gguf(MODEL_BYTES);
        let coordinator = build(directory, server, &archive, &model);
        (coordinator, archive, model)
    }

    /// Fails with the reason, not just the stage, so a broken fixture is obvious.
    fn assert_complete(status: &SetupStatus) {
        assert_eq!(
            status.stage,
            SetupStage::Complete,
            "error={:?} warnings={:?} plan={:?}",
            status.error_code,
            status.warnings,
            status.plan
        );
    }

    /// The installed model file of the fixture catalog.
    fn installed_model(roots: &ManagedRoots) -> PathBuf {
        roots.model_path_named("model.gguf")
    }

    fn wait_for_settle(coordinator: &SetupCoordinator) -> SetupStatus {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let status = coordinator.status();
            if !status.running {
                return status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the installation did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn run_install(coordinator: &SetupCoordinator, resume: bool) -> SetupStatus {
        coordinator
            .start(
                StartRequest {
                    consent_managed_paths: true,
                    resume,
                },
                None,
            )
            .expect("the slot is free");
        wait_for_settle(coordinator)
    }

    /// Cancels the run as soon as the download has delivered a few kilobytes.
    fn cancelling_sink(coordinator: &SetupCoordinator) -> Option<SetupEventSink> {
        let handle = coordinator.clone();
        Some(Arc::new(move |event: SetupEvent| {
            if event.downloaded_bytes >= CANCEL_AFTER_BYTES {
                handle.cancel();
            }
        }))
    }

    #[test]
    fn a_clean_install_serves_both_artifacts_and_activates_them() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        // Before anything runs, the plan is the one the page shows: one full
        // model in the temporary tree and nothing kept for rollback.
        let before = coordinator.status();
        assert_eq!(before.plan.temporary_bytes, model.len() as u64);
        assert_eq!(before.plan.rollback_bytes, 0);
        assert!(before.plan.fits());

        server.push(FakeResponse::full(archive.clone()));
        server.push(FakeResponse::full(model.clone()));

        let status = run_install(&coordinator, false);
        assert_complete(&status);
        assert_eq!(status.error_code, None);
        assert!(status.managed_paths_ready);
        assert_eq!(status.runtime.state, ComponentState::Ready);
        assert_eq!(status.model.state, ComponentState::Ready);
        // Nothing is left to install, so the remaining plan asks for nothing.
        assert_eq!(status.plan.download_bytes, 0);
        assert_eq!(status.plan.installed_bytes, 0);

        // The files are exactly what the fixtures hold, in the managed tree.
        let roots = ManagedRoots::new(directory.path());
        assert_eq!(
            fs::read(roots.runtime_server_path()).unwrap(),
            fixtures::pe_x64()
        );
        assert_eq!(fs::read(installed_model(&roots)).unwrap(), model);
        assert!(roots.runtime_dir().join("llama.dll").is_file());
        // A receipt is what authorizes a later removal.
        assert!(has_installation_receipt(&roots.runtime_dir(), "llama.cpp"));
        assert!(has_installation_receipt(&roots.model_dir(), "model"));
        // No temporary directory survives a success.
        let leftovers = fs::read_dir(roots.temp_root())
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn a_second_installation_is_refused_while_one_is_running() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        // A slow answer, so the run is still inside the download when the second
        // start is attempted.
        server.push(FakeResponse::throttled(archive.clone(), 4096));
        server.push(FakeResponse::full(model.clone()));

        coordinator
            .start(
                StartRequest {
                    consent_managed_paths: true,
                    resume: false,
                },
                cancelling_sink(&coordinator),
            )
            .unwrap();
        // Only one installation at a time, whatever the interface does.
        assert_eq!(
            coordinator
                .start(StartRequest::default(), None)
                .unwrap_err(),
            SetupErrorCode::AlreadyRunning
        );
        assert!(coordinator.is_running());
        assert!(coordinator.cancel());
        let status = wait_for_settle(&coordinator);
        assert_eq!(status.stage, SetupStage::Cancelled);
        assert!(!coordinator.is_running());
        // A cancelled run can be retried.
        assert!(status.can_retry);
    }

    #[test]
    fn a_cancelled_download_keeps_its_partial_file_and_a_retry_resumes_it() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        // A slow answer, so the cancellation lands in the middle of the transfer.
        server.push(FakeResponse::throttled(archive.clone(), 4096));
        server.push(FakeResponse::full(archive.clone()));
        server.push(FakeResponse::full(model.clone()));

        let roots = ManagedRoots::new(directory.path());
        coordinator
            .start(
                StartRequest {
                    consent_managed_paths: true,
                    resume: false,
                },
                cancelling_sink(&coordinator),
            )
            .unwrap();
        let status = wait_for_settle(&coordinator);
        assert_eq!(status.stage, SetupStage::Cancelled);

        // The partial file is what makes the retry a resume.
        let part = roots.staging_dir("runtime").join("runtime.zip.part");
        assert!(part.is_file());
        let partial = fs::metadata(&part).unwrap().len();
        assert!(partial > 0, "some bytes must have arrived");
        assert!(partial < archive.len() as u64);

        // A retry finishes the job and leaves no temporary directory behind.
        let status = run_install(&coordinator, true);
        assert_complete(&status);
        assert_eq!(
            fs::read(roots.runtime_server_path()).unwrap(),
            fixtures::pe_x64()
        );
        let leftovers = fs::read_dir(roots.temp_root())
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn a_restart_recognises_a_part_file_and_finishes_with_a_range_request() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let roots = ManagedRoots::new(directory.path());
        let archive = runtime_archive();
        let model = fixtures::pinned_gguf(MODEL_BYTES);

        // A crash between the last written byte and the verification: the
        // `.part` file and the identity of the resource are both on the disk.
        fs::create_dir_all(roots.staging_dir("runtime")).unwrap();
        super::super::layout::mark_temp_directory(&roots.staging_dir("runtime"), "runtime").unwrap();
        let half = &archive[..archive.len() / 2];
        let part = roots.staging_dir("runtime").join("runtime.zip.part");
        fs::write(&part, half).unwrap();
        crate::fsutil::write_json_atomic(
            &identity_path(&part),
            &PartIdentity {
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                expected_size: archive.len() as u64,
                sha256: fixtures::sha256_hex(&archive),
            },
        )
        .unwrap();
        write_persisted(
            directory.path(),
            &PersistedSetup::at_stage(SetupStage::DownloadRuntime),
        )
        .unwrap();

        // A brand new coordinator sees the partial file and the record.
        let coordinator = build(directory.path(), &server, &archive, &model);
        let status = coordinator.status();
        assert_eq!(status.recovery.partial_download_bytes, half.len() as u64);
        assert!(status.recovery.interrupted);
        assert!(status.can_retry);
        assert_eq!(status.runtime.state, ComponentState::Partial);

        // The retry asks for a range and gets only the rest.
        server.push(FakeResponse::ranged(archive.clone()));
        server.push(FakeResponse::full(model.clone()));
        let status = run_install(&coordinator, true);
        assert_complete(&status);
        assert!(
            server
                .requests()
                .iter()
                .any(|request| request.range_start == Some(half.len() as u64)),
            "the retry must resume with a range request"
        );
        assert_eq!(
            fs::read(roots.runtime_server_path()).unwrap(),
            fixtures::pe_x64()
        );
    }

    #[test]
    fn a_verified_staging_copy_is_activated_without_downloading_again() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, _archive, model) = coordinator(directory.path(), &server);
        let roots = ManagedRoots::new(directory.path());
        // A runtime is already installed, and the model is staged and complete.
        fs::create_dir_all(roots.runtime_dir()).unwrap();
        fs::write(roots.runtime_server_path(), fixtures::pe_x64()).unwrap();
        write_installation_receipt(
            &roots.runtime_dir(),
            &InstallationFacts {
                kind: "llama.cpp".to_string(),
                version: "b-test".to_string(),
                artifact_name: "runtime.zip".to_string(),
                sha256: "a".repeat(64),
                size_bytes: 1,
                files: vec![],
            },
        )
        .unwrap();
        fs::create_dir_all(roots.payload_dir("model")).unwrap();
        fs::write(roots.payload_dir("model").join("model.gguf"), model.clone()).unwrap();

        let status = run_install(&coordinator, false);
        assert_complete(&status);
        assert!(status
            .warnings
            .contains(&SetupWarningCode::RuntimeAlreadyInstalled));
        assert!(status.warnings.contains(&SetupWarningCode::StagingReused));
        // Nothing was requested from the server at all.
        assert!(server.requests().is_empty());
        assert_eq!(fs::read(installed_model(&roots)).unwrap(), model);
    }

    #[test]
    fn the_plan_checks_the_real_free_space_against_the_required_peak() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, _archive, _model) = coordinator(directory.path(), &server);

        // With one byte free nothing fits, and the shortfall is exactly reported.
        let tight = plan_for(
            &InstalledState::default(),
            Some(1),
            &coordinator.inner.catalog.spec(),
        );
        assert!(!tight.fits());
        assert_eq!(tight.available_bytes, 1);
        assert_eq!(tight.missing_bytes, tight.required_peak_bytes - 1);

        // On a real machine the same plan fits, and the status says so.
        let status = coordinator.status();
        assert!(status.plan.available_bytes > 0);
        assert_eq!(status.plan.missing_bytes, 0);
        assert!(status.can_start);
    }

    #[test]
    fn the_fixture_plan_matches_the_files_the_run_actually_writes() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        let spec = coordinator.inner.catalog.spec();
        let plan = plan_for(&InstalledState::default(), Some(u64::MAX), &spec);
        assert_eq!(plan.model_bytes, model.len() as u64);
        assert_eq!(plan.runtime.new_bytes, RUNTIME_PLAN_BYTES_FIXTURE);
        assert_eq!(plan.download_bytes, archive.len() as u64 + model.len() as u64);
        assert_eq!(
            plan.installed_bytes,
            RUNTIME_PLAN_BYTES_FIXTURE + model.len() as u64
        );
        assert_eq!(
            plan.required_peak_bytes,
            RUNTIME_PLAN_BYTES_FIXTURE + model.len() as u64 + 512 * 1024 * 1024
        );
        assert_eq!(plan.rollback_bytes, 0);
        // The temporary tree holds one model at its worst, exactly as the pinned
        // plan does: the model is larger than the archive plus the extraction.
        assert_eq!(plan.temporary_bytes, model.len() as u64);
    }

    #[test]
    fn a_failed_component_leaves_the_other_one_installed() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        server.push(FakeResponse::full(archive.clone()));
        // The model download fails on every attempt.
        for _ in 0..3 {
            server.push(FakeResponse::status(500));
        }

        let status = run_install(&coordinator, false);
        assert_eq!(status.stage, SetupStage::Failed);
        assert!(status.error_code.is_some());
        // The runtime that finished is still installed and usable.
        let roots = ManagedRoots::new(directory.path());
        assert_eq!(
            fs::read(roots.runtime_server_path()).unwrap(),
            fixtures::pe_x64()
        );
        assert_eq!(status.runtime.state, ComponentState::Ready);
        assert!(!installed_model(&roots).is_file());
        assert!(!has_installation_receipt(&roots.model_dir(), "model"));

        // A retry with a working server completes.
        server.push(FakeResponse::full(model.clone()));
        let status = run_install(&coordinator, true);
        assert_complete(&status);
        assert!(installed_model(&roots).is_file());
    }

    #[test]
    fn a_failed_hash_leaves_no_model_installed_and_can_be_retried() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        server.push(FakeResponse::full(archive.clone()));
        // A body of the right length that hashes to something else.
        server.push(FakeResponse::full(fixtures::bytes(MODEL_BYTES as usize, 99)));

        let status = run_install(&coordinator, false);
        assert_eq!(status.stage, SetupStage::Failed);
        assert_eq!(status.error_code, Some(SetupErrorCode::HashMismatch));
        let roots = ManagedRoots::new(directory.path());
        assert!(!installed_model(&roots).is_file());
        assert!(!roots.model_dir().exists());

        server.push(FakeResponse::full(model.clone()));
        let status = run_install(&coordinator, true);
        assert_complete(&status);
        assert_eq!(fs::read(installed_model(&roots)).unwrap(), model);
    }

    #[test]
    fn an_archive_whose_server_is_not_x86_64_never_becomes_a_runtime() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        // The catalog pins this archive, so the failure that is exercised is the
        // machine tag and not the hash.
        let bad = fixtures::zip_archive(&[
            ("llama-server.exe", fixtures::pe_stub(0x014c)),
            ("llama.dll", fixtures::bytes(RUNTIME_FILLER, 3)),
            ("ggml.dll", fixtures::bytes(1024, 4)),
        ]);
        let model = fixtures::pinned_gguf(MODEL_BYTES);
        let coordinator = build(directory.path(), &server, &bad, &model);
        server.push(FakeResponse::full(bad));
        server.push(FakeResponse::full(model));

        let status = run_install(&coordinator, false);
        assert_eq!(status.stage, SetupStage::Failed);
        assert_eq!(
            status.error_code,
            Some(SetupErrorCode::RuntimeArchitectureMismatch)
        );
        let roots = ManagedRoots::new(directory.path());
        assert!(!roots.runtime_server_path().is_file());
        // The already verified model was never reached, so nothing is installed.
        assert!(!installed_model(&roots).is_file());
    }

    #[test]
    fn cleanup_removes_only_owned_temporary_directories() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, _archive, _model) = coordinator(directory.path(), &server);
        let roots = ManagedRoots::new(directory.path());
        super::super::layout::mark_temp_directory(&roots.staging_dir("model"), "model").unwrap();
        fs::write(roots.staging_dir("model").join("junk.bin"), vec![0_u8; 1024]).unwrap();
        let foreign = roots.temp_root().join("user-folder");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("keep.txt"), b"keep").unwrap();

        let report = coordinator.cleanup_temp().unwrap();
        assert_eq!(report.removed_directories, 1);
        // The junk plus the small ownership marker.
        assert!(report.removed_bytes >= 1024);
        assert!(report.removed_bytes < 4096);
        assert_eq!(report.skipped_foreign, 1);
        assert!(foreign.join("keep.txt").is_file());
    }

    #[test]
    fn removal_refuses_a_directory_this_application_did_not_install() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, _archive, _model) = coordinator(directory.path(), &server);
        let roots = ManagedRoots::new(directory.path());
        // A directory in the managed location, but with no receipt of ours.
        fs::create_dir_all(roots.model_dir()).unwrap();
        fs::write(roots.model_dir().join("user-model.gguf"), b"mine").unwrap();
        assert_eq!(coordinator.remove_model(), Err(SetupErrorCode::NotOwned));
        assert!(roots.model_dir().join("user-model.gguf").is_file());
        // Removing something that is absent is not an error.
        assert_eq!(coordinator.remove_runtime(), Ok(0));
    }

    #[test]
    fn removal_deletes_an_installed_component_and_its_rollback_copy() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        server.push(FakeResponse::full(archive));
        server.push(FakeResponse::full(model));
        assert_complete(&run_install(&coordinator, false));

        let roots = ManagedRoots::new(directory.path());
        // A rollback copy from an earlier update goes with the component.
        let previous = roots.previous_dir("model");
        fs::create_dir_all(&previous).unwrap();
        fs::write(previous.join("model.gguf"), vec![0_u8; 512]).unwrap();

        let removed = coordinator.remove_model().unwrap();
        assert!(removed >= 512);
        assert!(!roots.model_dir().exists());
        assert!(!previous.exists());
        assert!(roots.runtime_server_path().is_file());

        let removed = coordinator.remove_runtime().unwrap();
        assert!(removed > 0);
        assert!(!roots.runtime_server_path().is_file());
    }

    #[test]
    fn validating_an_existing_installation_hashes_the_model() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        server.push(FakeResponse::full(archive));
        server.push(FakeResponse::full(model.clone()));
        run_install(&coordinator, false);

        let report = coordinator.validate_existing();
        assert_eq!(report.runtime_state, ComponentState::Ready);
        assert!(report.runtime_server_ok && report.runtime_architecture_ok);
        assert_eq!(report.model_state, ComponentState::Ready);
        assert!(report.model_size_ok && report.model_hash_ok && report.model_format_ok);
        assert_eq!(report.error_code, None);

        // A damaged model is reported as damaged, not as ready.
        let roots = ManagedRoots::new(directory.path());
        let mut damaged = model.clone();
        damaged[100] ^= 0xff;
        fs::write(installed_model(&roots), damaged).unwrap();
        let report = coordinator.validate_existing();
        assert_eq!(report.model_state, ComponentState::Damaged);
        assert!(!report.model_hash_ok);
        assert_eq!(report.error_code, Some(SetupErrorCode::HashMismatch));

        // A damaged server is reported too.
        fs::write(roots.runtime_server_path(), fixtures::not_a_pe()).unwrap();
        let report = coordinator.validate_existing();
        assert_eq!(report.runtime_state, ComponentState::Damaged);
        assert_eq!(
            report.error_code,
            Some(SetupErrorCode::RuntimeArchitectureMismatch)
        );
    }

    #[test]
    fn replacing_an_installed_model_is_planned_as_an_update_with_rollback_bytes() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        server.push(FakeResponse::full(archive));
        server.push(FakeResponse::full(model));
        assert_complete(&run_install(&coordinator, false));

        // A damaged installed model: the next run has to replace it. The cheap
        // scan notices that the file is no longer the pinned size.
        let roots = ManagedRoots::new(directory.path());
        fs::write(
            installed_model(&roots),
            fixtures::pinned_gguf(MODEL_BYTES / 2),
        )
        .unwrap();

        let spec = coordinator.inner.catalog.spec();
        let installed = scan_with(&roots, &scan_names(&spec));
        assert!(installed.model_damaged);
        let plan = plan_for(&installed, Some(u64::MAX), &spec);
        assert_eq!(plan.model.kind, ComponentPlanKind::Install);
        assert_eq!(plan.model.retained_bytes, installed.model_bytes);
        assert_eq!(plan.rollback_bytes, installed.model_bytes);
        assert_eq!(
            plan.required_peak_bytes,
            installed.model_bytes + model_len(&spec) + 512 * 1024 * 1024
        );
    }

    fn model_len(spec: &CatalogSpec) -> u64 {
        spec.model.expected_size
    }

    #[test]
    fn the_offer_describes_everything_the_page_shows_before_a_download() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, archive, model) = coordinator(directory.path(), &server);
        let status = coordinator.status();
        assert!(status.offer.runtime.pre_release);
        assert_eq!(status.offer.runtime.version, "b-test");
        assert_eq!(status.offer.runtime.source_label, "example.test/ggml");
        assert_eq!(status.offer.runtime.download_bytes, archive.len() as u64);
        assert_eq!(status.offer.model.download_bytes, model.len() as u64);
        assert_eq!(status.offer.model.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(status.offer.model.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(status.offer.install_root, INSTALL_ROOT_CODE);
        assert!(status.offer.internet_needed_for_download_only);
        assert!(status.offer.works_offline_after_install);
        assert_eq!(status.offer.steps.len(), 6);
        assert_eq!(status.offer.stage_codes.len(), 16);
        assert_eq!(status.offer.stage_codes[1], "preflight");
    }

    #[test]
    fn the_pinned_catalog_uses_the_compiled_manifests_and_nothing_else() {
        let spec = PinnedCatalog.spec();
        assert_eq!(spec.trust, ArtifactTrust::Pinned);
        assert_eq!(spec.runtime_version, "b10964");
        assert_eq!(spec.model.expected_size, 5_027_783_488);
        assert_eq!(spec.model_license, "Apache-2.0");
        assert!(spec.runtime_archive.url.starts_with("https://github.com/ggml-org/"));
        assert!(spec.model.url.starts_with("https://huggingface.co/Qwen/"));
        // The production trust level refuses a loopback address outright.
        let fake = ArtifactExpectation {
            url: "http://127.0.0.1:9/model.gguf".to_string(),
            filename: "model.gguf".to_string(),
            expected_size: 1,
            sha256: "a".repeat(64),
        };
        assert!(super::super::download::validate_expectation(&fake, spec.trust).is_err());
    }

    #[test]
    fn the_coordinator_never_exposes_a_staging_path_a_url_or_a_hash() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, _archive, _model) = coordinator(directory.path(), &server);
        let json = serde_json::to_string(&coordinator.status()).unwrap();
        // A source label is a host and a repository, never a fetchable URL.
        assert!(!json.contains("http://"));
        assert!(!json.contains("https://"));
        assert!(!json.contains("127.0.0.1"));
        // No absolute path and no internal staging directory.
        assert!(!json.contains("setup-temp"));
        assert!(!json.contains("payload"));
        assert!(!json.contains("\\\\?\\"));
        assert!(!json.contains(directory.path().to_string_lossy().as_ref()));
        // No hash and no command line.
        assert!(!json.contains("sha256"));
        assert!(!json.contains("--model"));
        assert!(!json.contains("llama-server.exe"));
        assert!(!json.contains("installation-receipt"));
    }

    #[test]
    fn source_labels_name_the_repository_without_becoming_a_url() {
        assert_eq!(
            source_label("https://github.com/ggml-org/llama.cpp/releases/download/b10964/x.zip"),
            "github.com/ggml-org/llama.cpp"
        );
        assert_eq!(
            source_label("https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/rev/file.gguf"),
            "huggingface.co/Qwen/Qwen3-8B-GGUF"
        );
        assert!(!source_label("https://example.test/a").contains("://"));
    }

    #[test]
    fn the_pinned_numbers_the_report_quotes_come_from_the_pure_planner() {
        let directory = tempdir().unwrap();
        let server = FakeServer::start();
        let (coordinator, _archive, _model) = coordinator(directory.path(), &server);
        assert!(coordinator.managed_paths().is_none());
        assert!(!coordinator.managed_ready());
        assert_eq!(super::super::plan::clean_install_required_bytes(), 5_665_317_696);
        assert_eq!(super::super::plan::update_required_bytes(), 10_693_101_184);
    }
}




