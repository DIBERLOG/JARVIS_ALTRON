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
const MAX_RUNTIME_COMPRESSION_RATIO: u64 = 100;

const RUNTIME_ARCHIVE_FILES: &[&str] = &[
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

const RUNTIME_INSTALLED_FILES: &[&str] = &[
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
    let receipt = InstallationReceipt {
        kind: "llama.cpp".to_string(),
        version: manifest.version.to_string(),
        artifact_name: manifest.artifact.filename.to_string(),
        sha256: manifest.artifact.sha256.to_string(),
        size_bytes: manifest.artifact.expected_size,
        installed_at: chrono::Utc::now().to_rfc3339(),
        files: RUNTIME_INSTALLED_FILES
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    };
    let receipt_path = final_dir.join("installation-receipt.json");
    crate::fsutil::write_json_atomic(&receipt_path, &receipt)
        .map_err(|_| RuntimeInstallError::Io)?;
    Ok(server)
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
    let manifest = managed_model_manifest();
    let final_dir = managed_model_directory(data_dir);
    let final_model = final_dir.join(manifest.artifact.filename);
    if final_model.is_file() {
        return validate_managed_model(&final_model).map(|_| final_model);
    }
    validate_managed_model(staging_model)?;
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
    if let Err(error) = validate_managed_model(&final_model) {
        let _ = fs::remove_dir_all(&final_dir);
        return Err(error);
    }
    let receipt = InstallationReceipt {
        kind: "model".to_string(),
        version: manifest.source_revision.to_string(),
        artifact_name: manifest.artifact.filename.to_string(),
        sha256: manifest.artifact.sha256.to_string(),
        size_bytes: manifest.artifact.expected_size,
        installed_at: chrono::Utc::now().to_rfc3339(),
        files: vec![manifest.artifact.filename.to_string()],
    };
    crate::fsutil::write_json_atomic(&final_dir.join("installation-receipt.json"), &receipt)
        .map_err(|_| ModelInstallError::Io)?;
    Ok(final_model)
}

/// Completed artifact, `.part`, staging, and a 512 MiB safety margin.
pub fn required_model_space_bytes() -> u64 {
    managed_model_manifest()
        .artifact
        .expected_size
        .checked_mul(2)
        .and_then(|value| value.checked_add(512 * 1024 * 1024))
        .expect("pinned model size fits u64")
}

/// Checks a completed managed model without loading tensors into memory. This is
/// intentionally separate from the interactive, permissive file picker: a
/// managed activation has a stricter pinned identity.
pub fn validate_managed_model(path: &Path) -> Result<super::GgufInfo, ModelInstallError> {
    let manifest = managed_model_manifest();
    let metadata = path.metadata().map_err(|_| ModelInstallError::Io)?;
    if !metadata.is_file() {
        return Err(ModelInstallError::GgufInvalid);
    }
    if metadata.len() != manifest.artifact.expected_size {
        return Err(ModelInstallError::SizeMismatch);
    }
    if sha256_file(path)? != manifest.artifact.sha256 {
        return Err(ModelInstallError::HashMismatch);
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

/// Validates and extracts the server's minimal runtime set into an empty staging
/// directory. It never starts an executable and never writes outside `staging`.
pub fn extract_runtime_archive(
    archive: &Path,
    staging: &Path,
) -> Result<PathBuf, RuntimeInstallError> {
    let file = File::open(archive).map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
    if zip.len() == 0 || zip.len() > MAX_RUNTIME_ARCHIVE_FILES {
        return Err(RuntimeInstallError::ArchiveTooLarge);
    }
    fs::create_dir_all(staging).map_err(|_| RuntimeInstallError::Io)?;
    let staging = staging
        .canonicalize()
        .map_err(|_| RuntimeInstallError::Io)?;
    let mut total = 0_u64;
    let mut server = None;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
        let name = entry.name().to_string();
        if !safe_archive_name(&name) {
            return Err(RuntimeInstallError::ArchivePathTraversal);
        }
        if entry.is_dir() || entry.is_symlink() || !RUNTIME_ARCHIVE_FILES.contains(&name.as_str()) {
            return Err(RuntimeInstallError::ArchiveUnexpectedFile);
        }
        let size = entry.size();
        let packed = entry.compressed_size();
        if size > MAX_RUNTIME_UNPACKED_BYTES
            || (packed > 0 && size / packed > MAX_RUNTIME_COMPRESSION_RATIO)
        {
            return Err(RuntimeInstallError::ArchiveTooLarge);
        }
        total = total
            .checked_add(size)
            .ok_or(RuntimeInstallError::ArchiveTooLarge)?;
        if total > MAX_RUNTIME_UNPACKED_BYTES {
            return Err(RuntimeInstallError::ArchiveTooLarge);
        }
        if !RUNTIME_INSTALLED_FILES.contains(&name.as_str()) {
            continue;
        }
        let target = staging.join(&name);
        if !target.starts_with(&staging) {
            return Err(RuntimeInstallError::ArchivePathTraversal);
        }
        let mut output = File::create(&target).map_err(|_| RuntimeInstallError::Io)?;
        std::io::copy(&mut entry, &mut output).map_err(|_| RuntimeInstallError::ArchiveInvalid)?;
        output.sync_all().map_err(|_| RuntimeInstallError::Io)?;
        if name == "llama-server.exe" {
            server = Some(target);
        }
    }
    let server = server.ok_or(RuntimeInstallError::RuntimeMissing)?;
    validate_pe_x64(&server)?;
    for required in RUNTIME_INSTALLED_FILES {
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
fn validate_pe_x64(path: &Path) -> Result<(), RuntimeInstallError> {
    let bytes = fs::read(path).map_err(|_| RuntimeInstallError::Io)?;
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return Err(RuntimeInstallError::RuntimeArchitectureMismatch);
    }
    let offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let machine = offset
        .checked_add(6)
        .filter(|value| *value <= bytes.len())
        .ok_or(RuntimeInstallError::RuntimeArchitectureMismatch)?;
    if bytes.get(offset..offset + 4) != Some(b"PE\0\0")
        || bytes.get(machine..machine + 2) != Some(&0x8664_u16.to_le_bytes())
    {
        return Err(RuntimeInstallError::RuntimeArchitectureMismatch);
    }
    Ok(())
}

/// Downloads a compiled-in artifact. `cancelled` and `progress` are polled for
/// every chunk, so callers can run this on a worker without freezing the UI.
/// On every failure the `.part` file is removed and an existing destination is
/// preserved.
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
    let part = destination.with_file_name(format!("{}.part", artifact.filename));
    let result = download_to_part(artifact, &part, &mut progress, &mut cancelled);
    match result {
        Ok(()) => fs::rename(&part, destination).map_err(|_| DownloadError::Io),
        Err(error) => {
            let _ = fs::remove_file(&part);
            Err(error)
        }
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

fn download_to_part<F, C>(
    artifact: &ManagedArtifact,
    part: &Path,
    progress: &mut F,
    cancelled: &mut C,
) -> Result<(), DownloadError>
where
    F: FnMut(DownloadProgress),
    C: FnMut() -> bool,
{
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|_| DownloadError::Network)?
        .get(artifact.source_url)
        .send()
        .map_err(|_| DownloadError::Network)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|n| n > artifact.expected_size)
    {
        return Err(DownloadError::Network);
    }
    let mut source = response;
    let mut output = File::create(part).map_err(|_| DownloadError::Io)?;
    let mut digest = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(DownloadError::Cancelled);
        }
        let count = source
            .read(&mut buffer)
            .map_err(|_| DownloadError::Network)?;
        if count == 0 {
            break;
        }
        downloaded = downloaded
            .checked_add(count as u64)
            .ok_or(DownloadError::TooLarge)?;
        if downloaded > artifact.expected_size {
            return Err(DownloadError::TooLarge);
        }
        output
            .write_all(&buffer[..count])
            .map_err(|_| DownloadError::Io)?;
        digest.update(&buffer[..count]);
        progress(DownloadProgress {
            downloaded,
            total: artifact.expected_size,
        });
    }
    output.sync_all().map_err(|_| DownloadError::Io)?;
    if downloaded != artifact.expected_size {
        return Err(DownloadError::SizeMismatch);
    }
    let actual = format!("{:x}", digest.finalize());
    if !actual.eq_ignore_ascii_case(artifact.sha256) {
        return Err(DownloadError::HashMismatch);
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
}
