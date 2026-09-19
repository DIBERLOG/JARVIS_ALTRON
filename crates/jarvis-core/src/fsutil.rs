//! Small filesystem helpers shared by the settings and AI configuration stores.
//!
//! Writes go through a temporary file plus a rename, so a crash or a full disk
//! cannot leave a half-written configuration behind: a reader either sees the old
//! file or the new one, never a mixture.

use std::fs;
use std::path::{Path, PathBuf};

/// Writes `contents` to `path` atomically.
pub fn write_bytes_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, contents)?;
    // `rename` replaces the destination on both Windows and Unix, so readers
    // either see the old file or the new one, never a mixture.
    fs::rename(&temporary, path)
}

/// Serializes `value` and writes it atomically.
pub fn write_json_atomic<T: serde::Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), std::io::Error> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_bytes_atomic(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    /// Reads a written document back, so the tests assert on real file content.
    fn read_sample(path: &Path) -> Sample {
        let text = fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn round_trips_json_and_creates_parent_directories() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("nested").join("settings.json");
        let value = Sample {
            name: "fixture".to_string(),
            count: 7,
        };
        write_json_atomic(&path, &value).unwrap();
        assert_eq!(read_sample(&path), value);
        // No temporary file is left behind.
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn replaces_an_existing_file_and_leaves_no_temporary_behind() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        write_json_atomic(
            &path,
            &Sample {
                name: "first".to_string(),
                count: 1,
            },
        )
        .unwrap();
        write_json_atomic(
            &path,
            &Sample {
                name: "second".to_string(),
                count: 2,
            },
        )
        .unwrap();
        assert_eq!(read_sample(&path).name, "second".to_string());
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "a temporary file was left behind");
    }
}
