//! Verified, managed local-AI artifacts.
//!
//! URLs live in this module rather than in settings, prompts, or model output. A
//! download is first written as `.part`, bounded, hashed, and renamed atomically.
//! This module deliberately does *not* execute downloaded files; the gateway
//! performs its existing validation before it can start `llama-server`.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Maximum accepted runtime archive size (128 MiB).
pub const MAX_RUNTIME_BYTES: u64 = 128 * 1024 * 1024;
/// Maximum accepted model size (12 GiB), intentionally above the pinned Q4 model.
pub const MAX_MODEL_BYTES: u64 = 12 * 1024 * 1024 * 1024;

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
    data_dir
        .join("runtime")
        .join(format!("llama.cpp-{version}"))
}

/// `%LOCALAPPDATA%\\com.priler.jarvis\\models` on Windows.
pub fn managed_model_directory(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
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
