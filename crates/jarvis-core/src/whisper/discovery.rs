//! Finding a Whisper build that is already on the machine.
//!
//! This is discovery, not installation: nothing is downloaded, nothing is
//! written, and nothing is chosen on the user's behalf. It looks in the places a
//! person would have put the files, validates every candidate with the same
//! checks the settings page uses, and returns a list for the user to confirm.
//!
//! The order is deliberate:
//!
//! 1. the runtime bundled next to the application (`resources/whisper`);
//! 2. the known user directory (`C:\AI\whisper.cpp`, including the subdirectories
//!    a `whisper.cpp` build produces);
//! 3. `PATH`.
//!
//! Two rules matter more than the search itself:
//!
//! * **a candidate that fails validation is not a candidate.** A file that is not
//!   a 64-bit executable, or not a model container, is reported with its reason
//!   instead of being offered;
//! * **several models are listed, never picked.** Choosing between
//!   `ggml-small.bin` and `ggml-large-v3.bin` is the user's decision: the sizes
//!   trade speed against accuracy, and guessing would be wrong in both
//!   directions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::WhisperError;
use super::model::{probe_binary, probe_model, ModelKind};

/// Where a candidate was found.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    /// Next to the application, in its own resources.
    BundledRuntime,
    /// A directory a person is likely to have used.
    KnownDirectory,
    /// Found through `PATH`.
    Path,
}

impl CandidateSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BundledRuntime => "bundled_runtime",
            Self::KnownDirectory => "known_directory",
            Self::Path => "path",
        }
    }

    /// The Fluent key the interface shows.
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::BundledRuntime => "whisper-discovery-source-bundled",
            Self::KnownDirectory => "whisper-discovery-source-known",
            Self::Path => "whisper-discovery-source-path",
        }
    }
}

/// One executable that passed the checks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutableCandidate {
    /// The full path, for the settings page only.
    pub path: String,
    /// The file's own name, safe to show anywhere.
    pub name: String,
    pub source: CandidateSource,
}

/// One model that passed the checks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelCandidate {
    pub path: String,
    pub name: String,
    pub source: CandidateSource,
    /// The size the name states, for the interface to describe.
    pub kind: ModelKind,
    pub size_bytes: u64,
}

/// One validated pair, ready for the user to confirm.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CandidatePair {
    pub executable: ExecutableCandidate,
    pub model: ModelCandidate,
}

/// Something that was found and refused, with the reason.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RejectedCandidate {
    pub name: String,
    pub source: CandidateSource,
    /// The content-free code of the refusal.
    pub code: String,
    pub detail: String,
}

/// What a search found.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiscoveryReport {
    /// Validated pairs: one entry per executable and model combination.
    pub pairs: Vec<CandidatePair>,
    /// Executables that passed, even when no model was found next to them.
    pub executables: Vec<ExecutableCandidate>,
    /// Models that passed, even when no executable was found next to them.
    pub models: Vec<ModelCandidate>,
    pub rejected: Vec<RejectedCandidate>,
    /// The directories that were searched, for the interface to explain an
    /// empty result. These are shown locally and are not part of the diagnostics
    /// export.
    pub searched: Vec<String>,
}

impl DiscoveryReport {
    /// Whether anything usable was found.
    pub fn found_anything(&self) -> bool {
        !self.executables.is_empty() && !self.models.is_empty()
    }

    /// Whether more than one pair has to be chosen between.
    pub fn needs_a_choice(&self) -> bool {
        self.pairs.len() > 1
    }

    /// A one-line summary with no path in it.
    pub fn summary(&self) -> String {
        if !self.found_anything() {
            return "nothing was found".to_string();
        }
        format!(
            "{} executable(s), {} model(s), {} pair(s)",
            self.executables.len(),
            self.models.len(),
            self.pairs.len()
        )
    }
}

/// The directory names a `whisper.cpp` checkout or release produces.
const KNOWN_SUBDIRECTORIES: [&str; 8] = [
    "",
    "runtime",
    "runtime/Release",
    "bin",
    "Release",
    "build",
    "build/bin",
    "build/bin/Release",
];

/// The executable names Whisper builds use.
const EXECUTABLE_NAMES: [&str; 4] = ["whisper-cli.exe", "main.exe", "whisper.exe", "whisper-cli"];

/// The home of a `whisper.cpp` setup on Windows, as used by the person who
/// reported the defect. It is only a hint: every file found there is validated.
pub const KNOWN_WINDOWS_DIRECTORY: &str = r"C:\AI\whisper.cpp";

/// Searches `roots` for validated candidates.
///
/// `roots` are the directories to search *in addition to* the bundled resources
/// and `PATH`, which keeps the function testable with temporary directories.
pub fn discover_with_roots(
    bundled: Option<&Path>,
    roots: &[PathBuf],
    path_variable: Option<&str>,
) -> DiscoveryReport {
    let mut report = DiscoveryReport::default();
    let mut executables: Vec<(PathBuf, CandidateSource)> = Vec::new();
    let mut models: Vec<(PathBuf, CandidateSource)> = Vec::new();

    // 1. The bundled runtime, then the known directories.
    let mut directories: Vec<(PathBuf, CandidateSource)> = Vec::new();
    if let Some(bundled) = bundled {
        directories.push((bundled.to_path_buf(), CandidateSource::BundledRuntime));
    }
    for root in roots {
        for subdirectory in KNOWN_SUBDIRECTORIES {
            let directory = if subdirectory.is_empty() {
                root.clone()
            } else {
                root.join(subdirectory)
            };
            directories.push((directory, CandidateSource::KnownDirectory));
        }
    }

    for (directory, source) in &directories {
        if !directory.is_dir() {
            continue;
        }
        report
            .searched
            .push(directory.to_string_lossy().into_owned());
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if EXECUTABLE_NAMES.contains(&name.as_str()) {
                match probe_binary(&path) {
                    Ok(_) => executables.push((path, *source)),
                    Err(error) => reject(&mut report, &path, *source, &error),
                }
            } else if name.ends_with(".bin") || name.ends_with(".gguf") {
                match probe_model(&path) {
                    Ok(_) => models.push((path, *source)),
                    Err(error) => reject(&mut report, &path, *source, &error),
                }
            }
        }
    }

    // 3. `PATH`: an executable only, because a model is not put on `PATH`.
    if let Some(path_variable) = path_variable {
        for entry in std::env::split_paths(path_variable) {
            for name in EXECUTABLE_NAMES {
                let candidate = entry.join(name);
                if !candidate.is_file() {
                    continue;
                }
                report.searched.push(entry.to_string_lossy().into_owned());
                match probe_binary(&candidate) {
                    Ok(_) => executables.push((candidate, CandidateSource::Path)),
                    Err(error) => reject(&mut report, &candidate, CandidateSource::Path, &error),
                }
            }
        }
    }

    // The same file can be reached through two directories: `.` and `runtime`
    // both contain the executable of a release, for example.
    executables.sort_by(|left, right| left.0.cmp(&right.0));
    executables.dedup_by(|left, right| left.0 == right.0);
    models.sort_by(|left, right| left.0.cmp(&right.0));
    models.dedup_by(|left, right| left.0 == right.0);

    report.executables = executables
        .iter()
        .map(|(path, source)| ExecutableCandidate {
            path: path.to_string_lossy().into_owned(),
            name: file_name(path),
            source: *source,
        })
        .collect();
    report.models = models
        .iter()
        .map(|(path, source)| {
            let probe = probe_model(path).ok();
            ModelCandidate {
                path: path.to_string_lossy().into_owned(),
                name: file_name(path),
                source: *source,
                kind: probe
                    .as_ref()
                    .map(|probe| probe.kind)
                    .unwrap_or(ModelKind::Unknown),
                size_bytes: probe.as_ref().map(|probe| probe.size_bytes).unwrap_or(0),
            }
        })
        .collect();

    // Every executable with every model: the choice belongs to the user, and a
    // list of pairs is the honest way to offer it.
    for executable in &report.executables {
        for model in &report.models {
            report.pairs.push(CandidatePair {
                executable: executable.clone(),
                model: model.clone(),
            });
        }
    }
    report
}

/// Searches the bundled resources, the known Windows directory, and `PATH`.
pub fn discover() -> DiscoveryReport {
    let bundled = crate::APP_DIR.join("resources").join("whisper");
    let known = PathBuf::from(KNOWN_WINDOWS_DIRECTORY);
    let path_variable = std::env::var("PATH").ok();
    discover_with_roots(Some(&bundled), &[known], path_variable.as_deref())
}

fn reject(
    report: &mut DiscoveryReport,
    path: &Path,
    source: CandidateSource,
    error: &WhisperError,
) {
    report.rejected.push(RejectedCandidate {
        name: file_name(path),
        source,
        code: error.code().to_string(),
        detail: error
            .detail()
            .map(|detail| crate::text::shorten(&detail, 140))
            .unwrap_or_else(|| error.to_string()),
    });
}

fn file_name(path: &Path) -> String {
    crate::text::file_label(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whisper::model::{MIN_MODEL_BYTES, PE_MACHINE_AMD64};
    use tempfile::tempdir;

    fn write_binary(directory: &Path, name: &str) -> PathBuf {
        let mut pe = vec![0u8; 0x100];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe[0x44..0x46].copy_from_slice(&PE_MACHINE_AMD64.to_le_bytes());
        let path = directory.join(name);
        std::fs::write(&path, &pe).unwrap();
        path
    }

    fn write_model(directory: &Path, name: &str, size: u64) -> PathBuf {
        let mut body = vec![0x6cu8, 0x6d, 0x67, 0x67];
        body.resize(size as usize, 0);
        let path = directory.join(name);
        std::fs::write(&path, &body).unwrap();
        path
    }

    #[test]
    fn nothing_anywhere_is_an_empty_report_and_not_an_invention() {
        let directory = tempdir().unwrap();
        let report = discover_with_roots(
            Some(&directory.path().join("missing")),
            &[directory.path().join("also-missing")],
            None,
        );
        assert!(report.pairs.is_empty());
        assert!(!report.found_anything());
        assert_eq!(report.summary(), "nothing was found");
        assert!(report.rejected.is_empty());
    }

    #[test]
    fn the_known_directory_layout_is_searched_including_a_release_subdirectory() {
        let directory = tempdir().unwrap();
        let release = directory.path().join("runtime").join("Release");
        std::fs::create_dir_all(&release).unwrap();
        write_binary(&release, "whisper-cli.exe");
        write_model(directory.path(), "ggml-small.bin", MIN_MODEL_BYTES + 32);

        let report = discover_with_roots(None, &[directory.path().to_path_buf()], None);
        assert_eq!(report.executables.len(), 1);
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.executables[0].name, "whisper-cli.exe");
        assert_eq!(report.models[0].kind, ModelKind::Small);
        assert_eq!(report.pairs.len(), 1);
        assert_eq!(report.pairs[0].model.name, "ggml-small.bin");
        assert!(!report.needs_a_choice());
        assert!(report.summary().contains("1 executable"));
        // The directories that were searched are reported, without a file path.
        assert!(!report.searched.is_empty());
    }

    #[test]
    fn several_models_are_listed_and_never_picked() {
        let directory = tempdir().unwrap();
        write_binary(directory.path(), "whisper-cli.exe");
        write_model(directory.path(), "ggml-small.bin", MIN_MODEL_BYTES + 32);
        write_model(directory.path(), "ggml-medium.bin", MIN_MODEL_BYTES + 64);

        let report = discover_with_roots(None, &[directory.path().to_path_buf()], None);
        assert_eq!(report.executables.len(), 1);
        assert_eq!(report.models.len(), 2);
        // Both pairs exist, and the report says a choice is needed.
        assert_eq!(report.pairs.len(), 2);
        assert!(report.needs_a_choice());
        let names: Vec<&str> = report
            .models
            .iter()
            .map(|model| model.name.as_str())
            .collect();
        assert!(names.contains(&"ggml-small.bin"));
        assert!(names.contains(&"ggml-medium.bin"));
    }

    #[test]
    fn a_file_that_does_not_pass_validation_is_reported_and_not_offered() {
        let directory = tempdir().unwrap();
        // A 32-bit build: refused by the architecture check.
        let mut pe = vec![0u8; 0x100];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe[0x44..0x46].copy_from_slice(&0x014cu16.to_le_bytes());
        std::fs::write(directory.path().join("whisper-cli.exe"), &pe).unwrap();
        // A `ggml-*.bin` that is not a container.
        let mut body = b"NOTM".to_vec();
        body.resize((MIN_MODEL_BYTES + 32) as usize, 0);
        std::fs::write(directory.path().join("ggml-small.bin"), &body).unwrap();

        let report = discover_with_roots(None, &[directory.path().to_path_buf()], None);
        assert!(report.executables.is_empty());
        assert!(report.models.is_empty());
        assert!(!report.found_anything());
        assert_eq!(report.rejected.len(), 2);
        assert!(report
            .rejected
            .iter()
            .any(|candidate| candidate.code == "wrong_architecture"));
        assert!(report
            .rejected
            .iter()
            .any(|candidate| candidate.code == "model_unavailable"));
        // A rejection carries a file name and a reason, never a directory.
        for candidate in &report.rejected {
            assert!(!candidate.detail.contains(":\\"), "{}", candidate.detail);
            assert!(!candidate.name.contains('\\'));
        }
    }

    #[test]
    fn path_is_searched_for_an_executable_and_a_model_is_not_expected_there() {
        let directory = tempdir().unwrap();
        write_binary(directory.path(), "whisper-cli.exe");
        let path_variable = directory.path().to_string_lossy().into_owned();
        let report = discover_with_roots(None, &[], Some(&path_variable));
        assert_eq!(report.executables.len(), 1);
        assert_eq!(report.executables[0].source, CandidateSource::Path);
        assert_eq!(report.executables[0].source.as_str(), "path");
        assert_eq!(
            report.executables[0].source.label_key(),
            "whisper-discovery-source-path"
        );
        assert!(report.models.is_empty());
        // An executable without a model is a finding, not a pair.
        assert!(report.pairs.is_empty());
        assert!(!report.found_anything());
    }

    #[test]
    fn the_bundled_runtime_comes_first_and_the_same_file_is_not_listed_twice() {
        let directory = tempdir().unwrap();
        let bundled = directory.path().join("resources").join("whisper");
        std::fs::create_dir_all(&bundled).unwrap();
        write_binary(&bundled, "whisper-cli.exe");
        write_model(&bundled, "ggml-tiny.bin", MIN_MODEL_BYTES + 32);

        let report = discover_with_roots(Some(&bundled), &[bundled.clone()], None);
        assert_eq!(
            report.executables.len(),
            1,
            "the same file must be listed once"
        );
        assert_eq!(report.models.len(), 1);
        assert_eq!(
            report.executables[0].source,
            CandidateSource::BundledRuntime
        );
        assert_eq!(report.models[0].source, CandidateSource::BundledRuntime);
    }

    #[test]
    fn the_summary_counts_without_naming_a_path() {
        let directory = tempdir().unwrap();
        write_binary(directory.path(), "whisper-cli.exe");
        write_model(directory.path(), "ggml-small.bin", MIN_MODEL_BYTES + 32);
        let report = discover_with_roots(None, &[directory.path().to_path_buf()], None);
        let summary = report.summary();
        assert!(!summary.contains(":\\"), "{summary}");
        assert!(!summary.contains('/'), "{summary}");
        assert!(summary.contains("1 pair"));
    }

    #[test]
    fn the_default_discovery_does_not_panic_without_any_known_directory() {
        // The real entry point runs on a machine that may have none of this.
        let report = discover();
        // Whatever it finds, it must be a value, and a rejection must carry no path.
        for candidate in &report.rejected {
            assert!(!candidate.detail.contains(":\\"), "{}", candidate.detail);
        }
        assert!(report.pairs.len() >= report.executables.len().min(report.models.len()));
    }
}
