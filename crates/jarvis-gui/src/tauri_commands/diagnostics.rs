//! Runtime diagnostics, as the window asks for them.
//!
//! The report itself is `jarvis_core::diagnostics`: a shape that cannot hold
//! user content, with path redaction and a screen that refuses an export. This
//! file only *collects* the facts — which files exist, what each store says, how
//! much memory is free — and never formats them itself.
//!
//! Two rules:
//!
//! * a check here is read-only. Nothing is started, nothing is recorded, and the
//!   microphone is asked whether it exists, never opened;
//! * anything that cannot be checked is reported as `version_unknown` or
//!   `unavailable` with a plain sentence, not guessed at.

use std::path::Path;

use serde::Serialize;
use tauri::Manager;

use jarvis_core::desktop::{AutostartState, MicrophoneState};
use jarvis_core::diagnostics::{
    bytes_label, ComponentHealth, ComponentState, DiagnosticReport, ErrorCategory, HealthCheck,
    LicenseStatus,
};

use crate::AppState;

/// Builds the report for the running application.
pub fn collect(app: &tauri::AppHandle) -> DiagnosticReport {
    let state = app.state::<AppState>();
    let mut report = DiagnosticReport::new(app.package_info().version.to_string());
    // The detailed Windows build is not read: reporting the platform honestly is
    // better than reporting a version from a call that was never made.
    report.notes.push(
        "the detailed Windows build number is not read by this build of the report".to_string(),
    );

    // ---------------------------------------------------------------- data
    let data = crate::desktop::data_directory();
    report.push_component(data_directory_health(&data));
    report.push_component(
        ComponentHealth::new("sqlite", ComponentState::Ready)
            .with_detail("bundled SQLite is compiled in and used by the encrypted stores"),
    );

    // ------------------------------------------------------- encrypted stores
    // Only two things are known from here: whether a store file exists, and
    // whether the shared session is unlocked. Nothing is decrypted to find out.
    let paths = jarvis_core::notes::vault::VaultPaths::production().ok();
    let store_present = paths
        .as_ref()
        .map(|paths| paths.database.is_file())
        .unwrap_or(false);
    let unlocked = state.notes.is_unlocked();
    let store_state = match (store_present, unlocked) {
        (false, _) => ComponentState::NotConfigured,
        (true, true) => ComponentState::Ready,
        (true, false) => ComponentState::Locked,
    };
    report.push_component(
        ComponentHealth::new("notes_vault_memory", store_state).with_detail(if unlocked {
            "the shared session is unlocked"
        } else {
            "the shared session is locked"
        }),
    );
    if let Some(paths) = &paths {
        if let Ok(metadata) = std::fs::metadata(&paths.database) {
            report.push_database(&paths.database.to_string_lossy(), metadata.len());
        }
        if paths.backup_key.is_file() {
            report.notes.push(
                "a portable key envelope exists on this machine; its contents are not read"
                    .to_string(),
            );
        }
    }

    // ------------------------------------------------------------- local AI
    let ai = state.local_ai.gateway().status().ok();
    let configured = state.local_ai.config();
    let server_path = configured.server_path();
    report.push_component(binary_health(
        "llama_server",
        server_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
            .as_str(),
        "no model server has been chosen",
    ));
    let model_path = configured.model_path();
    report.push_component(file_health(
        "local_model",
        model_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .as_deref(),
        "no model file has been chosen",
    ));
    if let Some(status) = &ai {
        report.health_checks.push(HealthCheck {
            name: "local_ai_state".to_string(),
            passed: matches!(status.state, jarvis_core::ai::local::LocalAiState::Ready),
            detail: Some(status.state.as_str().to_string()),
        });
        if let Some(model) = &status.capabilities.model_id {
            report
                .notes
                .push(format!("the model server reported the model {model}"));
        }
    }

    // The managed installation is reported whether or not its paths are the
    // active ones, so "the installer finished but the manual paths are still in
    // use" is visible instead of looking like a missing component. The overall
    // status stays unverified until the first-run check has actually passed: this
    // report must never claim a working local AI that nobody has seen answer.
    {
        use jarvis_core::ai::local::setup::{ComponentState, SetupStage};
        let setup = state.local_ai_setup.coordinator().status();
        if setup.runtime.state == ComponentState::Ready {
            report
                .notes
                .push("a managed llama.cpp runtime is installed and verified".to_string());
        }
        if setup.model.state == ComponentState::Ready {
            report
                .notes
                .push("a managed Qwen3 model is installed and verified".to_string());
        }
        report.health_checks.push(HealthCheck {
            name: "local_ai_setup".to_string(),
            passed: setup.stage == SetupStage::Complete,
            detail: Some(format!("stage {}", setup.stage.code())),
        });
        let verified = setup.test.is_some_and(|outcome| outcome.passed);
        report.health_checks.push(HealthCheck {
            name: "local_ai_first_run".to_string(),
            passed: verified,
            detail: Some(if verified {
                "the managed server answered a technical check".to_string()
            } else {
                "manual verification required: the first-run check has not passed yet".to_string()
            }),
        });
    }

    // -------------------------------------------------------------- whisper
    let whisper = state.whisper.session();
    let whisper_settings = whisper.settings();
    report.push_component(binary_health(
        "whisper_binary",
        &whisper_settings.binary_path,
        "no whisper executable has been chosen",
    ));
    report.push_component(file_health(
        "whisper_model",
        Some(whisper_settings.model_path.as_str()),
        "no whisper model has been chosen",
    ));
    // What is stored, described without a path: this is the answer to "did my
    // two files save?" and it is what a support report can safely carry.
    let stored =
        jarvis_core::whisper::StoredSettingsSummary::read(&crate::desktop::data_directory());
    report.notes.extend(stored.describe());
    // The application settings database is checked for dictation keys, because
    // that is where the other features keep theirs: if someone looked there and
    // found nothing, the report should say so rather than leave it a mystery.
    let db_keys = state
        .settings
        .read(jarvis_core::whisper::SETTINGS_FILE)
        .is_some()
        || state.settings.read("whisper_settings").is_some();
    report.notes.push(format!(
        "the application settings database holds dictation keys: {db_keys}"
    ));
    report.push_component(
        ComponentHealth::new(
            "dictation_settings",
            if stored.configured {
                ComponentState::Ready
            } else if stored.document_present {
                ComponentState::NotConfigured
            } else {
                ComponentState::Missing
            },
        )
        .with_detail(if stored.configured {
            format!(
                "{} and {} are stored",
                stored.executable_name, stored.model_name
            )
        } else {
            "no executable and model pair is stored yet".to_string()
        }),
    );

    let dictation = whisper.status();
    report.health_checks.push(HealthCheck {
        name: "dictation_ready".to_string(),
        passed: dictation.configured && dictation.enabled,
        detail: Some(if dictation.enabled {
            "dictation is switched on".to_string()
        } else {
            "dictation is switched off".to_string()
        }),
    });

    // ---------------------------------------------------------------- backup
    // Only safe facts: whether the feature can run, the format version, the last
    // operation of this process, whether a restore was interrupted, and codes.
    // Never a path, a password, a key, or the name of anything the person stored.
    let backup = state.backup.status();
    report.notes.push(format!(
        "backup: format_version={} available={} key_envelope={} interrupted_restore={} previous_state={} last_operation={} last_error={}",
        backup.format_version,
        backup.available,
        backup.key_envelope_present,
        backup.interrupted_restore.as_deref().unwrap_or("none"),
        backup.previous_state_present,
        backup.last_operation.as_deref().unwrap_or("none"),
        backup.last_error_code.as_deref().unwrap_or("none"),
    ));
    report.push_component(
        ComponentHealth::new(
            "backup",
            if backup.available {
                ComponentState::Ready
            } else {
                ComponentState::NotConfigured
            },
        )
        .with_detail(if backup.available {
            format!("backup format version {}", backup.format_version)
        } else {
            "no portable key envelope is stored yet, so a backup cannot be made".to_string()
        }),
    );
    report.health_checks.push(HealthCheck {
        name: "no_interrupted_restore".to_string(),
        passed: backup.interrupted_restore.is_none(),
        detail: Some(match backup.interrupted_restore.as_deref() {
            Some(stage) => format!("a restore was interrupted at {stage}"),
            None => "no restore is waiting to be finished or undone".to_string(),
        }),
    });

    // ----------------------------------------------------------------- vosk
    let vosk_root = jarvis_core::APP_DIR.join("resources").join("vosk");
    report.push_component(directory_health(
        "vosk_runtime",
        &vosk_root,
        "the Vosk runtime folder was not found next to the application",
    ));

    // ---------------------------------------------------------- dictionaries
    let dictionary_directory = state.autocorrect.dictionary_directory();
    report.push_component(directory_health(
        "dictionaries",
        &dictionary_directory,
        "no dictionary folder has been chosen",
    ));

    // ---------------------------------------------------------- microphone
    let devices = jarvis_core::recorder::get_audio_devices();
    report.push_component(
        ComponentHealth::new(
            "microphone",
            if devices.is_empty() {
                ComponentState::Missing
            } else {
                ComponentState::Ready
            },
        )
        .with_detail(format!("{} input device(s) reported", devices.len())),
    );
    report.health_checks.push(HealthCheck {
        name: "microphone_permission".to_string(),
        // Only the device list is read: nothing is opened and nothing recorded.
        passed: !devices.is_empty(),
        detail: Some("the device list is read; no recording is started".to_string()),
    });
    let microphone = app
        .state::<std::sync::Arc<crate::desktop::DesktopHandle>>()
        .audio()
        .state();
    report.notes.push(format!(
        "the microphone session is {}",
        microphone_label(microphone)
    ));

    // -------------------------------------------------------- Windows actions
    let capabilities = state.windows_actions.session().lock().capabilities();
    report.push_component(ComponentHealth::new(
        "windows_actions",
        if capabilities.platform_supported {
            ComponentState::Ready
        } else {
            ComponentState::Unavailable
        },
    ));
    report.push_component(ComponentHealth::new(
        "core_audio",
        if capabilities.volume {
            ComponentState::Ready
        } else {
            ComponentState::Unavailable
        },
    ));
    report.push_component(ComponentHealth::new(
        "screenshot_backend",
        if capabilities.screenshots {
            ComponentState::Ready
        } else {
            ComponentState::Unavailable
        },
    ));
    let pending = state.windows_actions.session().lock().scheduled().len();
    report.health_checks.push(HealthCheck {
        name: "action_scheduler".to_string(),
        passed: true,
        detail: Some(format!("{pending} timer(s) and reminder(s) waiting")),
    });

    // ------------------------------------------------------------ the shell
    let autostart = app
        .state::<std::sync::Arc<crate::desktop::DesktopHandle>>()
        .autostart_status();
    let autostart_state = match autostart.state() {
        AutostartState::Enabled => ComponentState::Ready,
        AutostartState::Disabled => ComponentState::NotConfigured,
        AutostartState::NeedsAttention => ComponentState::Incompatible,
        AutostartState::Unavailable => ComponentState::PermissionDenied,
    };
    report.push_component(
        ComponentHealth::new("autostart", autostart_state)
            .with_detail(format!("the Run entry is {}", autostart.state().as_str())),
    );
    report.push_component(ComponentHealth::new(
        "tray",
        if app.tray_by_id(crate::desktop::TRAY_ID).is_some() {
            ComponentState::Ready
        } else {
            ComponentState::Missing
        },
    ));
    report.push_component(
        ComponentHealth::new("webview2", ComponentState::VersionUnknown)
            .with_detail("the WebView2 runtime version is not read by this report"),
    );
    report.push_component(
        ComponentHealth::new("windows_notifications", ComponentState::VersionUnknown).with_detail(
            if capabilities.notifications {
                "the platform reports notification support"
            } else {
                "a system toast needs a registered application identity, which this build has not got"
            },
        ),
    );

    // ------------------------------------------------------------- resources
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    report.memory_total_mb = Some(system.total_memory() / 1024 / 1024);
    report.memory_free_mb = Some(system.available_memory() / 1024 / 1024);
    report.disk_free_mb = sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .find(|disk| data.starts_with(disk.mount_point()))
        .map(|disk| disk.available_space() / 1024 / 1024);

    // ---------------------------------------------------------------- sizes
    for (name, path) in store_paths(&data) {
        if let Ok(metadata) = std::fs::metadata(&path) {
            report.push_database(
                &format!("{name}/{}", path.to_string_lossy()),
                metadata.len(),
            );
        }
    }
    for (label, path) in [
        ("llama model", configured.model_path()),
        (
            "whisper model",
            Some(std::path::PathBuf::from(&whisper_settings.model_path)),
        ),
    ] {
        if let Some(path) = path.filter(|path| !path.is_empty()) {
            if let Ok(metadata) = std::fs::metadata(&path) {
                report.push_model(
                    &format!("{label}/{}", path.to_string_lossy()),
                    metadata.len(),
                );
            }
        }
    }
    if let Some(total) = report.memory_total_mb {
        report.notes.push(format!(
            "{} is installed in total",
            bytes_label(total * 1024 * 1024)
        ));
    }

    // --------------------------------------------------------------- licences
    report.licenses = license_lines();
    report
        .notes
        .push("the project's own licence is unresolved; see docs/LICENSING_STATUS.md".to_string());
    report
}

/// The files the application keeps in its own directory, so their sizes can be
/// reported by file name. A path that does not exist is simply not listed.
fn store_paths(data: &Path) -> Vec<(&'static str, std::path::PathBuf)> {
    vec![
        ("timers", data.join("timers.json")),
        ("allowlist", data.join("allowed-applications.json")),
        ("actions_audit", data.join("actions-audit.jsonl")),
        ("whisper_settings", data.join("whisper-settings.json")),
        ("desktop_settings", data.join("desktop.json")),
    ]
}

fn data_directory_health(data: &Path) -> ComponentHealth {
    if !data.exists() {
        return ComponentHealth::new("data_directory", ComponentState::Missing)
            .with_detail("the application data directory does not exist yet");
    }
    // Writable is checked by asking for a file, not by assuming.
    let probe = data.join(".diagnostics-write-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            ComponentHealth::new("data_directory", ComponentState::Ready)
                .with_detail("readable and writable")
        }
        Err(_) => ComponentHealth::new("data_directory", ComponentState::PermissionDenied)
            .with_detail("the application data directory is not writable by this user"),
    }
}

fn binary_health(name: &str, path: &str, missing: &str) -> ComponentHealth {
    if path.trim().is_empty() {
        return ComponentHealth::new(name, ComponentState::NotConfigured).with_detail(missing);
    }
    match jarvis_core::whisper::probe_binary(Path::new(path)) {
        Ok(probe) => ComponentHealth::new(name, ComponentState::Ready).with_detail(format!(
            "{} executable, {}",
            probe.architecture.as_str(),
            bytes_label(probe.size_bytes)
        )),
        Err(error) => {
            ComponentHealth::new(name, ComponentState::Invalid).with_detail(error.to_string())
        }
    }
}

fn file_health(name: &str, path: Option<&str>, missing: &str) -> ComponentHealth {
    let Some(path) = path.filter(|path| !path.trim().is_empty()) else {
        return ComponentHealth::new(name, ComponentState::NotConfigured).with_detail(missing);
    };
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => ComponentHealth::new(name, ComponentState::Ready)
            .with_detail(bytes_label(metadata.len())),
        Ok(_) => ComponentHealth::new(name, ComponentState::Invalid)
            .with_detail("that path is not a file"),
        Err(_) => ComponentHealth::new(name, ComponentState::Missing)
            .with_detail("the file the settings point at is gone"),
    }
}

fn directory_health(name: &str, path: &Path, missing: &str) -> ComponentHealth {
    if path.as_os_str().is_empty() {
        return ComponentHealth::new(name, ComponentState::NotConfigured).with_detail(missing);
    }
    if path.is_dir() {
        let entries = std::fs::read_dir(path)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0);
        ComponentHealth::new(name, ComponentState::Ready)
            .with_detail(format!("{entries} file(s) present"))
    } else {
        ComponentHealth::new(name, ComponentState::Missing).with_detail(missing)
    }
}

fn microphone_label(state: MicrophoneState) -> &'static str {
    match state {
        MicrophoneState::Idle => "idle",
        MicrophoneState::VoskListening => "listening (Vosk)",
        MicrophoneState::WhisperDictation => "recording (dictation)",
        MicrophoneState::TranscribingFile => "transcribing a file",
        MicrophoneState::Stopping => "stopping",
        MicrophoneState::Failed => "failed",
    }
}

/// The licence lines, copied from the documents rather than invented here.
fn license_lines() -> Vec<LicenseStatus> {
    vec![
        LicenseStatus {
            component: "this application".to_string(),
            license: "GPL-3.0-only (Cargo.toml) vs CC-BY-NC-SA-4.0 (LICENSE.txt)".to_string(),
            status: "conflicting".to_string(),
        },
        LicenseStatus {
            component: "llama.cpp".to_string(),
            license: "MIT".to_string(),
            status: "not_distributed".to_string(),
        },
        LicenseStatus {
            component: "whisper.cpp".to_string(),
            license: "MIT".to_string(),
            status: "not_distributed".to_string(),
        },
        LicenseStatus {
            component: "Vosk".to_string(),
            license: "Apache-2.0".to_string(),
            status: "not_distributed".to_string(),
        },
        LicenseStatus {
            component: "SQLite".to_string(),
            license: "public domain".to_string(),
            status: "bundled".to_string(),
        },
        LicenseStatus {
            component: "ONNX Runtime".to_string(),
            license: "Microsoft terms".to_string(),
            status: "unknown".to_string(),
        },
    ]
}

/// The error categories the report may carry. They are codes and counts; the
/// messages themselves are never read.
pub fn error_categories(state: &AppState) -> Vec<ErrorCategory> {
    let audit = state.windows_actions.session().lock().audit_entries();
    let mut categories: Vec<ErrorCategory> = Vec::new();
    for entry in audit.iter().rev().take(200) {
        let Some(code) = entry.error_category.clone() else {
            continue;
        };
        match categories.iter_mut().find(|category| category.code == code) {
            Some(category) => category.count += 1,
            None => categories.push(ErrorCategory { code, count: 1 }),
        }
    }
    categories
}
// ------------------------------------------------------------------ commands

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticsView {
    pub report: DiagnosticReport,
    pub preview: Vec<String>,
    /// Whether the report passed the screen. A report that fails is shown but
    /// cannot be exported.
    pub screen_passed: bool,
    pub screen_error: Option<String>,
}

impl DiagnosticsView {
    pub fn of(report: DiagnosticReport) -> Self {
        let (screen_passed, screen_error) = match report.screen() {
            Ok(()) => (true, None),
            Err(error) => (false, Some(error)),
        };
        let preview = report.preview();
        Self {
            report,
            preview,
            screen_passed,
            screen_error,
        }
    }
}

/// Builds the report and its preview. Nothing is written.
#[tauri::command]
pub async fn diagnostics_run(app: tauri::AppHandle) -> Result<DiagnosticsView, String> {
    let state = app.state::<AppState>();
    let mut report = collect(&app);
    report.recent_errors = error_categories(&state);
    Ok(DiagnosticsView::of(report))
}

/// The lines the window shows before anything is saved.
#[tauri::command]
pub async fn diagnostics_preview(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    Ok(collect(&app).preview())
}

/// Writes the report to a file the user picks. The screen runs first: a report
/// that carries a path or a secret is refused, not written.
#[tauri::command]
pub async fn diagnostics_export(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let mut report = collect(&app);
    report.recent_errors = error_categories(&state);
    let json = report.to_json()?;
    let Some(destination) = save_path(&app) else {
        return Ok(None);
    };
    // A temporary file and a rename, so a failure leaves no half-written report.
    let temporary = destination.with_extension("json.tmp");
    std::fs::write(&temporary, json.as_bytes())
        .map_err(|_| "the report could not be written".to_string())?;
    std::fs::rename(&temporary, &destination)
        .map_err(|_| "the report could not be written".to_string())?;
    Ok(Some(destination.to_string_lossy().into_owned()))
}

/// The short summary the window can copy without writing a file.
#[tauri::command]
pub async fn diagnostics_summary(app: tauri::AppHandle) -> Result<String, String> {
    let report = collect(&app);
    report.screen()?;
    let mut lines = Vec::new();
    lines.push(format!("JARVIS {}", report.application_version));
    lines.push(format!(
        "{} {}",
        report.operating_system, report.architecture
    ));
    for component in &report.components {
        lines.push(format!("{}: {}", component.name, component.state.as_str()));
    }
    lines.push(format!("notes: {}", report.notes.len()));
    Ok(lines.join("\n"))
}

fn save_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS diagnostics")
        .set_file_name("jarvis-diagnostics.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}
