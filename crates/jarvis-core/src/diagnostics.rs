//! The runtime diagnostics report: what is present, what is missing, and what
//! could not be checked.
//!
//! The report exists so a problem can be described without describing the
//! person. It is built from fields that cannot hold user content — there is no
//! field for a note, a conversation, a transcript, a password, a token, or a
//! full path — and the renderer is checked by a second guard before the text is
//! offered.
//!
//! What it contains:
//!
//! * the application version, the operating system, and the architecture;
//! * one line per component: ready, missing, wrong architecture, version
//!   unknown, incompatible, or refused by permission;
//! * the schema versions of the databases and of the settings documents;
//! * the last error categories, by code and count, never by message;
//! * the health checks that were run, and whether they passed;
//! * the size of each database and each model, named by **file name only**;
//! * free and total memory, and free disk space;
//! * the licence status of the components that have one.
//!
//! What it deliberately does not contain: a user name, a home directory, a note,
//! a password, a conversation, a fact, a transcript, a window title, a token, a
//! key, the content of the audit log, and any prompt.

use serde::{Deserialize, Serialize};

use crate::text::{
    file_label, looks_like_it_carries_a_path_or_a_secret, redact, shorten, MAX_LINE_CHARS,
};

/// Longest the rendered report may be, in lines.
pub const MAX_REPORT_LINES: usize = 400;

/// The state of one dependency, in the vocabulary the interface uses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    Ready,
    Missing,
    WrongArchitecture,
    VersionUnknown,
    Incompatible,
    PermissionDenied,
    /// Nothing was configured, which is a choice and not a fault.
    NotConfigured,
    /// The feature is switched off.
    Disabled,
}

impl ComponentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::WrongArchitecture => "wrong_architecture",
            Self::VersionUnknown => "version_unknown",
            Self::Incompatible => "incompatible",
            Self::PermissionDenied => "permission_denied",
            Self::NotConfigured => "not_configured",
            Self::Disabled => "disabled",
        }
    }

    /// Whether a person has to do something about it.
    pub fn needs_attention(&self) -> bool {
        !matches!(self, Self::Ready | Self::Disabled | Self::NotConfigured)
    }

    /// Whether the state means "you have not set this up", which the wizard can
    /// offer to do.
    pub fn is_setup(&self) -> bool {
        matches!(self, Self::NotConfigured | Self::Disabled)
    }
}

/// One dependency, with a content-free explanation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ComponentHealth {
    /// A stable name: `vosk_runtime`, `whisper_binary`, `sqlite`, …
    pub name: String,
    pub state: ComponentState,
    /// One short sentence, already free of paths and secrets.
    pub detail: Option<String>,
}

impl ComponentHealth {
    pub fn new(name: impl Into<String>, state: ComponentState) -> Self {
        Self {
            name: name.into(),
            state,
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(redact(&detail.into()));
        self
    }
}

/// A size, named by file name only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileSize {
    pub name: String,
    pub bytes: u64,
}

impl FileSize {
    /// Builds a size from a path, keeping only the file name.
    pub fn of(path: &str, bytes: u64) -> Self {
        Self {
            name: file_label(path),
            bytes,
        }
    }

    /// The size as a short label.
    pub fn label(&self) -> String {
        bytes_label(self.bytes)
    }
}

/// One error category, by code and count.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorCategory {
    pub code: String,
    pub count: u64,
}

/// One health check that was run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthCheck {
    pub name: String,
    pub passed: bool,
    pub detail: Option<String>,
}

/// The licence status of one component.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LicenseStatus {
    pub component: String,
    pub license: String,
    /// `confirmed`, `conflicting`, `unknown`, or `not_distributed`.
    pub status: String,
}

/// Everything the report holds.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DiagnosticReport {
    pub application_version: String,
    pub operating_system: String,
    pub architecture: String,
    pub components: Vec<ComponentHealth>,
    pub schema_versions: Vec<(String, u32)>,
    pub recent_errors: Vec<ErrorCategory>,
    pub health_checks: Vec<HealthCheck>,
    pub database_sizes: Vec<FileSize>,
    pub model_sizes: Vec<FileSize>,
    pub memory_total_mb: Option<u64>,
    pub memory_free_mb: Option<u64>,
    pub disk_free_mb: Option<u64>,
    pub licenses: Vec<LicenseStatus>,
    /// Notes about what could not be read, in plain words.
    pub notes: Vec<String>,
}

impl DiagnosticReport {
    /// A report with the three facts that are always known.
    pub fn new(application_version: impl Into<String>) -> Self {
        Self {
            application_version: application_version.into(),
            operating_system: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            ..Self::default()
        }
    }

    /// Adds a component line.
    pub fn push_component(&mut self, component: ComponentHealth) {
        self.components.push(component);
    }

    /// Adds a database size, keeping only the file name.
    pub fn push_database(&mut self, path: &str, bytes: u64) {
        self.database_sizes.push(FileSize::of(path, bytes));
    }

    /// Adds a model size, keeping only the file name.
    pub fn push_model(&mut self, path: &str, bytes: u64) {
        self.model_sizes.push(FileSize::of(path, bytes));
    }

    /// The components a person has to look at.
    pub fn needs_attention(&self) -> Vec<&ComponentHealth> {
        self.components
            .iter()
            .filter(|component| component.state.needs_attention())
            .collect()
    }

    /// Whether every component is ready (or deliberately not configured).
    pub fn is_healthy(&self) -> bool {
        self.components
            .iter()
            .all(|component| !component.state.needs_attention())
    }

    /// The report as text, one line per fact, ready to be read and pasted.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("JARVIS diagnostics — {}", self.application_version));
        lines.push(format!(
            "system: {} {}",
            self.operating_system, self.architecture
        ));
        lines.push(String::new());
        lines.push("components:".to_string());
        for component in &self.components {
            match &component.detail {
                Some(detail) => lines.push(format!(
                    "  {}: {} — {}",
                    component.name,
                    component.state.as_str(),
                    detail
                )),
                None => lines.push(format!(
                    "  {}: {}",
                    component.name,
                    component.state.as_str()
                )),
            }
        }
        if !self.schema_versions.is_empty() {
            lines.push(String::new());
            lines.push("schema versions:".to_string());
            for (name, version) in &self.schema_versions {
                lines.push(format!("  {name}: {version}"));
            }
        }
        if !self.health_checks.is_empty() {
            lines.push(String::new());
            lines.push("health checks:".to_string());
            for check in &self.health_checks {
                let outcome = if check.passed { "ok" } else { "failed" };
                match &check.detail {
                    Some(detail) => lines.push(format!("  {}: {outcome} — {detail}", check.name)),
                    None => lines.push(format!("  {}: {outcome}", check.name)),
                }
            }
        }
        if !self.database_sizes.is_empty() {
            lines.push(String::new());
            lines.push("databases:".to_string());
            for database in &self.database_sizes {
                lines.push(format!("  {}: {}", database.name, database.label()));
            }
        }
        if !self.model_sizes.is_empty() {
            lines.push(String::new());
            lines.push("models:".to_string());
            for model in &self.model_sizes {
                lines.push(format!("  {}: {}", model.name, model.label()));
            }
        }
        if self.memory_total_mb.is_some() || self.disk_free_mb.is_some() {
            lines.push(String::new());
            lines.push("resources:".to_string());
            if let Some(total) = self.memory_total_mb {
                let free = self.memory_free_mb.unwrap_or(0);
                lines.push(format!("  memory: {free} MB free of {total} MB"));
            }
            if let Some(free) = self.disk_free_mb {
                lines.push(format!("  disk: {free} MB free"));
            }
        }
        if !self.recent_errors.is_empty() {
            lines.push(String::new());
            lines.push("recent error categories:".to_string());
            for error in &self.recent_errors {
                lines.push(format!("  {}: {}", error.code, error.count));
            }
        }
        if !self.licenses.is_empty() {
            lines.push(String::new());
            lines.push("licences:".to_string());
            for license in &self.licenses {
                lines.push(format!(
                    "  {}: {} ({})",
                    license.component, license.license, license.status
                ));
            }
        }
        if !self.notes.is_empty() {
            lines.push(String::new());
            lines.push("notes:".to_string());
            for note in &self.notes {
                lines.push(format!("  {}", shorten(note, MAX_LINE_CHARS)));
            }
        }
        lines.truncate(MAX_REPORT_LINES);
        lines.join("\n")
    }

    /// The lines of the preview the window shows before anything is written.
    pub fn preview(&self) -> Vec<String> {
        self.render().lines().map(str::to_string).collect()
    }

    /// Checks the rendered report before it is offered to the user.
    ///
    /// The report is built from fields that cannot hold a path or a secret, and
    /// this is the check that fails if one of them ever learns how. A caller
    /// may show the preview anyway; an export refuses.
    pub fn screen(&self) -> Result<(), String> {
        let rendered = self.render();
        for (index, line) in rendered.lines().enumerate() {
            if let Some(reason) = looks_like_it_carries_a_path_or_a_secret(line) {
                return Err(format!("line {} carries a {reason}", index + 1));
            }
        }
        Ok(())
    }

    /// The report as a JSON document, for an export.
    pub fn to_json(&self) -> Result<String, String> {
        self.screen()?;
        serde_json::to_string_pretty(self).map_err(|_| "the report cannot be encoded".to_string())
    }
}

/// Bytes as a short label.
pub fn bytes_label(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DiagnosticReport {
        let mut report = DiagnosticReport::new("0.0.0-test");
        report.push_component(ComponentHealth::new("sqlite", ComponentState::Ready));
        report.push_component(
            ComponentHealth::new("whisper_binary", ComponentState::Missing)
                .with_detail("no executable has been chosen"),
        );
        report.push_component(ComponentHealth::new(
            "vosk_runtime",
            ComponentState::WrongArchitecture,
        ));
        report.push_component(ComponentHealth::new("notes", ComponentState::NotConfigured));
        report.push_component(ComponentHealth::new("dictation", ComponentState::Disabled));
        report.schema_versions.push(("notes".to_string(), 7));
        report.health_checks.push(HealthCheck {
            name: "data_directory_writable".to_string(),
            passed: true,
            detail: None,
        });
        report.push_database("C:/Users/someone/AppData/notes.db", 12 * 1024 * 1024);
        report.push_model(
            "C:/Users/someone/models/ggml-small.bin",
            3 * 1024 * 1024 * 1024,
        );
        report.memory_total_mb = Some(32_768);
        report.memory_free_mb = Some(12_000);
        report.disk_free_mb = Some(240_000);
        report.recent_errors.push(ErrorCategory {
            code: "process_failed".to_string(),
            count: 2,
        });
        report.licenses.push(LicenseStatus {
            component: "whisper.cpp".to_string(),
            license: "MIT".to_string(),
            status: "not_distributed".to_string(),
        });
        report.notes.push("the model was not verified".to_string());
        report
    }

    #[test]
    fn the_report_names_a_file_without_naming_the_folder() {
        let report = sample();
        let rendered = report.render();
        assert!(rendered.contains("notes.db: 12.0 MB"));
        assert!(rendered.contains("ggml-small.bin: 3.0 GB"));
        assert!(!rendered.contains("C:/Users"));
        assert!(!rendered.contains("someone"));
        assert!(!rendered.contains('/'));
    }

    #[test]
    fn the_report_says_what_is_ready_and_what_needs_attention() {
        let report = sample();
        let attention = report.needs_attention();
        // `missing` and `wrong_architecture` need attention; `not_configured`
        // and `disabled` are choices, not faults.
        assert_eq!(attention.len(), 2);
        assert!(attention
            .iter()
            .any(|component| component.name == "whisper_binary"));
        assert!(!report.is_healthy());
        let rendered = report.render();
        assert!(rendered.contains("whisper_binary: missing"));
        assert!(rendered.contains("vosk_runtime: wrong_architecture"));
        assert!(rendered.contains("notes: not_configured"));
        assert!(rendered.contains("dictation: disabled"));
        assert!(rendered.contains("data_directory_writable: ok"));
        assert!(rendered.contains("process_failed: 2"));
        assert!(rendered.contains("whisper.cpp: MIT (not_distributed)"));
    }

    #[test]
    fn a_healthy_report_says_so() {
        let mut report = DiagnosticReport::new("1.0.0");
        report.push_component(ComponentHealth::new("sqlite", ComponentState::Ready));
        report.push_component(ComponentHealth::new("dictation", ComponentState::Disabled));
        assert!(report.is_healthy());
        assert!(report.needs_attention().is_empty());
        assert!(report.screen().is_ok());
    }

    #[test]
    fn a_detail_that_carries_a_path_is_redacted_when_it_is_added() {
        let component = ComponentHealth::new("llama_server", ComponentState::Missing)
            .with_detail("looked for C:/Users/someone/tools/llama-server.exe");
        assert_eq!(component.detail.as_deref(), Some("looked for <path>"));
        let mut report = DiagnosticReport::new("1.0.0");
        report.push_component(component);
        assert!(!report.render().contains("someone"));
    }

    #[test]
    fn the_screen_refuses_a_report_that_carries_a_path_or_a_key() {
        let mut report = DiagnosticReport::new("1.0.0");
        // A name that slipped a path in: the screen is what catches it.
        report
            .schema_versions
            .push(("C:/Users/someone/notes.db".to_string(), 1));
        let error = report.screen().unwrap_err();
        assert!(error.contains("path"), "{error}");
        // The key file of the local storage is never part of a report.
        let mut report = DiagnosticReport::new("1.0.0");
        report.notes.push("restored from key.dpapi".to_string());
        assert!(report.screen().unwrap_err().contains("key file"));
        // A secret-shaped line is refused as well.
        let mut report = DiagnosticReport::new("1.0.0");
        report.recent_errors.push(ErrorCategory {
            code: "api_key=abc".to_string(),
            count: 1,
        });
        assert!(report.screen().unwrap_err().contains("secret"));
    }

    #[test]
    fn an_export_refuses_what_the_screen_refuses() {
        let mut report = DiagnosticReport::new("1.0.0");
        report
            .notes
            .push("see C:/Users/someone/notes.log".to_string());
        assert!(report.to_json().is_err());
        let clean = sample();
        let json = clean.to_json().unwrap();
        assert!(json.contains("application_version"));
        assert!(!json.contains("someone"));
    }

    #[test]
    fn the_preview_is_the_rendered_report_line_by_line() {
        let report = sample();
        let lines = report.preview();
        assert_eq!(lines.join("\n"), report.render());
        assert!(lines.len() > 10);
        // The first line names the application, so the preview is recognisable.
        assert!(lines[0].contains("JARVIS diagnostics"));
    }

    #[test]
    fn the_report_carries_no_field_for_user_content() {
        // The shape itself is the guarantee: there is nowhere for a note, a
        // conversation, a transcript, or a password to be written.
        let json = serde_json::to_value(sample()).unwrap();
        let object = json.as_object().unwrap();
        for forbidden in [
            "notes_text",
            "content",
            "conversations",
            "facts",
            "transcript",
            "transcripts",
            "password",
            "token",
            "key",
            "window_title",
            "prompt",
            "audit",
        ] {
            assert!(
                !object.contains_key(forbidden),
                "the report must have no {forbidden} field"
            );
        }
        let rendered = sample().render();
        for forbidden in ["FICTIONAL", "password", "Bearer", "key.dpapi"] {
            assert!(!rendered.contains(forbidden), "{forbidden} must not appear");
        }
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(bytes_label(512), "512 B");
        assert_eq!(bytes_label(2048), "2.0 KB");
        assert_eq!(bytes_label(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(bytes_label(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn the_report_is_bounded() {
        let mut report = DiagnosticReport::new("1.0.0");
        for index in 0..MAX_REPORT_LINES + 50 {
            report.schema_versions.push((format!("schema_{index}"), 1));
        }
        assert!(report.preview().len() <= MAX_REPORT_LINES);
    }

    #[test]
    fn every_state_has_a_name_and_a_meaning() {
        let states = [
            ComponentState::Ready,
            ComponentState::Missing,
            ComponentState::WrongArchitecture,
            ComponentState::VersionUnknown,
            ComponentState::Incompatible,
            ComponentState::PermissionDenied,
            ComponentState::NotConfigured,
            ComponentState::Disabled,
        ];
        let mut names: Vec<&str> = states.iter().map(ComponentState::as_str).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "two states share a name");
        assert!(!ComponentState::Ready.needs_attention());
        assert!(!ComponentState::Disabled.needs_attention());
        assert!(!ComponentState::NotConfigured.needs_attention());
        assert!(ComponentState::Missing.needs_attention());
        assert!(ComponentState::PermissionDenied.needs_attention());
        assert!(ComponentState::NotConfigured.is_setup());
        assert!(!ComponentState::Ready.is_setup());
    }
}
