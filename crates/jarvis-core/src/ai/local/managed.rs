//! Verified, managed local-AI artifacts.
//!
//! URLs live in this module rather than in settings, prompts, or model output. A
//! download is first written as `.part`, bounded, hashed, and renamed atomically.
//! This module deliberately does *not* execute downloaded files; the gateway
//! performs its existing validation before it can start `llama-server`.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum accepted runtime archive size (128 MiB).
pub const MAX_RUNTIME_BYTES: u64 = 128 * 1024 * 1024;
/// Maximum accepted model size (12 GiB), intentionally above the pinned Q4 model.
pub const MAX_MODEL_BYTES: u64 = 12 * 1024 * 1024 * 1024;
/// A release archive must never contain an unbounded number of entries.
pub const MAX_RUNTIME_ARCHIVE_FILES: usize = 64;
/// The CPU archive is small; this cap stops decompression bombs before disk use.
pub const MAX_RUNTIME_UNPACKED_BYTES: u64 = 96 * 1024 * 1024;
pub const MAX_RUNTIME_COMPRESSION_RATIO: u64 = 100;

pub const RUNTIME_ARCHIVE_FILES: &[&str] = &[
    "ggml-base.dll",
    "ggml-cpu-alderlake.dll",
    "ggml-cpu-cannonlake.dll",
    "ggml-cpu-cascadelake.dll",
    "ggml-cpu-cooperlake.dll",
    "ggml-cpu-haswell.dll",
    "ggml-cpu-icelake.dll",
    "ggml-cpu-ivybridge.dll",
    "ggml-cpu-piledriver.dll",
    "ggml-cpu-sandybridge.dll",
    "ggml-cpu-sapphirerapids.dll",
    "ggml-cpu-skylakex.dll",
    "ggml-cpu-sse42.dll",
    "ggml-cpu-x64.dll",
    "ggml-cpu-zen4.dll",
    "ggml-rpc-server.exe",
    "ggml-rpc.dll",
    "ggml.dll",
    "libomp.dll",
    "LICENSE-LLVM-OpenMP",
    "llama-batched-bench-impl.dll",
    "llama-batched-bench.exe",
    "llama-bench-impl.dll",
    "llama-bench.exe",
    "llama-cli-impl.dll",
    "llama-cli.exe",
    "llama-common.dll",
    "llama-completion-impl.dll",
    "llama-completion.exe",
    "llama-fit-params-impl.dll",
    "llama-fit-params.exe",
    "llama-gemma3-cli.exe",
    "llama-gguf-split.exe",
    "llama-imatrix.exe",
    "llama-llava-cli.exe",
    "llama-minicpmv-cli.exe",
    "llama-mtmd-cli.exe",
    "llama-mtmd-debug.exe",
    "llama-perplexity-impl.dll",
    "llama-perplexity.exe",
    "llama-quantize-impl.dll",
    "llama-quantize.exe",
    "llama-qwen2vl-cli.exe",
    "llama-results.exe",
    "llama-server-impl.dll",
    "llama-server.exe",
    "llama-tokenize.exe",
    "llama-tts.exe",
    "llama.dll",
    "llama.exe",
    "mtmd.dll",
];

pub const RUNTIME_INSTALLED_FILES: &[&str] = &[
    "ggml-base.dll",
    "ggml-cpu-alderlake.dll",
    "ggml-cpu-cannonlake.dll",
    "ggml-cpu-cascadelake.dll",
    "ggml-cpu-cooperlake.dll",
    "ggml-cpu-haswell.dll",
    "ggml-cpu-icelake.dll",
    "ggml-cpu-ivybridge.dll",
    "ggml-cpu-piledriver.dll",
    "ggml-cpu-sandybridge.dll",
    "ggml-cpu-sapphirerapids.dll",
    "ggml-cpu-skylakex.dll",
    "ggml-cpu-sse42.dll",
    "ggml-cpu-x64.dll",
    "ggml-cpu-zen4.dll",
    "ggml.dll",
    "libomp.dll",
    "LICENSE-LLVM-OpenMP",
    "llama-common.dll",
    "llama-server-impl.dll",
    "llama-server.exe",
    "llama.dll",
];

/// An artifact compiled into the application manifest, never obtained from AI or
/// user-provided text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedArtifact {
    pub source_url: &'static str,
    pub filename: &'static str,
    pub expected_size: u64,
    pub sha256: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedRuntimeManifest {
    pub version: &'static str,
    pub artifact: ManagedArtifact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedModelManifest {
    pub model_id: &'static str,
    pub display_name: &'static str,
    pub source_revision: &'static str,
    pub artifact: ManagedArtifact,
    pub quantization: &'static str,
    pub context_recommendation: u32,
    pub ram_recommendation_bytes: u64,
    pub license_identifier: &'static str,
    pub license_url: &'static str,
}

/// Official ggml-org release. This is deliberately pinned instead of resolving
/// `latest`, which makes the hash and allowed URL stable.
pub fn managed_runtime_manifest() -> ManagedRuntimeManifest {
    ManagedRuntimeManifest {
        version: "b10964",
        artifact: ManagedArtifact {
            source_url: "https://github.com/ggml-org/llama.cpp/releases/download/b10964/llama-b10964-bin-win-cpu-x64.zip",
            filename: "llama-b10964-bin-win-cpu-x64.zip",
            expected_size: 18_427_629,
            sha256: "917f39c076402c421224824607397af20f53625a60defc20e8dd22446bf4c5d7",
        },
    }
}

/// Official Qwen GGUF repository, pinned to a revision and LFS SHA-256.
pub fn managed_model_manifest() -> ManagedModelManifest {
    ManagedModelManifest {
        model_id: "Qwen/Qwen3-8B-GGUF:Qwen3-8B-Q4_K_M",
        display_name: "Qwen3-8B Q4_K_M",
        source_revision: "7c41481f57cb95916b40956ab2f0b139b296d974",
        artifact: ManagedArtifact {
            source_url: "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/7c41481f57cb95916b40956ab2f0b139b296d974/Qwen3-8B-Q4_K_M.gguf",
            filename: "Qwen3-8B-Q4_K_M.gguf",
            expected_size: 5_027_783_488,
            sha256: "a56061d03bd2055a8236c8a80ec2440a550a53eaecf935fb2ddf37c93995667c",
        },
        quantization: "Q4_K_M",
        context_recommendation: 8192,
        ram_recommendation_bytes: 14 * 1024 * 1024 * 1024,
        license_identifier: "Apache-2.0",
        license_url: "https://huggingface.co/Qwen/Qwen3-8B-GGUF/blob/main/LICENSE",
    }
}

/// `%LOCALAPPDATA%\\com.priler.jarvis\\runtime\\llama.cpp-<version>` on Windows.
pub fn managed_runtime_directory(data_dir: &Path, version: &str) -> PathBuf {
    data_dir.join("runtime").join("llama.cpp").join(version)
}

/// `%LOCALAPPDATA%\\com.priler.jarvis\\models` on Windows.
pub fn managed_model_directory(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join("qwen3-8b-q4_k_m")
}

/// A path-free local record proving which compiled-in artifact was activated.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstallationReceipt {
    pub kind: String,
    pub version: String,
    pub artifact_name: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub installed_at: String,
    pub files: Vec<String>,
}

/// Atomically promotes a validated staging runtime. A previous runtime remains
/// untouched until the rename succeeds; a repeat run returns the existing
/// verified server path rather than copying again.
pub fn activate_runtime(staging: &Path, data_dir: &Path) -> Result<PathBuf, RuntimeInstallError> {
    let manifest = managed_runtime_manifest();
    let final_dir = managed_runtime_directory(data_dir, manifest.version);
    let server_name = "llama-server.exe";
    if final_dir.join(server_name).is_file() {
        validate_pe_x64(&final_dir.join(server_name))?;
        return Ok(final_dir.join(server_name));
    }
    let parent = final_dir.parent().ok_or(RuntimeInstallError::Io)?;
    fs::create_dir_all(parent).map_err(|_| RuntimeInstallError::Io)?;
    if !staging.join(server_name).is_file() {
        return Err(RuntimeInstallError::RuntimeMissing);
    }
    if final_dir.exists() {
        return Err(RuntimeInstallError::Io);
    }
    fs::rename(staging, &final_dir).map_err(|_| RuntimeInstallError::Io)?;
    let server = final_dir.join(server_name);
    if let Err(error) = validate_pe_x64(&server) {
        let _ = fs::remove_dir_all(&final_dir);
        return Err(error);
    }
    let facts = InstallationFacts {
        kind: "llama.cpp".to_string(),
        version: manifest.version.to_string(),
        artifact_name: manifest.artifact.filename.to_string(),
        sha256: manifest.artifact.sha256.to_string(),
        size_bytes: manifest.artifact.expected_size,
        files: RUNTIME_INSTALLED_FILES
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    };
    write_installation_receipt(&final_dir, &facts).map_err(|_| RuntimeInstallError::Io)?;
    Ok(server)
}

/// What an installation receipt records.
///
/// A receipt is proof of ownership: the removal commands only delete a directory
/// that carries one of these, so a folder the user created is never touched. It
/// holds no path and no conversation content.
#[derive(Clone, Debug)]
pub struct InstallationFacts {
    pub kind: String,
    pub version: String,
    pub artifact_name: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub files: Vec<String>,
}

/// Name of the receipt inside an installed component directory.
pub const RECEIPT_FILE: &str = "installation-receipt.json";

/// Writes the receipt for an installed component.
pub fn write_installation_receipt(
    directory: &Path,
    facts: &InstallationFacts,
) -> std::io::Result<()> {
    let receipt = InstallationReceipt {
        kind: facts.kind.clone(),
        version: facts.version.clone(),
        artifact_name: facts.artifact_name.clone(),
        sha256: facts.sha256.clone(),
        size_bytes: facts.size_bytes,
        installed_at: chrono::Utc::now().to_rfc3339(),
        files: facts.files.clone(),
    };
    crate::fsutil::write_json_atomic(&directory.join(RECEIPT_FILE), &receipt)
}

/// Reads the receipt of an installed component, when it carries one.
pub fn read_installation_receipt(directory: &Path) -> Option<InstallationReceipt> {
    let text = fs::read_to_string(directory.join(RECEIPT_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether a directory was installed by this application and can therefore be
/// removed by it.
pub fn has_installation_receipt(directory: &Path, kind: &str) -> bool {
    read_installation_receipt(directory).is_some_and(|receipt| receipt.kind == kind)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadError {
    RefusedUrl,
    Destination,
    TooLarge,
    SizeMismatch,
    HashMismatch,
    Cancelled,
    Network,
    Io,
}

/// Errors from offline archive validation and extraction. They are deliberately
/// key-like: the UI translates them and no file path leaks into its DTO.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeInstallError {
    ArchiveInvalid,
    ArchivePathTraversal,
    ArchiveTooLarge,
    ArchiveUnexpectedFile,
    RuntimeMissing,
    RuntimeArchitectureMismatch,
    /// The caller asked to stop while the archive was being unpacked.
    Cancelled,
    Io,
}

/// Validation result for the pinned GGUF before it may become active.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelInstallError {
    SizeMismatch,
    HashMismatch,
    GgufInvalid,
    ArchitectureMismatch,
    QuantizationMismatch,
    Io,
}

/// Promotes a previously downloaded and strictly validated model without ever
/// touching a user-selected GGUF. The source must be in app staging; the final
/// name is fixed by the compiled manifest.
pub fn activate_managed_model(
    staging_model: &Path,
    data_dir: &Path,
) -> Result<PathBuf, ModelInstallError> {
    validate_managed_model(staging_model)?;
    activate_managed_model_after_hash(staging_model, data_dir)
}

/// [`activate_managed_model`] for a caller that has just verified the full
/// SHA-256 of the staging file.
///
/// The bytes cannot have changed since, because the promotion is a rename inside
/// one volume, so re-reading several gigabytes would only repeat work. The size,
/// the GGUF header, the architecture, and the quantisation are still checked.
pub fn activate_managed_model_after_hash(
    staging_model: &Path,
    data_dir: &Path,
) -> Result<PathBuf, ModelInstallError> {
    let manifest = managed_model_manifest();
    let final_dir = managed_model_directory(data_dir);
    let final_model = final_dir.join(manifest.artifact.filename);
    if final_model.is_file() {
        return validate_managed_model(&final_model).map(|_| final_model);
    }
    validate_managed_model_metadata(staging_model)?;
    let parent = final_dir.parent().ok_or(ModelInstallError::Io)?;
    fs::create_dir_all(parent).map_err(|_| ModelInstallError::Io)?;
    if final_dir.exists() {
        return Err(ModelInstallError::Io);
    }
    let staging_parent = staging_model.parent().ok_or(ModelInstallError::Io)?;
    if staging_model.file_name().and_then(|name| name.to_str()) != Some(manifest.artifact.filename)
    {
        return Err(ModelInstallError::GgufInvalid);
    }
    fs::rename(staging_parent, &final_dir).map_err(|_| ModelInstallError::Io)?;
    if let Err(error) = validate_managed_model_metadata(&final_model) {
        let _ = fs::remove_dir_all(&final_dir);
        return Err(error);
    }
    let facts = InstallationFacts {
        kind: "model".to_string(),
        version: manifest.source_revision.to_string(),
        artifact_name: manifest.artifact.filename.to_string(),
        sha256: manifest.artifact.sha256.to_string(),
        size_bytes: manifest.artifact.expected_size,
        files: vec![manifest.artifact.filename.to_string()],
    };
    write_installation_receipt(&final_dir, &facts).map_err(|_| ModelInstallError::Io)?;
    Ok(final_model)
}

/// Checks a completed managed model without loading tensors into memory. This is
/// intentionally separate from the interactive, permissive file picker: a
/// managed activation has a stricter pinned identity.
pub fn validate_managed_model(path: &Path) -> Result<super::GgufInfo, ModelInstallError> {
    let manifest = managed_model_manifest();
    let info = validate_managed_model_metadata(path)?;
    if sha256_file(path)? != manifest.artifact.sha256 {
        return Err(ModelInstallError::HashMismatch);
    }
    Ok(info)
}

/// Everything a managed model check needs except the full-file hash: the size,
/// the GGUF header, the architecture, and the quantisation.
pub fn validate_managed_model_metadata(path: &Path) -> Result<super::GgufInfo, ModelInstallError> {
    let manifest = managed_model_manifest();
    let metadata = path.metadata().map_err(|_| ModelInstallError::Io)?;
    if !metadata.is_file() {
        return Err(ModelInstallError::GgufInvalid);
    }
    if metadata.len() != manifest.artifact.expected_size {
        return Err(ModelInstallError::SizeMismatch);
    }
    let info = super::read_gguf_info(path).map_err(|_| ModelInstallError::GgufInvalid)?;
    if !info
        .architecture
        .as_deref()
        .is_some_and(|architecture| architecture.eq_ignore_ascii_case("qwen3"))
    {
        return Err(ModelInstallError::ArchitectureMismatch);
    }
    if info.quantisation.as_deref() != Some(manifest.quantization) {
        return Err(ModelInstallError::QuantizationMismatch);
    }
    Ok(info)
}

fn sha256_file(path: &Path) -> Result<String, ModelInstallError> {
    let mut file = File::open(path).map_err(|_| ModelInstallError::Io)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| ModelInstallError::Io)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// The rules one runtime archive is checked against.
///
/// Pinned in production, and supplied by a test fixture in the setup tests, so
/// the same extraction code path is exercised with a few kilobytes instead of a
/// real release download.
pub struct RuntimeArchiveSpec<'a> {
    /// Names the archive is allowed to contain. Anything else is refused, which
    /// also rejects a directory entry or a symbolic link.
    pub allowed: &'a [&'static str],
    /// Names that must be written and must exist afterwards.
    pub keep: &'a [&'static str],
    pub max_files: usize,
    pub max_unpacked_bytes: u64,
    /// Largest unpacked-to-packed ratio a single entry may have.
    pub max_compression_ratio: u64,
    /// Whether `llama-server.exe` must be an x86-64 PE image.
    pub require_pe_x64: bool,
    /// Entry name of the server executable.
    pub server_name: &'a str,
}

/// The spec for the pinned release archive.
pub fn pinned_runtime_archive_spec() -> RuntimeArchiveSpec<'static> {
    RuntimeArchiveSpec {
        allowed: RUNTIME_ARCHIVE_FILES,
        keep: RUNTIME_INSTALLED_FILES,
        max_files: MAX_RUNTIME_ARCHIVE_FILES,
        max_unpacked_bytes: MAX_RUNTIME_UNPACKED_BYTES,
        max_compression_ratio: MAX_RUNTIME_COMPRESSION_RATIO,
        require_pe_x64: true,
        // The pinned release contains exactly one server executable.
        server_name: "llama-server.exe",
    }
}

/// Checks the shape of an archive without writing anything.
///
/// Returns the total unpacked size, so a caller can compare it with what it
/// planned for. Refuses a traversal, an unexpected entry, a directory, a
/// symbolic link, and a decompression bomb.
pub fn validate_runtime_archive(
    archive: &Path,
    spec: &RuntimeArchiveSpec<'_>,
) -> Result<u64, RuntimeInstallError> {
    let file = File::open(archive).map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
    if zip.is_empty() || zip.len() > spec.max_files {
        return Err(RuntimeInstallError::ArchiveTooLarge);
    }
    let mut total = 0_u64;
    let mut present: Vec<String> = Vec::new();
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
        let name = entry.name().to_string();
        if !safe_archive_name(&name) {
            return Err(RuntimeInstallError::ArchivePathTraversal);
        }
        if entry.is_dir() || entry.is_symlink() || !spec.allowed.contains(&name.as_str()) {
            return Err(RuntimeInstallError::ArchiveUnexpectedFile);
        }
        let size = entry.size();
        let packed = entry.compressed_size();
        if size > spec.max_unpacked_bytes
            || (packed > 0 && size / packed > spec.max_compression_ratio)
        {
            return Err(RuntimeInstallError::ArchiveTooLarge);
        }
        total = total
            .checked_add(size)
            .ok_or(RuntimeInstallError::ArchiveTooLarge)?;
        if total > spec.max_unpacked_bytes {
            return Err(RuntimeInstallError::ArchiveTooLarge);
        }
        present.push(name);
    }
    for required in spec.keep {
        if !present.iter().any(|name| name == required) {
            return Err(RuntimeInstallError::RuntimeMissing);
        }
    }
    Ok(total)
}

/// Validates and extracts the server's minimal runtime set into an empty staging
/// directory. It never starts an executable and never writes outside `staging`.
pub fn extract_runtime_archive(
    archive: &Path,
    staging: &Path,
) -> Result<PathBuf, RuntimeInstallError> {
    extract_runtime_archive_with(archive, staging, &pinned_runtime_archive_spec())
}

/// [`extract_runtime_archive`] against an explicit set of rules.
pub fn extract_runtime_archive_with(
    archive: &Path,
    staging: &Path,
    spec: &RuntimeArchiveSpec<'_>,
) -> Result<PathBuf, RuntimeInstallError> {
    extract_runtime_archive_cancellable(archive, staging, spec, &|| false)
}

/// [`extract_runtime_archive_with`] that can be stopped between chunks.
///
/// `cancelled` is polled for every written chunk, so the setup coordinator can
/// honour a cancellation while a large library is being unpacked instead of after
/// the whole archive has been written.
pub fn extract_runtime_archive_cancellable(
    archive: &Path,
    staging: &Path,
    spec: &RuntimeArchiveSpec<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<PathBuf, RuntimeInstallError> {
    // The shape is checked first, so nothing is written for an archive that will
    // be refused anyway.
    validate_runtime_archive(archive, spec)?;
    let file = File::open(archive).map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
    fs::create_dir_all(staging).map_err(|_| RuntimeInstallError::Io)?;
    let staging = staging
        .canonicalize()
        .map_err(|_| RuntimeInstallError::Io)?;
    let mut server = None;
    let mut buffer = vec![0_u8; 64 * 1024];
    for index in 0..zip.len() {
        if cancelled() {
            return Err(RuntimeInstallError::Cancelled);
        }
        let mut entry = zip
            .by_index(index)
            .map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
        let name = entry.name().to_string();
        if !spec.keep.contains(&name.as_str()) {
            continue;
        }
        let target = staging.join(&name);
        if !target.starts_with(&staging) {
            return Err(RuntimeInstallError::ArchivePathTraversal);
        }
        let mut output = File::create(&target).map_err(|_| RuntimeInstallError::Io)?;
        loop {
            if cancelled() {
                // The half-written file is removed: a partially extracted
                // library must never look like a verified one.
                drop(output);
                let _ = fs::remove_file(&target);
                return Err(RuntimeInstallError::Cancelled);
            }
            let count = entry
                .read(&mut buffer)
                .map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|_| RuntimeInstallError::Io)?;
        }
        output.sync_all().map_err(|_| RuntimeInstallError::Io)?;
        if name == spec.server_name {
            server = Some(target);
        }
    }
    let server = server.ok_or(RuntimeInstallError::RuntimeMissing)?;
    if spec.require_pe_x64 {
        validate_pe_x64(&server)?;
    }
    for required in spec.keep {
        if !staging.join(required).is_file() {
            return Err(RuntimeInstallError::RuntimeMissing);
        }
    }
    Ok(server)
}

fn safe_archive_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with('.')
        && name.len() <= 120
}

/// Checks only the DOS/PE headers and the x86-64 machine tag. This deliberately
/// does not execute or load the binary.
///
/// The layout is the one the PE format defines: `e_lfanew` points at the
/// `IMAGE_NT_HEADERS`, whose first four bytes are the `PE\0\0` signature and
/// whose next two bytes are `IMAGE_FILE_HEADER.Machine`. The machine tag is
/// therefore at `e_lfanew + 4`, not `e_lfanew + 6`; reading the wrong offset
/// would report every real x64 executable as the wrong architecture and refuse
/// a correct installation.
pub fn validate_pe_x64(path: &Path) -> Result<(), RuntimeInstallError> {
    let bytes = fs::read(path).map_err(|_| RuntimeInstallError::Io)?;
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return Err(RuntimeInstallError::RuntimeArchitectureMismatch);
    }
    let offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    if bytes.get(offset..offset + 4) != Some(b"PE\0\0") {
        return Err(RuntimeInstallError::RuntimeArchitectureMismatch);
    }
    let machine_at = offset
        .checked_add(4)
        .filter(|value| *value <= bytes.len().saturating_sub(2))
        .ok_or(RuntimeInstallError::RuntimeArchitectureMismatch)?;
    if bytes.get(machine_at..machine_at + 2) != Some(&0x8664_u16.to_le_bytes()) {
        return Err(RuntimeInstallError::RuntimeArchitectureMismatch);
    }
    Ok(())
}

/// Downloads a compiled-in artifact. `cancelled` and `progress` are polled for
/// every chunk, so callers can run this on a worker without freezing the UI.
///
/// This is a thin wrapper over the setup module's downloader, so there is exactly
/// one implementation of "fetch, bound, hash, then rename". A failed download
/// keeps its `.part` file, which is what makes the setup retry a resume.
pub fn download_verified<F, C>(
    artifact: &ManagedArtifact,
    destination: &Path,
    mut progress: F,
    mut cancelled: C,
) -> Result<(), DownloadError>
where
    F: FnMut(DownloadProgress),
    C: FnMut() -> bool,
{
    validate_artifact(artifact)?;
    if !is_allowlisted(artifact) {
        return Err(DownloadError::RefusedUrl);
    }
    if destination.file_name().and_then(|name| name.to_str()) != Some(artifact.filename) {
        return Err(DownloadError::Destination);
    }
    let parent = destination.parent().ok_or(DownloadError::Destination)?;
    fs::create_dir_all(parent).map_err(|_| DownloadError::Io)?;
    let expectation = super::setup::download::ArtifactExpectation::from(artifact.clone());
    let part = destination.with_file_name(format!("{}.part", artifact.filename));
    let transport =
        super::setup::download::ReqwestTransport::new().map_err(|_| DownloadError::Network)?;
    let request = super::setup::download::DownloadRequest {
        expectation: &expectation,
        part: &part,
        trust: super::setup::download::ArtifactTrust::Pinned,
    };
    match super::setup::download::download_artifact(
        &transport,
        request,
        &mut progress,
        &mut cancelled,
    ) {
        // Verified: promote by a rename, which costs no space.
        Ok(_) => super::setup::layout::promote_file(&part, destination)
            .map_err(|_| DownloadError::Io),
        Err(error) => Err(map_setup_download_error(error)),
    }
}

/// Maps the setup downloader's error to the vocabulary this module already had.
fn map_setup_download_error(
    error: super::setup::download::DownloadError,
) -> DownloadError {
    use super::setup::download::DownloadError as Setup;
    match error {
        Setup::RefusedUrl => DownloadError::RefusedUrl,
        Setup::Destination => DownloadError::Destination,
        Setup::TooLarge => DownloadError::TooLarge,
        Setup::SizeMismatch => DownloadError::SizeMismatch,
        Setup::HashMismatch => DownloadError::HashMismatch,
        Setup::Cancelled => DownloadError::Cancelled,
        Setup::Network | Setup::Timeout | Setup::NoProgress => DownloadError::Network,
        Setup::ContentLengthMismatch
        | Setup::RangeMismatch
        | Setup::IdentityChanged
        | Setup::Io => DownloadError::Io,
    }
}

/// The downloader accepts only one of the manifests compiled into this build.
/// Keeping this separate from the syntactic HTTPS check prevents a future UI,
/// setting, or AI response from redirecting installation to another host.
fn is_allowlisted(artifact: &ManagedArtifact) -> bool {
    artifact == &managed_runtime_manifest().artifact
        || artifact == &managed_model_manifest().artifact
}

fn validate_artifact(artifact: &ManagedArtifact) -> Result<(), DownloadError> {
    if !artifact.source_url.starts_with("https://")
        || artifact.filename.is_empty()
        || artifact.filename.contains(['/', '\\'])
        || artifact.expected_size == 0
        || artifact.sha256.len() != 64
        || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DownloadError::RefusedUrl);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn manifests_are_pinned_https_and_self_describing() {
        let runtime = managed_runtime_manifest();
        let model = managed_model_manifest();
        validate_artifact(&runtime.artifact).unwrap();
        validate_artifact(&model.artifact).unwrap();
        assert_eq!(model.artifact.expected_size, 5_027_783_488);
        assert_eq!(model.license_identifier, "Apache-2.0");
        assert_eq!(model.quantization, "Q4_K_M");
    }

    #[test]
    fn user_supplied_or_non_https_artifacts_are_refused() {
        let artifact = ManagedArtifact {
            source_url: "http://example.test/model",
            filename: "x.gguf",
            expected_size: 1,
            sha256: "0",
        };
        assert_eq!(validate_artifact(&artifact), Err(DownloadError::RefusedUrl));
    }

    #[test]
    fn a_well_formed_unlisted_url_is_still_refused() {
        let artifact = ManagedArtifact {
            source_url: "https://example.test/model.gguf",
            filename: "model.gguf",
            expected_size: 1,
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
        };
        assert!(validate_artifact(&artifact).is_ok());
        assert!(!is_allowlisted(&artifact));
        let directory = tempdir().unwrap();
        assert_eq!(
            download_verified(
                &artifact,
                &directory.path().join("model.gguf"),
                |_| {},
                || false
            ),
            Err(DownloadError::RefusedUrl)
        );
    }

    #[test]
    fn destination_must_have_the_manifest_filename() {
        let directory = tempdir().unwrap();
        let artifact = managed_model_manifest().artifact;
        let result = download_verified(
            &artifact,
            &directory.path().join("other.gguf"),
            |_| {},
            || false,
        );
        assert_eq!(result, Err(DownloadError::Destination));
    }

    /// A small stand-in for the release archive, checked by the same code path.
    fn fixture_spec<'a>(
        allowed: &'a [&'static str],
        keep: &'a [&'static str],
    ) -> RuntimeArchiveSpec<'a> {
        RuntimeArchiveSpec {
            allowed,
            keep,
            max_files: 8,
            max_unpacked_bytes: 1024 * 1024,
            max_compression_ratio: 1000,
            require_pe_x64: true,
            server_name: "llama-server.exe",
        }
    }

    #[test]
    fn a_fixture_archive_is_validated_and_extracted_by_the_same_rules() {
        use super::super::setup::fixtures;
        let directory = tempdir().unwrap();
        let allowed = ["llama-server.exe", "llama.dll"];
        let keep = ["llama-server.exe", "llama.dll"];
        let spec = fixture_spec(&allowed, &keep);
        let server_bytes = fixtures::pe_x64();
        let library_bytes = fixtures::bytes(1024, 3);
        let archive = directory.path().join("runtime.zip");
        fs::write(
            &archive,
            fixtures::zip_archive(&[
                ("llama-server.exe", server_bytes.clone()),
                ("llama.dll", library_bytes.clone()),
            ]),
        )
        .unwrap();

        let unpacked = validate_runtime_archive(&archive, &spec).unwrap();
        assert_eq!(unpacked, server_bytes.len() as u64 + library_bytes.len() as u64);

        let staging = directory.path().join("payload");
        let server = extract_runtime_archive_with(&archive, &staging, &spec).unwrap();
        assert!(server.is_file());
        assert_eq!(fs::read(&server).unwrap(), server_bytes);
        assert!(staging.join("llama.dll").is_file());
    }

    #[test]
    fn an_archive_with_an_unexpected_entry_is_refused_before_anything_is_written() {
        use super::super::setup::fixtures;
        let directory = tempdir().unwrap();
        let allowed = ["llama-server.exe"];
        let keep = ["llama-server.exe"];
        let spec = fixture_spec(&allowed, &keep);
        let archive = directory.path().join("runtime.zip");
        fs::write(
            &archive,
            fixtures::zip_archive(&[
                ("llama-server.exe", fixtures::pe_x64()),
                ("payload.exe", fixtures::pe_x64()),
            ]),
        )
        .unwrap();
        assert_eq!(
            validate_runtime_archive(&archive, &spec),
            Err(RuntimeInstallError::ArchiveUnexpectedFile)
        );
        let staging = directory.path().join("payload");
        assert_eq!(
            extract_runtime_archive_with(&archive, &staging, &spec),
            Err(RuntimeInstallError::ArchiveUnexpectedFile)
        );
        assert!(!staging.exists(), "nothing may be written for a refused archive");
    }

    #[test]
    fn an_archive_whose_server_is_not_x86_64_is_refused() {
        use super::super::setup::fixtures;
        let directory = tempdir().unwrap();
        let allowed = ["llama-server.exe"];
        let keep = ["llama-server.exe"];
        let spec = fixture_spec(&allowed, &keep);
        let archive = directory.path().join("runtime.zip");
        // A 32-bit image: the shape is valid but the machine is wrong.
        fs::write(
            &archive,
            fixtures::zip_archive(&[("llama-server.exe", fixtures::pe_stub(0x014c))]),
        )
        .unwrap();
        assert_eq!(
            extract_runtime_archive_with(&archive, &directory.path().join("payload"), &spec),
            Err(RuntimeInstallError::RuntimeArchitectureMismatch)
        );
    }

    #[test]
    fn an_extraction_can_be_stopped_and_leaves_no_partial_library_behind() {
        use super::super::setup::fixtures;
        let directory = tempdir().unwrap();
        let allowed = ["llama-server.exe", "llama.dll"];
        let keep = ["llama-server.exe", "llama.dll"];
        let spec = RuntimeArchiveSpec {
            allowed: &allowed,
            keep: &keep,
            max_files: 8,
            max_unpacked_bytes: 8 * 1024 * 1024,
            max_compression_ratio: 1000,
            require_pe_x64: true,
            server_name: "llama-server.exe",
        };
        let archive = directory.path().join("runtime.zip");
        fs::write(
            &archive,
            fixtures::zip_archive(&[
                ("llama-server.exe", fixtures::pe_x64()),
                ("llama.dll", fixtures::bytes(1024 * 1024, 9)),
            ]),
        )
        .unwrap();

        let staging = directory.path().join("payload");
        // Cancelled before the first entry is written.
        assert_eq!(
            extract_runtime_archive_cancellable(&archive, &staging, &spec, &|| true),
            Err(RuntimeInstallError::Cancelled)
        );
        assert!(
            !staging.join("llama-server.exe").exists(),
            "a cancelled extraction must not leave a file that looks installed"
        );

        // And it still works when the caller never cancels.
        assert!(extract_runtime_archive_cancellable(&archive, &staging, &spec, &|| false).is_ok());
        assert!(staging.join("llama.dll").is_file());
    }

    #[test]
    fn an_archive_that_is_missing_a_required_file_is_refused() {        use super::super::setup::fixtures;
        let directory = tempdir().unwrap();
        let allowed = ["llama-server.exe", "llama.dll"];
        let keep = ["llama-server.exe", "llama.dll"];
        let spec = fixture_spec(&allowed, &keep);
        let archive = directory.path().join("runtime.zip");
        fs::write(
            &archive,
            fixtures::zip_archive(&[("llama-server.exe", fixtures::pe_x64())]),
        )
        .unwrap();
        assert_eq!(
            validate_runtime_archive(&archive, &spec),
            Err(RuntimeInstallError::RuntimeMissing)
        );
    }

    #[test]
    fn the_pe_check_reads_the_machine_tag_where_the_format_puts_it() {
        use super::super::setup::fixtures;
        let directory = tempdir().unwrap();
        let server = directory.path().join("llama-server.exe");
        // The machine tag sits at `e_lfanew + 4`, right after the PE signature.
        fs::write(&server, fixtures::pe_x64()).unwrap();
        assert_eq!(validate_pe_x64(&server), Ok(()));

        // A 32-bit image with the same layout is refused.
        fs::write(&server, fixtures::pe_stub(0x014c)).unwrap();
        assert_eq!(
            validate_pe_x64(&server),
            Err(RuntimeInstallError::RuntimeArchitectureMismatch)
        );

        // Something that is not a PE image at all is refused.
        fs::write(&server, fixtures::not_a_pe()).unwrap();
        assert_eq!(
            validate_pe_x64(&server),
            Err(RuntimeInstallError::RuntimeArchitectureMismatch)
        );

        // An `e_lfanew` that points past the end of the file is refused rather
        // than read out of bounds.
        let mut truncated = fixtures::pe_x64();
        truncated[0x3c..0x40].copy_from_slice(&0xffff_u32.to_le_bytes());
        fs::write(&server, truncated).unwrap();
        assert_eq!(
            validate_pe_x64(&server),
            Err(RuntimeInstallError::RuntimeArchitectureMismatch)
        );

        // A file too short for even the DOS header is refused.
        fs::write(&server, b"MZ").unwrap();
        assert_eq!(
            validate_pe_x64(&server),
            Err(RuntimeInstallError::RuntimeArchitectureMismatch)
        );
    }

    #[test]
    fn a_receipt_is_what_makes_a_directory_removable_by_this_application() {        let directory = tempdir().unwrap();
        let installed = directory.path().join("llama.cpp-b10964");
        fs::create_dir_all(&installed).unwrap();
        assert!(!has_installation_receipt(&installed, "llama.cpp"));

        let facts = InstallationFacts {
            kind: "llama.cpp".to_string(),
            version: "b10964".to_string(),
            artifact_name: "llama-b10964-bin-win-cpu-x64.zip".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 1,
            files: vec!["llama-server.exe".to_string()],
        };
        write_installation_receipt(&installed, &facts).unwrap();
        assert!(has_installation_receipt(&installed, "llama.cpp"));
        // A receipt for another kind does not authorize a removal.
        assert!(!has_installation_receipt(&installed, "model"));
        let receipt = read_installation_receipt(&installed).unwrap();
        assert_eq!(receipt.version, "b10964");
        assert_eq!(receipt.files, vec!["llama-server.exe".to_string()]);
    }

    #[test]
    fn a_model_activation_after_a_verified_hash_still_checks_the_metadata() {
        use super::super::setup::fixtures;
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let staging = data_dir.join("setup-temp").join("model").join("payload");
        fs::create_dir_all(&staging).unwrap();
        let name = managed_model_manifest().artifact.filename;
        // A GGUF file, but not at the pinned size: refused without a full read.
        fs::write(staging.join(name), fixtures::pinned_gguf(4096)).unwrap();
        assert_eq!(
            activate_managed_model_after_hash(&staging.join(name), &data_dir),
            Err(ModelInstallError::SizeMismatch)
        );
        // Nothing was promoted, so no half-installed directory exists.
        assert!(!managed_model_directory(&data_dir).exists());
        assert!(staging.join(name).is_file());
    }
}

