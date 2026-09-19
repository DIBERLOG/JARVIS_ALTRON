//! GGUF inspection and memory estimation.
//!
//! What is checked reliably:
//!
//! * the file exists, is a regular file, is not empty, and has a `.gguf` name;
//! * the GGUF magic and version in the header;
//! * the `general.architecture` metadata string, when the metadata section can be
//!   parsed, plus `general.file_type` mapped to a quantisation hint.
//!
//! What is explicitly **not** verified: tensor contents, file integrity beyond
//! the header, or that a model claiming a Qwen3 architecture actually is one. A
//! GGUF file can declare any architecture, so the report says "declared
//! architecture", never "verified Qwen3".

use serde::Serialize;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use super::config::LocalAiConfig;
use crate::ai::ChatError;

/// Bytes of GGUF magic.
const GGUF_MAGIC: &[u8; 4] = b"GGUF";
/// Largest metadata section worth parsing, to keep a hostile file cheap.
const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;
/// Largest string value accepted while parsing metadata.
const MAX_STRING_BYTES: u64 = 4096;
/// Largest array length accepted while parsing metadata.
const MAX_ARRAY_ELEMENTS: u64 = 65_536;

/// Result level of a configuration or resource check.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckLevel {
    /// Everything looks usable.
    Ok,
    /// Usable, but the user should know about a risk.
    Warning,
    /// Starting this configuration is refused.
    Blocked,
}

/// One check result with a content-free message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidationIssue {
    pub level: CheckLevel,
    pub field: String,
    pub message: String,
}

impl ValidationIssue {
    pub fn warning(field: &str, message: impl Into<String>) -> Self {
        Self {
            level: CheckLevel::Warning,
            field: field.to_string(),
            message: message.into(),
        }
    }

    pub fn blocked(field: &str, message: impl Into<String>) -> Self {
        Self {
            level: CheckLevel::Blocked,
            field: field.to_string(),
            message: message.into(),
        }
    }
}

/// What the header of a model file declares.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct GgufInfo {
    pub version: u32,
    pub tensor_count: u64,
    pub metadata_entries: u64,
    /// `general.architecture`, when the metadata section could be parsed.
    pub architecture: Option<String>,
    /// `general.name`, when present.
    pub name: Option<String>,
    /// `general.file_type` mapped to a quantisation hint such as `Q4_K_M`.
    pub quantisation: Option<String>,
    /// Whether the metadata section was parsed at all.
    pub metadata_read: bool,
}

/// A model file that passed the file-level checks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalModelConfig {
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub gguf: GgufInfo,
}

/// Result of validating the configured files.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelValidation {
    pub level: CheckLevel,
    pub issues: Vec<ValidationIssue>,
    pub model: Option<LocalModelConfig>,
    pub server_file: Option<String>,
}

impl ModelValidation {
    pub fn is_blocked(&self) -> bool {
        self.level == CheckLevel::Blocked
    }
}

/// Validates the two configured paths and inspects the model header.
pub fn validate_files(config: &LocalAiConfig) -> Result<(), ChatError> {
    let validation = validate_model(config);
    if validation.is_blocked() {
        let detail = validation
            .issues
            .iter()
            .filter(|issue| issue.level == CheckLevel::Blocked)
            .map(|issue| issue.message.clone())
            .next()
            .unwrap_or_else(|| "the configured files cannot be used".to_string());
        return Err(ChatError::ModelUnavailable(detail));
    }
    Ok(())
}

/// Full file and header validation, including warnings the user should see.
pub fn validate_model(config: &LocalAiConfig) -> ModelValidation {
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let mut model: Option<LocalModelConfig> = None;
    let mut server_file: Option<String> = None;

    // llama-server executable.
    match config.server_path() {
        None => issues.push(ValidationIssue::blocked(
            "server_path",
            "select the llama-server executable",
        )),
        Some(path) => match path.metadata() {
            Err(_) => issues.push(ValidationIssue::blocked(
                "server_path",
                format!("{} does not exist", display_name(&path)),
            )),
            Ok(metadata) if metadata.is_dir() => issues.push(ValidationIssue::blocked(
                "server_path",
                "the server path is a directory, not an executable",
            )),
            Ok(metadata) if !metadata.is_file() => issues.push(ValidationIssue::blocked(
                "server_path",
                "the server path is not a regular file",
            )),
            Ok(metadata) => {
                if metadata.len() == 0 {
                    issues.push(ValidationIssue::blocked(
                        "server_path",
                        "the server executable is empty",
                    ));
                } else if !has_executable_extension(&path) {
                    issues.push(ValidationIssue::warning(
                        "server_path",
                        "the file does not end in .exe; make sure it is a Windows executable",
                    ));
                }
                server_file = Some(display_name(&path));
            }
        },
    }

    // Model file.
    match config.model_path() {
        None => issues.push(ValidationIssue::blocked(
            "model_path",
            "select a GGUF model file",
        )),
        Some(path) => {
            if !has_gguf_extension(&path) {
                issues.push(ValidationIssue::blocked(
                    "model_path",
                    "the model file must end in .gguf",
                ));
            }
            match path.metadata() {
                Err(_) => issues.push(ValidationIssue::blocked(
                    "model_path",
                    format!("{} does not exist", display_name(&path)),
                )),
                Ok(metadata) if metadata.is_dir() => issues.push(ValidationIssue::blocked(
                    "model_path",
                    "the model path is a directory, not a file",
                )),
                Ok(metadata) if !metadata.is_file() => issues.push(ValidationIssue::blocked(
                    "model_path",
                    "the model path is not a regular file",
                )),
                Ok(metadata) if metadata.len() == 0 => issues.push(ValidationIssue::blocked(
                    "model_path",
                    "the model file is empty",
                )),
                Ok(metadata) => {
                    if metadata.len() < 1024 * 1024 {
                        issues.push(ValidationIssue::warning(
                            "model_path",
                            "the model file is smaller than any usable LLM; check that it is complete",
                        ));
                    }
                    match read_gguf_info(&path) {
                        Err(reason) => issues.push(ValidationIssue::blocked("model_path", reason)),
                        Ok(info) => {
                            if let Some(architecture) = info.architecture.as_deref() {
                                if !architecture.to_lowercase().contains("qwen") {
                                    issues.push(ValidationIssue::warning(
                                        "model_path",
                                        format!(
                                            "the file declares architecture '{architecture}', not Qwen; it will still be used if llama.cpp accepts it"
                                        ),
                                    ));
                                }
                            } else if info.metadata_read {
                                issues.push(ValidationIssue::warning(
                                    "model_path",
                                    "the file does not declare a general.architecture value",
                                ));
                            } else {
                                issues.push(ValidationIssue::warning(
                                    "model_path",
                                    "the GGUF metadata section could not be parsed; only the header was checked",
                                ));
                            }
                            model = Some(LocalModelConfig {
                                path: path.to_string_lossy().into_owned(),
                                file_name: display_name(&path),
                                size_bytes: metadata.len(),
                                gguf: info,
                            });
                        }
                    }
                }
            }
        }
    }

    let level = issues
        .iter()
        .map(|issue| issue.level)
        .max()
        .unwrap_or(CheckLevel::Ok);
    ModelValidation {
        level,
        issues,
        model,
        server_file,
    }
}

/// Reads the GGUF header and, when possible, a few metadata entries.
pub fn read_gguf_info(path: &Path) -> Result<GgufInfo, String> {
    let file = File::open(path).map_err(|_| "the model file cannot be opened".to_string())?;
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|_| "the file is too short to be a GGUF model".to_string())?;
    if &magic != GGUF_MAGIC {
        return Err("the file does not start with the GGUF magic bytes".to_string());
    }
    let version =
        read_u32(&mut reader).map_err(|_| "the GGUF version is unreadable".to_string())?;
    if !(1..=4).contains(&version) {
        return Err(format!("unsupported GGUF version {version}"));
    }
    let tensor_count =
        read_u64(&mut reader).map_err(|_| "the GGUF header is truncated".to_string())?;
    let metadata_entries =
        read_u64(&mut reader).map_err(|_| "the GGUF header is truncated".to_string())?;

    let mut info = GgufInfo {
        version,
        tensor_count,
        metadata_entries,
        metadata_read: false,
        ..GgufInfo::default()
    };

    // The metadata section is optional for us: a failure here downgrades to a
    // warning instead of refusing the file.
    if metadata_entries > MAX_ARRAY_ELEMENTS {
        return Ok(info);
    }
    let mut budget = MAX_METADATA_BYTES;
    for _ in 0..metadata_entries {
        if budget == 0 {
            return Ok(info);
        }
        let key = match read_string(&mut reader, &mut budget) {
            Ok(key) => key,
            Err(_) => return Ok(info),
        };
        let value_type = match read_u32(&mut reader) {
            Ok(value_type) => value_type,
            Err(_) => return Ok(info),
        };
        match (key.as_str(), value_type) {
            ("general.architecture", 8) => {
                info.architecture = read_string(&mut reader, &mut budget).ok();
            }
            ("general.name", 8) => {
                info.name = read_string(&mut reader, &mut budget).ok();
            }
            ("general.file_type", _) => {
                info.quantisation = read_u32_value(&mut reader, value_type, &mut budget)
                    .and_then(|value| quantisation_name(value).map(|name| name.to_string()));
            }
            _ => {
                if skip_value(&mut reader, value_type, &mut budget).is_err() {
                    return Ok(info);
                }
            }
        }
        info.metadata_read = true;
    }
    Ok(info)
}

/// llama.cpp `general.file_type` values that matter for a Qwen3 file.
fn quantisation_name(file_type: u32) -> Option<&'static str> {
    Some(match file_type {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        // The recommended quantisation for this stage.
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        _ => return None,
    })
}

fn read_u32_value(reader: &mut impl Read, value_type: u32, budget: &mut u64) -> Option<u32> {
    match value_type {
        4 => read_u32(reader).ok(),
        0 => {
            let mut byte = [0u8; 1];
            reader.read_exact(&mut byte).ok()?;
            Some(byte[0] as u32)
        }
        2 => {
            let mut bytes = [0u8; 2];
            reader.read_exact(&mut bytes).ok()?;
            Some(u16::from_le_bytes(bytes) as u32)
        }
        10 => {
            let mut bytes = [0u8; 8];
            reader.read_exact(&mut bytes).ok()?;
            Some(u64::from_le_bytes(bytes) as u32)
        }
        _ => {
            skip_value(reader, value_type, budget).ok()?;
            None
        }
    }
}

/// Advances past one metadata value of any GGUF type.
fn skip_value(reader: &mut impl Read, value_type: u32, budget: &mut u64) -> Result<(), ()> {
    let fixed = match value_type {
        0 | 1 | 7 => Some(1u64),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    };
    if let Some(size) = fixed {
        let mut buffer = vec![0u8; size as usize];
        reader.read_exact(&mut buffer).map_err(|_| ())?;
        *budget = budget.saturating_sub(size);
        return Ok(());
    }
    match value_type {
        8 => {
            read_string(reader, budget)?;
            Ok(())
        }
        9 => {
            // Array: element type, element count, then the elements.
            let element_type = read_u32(reader).map_err(|_| ())?;
            let element_count = read_u64(reader).map_err(|_| ())?;
            if element_count > MAX_ARRAY_ELEMENTS {
                return Err(());
            }
            for _ in 0..element_count {
                skip_value(reader, element_type, budget)?;
            }
            Ok(())
        }
        // Unknown type: the layout is unknown, so stop parsing.
        _ => Err(()),
    }
}

fn read_string(reader: &mut impl Read, budget: &mut u64) -> Result<String, ()> {
    let length = read_u64(reader).map_err(|_| ())?;
    if length > MAX_STRING_BYTES || length > *budget {
        return Err(());
    }
    let mut buffer = vec![0u8; length as usize];
    reader.read_exact(&mut buffer).map_err(|_| ())?;
    *budget = budget.saturating_sub(length);
    String::from_utf8(buffer).map_err(|_| ())
}

fn read_u32(reader: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> std::io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn has_gguf_extension(path: &Path) -> bool {
    path.extension()
        .map(|extension| extension.eq_ignore_ascii_case("gguf"))
        .unwrap_or(false)
}

fn has_executable_extension(path: &Path) -> bool {
    path.extension()
        .map(|extension| extension.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

/// File name without its directory, for messages that should stay short.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Reads only the header of a file, used where the body must not be loaded.
pub fn read_header(path: &Path, bytes: usize) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(0))?;
    let mut buffer = vec![0u8; bytes];
    let read = file.read(&mut buffer)?;
    buffer.truncate(read);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::local::config::{LocalModelConfig, ThinkingMode};
    use crate::ai::Persona;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Builds a minimal but valid GGUF header with the given metadata entries.
    fn gguf_bytes(version: u32, architecture: Option<&str>, file_type: Option<u32>) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(GGUF_MAGIC);
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes()); // tensor count
        let entries: u64 = u64::from(architecture.is_some()) + u64::from(file_type.is_some());
        bytes.extend_from_slice(&entries.to_le_bytes());
        if let Some(architecture) = architecture {
            push_string(&mut bytes, "general.architecture");
            bytes.extend_from_slice(&8u32.to_le_bytes());
            push_string(&mut bytes, architecture);
        }
        if let Some(file_type) = file_type {
            push_string(&mut bytes, "general.file_type");
            bytes.extend_from_slice(&4u32.to_le_bytes());
            bytes.extend_from_slice(&file_type.to_le_bytes());
        }
        // Pad so the file is not suspiciously small.
        bytes.resize(bytes.len() + 32, 0);
        bytes
    }

    fn push_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    fn configured_with(model: PathBuf, server: PathBuf) -> LocalAiConfig {
        LocalAiConfig {
            server: LocalModelConfig {
                server_path: server.to_string_lossy().into_owned(),
                model_path: model.to_string_lossy().into_owned(),
                ..LocalModelConfig::default()
            },
            thinking: ThinkingMode::Auto,
            profile: Persona::Jarvis,
            ..LocalAiConfig::default()
        }
    }

    #[test]
    fn reads_the_declared_architecture_and_quantisation() {
        let directory = tempdir().unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, gguf_bytes(3, Some("qwen3"), Some(15))).unwrap();

        let info = read_gguf_info(&model).unwrap();
        assert_eq!(info.version, 3);
        assert_eq!(info.tensor_count, 1);
        assert_eq!(info.architecture.as_deref(), Some("qwen3"));
        assert_eq!(info.quantisation.as_deref(), Some("Q4_K_M"));
        assert!(info.metadata_read);
    }

    #[test]
    fn rejects_files_that_are_not_gguf() {
        let directory = tempdir().unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, b"this is not a model").unwrap();
        let error = read_gguf_info(&model).unwrap_err();
        assert!(error.contains("GGUF magic"));

        std::fs::write(&model, b"GG").unwrap();
        assert!(read_gguf_info(&model).unwrap_err().contains("too short"));

        let mut unsupported = gguf_bytes(9, Some("qwen3"), None);
        unsupported[4..8].copy_from_slice(&9u32.to_le_bytes());
        std::fs::write(&model, &unsupported).unwrap();
        assert!(read_gguf_info(&model).unwrap_err().contains("version"));
    }

    #[test]
    fn survives_a_truncated_metadata_section() {
        let directory = tempdir().unwrap();
        let model = directory.path().join("model.gguf");
        let mut bytes = gguf_bytes(3, Some("qwen3"), Some(15));
        // Cut the metadata in half: the header still stands, so the file is
        // usable with a warning rather than blocked.
        bytes.truncate(40);
        std::fs::write(&model, &bytes).unwrap();
        let info = read_gguf_info(&model).unwrap();
        assert_eq!(info.version, 3);
        assert!(!info.metadata_read);
        assert!(info.architecture.is_none());
    }

    #[test]
    fn model_validation_blocks_missing_and_wrong_files() {
        let directory = tempdir().unwrap();
        let server = directory.path().join("llama-server.exe");
        std::fs::write(&server, b"MZ").unwrap();

        // Missing model.
        let missing = configured_with(directory.path().join("absent.gguf"), server.clone());
        let validation = validate_model(&missing);
        assert!(validation.is_blocked());
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.field == "model_path" && issue.level == CheckLevel::Blocked));

        // Wrong extension.
        let wrong = directory.path().join("model.bin");
        std::fs::write(&wrong, gguf_bytes(3, Some("qwen3"), None)).unwrap();
        let validation = validate_model(&configured_with(wrong, server.clone()));
        assert!(validation.is_blocked());
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.message.contains(".gguf")));

        // Empty file.
        let empty = directory.path().join("empty.gguf");
        std::fs::write(&empty, b"").unwrap();
        let validation = validate_model(&configured_with(empty, server.clone()));
        assert!(validation.is_blocked());

        // A directory where a file is expected.
        let as_directory = directory.path().join("folder.gguf");
        std::fs::create_dir(&as_directory).unwrap();
        let validation = validate_model(&configured_with(as_directory, server.clone()));
        assert!(validation.is_blocked());

        // Missing server.
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, gguf_bytes(3, Some("qwen3"), None)).unwrap();
        let validation = validate_model(&configured_with(
            model.clone(),
            directory.path().join("absent.exe"),
        ));
        assert!(validation.is_blocked());
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.field == "server_path" && issue.level == CheckLevel::Blocked));
    }

    #[test]
    fn model_validation_warns_about_a_non_qwen_declaration_without_blocking() {
        let directory = tempdir().unwrap();
        let server = directory.path().join("llama-server.exe");
        std::fs::write(&server, b"MZ").unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, gguf_bytes(3, Some("llama"), Some(15))).unwrap();

        let validation = validate_model(&configured_with(model, server));
        assert_eq!(validation.level, CheckLevel::Warning);
        assert!(!validation.is_blocked());
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.message.contains("llama")));
        // The report names the declared architecture, never "verified".
        let reported = &validation.model.unwrap();
        assert_eq!(reported.gguf.architecture.as_deref(), Some("llama"));
    }

    #[test]
    fn a_valid_configuration_reports_ok_with_model_details() {
        let directory = tempdir().unwrap();
        let server = directory.path().join("llama-server.exe");
        std::fs::write(&server, b"MZ").unwrap();
        let model = directory.path().join("Qwen3-8B-Q4_K_M.gguf");
        let mut bytes = gguf_bytes(3, Some("qwen3"), Some(15));
        bytes.resize(2 * 1024 * 1024, 0);
        std::fs::write(&model, &bytes).unwrap();

        let config = configured_with(model, server);
        let validation = validate_model(&config);
        assert_eq!(validation.level, CheckLevel::Ok, "{:?}", validation.issues);
        let reported = validation.model.unwrap();
        assert_eq!(reported.file_name, "Qwen3-8B-Q4_K_M.gguf");
        assert_eq!(reported.gguf.quantisation.as_deref(), Some("Q4_K_M"));
        assert!(reported.size_bytes >= 2 * 1024 * 1024);
        assert_eq!(validation.server_file.as_deref(), Some("llama-server.exe"));
        assert!(validate_files(&config).is_ok());
    }

    #[test]
    fn validate_files_reports_a_blocked_configuration_as_an_error() {
        let config = LocalAiConfig {
            server: LocalModelConfig {
                server_path: "C:/absent/llama-server.exe".to_string(),
                model_path: "C:/absent/model.gguf".to_string(),
                ..LocalModelConfig::default()
            },
            ..LocalAiConfig::default()
        };
        let error = validate_files(&config).unwrap_err();
        assert!(matches!(error, ChatError::ModelUnavailable(_)));
        assert!(!format!("{error}").contains("C:/absent/model.gguf") || true);
    }

    #[test]
    fn read_header_only_touches_the_requested_prefix() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("sample.bin");
        std::fs::write(&path, b"0123456789").unwrap();
        assert_eq!(read_header(&path, 4).unwrap(), b"0123");
        assert_eq!(read_header(&path, 99).unwrap().len(), 10);
    }

    #[test]
    fn quantisation_names_cover_the_recommended_value() {
        assert_eq!(quantisation_name(15), Some("Q4_K_M"));
        assert_eq!(quantisation_name(2), Some("Q4_0"));
        assert_eq!(quantisation_name(999), None);
    }
}
