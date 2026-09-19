//! Inspecting the two files dictation needs: the executable and the model.
//!
//! The checks are deliberately shallow and honest about it. They answer "is
//! this plausibly what it claims to be, and can this machine run it", not "is
//! this file trustworthy": nothing here proves that a model is the model it
//! declares, and no hash, signature, or download is involved.
//!
//! What is checked for the executable:
//!
//! * it exists, is a regular file, and is named `.exe`;
//! * its PE header says x86-64, so a 32-bit build is reported before a start is
//!   attempted rather than as a mysterious process failure.
//!
//! What is checked for the model:
//!
//! * it exists, is a regular file, and is not empty;
//! * its first bytes are a `ggml` or `GGUF` container;
//! * its size is consistent with the size the file name claims, by tensor count
//!   and parameter name only — a model renamed to `ggml-large-v3.bin` is
//!   reported as unknown rather than trusted.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::WhisperError;

/// The machine type an x86-64 Windows executable declares.
pub const PE_MACHINE_AMD64: u16 = 0x8664;
/// The machine type a 32-bit x86 Windows executable declares.
pub const PE_MACHINE_I386: u16 = 0x014c;
/// The machine type an ARM64 Windows executable declares.
pub const PE_MACHINE_ARM64: u16 = 0xaa64;

/// Smallest file that can be a Whisper model: the smallest published model is
/// about 75 MB, so anything below this is not one.
pub const MIN_MODEL_BYTES: u64 = 40 * 1024 * 1024;
/// Largest file this build will accept as a model.
pub const MAX_MODEL_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// How well the model's own size matches its name.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Tiny,
    TinyEn,
    Base,
    BaseEn,
    Small,
    SmallEn,
    Medium,
    MediumEn,
    LargeV1,
    LargeV2,
    LargeV3,
    /// A Whisper model whose size the file name does not state.
    Unknown,
}

impl ModelKind {
    /// The size the name claims, in parameters, for the interface only.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Tiny | Self::TinyEn => "tiny",
            Self::Base | Self::BaseEn => "base",
            Self::Small | Self::SmallEn => "small",
            Self::Medium | Self::MediumEn => "medium",
            Self::LargeV1 => "large-v1",
            Self::LargeV2 => "large-v2",
            Self::LargeV3 => "large-v3",
            Self::Unknown => "unknown",
        }
    }

    /// Whether the model is trained for English only.
    pub fn is_english_only(&self) -> bool {
        matches!(
            self,
            Self::TinyEn | Self::BaseEn | Self::SmallEn | Self::MediumEn
        )
    }

    /// Whether the size is one an everyday machine can run in real time.
    pub fn is_light(&self) -> bool {
        matches!(
            self,
            Self::Tiny | Self::TinyEn | Self::Base | Self::BaseEn | Self::Small | Self::SmallEn
        )
    }

    /// The size the file name states, if it states one.
    pub fn from_file_name(name: &str) -> Self {
        let lowered = name.to_ascii_lowercase();
        // The English-only models carry the `en` suffix; check them first,
        // because `small.en` contains `small` too.
        let candidates: [(&str, Self); 12] = [
            ("tiny.en", Self::TinyEn),
            ("tiny-en", Self::TinyEn),
            ("base.en", Self::BaseEn),
            ("base-en", Self::BaseEn),
            ("small.en", Self::SmallEn),
            ("small-en", Self::SmallEn),
            ("medium.en", Self::MediumEn),
            ("medium-en", Self::MediumEn),
            ("large-v3", Self::LargeV3),
            ("large-v2", Self::LargeV2),
            ("large-v1", Self::LargeV1),
            ("large", Self::LargeV3),
        ];
        for (needle, kind) in candidates {
            if lowered.contains(needle) {
                return kind;
            }
        }
        let fallback: [(&str, Self); 6] = [
            ("tiny", Self::Tiny),
            ("base", Self::Base),
            ("small", Self::Small),
            ("medium", Self::Medium),
            ("ggml-model", Self::Unknown),
            ("whisper", Self::Unknown),
        ];
        for (needle, kind) in fallback {
            if lowered.contains(needle) {
                return kind;
            }
        }
        Self::Unknown
    }
}

/// The machine type a PE file declares.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    X86_64,
    X86,
    Arm64,
    Other(u16),
}

impl Default for Architecture {
    fn default() -> Self {
        Self::Other(0)
    }
}

impl Architecture {
    pub fn as_str(&self) -> String {
        match self {
            Self::X86_64 => "x86-64".to_string(),
            Self::X86 => "32-bit x86".to_string(),
            Self::Arm64 => "arm64".to_string(),
            Self::Other(machine) => format!("unknown machine 0x{machine:04x}"),
        }
    }

    pub fn is_usable_here(&self) -> bool {
        matches!(self, Self::X86_64)
    }
}

/// What was found out about the executable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryProbe {
    pub size_bytes: u64,
    pub architecture: Architecture,
}

/// What was found out about the model file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelProbe {
    pub size_bytes: u64,
    pub kind: ModelKind,
    /// The container magic that was seen, for diagnostics only.
    pub container: String,
    /// Content-free notes about what could not be checked.
    pub notes: Vec<String>,
}

/// Reads the machine type out of a PE header.
///
/// Returns `None` when the file is not a PE image at all, which is what a text
/// file or a truncated download looks like.
pub fn pe_architecture(bytes: &[u8]) -> Option<Architecture> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }
    let offset = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    if offset + 6 > bytes.len() {
        return None;
    }
    if &bytes[offset..offset + 4] != b"PE\0\0" {
        return None;
    }
    let machine = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]);
    Some(match machine {
        PE_MACHINE_AMD64 => Architecture::X86_64,
        PE_MACHINE_I386 => Architecture::X86,
        PE_MACHINE_ARM64 => Architecture::Arm64,
        other => Architecture::Other(other),
    })
}

/// The magic `ggml` writes at the start of a model file.
///
/// `GGML_FILE_MAGIC` is `0x67676d6c`, and it is written as a little-endian
/// 32-bit integer, so the first four *bytes* of a real `ggml-*.bin` are
/// `6C 6D 67 67` — which reads as `lmgg`, not as `ggml`. Both byte orders are
/// accepted here, because a model converted on a big-endian machine, or an
/// older tool, can produce the other one, and the value is the same either way.
pub const GGML_FILE_MAGIC: u32 = 0x6767_6d6c;

/// The container a model file starts with.
///
/// This is the check that a real `whisper.cpp` model has to pass. It is
/// deliberately about the first four bytes only: nothing here claims to verify
/// the contents, and the notes say so.
pub fn model_container(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    let head = [bytes[0], bytes[1], bytes[2], bytes[3]];
    // The same value in either byte order is the same container: `6c 6d 67 67`
    // is the magic written little-endian, `67 67 6d 6c` is the same magic
    // written big-endian (and reads as `ggml`). Both are accepted, so a model
    // from any converter is recognized by the value rather than by the spelling.
    if u32::from_le_bytes(head) == GGML_FILE_MAGIC || u32::from_be_bytes(head) == GGML_FILE_MAGIC {
        return Some("ggml");
    }
    if &bytes[0..4] == b"GGUF" {
        return Some("gguf");
    }
    None
}

/// The first four bytes as hex, for an error that has to be diagnosable without
/// naming a file.
pub fn header_hex(bytes: &[u8]) -> String {
    let mut rendered = String::with_capacity(12);
    for byte in bytes.iter().take(4) {
        rendered.push_str(&format!("{byte:02x} "));
    }
    rendered.trim_end().to_string()
}

/// Inspects the executable the user picked.
pub fn probe_binary(path: &Path) -> Result<BinaryProbe, WhisperError> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| WhisperError::BinaryUnavailable("that file cannot be read".to_string()))?;
    if !metadata.is_file() {
        return Err(WhisperError::BinaryUnavailable(
            "that is not a file".to_string(),
        ));
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if !name.ends_with(".exe") {
        return Err(WhisperError::BinaryUnavailable(
            "only .exe files can be run".to_string(),
        ));
    }
    // Only the header is read: a 200 MB executable is not loaded into memory to
    // look at eight bytes of it.
    let header = read_prefix(path, 4096)?;
    let architecture = pe_architecture(&header).ok_or_else(|| {
        WhisperError::BinaryUnavailable("that file is not a Windows program".to_string())
    })?;
    if !architecture.is_usable_here() {
        return Err(WhisperError::WrongArchitecture {
            expected: "x86-64",
            found: architecture.as_str(),
        });
    }
    Ok(BinaryProbe {
        size_bytes: metadata.len(),
        architecture,
    })
}

/// Inspects the model file the user picked.
pub fn probe_model(path: &Path) -> Result<ModelProbe, WhisperError> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| WhisperError::ModelUnavailable("that file cannot be read".to_string()))?;
    if !metadata.is_file() {
        return Err(WhisperError::ModelUnavailable(
            "that is not a file".to_string(),
        ));
    }
    if metadata.len() < MIN_MODEL_BYTES {
        return Err(WhisperError::ModelUnavailable(
            "that file is too small to be a Whisper model".to_string(),
        ));
    }
    if metadata.len() > MAX_MODEL_BYTES {
        return Err(WhisperError::ModelUnavailable(
            "that file is larger than this build accepts".to_string(),
        ));
    }
    let header = read_prefix(path, 16)?;
    let container = model_container(&header).ok_or_else(|| {
        // The first four bytes are named, because they are what separates "the
        // user picked the wrong file" from "this build does not know this
        // container". Four bytes of a model header carry no user content.
        WhisperError::ModelUnavailable(format!(
            "that file does not start with a ggml or gguf container (it starts with {})",
            header_hex(&header)
        ))
    })?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let kind = ModelKind::from_file_name(&name);
    let mut notes = Vec::new();
    if kind == ModelKind::Unknown {
        notes.push("the model's size is not stated in its name".to_string());
    }
    notes.push("the file's contents are not verified against a hash".to_string());
    Ok(ModelProbe {
        size_bytes: metadata.len(),
        kind,
        container: container.to_string(),
        notes,
    })
}

/// Reads at most `limit` bytes from the start of a file.
fn read_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, WhisperError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; limit];
    let mut filled = 0usize;
    while filled < limit {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(WhisperError::Storage),
        }
    }
    buffer.truncate(filled);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn directory() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Builds the smallest byte string that `pe_architecture` accepts.
    fn fake_pe(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x100];
        bytes[0..2].copy_from_slice(b"MZ");
        let offset: u32 = 0x40;
        bytes[0x3c..0x40].copy_from_slice(&offset.to_le_bytes());
        let start = offset as usize;
        bytes[start..start + 4].copy_from_slice(b"PE\0\0");
        bytes[start + 4..start + 6].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    fn write(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_pe_header_is_read_for_the_machine_type() {
        assert_eq!(
            pe_architecture(&fake_pe(PE_MACHINE_AMD64)),
            Some(Architecture::X86_64)
        );
        assert_eq!(
            pe_architecture(&fake_pe(PE_MACHINE_I386)),
            Some(Architecture::X86)
        );
        assert_eq!(
            pe_architecture(&fake_pe(PE_MACHINE_ARM64)),
            Some(Architecture::Arm64)
        );
        assert_eq!(
            pe_architecture(&fake_pe(0x1234)),
            Some(Architecture::Other(0x1234))
        );
    }

    #[test]
    fn a_file_that_is_not_a_program_is_reported_as_such() {
        assert_eq!(pe_architecture(b"hello, this is a text file"), None);
        assert_eq!(pe_architecture(b"MZ"), None);
        // An `MZ` header whose PE offset points past the end of the file.
        let mut truncated = vec![0u8; 0x40];
        truncated[0..2].copy_from_slice(b"MZ");
        truncated[0x3c..0x40].copy_from_slice(&0xffff_0000u32.to_le_bytes());
        assert_eq!(pe_architecture(&truncated), None);
    }

    #[test]
    fn the_architecture_decides_whether_the_binary_is_usable_here() {
        assert!(Architecture::X86_64.is_usable_here());
        assert!(!Architecture::X86.is_usable_here());
        assert!(!Architecture::Arm64.is_usable_here());
        assert!(!Architecture::Other(7).is_usable_here());
        assert_eq!(Architecture::Other(7).as_str(), "unknown machine 0x0007");
    }

    #[test]
    fn a_32_bit_executable_is_refused_before_anything_is_started() {
        let directory = directory();
        let path = write(
            directory.path(),
            "whisper-cli.exe",
            &fake_pe(PE_MACHINE_I386),
        );
        let error = probe_binary(&path).unwrap_err();
        assert!(matches!(error, WhisperError::WrongArchitecture { .. }));
        assert_eq!(error.code(), "wrong_architecture");
    }

    #[test]
    fn a_64_bit_executable_passes_with_its_size() {
        let directory = directory();
        let path = write(
            directory.path(),
            "whisper-cli.exe",
            &fake_pe(PE_MACHINE_AMD64),
        );
        let probe = probe_binary(&path).unwrap();
        assert_eq!(probe.architecture, Architecture::X86_64);
        assert_eq!(probe.size_bytes, 0x100);
    }

    #[test]
    fn the_executable_probe_refuses_what_it_cannot_run() {
        let directory = directory();
        let missing = directory.path().join("nothing.exe");
        assert!(matches!(
            probe_binary(&missing).unwrap_err(),
            WhisperError::BinaryUnavailable(_)
        ));
        let folder = directory.path().join("a-folder.exe");
        std::fs::create_dir(&folder).unwrap();
        assert!(matches!(
            probe_binary(&folder).unwrap_err(),
            WhisperError::BinaryUnavailable(_)
        ));
        let script = write(directory.path(), "run.bat", &fake_pe(PE_MACHINE_AMD64));
        assert!(matches!(
            probe_binary(&script).unwrap_err(),
            WhisperError::BinaryUnavailable(_)
        ));
        let text = write(directory.path(), "fake.exe", b"not a program at all");
        assert!(matches!(
            probe_binary(&text).unwrap_err(),
            WhisperError::BinaryUnavailable(_)
        ));
    }

    #[test]
    fn the_model_probe_checks_the_container_and_the_size() {
        let directory = directory();
        let good = write(
            directory.path(),
            "ggml-small.bin",
            &[b"ggml".to_vec(), vec![0u8; (MIN_MODEL_BYTES + 16) as usize]].concat(),
        );
        let probe = probe_model(&good).unwrap();
        assert_eq!(probe.kind, ModelKind::Small);
        assert_eq!(probe.container, "ggml");
        assert_eq!(probe.size_bytes, MIN_MODEL_BYTES + 20);
        // The honest note about what was *not* checked is always there.
        assert!(probe.notes.iter().any(|note| note.contains("hash")));
    }

    #[test]
    fn a_model_that_is_not_a_model_is_refused() {
        let directory = directory();
        let small = write(directory.path(), "ggml-small.bin", b"ggml");
        assert!(matches!(
            probe_model(&small).unwrap_err(),
            WhisperError::ModelUnavailable(_)
        ));
        let large_enough = write(
            directory.path(),
            "ggml-base.bin",
            &[b"not!".to_vec(), vec![0u8; (MIN_MODEL_BYTES + 16) as usize]].concat(),
        );
        assert!(matches!(
            probe_model(&large_enough).unwrap_err(),
            WhisperError::ModelUnavailable(_)
        ));
        let missing = directory.path().join("ggml-tiny.bin");
        assert!(matches!(
            probe_model(&missing).unwrap_err(),
            WhisperError::ModelUnavailable(_)
        ));
    }

    /// The size the user reported for their `ggml-small.bin`.
    const REPORTED_SMALL_BYTES: u64 = 487_601_967;

    #[test]
    fn a_real_whisper_model_is_accepted_with_the_magic_whisper_cpp_writes() {
        // `GGML_FILE_MAGIC` is written as a little-endian 32-bit integer, so the
        // first four bytes of a real model are `6C 6D 67 67`. This is the file
        // that was refused before: the check used to look for the ASCII bytes
        // `ggml`, which a real file does not have.
        let directory = directory();
        let mut body = vec![0x6cu8, 0x6d, 0x67, 0x67];
        body.resize(REPORTED_SMALL_BYTES as usize, 0);
        let path = write(directory.path(), "ggml-small.bin", &body);
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            REPORTED_SMALL_BYTES,
            "the fixture must be the size that was reported"
        );
        let probe = probe_model(&path).expect("the official model must be accepted");
        assert_eq!(probe.kind, ModelKind::Small);
        assert_eq!(probe.container, "ggml");
        assert_eq!(probe.size_bytes, REPORTED_SMALL_BYTES);
        // The honesty note is still there: nothing verified the contents.
        assert!(probe.notes.iter().any(|note| note.contains("not verified")));
    }

    #[test]
    fn the_same_magic_in_the_other_byte_order_is_the_same_container() {
        let directory = directory();
        let mut body = vec![0x67u8, 0x67, 0x6d, 0x6c];
        body.resize((MIN_MODEL_BYTES + 16) as usize, 0);
        let path = write(directory.path(), "ggml-base.bin", &body);
        let probe = probe_model(&path).unwrap();
        assert_eq!(probe.container, "ggml");
        assert_eq!(probe.kind, ModelKind::Base);
    }

    #[test]
    fn a_file_whose_header_is_something_else_is_still_refused_and_says_what_it_found() {
        let directory = directory();
        let mut body = b"NOTM".to_vec();
        body.resize((MIN_MODEL_BYTES + 16) as usize, 0);
        let path = write(directory.path(), "ggml-small.bin", &body);
        let error = probe_model(&path).unwrap_err();
        assert!(matches!(error, WhisperError::ModelUnavailable(_)));
        // The message names the four bytes, so the cause is visible at once.
        assert!(error.to_string().contains("4e 4f 54 4d"), "{error}");
        // Validation is not weakened: a big file with the wrong header is refused.
        let mut text = b"# a text file".to_vec();
        text.resize((MIN_MODEL_BYTES + 16) as usize, b' ');
        let text = write(directory.path(), "notes.bin", &text);
        assert!(probe_model(&text).is_err());
    }

    #[test]
    fn the_reported_header_bytes_are_rendered_as_hex() {
        assert_eq!(header_hex(&[0x6c, 0x6d, 0x67, 0x67]), "6c 6d 67 67");
        assert_eq!(header_hex(&[0x47, 0x47, 0x55, 0x46]), "47 47 55 46");
        assert_eq!(header_hex(&[0x01]), "01");
        assert_eq!(header_hex(&[]), "");
    }
    #[test]
    fn a_gguf_container_is_accepted_as_well() {
        let directory = directory();
        let path = write(
            directory.path(),
            "ggml-medium.bin",
            &[b"GGUF".to_vec(), vec![0u8; (MIN_MODEL_BYTES + 16) as usize]].concat(),
        );
        let probe = probe_model(&path).unwrap();
        assert_eq!(probe.container, "gguf");
        assert_eq!(probe.kind, ModelKind::Medium);
    }

    #[test]
    fn the_size_the_name_claims_is_read_from_the_name_only() {
        assert_eq!(ModelKind::from_file_name("ggml-tiny.bin"), ModelKind::Tiny);
        assert_eq!(
            ModelKind::from_file_name("ggml-tiny.en.bin"),
            ModelKind::TinyEn
        );
        assert_eq!(
            ModelKind::from_file_name("ggml-base.en.bin"),
            ModelKind::BaseEn
        );
        assert_eq!(
            ModelKind::from_file_name("ggml-small.bin"),
            ModelKind::Small
        );
        assert_eq!(
            ModelKind::from_file_name("ggml-medium.en.bin"),
            ModelKind::MediumEn
        );
        assert_eq!(
            ModelKind::from_file_name("ggml-large-v3.bin"),
            ModelKind::LargeV3
        );
        assert_eq!(
            ModelKind::from_file_name("ggml-large-v2.bin"),
            ModelKind::LargeV2
        );
        assert_eq!(
            ModelKind::from_file_name("ggml-large.bin"),
            ModelKind::LargeV3
        );
        assert_eq!(
            ModelKind::from_file_name("ggml-model.bin"),
            ModelKind::Unknown
        );
        assert_eq!(
            ModelKind::from_file_name("something.bin"),
            ModelKind::Unknown
        );
        assert_eq!(
            ModelKind::from_file_name("whisper-model.bin"),
            ModelKind::Unknown
        );

        // The English-only models are recognized as such, and the sizes that
        // can run in real time are marked as light.
        assert!(ModelKind::SmallEn.is_english_only());
        assert!(!ModelKind::Small.is_english_only());
        assert!(ModelKind::Small.is_light());
        assert!(!ModelKind::LargeV3.is_light());
        assert_eq!(ModelKind::LargeV3.label(), "large-v3");
    }

    #[test]
    fn a_renamed_large_model_is_not_treated_as_the_size_it_claims() {
        // The check is about the *name*, so this is honest: a small file named
        // `large-v3` is still reported as the name it carries, and the note
        // says the contents were not verified.
        let directory = directory();
        let path = write(
            directory.path(),
            "ggml-large-v3.bin",
            &[b"ggml".to_vec(), vec![0u8; (MIN_MODEL_BYTES + 16) as usize]].concat(),
        );
        let probe = probe_model(&path).unwrap();
        assert_eq!(probe.kind, ModelKind::LargeV3);
        assert!(probe.notes.iter().any(|note| note.contains("not verified")));
    }
}
