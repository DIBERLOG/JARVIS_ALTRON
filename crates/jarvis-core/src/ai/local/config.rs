//! Local runtime configuration: llama-server process settings and generation
//! defaults.
//!
//! The configuration is not secret, but it is validated before anything is
//! started and it never stores conversation text, notes, stored secrets, or key
//! material. Defaults are conservative: CPU-only offload, a modest context, and a
//! loopback-only listener.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::super::{AiProfile, Persona};
use crate::ai::ChatError;

/// Settings key used by the existing application settings store (`app.db`).
pub const SETTINGS_KEY: &str = "local_ai_config";

/// Default host: loopback only.
pub const DEFAULT_HOST: &str = "127.0.0.1";
/// Default llama-server port.
pub const DEFAULT_PORT: u16 = 8080;
/// Default context window: modest, because memory is the user's machine.
pub const DEFAULT_CONTEXT_SIZE: u32 = 8192;
/// Default maximum number of generated tokens.
pub const DEFAULT_MAX_TOKENS: u32 = 1024;
/// Default sampling temperature.
pub const DEFAULT_TEMPERATURE: f32 = 0.7;
/// Default nucleus sampling value.
pub const DEFAULT_TOP_P: f32 = 0.95;
/// Default time allowed for the server to become ready.
pub const DEFAULT_STARTUP_TIMEOUT_SECONDS: u64 = 120;

const MIN_CONTEXT_SIZE: u32 = 512;
const MAX_CONTEXT_SIZE: u32 = 262_144;
const MAX_THREADS: u32 = 256;
const MAX_GPU_LAYERS: u32 = 1_000;
const MIN_PORT: u16 = 1024;
const MIN_MAX_TOKENS: u32 = 1;
const MAX_MAX_TOKENS: u32 = 32_768;
const MIN_STARTUP_TIMEOUT_SECONDS: u64 = 5;
const MAX_STARTUP_TIMEOUT_SECONDS: u64 = 900;

/// How the model should treat its reasoning output.
///
/// Support depends on the chat template of the loaded model and on the
/// llama.cpp build; [`LocalAiCapabilities`](super::LocalAiCapabilities) reports
/// what the running server actually accepted.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    /// Ask the server not to emit reasoning.
    Disabled,
    /// Ask the server to emit reasoning.
    Enabled,
    /// Send no preference and let the chat template decide.
    #[default]
    Auto,
}

impl ThinkingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Auto => "auto",
        }
    }

    pub fn all() -> [Self; 3] {
        [Self::Disabled, Self::Enabled, Self::Auto]
    }
}

/// llama-server location and launch parameters.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct LocalModelConfig {
    /// Path to `llama-server.exe`.
    pub server_path: String,
    /// Path to the `.gguf` model file.
    pub model_path: String,
    /// Interface the server binds. Loopback only in this version.
    pub host: String,
    pub port: u16,
    /// Context window in tokens.
    pub context_size: u32,
    /// Worker threads; `0` lets llama.cpp decide.
    pub cpu_threads: u32,
    /// Layers offloaded to the GPU; `0` is CPU-only.
    pub gpu_layers: u32,
    /// Seconds allowed for the server to become ready.
    pub startup_timeout_seconds: u64,
}

impl Default for LocalModelConfig {
    fn default() -> Self {
        Self {
            server_path: String::new(),
            model_path: String::new(),
            host: DEFAULT_HOST.to_string(),
            port: DEFAULT_PORT,
            context_size: DEFAULT_CONTEXT_SIZE,
            cpu_threads: 0,
            gpu_layers: 0,
            startup_timeout_seconds: DEFAULT_STARTUP_TIMEOUT_SECONDS,
        }
    }
}

/// Generation defaults and the active profile.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct LocalAiConfig {
    pub server: LocalModelConfig,
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    pub profile: Persona,
    pub thinking: ThinkingMode,
    /// Requested LAN exposure. Kept in the format so the field is explicit, but
    /// rejected by validation in this version.
    pub allow_lan: bool,
    /// Schema version of this configuration, for future migrations.
    pub schema_version: u32,
}

impl Default for LocalAiConfig {
    fn default() -> Self {
        Self {
            server: LocalModelConfig::default(),
            temperature: DEFAULT_TEMPERATURE,
            top_p: DEFAULT_TOP_P,
            max_tokens: DEFAULT_MAX_TOKENS,
            profile: Persona::Jarvis,
            thinking: ThinkingMode::Auto,
            allow_lan: false,
            schema_version: CONFIG_SCHEMA_VERSION,
        }
    }
}

/// Current configuration schema version.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// A field-level problem, so the interface can point at the exact setting.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfigIssue {
    /// Configuration field name, for example `port`.
    pub field: String,
    /// Content-free explanation.
    pub message: String,
}

impl LocalAiConfig {
    /// Path to the server executable, when set.
    pub fn server_path(&self) -> Option<PathBuf> {
        non_empty_path(&self.server.server_path)
    }

    /// Path to the model file, when set.
    pub fn model_path(&self) -> Option<PathBuf> {
        non_empty_path(&self.server.model_path)
    }

    /// Adds the host/port pair to a URL path. The result is loopback only.
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.server.host, self.server.port)
    }

    /// Validates every field that can be checked without touching the disk.
    ///
    /// File existence is checked separately by
    /// [`ModelValidation`](super::ModelValidation) so the interface can report
    /// both without duplicating work.
    pub fn validate_shape(&self) -> Result<(), Vec<ConfigIssue>> {
        let mut issues = Vec::new();
        let mut push = |field: &str, message: &str| {
            issues.push(ConfigIssue {
                field: field.to_string(),
                message: message.to_string(),
            })
        };

        if self.schema_version != CONFIG_SCHEMA_VERSION {
            push(
                "schema_version",
                "unsupported settings version; the AI settings need to be reviewed",
            );
        }
        if self.allow_lan {
            // Deliberately not a setting yet: the local server stays private.
            push(
                "allow_lan",
                "binding beyond loopback is not available in this version",
            );
        }
        match self.server.host.trim() {
            "127.0.0.1" | "::1" | "localhost" => {}
            "0.0.0.0" | "::" => push(
                "host",
                "binding to all interfaces is refused; use 127.0.0.1",
            ),
            _ => push("host", "only a loopback host is accepted in this version"),
        }
        if self.server.port < MIN_PORT {
            push("port", "choose a port between 1024 and 65535");
        }
        if !(MIN_CONTEXT_SIZE..=MAX_CONTEXT_SIZE).contains(&self.server.context_size) {
            push(
                "context_size",
                "context must be between 512 and 262144 tokens",
            );
        }
        if self.server.cpu_threads > MAX_THREADS {
            push("cpu_threads", "thread count is unrealistically high");
        }
        if self.server.gpu_layers > MAX_GPU_LAYERS {
            push("gpu_layers", "GPU layer count is unrealistically high");
        }
        if !(MIN_STARTUP_TIMEOUT_SECONDS..=MAX_STARTUP_TIMEOUT_SECONDS)
            .contains(&self.server.startup_timeout_seconds)
        {
            push("startup_timeout_seconds", "timeout must be 5..900 seconds");
        }
        if !(0.0..=2.0).contains(&self.temperature) || !self.temperature.is_finite() {
            push("temperature", "temperature must be between 0 and 2");
        }
        if !(0.05..=1.0).contains(&self.top_p) || !self.top_p.is_finite() {
            push("top_p", "top-p must be between 0.05 and 1");
        }
        if !(MIN_MAX_TOKENS..=MAX_MAX_TOKENS).contains(&self.max_tokens) {
            push("max_tokens", "maximum tokens must be between 1 and 32768");
        }
        if self.server_path().is_none() {
            push("server_path", "select the llama-server executable");
        }
        if self.model_path().is_none() {
            push("model_path", "select the GGUF model file");
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    /// Validates the configuration, including that both paths exist and look
    /// usable. Returns the first problem as a [`ChatError`].
    pub fn validate_for_start(&self) -> Result<(), ChatError> {
        self.validate_shape()
            .map_err(|issues| ChatError::InvalidConfiguration(describe_issues(&issues)))?;
        super::model::validate_files(self)
    }

    /// Command-line arguments for `llama-server`, as separate arguments.
    ///
    /// The model path is passed as its own argument and never concatenated into
    /// a shell string, so a path containing spaces or shell metacharacters cannot
    /// change the command that runs.
    pub fn server_arguments(&self) -> Result<Vec<String>, ChatError> {
        let model = self.model_path().ok_or(ChatError::InvalidConfiguration(
            "select the GGUF model file".to_string(),
        ))?;
        let mut arguments = vec![
            "--model".to_string(),
            path_argument(&model),
            "--host".to_string(),
            self.server.host.trim().to_string(),
            "--port".to_string(),
            self.server.port.to_string(),
            "--ctx-size".to_string(),
            self.server.context_size.to_string(),
        ];
        if self.server.cpu_threads > 0 {
            arguments.push("--threads".to_string());
            arguments.push(self.server.cpu_threads.to_string());
        }
        if self.server.gpu_layers > 0 {
            arguments.push("--n-gpu-layers".to_string());
            arguments.push(self.server.gpu_layers.to_string());
        }
        Ok(arguments)
    }

    pub fn to_json(&self) -> Result<String, ChatError> {
        serde_json::to_string(self)
            .map_err(|_| ChatError::InvalidConfiguration("settings cannot be encoded".to_string()))
    }

    pub fn from_json(text: &str) -> Result<Self, ChatError> {
        serde_json::from_str(text).map_err(|_| {
            ChatError::InvalidConfiguration("settings file is not valid JSON".to_string())
        })
    }

    /// Saves settings atomically, for the portable export path.
    ///
    /// The running application keeps its settings inside the existing settings
    /// store under [`SETTINGS_KEY`]; this writes a file the user chose.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ChatError> {
        crate::fsutil::write_json_atomic(path, self).map_err(|_| {
            ChatError::InvalidConfiguration("settings could not be written".to_string())
        })
    }
}

impl LocalAiConfig {
    /// Applies a user-visible patch without touching unrelated fields.
    pub fn with_profile(&self, profile: AiProfile) -> Self {
        Self {
            profile,
            ..self.clone()
        }
    }
}

fn non_empty_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn path_argument(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Joins field names and messages into one content-free sentence.
pub fn describe_issues(issues: &[ConfigIssue]) -> String {
    issues
        .iter()
        .map(|issue| format!("{}: {}", issue.field, issue.message))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn configured() -> LocalAiConfig {
        LocalAiConfig {
            server: LocalModelConfig {
                server_path: "C:/tools/llama-server.exe".to_string(),
                model_path: "C:/models/Qwen3-8B-Q4_K_M.gguf".to_string(),
                ..LocalModelConfig::default()
            },
            ..LocalAiConfig::default()
        }
    }

    #[test]
    fn defaults_are_conservative_and_loopback_only() {
        let config = LocalAiConfig::default();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.context_size, 8192);
        assert_eq!(config.server.cpu_threads, 0);
        assert_eq!(config.server.gpu_layers, 0);
        assert!(!config.allow_lan);
        assert_eq!(config.profile, Persona::Jarvis);
        assert_eq!(config.thinking, ThinkingMode::Auto);
        assert_eq!(config.max_tokens, 1024);
        assert!((config.temperature - 0.7).abs() < f32::EPSILON);
        // The defaults are incomplete on purpose: no paths are guessed.
        assert!(config.server_path().is_none());
        assert!(config.model_path().is_none());
    }

    #[test]
    fn shape_validation_reports_each_problem_with_its_field() {
        let valid = configured();
        assert!(valid.validate_shape().is_ok());

        let mut lan = configured();
        lan.allow_lan = true;
        let issues = lan.validate_shape().unwrap_err();
        assert!(issues.iter().any(|issue| issue.field == "allow_lan"));

        let mut wildcard = configured();
        wildcard.server.host = "0.0.0.0".to_string();
        let issues = wildcard.validate_shape().unwrap_err();
        assert!(issues.iter().any(|issue| issue.field == "host"));

        let mut any = configured();
        any.server.host = "192.168.1.10".to_string();
        assert!(any
            .validate_shape()
            .unwrap_err()
            .iter()
            .any(|issue| issue.field == "host"));

        let mut port = configured();
        port.server.port = 80;
        assert!(port
            .validate_shape()
            .unwrap_err()
            .iter()
            .any(|issue| issue.field == "port"));

        let mut context = configured();
        context.server.context_size = 16;
        assert!(context
            .validate_shape()
            .unwrap_err()
            .iter()
            .any(|issue| issue.field == "context_size"));

        let mut temperature = configured();
        temperature.temperature = 9.0;
        assert!(temperature
            .validate_shape()
            .unwrap_err()
            .iter()
            .any(|issue| issue.field == "temperature"));

        let mut top_p = configured();
        top_p.top_p = 0.0;
        assert!(top_p
            .validate_shape()
            .unwrap_err()
            .iter()
            .any(|issue| issue.field == "top_p"));

        let mut tokens = configured();
        tokens.max_tokens = 0;
        assert!(tokens
            .validate_shape()
            .unwrap_err()
            .iter()
            .any(|issue| issue.field == "max_tokens"));

        let mut timeout = configured();
        timeout.server.startup_timeout_seconds = 1;
        assert!(timeout
            .validate_shape()
            .unwrap_err()
            .iter()
            .any(|issue| issue.field == "startup_timeout_seconds"));

        let missing = LocalAiConfig {
            schema_version: 99,
            ..LocalAiConfig::default()
        };
        let issues = missing.validate_shape().unwrap_err();
        assert!(issues.iter().any(|issue| issue.field == "schema_version"));
        assert!(issues.iter().any(|issue| issue.field == "server_path"));
        assert!(issues.iter().any(|issue| issue.field == "model_path"));
    }

    #[test]
    fn server_arguments_are_separate_and_exclude_unset_overrides() {
        let config = configured();
        let arguments = config.server_arguments().unwrap();
        assert_eq!(arguments[0], "--model");
        assert_eq!(arguments[1], "C:/models/Qwen3-8B-Q4_K_M.gguf");
        assert!(arguments.contains(&"--host".to_string()));
        assert!(arguments.contains(&"127.0.0.1".to_string()));
        assert!(arguments.contains(&"--ctx-size".to_string()));
        assert!(arguments.contains(&"8192".to_string()));
        // Zero means "leave it to llama.cpp", so no flag is added.
        assert!(!arguments.contains(&"--threads".to_string()));
        assert!(!arguments.contains(&"--n-gpu-layers".to_string()));

        let mut with_overrides = configured();
        with_overrides.server.cpu_threads = 6;
        with_overrides.server.gpu_layers = 20;
        let arguments = with_overrides.server_arguments().unwrap();
        assert!(arguments.contains(&"--threads".to_string()));
        assert!(arguments.contains(&"6".to_string()));
        assert!(arguments.contains(&"--n-gpu-layers".to_string()));
        assert!(arguments.contains(&"20".to_string()));
    }

    #[test]
    fn a_path_with_spaces_stays_a_single_argument() {
        let mut config = configured();
        config.server.model_path = "C:/my models/Qwen3 8B Q4_K_M.gguf".to_string();
        let arguments = config.server_arguments().unwrap();
        assert_eq!(arguments[1], "C:/my models/Qwen3 8B Q4_K_M.gguf");
        // Nothing is quoted, escaped, or joined: it is one argument.
        assert!(!arguments[1].contains('"'));
    }

    #[test]
    fn settings_round_trip_through_a_file_and_refuse_corruption() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("local_ai.json");
        let config = configured();

        config.save_to_file(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let loaded = LocalAiConfig::from_json(&written).unwrap();
        assert_eq!(loaded.server.model_path, config.server.model_path);
        assert_eq!(loaded.profile, config.profile);

        // A damaged file is reported, never guessed at: the caller decides
        // whether to keep its current settings, and nothing panics.
        std::fs::write(&path, b"{ truncated").unwrap();
        let damaged = std::fs::read_to_string(&path).unwrap();
        assert!(LocalAiConfig::from_json(&damaged).is_err());

        // An empty file is the same kind of problem.
        std::fs::write(&path, b"").unwrap();
        let empty = std::fs::read_to_string(&path).unwrap();
        assert!(LocalAiConfig::from_json(&empty).is_err());
    }

    #[test]
    fn json_round_trip_and_unknown_fields_are_tolerated() {
        let config = configured();
        let json = config.to_json().unwrap();
        assert_eq!(LocalAiConfig::from_json(&json).unwrap(), config);

        // A newer file with extra fields must still load.
        let extended = json.replace(
            "\"schema_version\":1",
            "\"schema_version\":1,\"future\":true",
        );
        assert!(LocalAiConfig::from_json(&extended).is_ok());

        assert_eq!(
            LocalAiConfig::from_json("not json"),
            Err(ChatError::InvalidConfiguration(
                "settings file is not valid JSON".to_string()
            ))
        );
    }

    #[test]
    fn partial_json_uses_defaults_for_missing_fields() {
        let config = LocalAiConfig::from_json(r#"{"temperature":0.2}"#).unwrap();
        assert!((config.temperature - 0.2).abs() < f32::EPSILON);
        assert_eq!(config.server.port, DEFAULT_PORT);
        assert_eq!(config.server.host, DEFAULT_HOST);
    }

    #[test]
    fn profile_patch_keeps_every_other_field() {
        let config = configured();
        let patched = config.with_profile(Persona::Altron);
        assert_eq!(patched.profile, Persona::Altron);
        assert_eq!(patched.server, config.server);
        assert_eq!(patched.thinking, config.thinking);
    }

    #[test]
    fn thinking_mode_and_profile_use_stable_wire_names() {
        assert_eq!(ThinkingMode::Auto.as_str(), "auto");
        assert_eq!(ThinkingMode::all().len(), 3);
        let json = serde_json::to_string(&ThinkingMode::Disabled).unwrap();
        assert_eq!(json, "\"disabled\"");
        let json = serde_json::to_string(&Persona::Altron).unwrap();
        assert_eq!(json, "\"altron\"");
        // AiProfile is the same type as Persona, not a parallel one.
        let profile: AiProfile = Persona::Jarvis;
        assert_eq!(profile, Persona::Jarvis);
    }
}
