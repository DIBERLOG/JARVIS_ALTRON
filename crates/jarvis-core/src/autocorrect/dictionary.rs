//! Dictionary loading: Hunspell-compatible `.aff`/`.dic` pairs from a local folder.
//!
//! The engine is [`spellbook`](https://crates.io/crates/spellbook) 0.4, a pure-Rust
//! rewrite of Nuspell: no native library, no DLL, no network, no data sent anywhere.
//! It is MPL-2.0, and it carries only `hashbrown` as a dependency.
//!
//! **No dictionary is shipped with the application.** Russian and English Hunspell
//! dictionaries are multi-megabyte files with their own upstream licences, so the user
//! installs the pairs they want into the dictionaries folder, and `docs/AUTOCORRECT.md`
//! explains where to get them, which file names are expected, and how to record the
//! checksums of what was installed. Until a pair is installed, the checker reports
//! [`AutocorrectError::DictionaryMissing`] with the exact paths it looked at, and the
//! feature simply does not run: it never guesses words from nothing.
//!
//! A `dictionaries.json` manifest next to the pairs is optional. It is how a user pins the
//! SHA-256 of every installed file and its origin, and when it is present those hashes are
//! verified, so a dictionary that was replaced or corrupted is reported instead of being
//! used silently.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::error::AutocorrectError;
use super::model::Language;

/// Folder inside the application data directory that holds the dictionary pairs.
pub const DICTIONARIES_DIR: &str = "dictionaries";
/// Optional manifest with the expected checksums and origins.
pub const DICTIONARY_MANIFEST_FILE: &str = "dictionaries.json";

/// Paths of one dictionary pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DictionaryPaths {
    pub aff: PathBuf,
    pub dic: PathBuf,
}

/// What the interface shows about one language.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DictionaryState {
    /// The pair is installed and parsed.
    Ready {
        language: Language,
        aff: String,
        dic: String,
        /// Stems in the dictionary, when the file could be counted.
        words: Option<usize>,
        /// Origin recorded in the manifest, when there is one.
        source: Option<String>,
    },
    /// No pair is installed for this language.
    Missing {
        language: Language,
        /// The exact paths the checker looked for, so the user knows what to add.
        expected: Vec<String>,
    },
    /// A pair exists but could not be used.
    Invalid {
        language: Language,
        reason: String,
        expected: Vec<String>,
    },
}

impl DictionaryState {
    pub fn language(&self) -> Language {
        match self {
            Self::Ready { language, .. }
            | Self::Missing { language, .. }
            | Self::Invalid { language, .. } => *language,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// One optional manifest entry: where a dictionary came from and what it must hash to.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DictionaryManifestEntry {
    pub language: Language,
    /// SHA-256 of the `.aff` file, lowercase hex.
    pub aff_sha256: String,
    /// SHA-256 of the `.dic` file, lowercase hex.
    pub dic_sha256: String,
    pub source: Option<String>,
    pub version: Option<String>,
    pub license: Option<String>,
}

/// The optional manifest file.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DictionaryManifest {
    #[serde(default)]
    pub entries: Vec<DictionaryManifestEntry>,
}

impl DictionaryManifest {
    pub fn entry(&self, language: Language) -> Option<&DictionaryManifestEntry> {
        self.entries.iter().find(|entry| entry.language == language)
    }

    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// A parsed dictionary with its word count.
pub struct LoadedDictionary {
    language: Language,
    dictionary: Mutex<spellbook::Dictionary>,
    words: Option<usize>,
}

impl LoadedDictionary {
    pub fn language(&self) -> Language {
        self.language
    }

    pub fn word_count(&self) -> Option<usize> {
        self.words
    }

    /// Whether the dictionary knows this exact spelling.
    pub fn check(&self, word: &str) -> bool {
        if word.is_empty() {
            return true;
        }
        self.dictionary.lock().check(word)
    }

    /// Suggestions for a word, at most `limit` of them, in the engine's own order.
    pub fn suggestions(&self, word: &str, limit: usize) -> Vec<String> {
        if limit == 0 || word.is_empty() {
            return Vec::new();
        }
        let dictionary = self.dictionary.lock();
        let mut out: Vec<String> = Vec::new();
        dictionary.suggest(word, &mut out);
        out.truncate(limit);
        out
    }
}

impl std::fmt::Debug for LoadedDictionary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoadedDictionary")
            .field("language", &self.language)
            .field("words", &self.words)
            .finish()
    }
}

/// Loads and caches the installed dictionaries.
///
/// The manager is shared by every check and never reloads a dictionary per word: a pair
/// is parsed once, kept in memory, and only re-read by [`DictionaryManager::reload`],
/// which the interface calls when the folder changes.
pub struct DictionaryManager {
    dir: PathBuf,
    loaded: RwLock<HashMap<Language, Arc<LoadedDictionary>>>,
    states: RwLock<HashMap<Language, DictionaryState>>,
    manifest: RwLock<Option<DictionaryManifest>>,
}

impl DictionaryManager {
    /// Builds a manager for one folder. Nothing is read until it is used.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            loaded: RwLock::new(HashMap::new()),
            states: RwLock::new(HashMap::new()),
            manifest: RwLock::new(None),
        }
    }

    /// The folder the manager reads dictionaries from.
    pub fn directory(&self) -> &Path {
        &self.dir
    }

    /// Paths the manager looks for one language.
    pub fn paths(&self, language: Language) -> DictionaryPaths {
        DictionaryPaths {
            aff: self.dir.join(language.aff_file()),
            dic: self.dir.join(language.dic_file()),
        }
    }

    /// Expected paths as strings, for a `DictionaryMissing` report.
    pub fn expected_paths(&self, language: Language) -> Vec<String> {
        let paths = self.paths(language);
        vec![
            paths.aff.display().to_string(),
            paths.dic.display().to_string(),
        ]
    }

    /// Current state of one language, loading the pair on first use.
    pub fn state(&self, language: Language) -> DictionaryState {
        if let Some(state) = self.states.read().get(&language).cloned() {
            return state;
        }
        match self.load(language) {
            Ok(dictionary) => {
                let source = self
                    .manifest_entry(language)
                    .and_then(|entry| entry.source.clone());
                let state = DictionaryState::Ready {
                    language,
                    aff: dictionary_paths_label(&self.paths(language).aff),
                    dic: dictionary_paths_label(&self.paths(language).dic),
                    words: dictionary.word_count(),
                    source,
                };
                self.states.write().insert(language, state.clone());
                state
            }
            Err(AutocorrectError::DictionaryMissing { expected, .. }) => {
                let state = DictionaryState::Missing { language, expected };
                self.states.write().insert(language, state.clone());
                state
            }
            Err(error) => {
                let state = DictionaryState::Invalid {
                    language,
                    reason: error.to_string(),
                    expected: self.expected_paths(language),
                };
                self.states.write().insert(language, state.clone());
                state
            }
        }
    }

    /// Every language's state.
    pub fn states(&self) -> Vec<DictionaryState> {
        Language::all()
            .iter()
            .map(|language| self.state(*language))
            .collect()
    }

    /// Loads a dictionary, or reports why it cannot be used.
    pub fn load(&self, language: Language) -> Result<Arc<LoadedDictionary>, AutocorrectError> {
        if let Some(dictionary) = self.loaded.read().get(&language).cloned() {
            return Ok(dictionary);
        }
        let dictionary = self.read(language)?;
        self.loaded
            .write()
            .insert(language, Arc::clone(&dictionary));
        Ok(dictionary)
    }

    /// The loaded dictionary, when it is already in memory.
    pub fn loaded(&self, language: Language) -> Option<Arc<LoadedDictionary>> {
        self.loaded.read().get(&language).cloned()
    }

    /// Forgets everything; the next use re-reads the folder.
    pub fn reload(&self) {
        self.loaded.write().clear();
        self.states.write().clear();
        *self.manifest.write() = None;
    }

    /// Whether a language can be checked right now.
    pub fn is_ready(&self, language: Language) -> bool {
        self.state(language).is_ready()
    }

    fn manifest_entry(&self, language: Language) -> Option<DictionaryManifestEntry> {
        if self.manifest.read().is_none() {
            let manifest = DictionaryManifest::load(&self.dir.join(DICTIONARY_MANIFEST_FILE));
            *self.manifest.write() = manifest;
        }
        self.manifest
            .read()
            .as_ref()
            .and_then(|manifest| manifest.entry(language).cloned())
    }

    fn read(&self, language: Language) -> Result<Arc<LoadedDictionary>, AutocorrectError> {
        let paths = self.paths(language);
        let aff_bytes = read_dictionary_file(&paths.aff, language, &self.expected_paths(language))?;
        let dic_bytes = read_dictionary_file(&paths.dic, language, &self.expected_paths(language))?;

        if let Some(entry) = self.manifest_entry(language) {
            let aff_hash = sha256_hex(&aff_bytes);
            let dic_hash = sha256_hex(&dic_bytes);
            if !aff_hash.eq_ignore_ascii_case(&entry.aff_sha256) {
                return Err(AutocorrectError::DictionaryInvalid {
                    language: language.as_str().to_string(),
                    reason: format!(
                        "{} does not match the manifest checksum",
                        paths.aff.display()
                    ),
                });
            }
            if !dic_hash.eq_ignore_ascii_case(&entry.dic_sha256) {
                return Err(AutocorrectError::DictionaryInvalid {
                    language: language.as_str().to_string(),
                    reason: format!(
                        "{} does not match the manifest checksum",
                        paths.dic.display()
                    ),
                });
            }
        }

        let aff = decode_text(&aff_bytes)?;
        let encoding = declared_encoding(&aff);
        let dic = decode_with(&dic_bytes, encoding)?;
        let words = count_entries(&dic);

        let dictionary = spellbook::Dictionary::new(&aff, &dic).map_err(|error| {
            AutocorrectError::DictionaryInvalid {
                language: language.as_str().to_string(),
                reason: format!("{error}"),
            }
        })?;
        Ok(Arc::new(LoadedDictionary {
            language,
            dictionary: Mutex::new(dictionary),
            words,
        }))
    }
}

impl std::fmt::Debug for DictionaryManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DictionaryManager")
            .field("dir", &self.dir)
            .field("loaded", &self.loaded.read().len())
            .finish()
    }
}

/// Largest dictionary file the loader will read.
///
/// A dictionary is public data the user installed, but it is still a file this process
/// parses: without a bound, a wrong path or a hostile file could ask for an unbounded
/// allocation. The biggest real Hunspell pairs are a few megabytes, so this is generous.
pub const MAX_DICTIONARY_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Reads one dictionary file, bounded and with a clear error for each failure.
///
/// A missing file is `DictionaryMissing` with the paths the user has to fill in; a file
/// that is too large, unreadable, or a directory is `DictionaryInvalid`, because the file
/// exists and the reason is about its content.
fn read_dictionary_file(
    path: &Path,
    language: Language,
    expected: &[String],
) -> Result<Vec<u8>, AutocorrectError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return Err(AutocorrectError::DictionaryMissing {
                language: language.as_str().to_string(),
                expected: expected.to_vec(),
            })
        }
    };
    if !metadata.is_file() {
        return Err(AutocorrectError::DictionaryInvalid {
            language: language.as_str().to_string(),
            reason: format!("{} is not a file", path.display()),
        });
    }
    if metadata.len() > MAX_DICTIONARY_FILE_BYTES {
        return Err(AutocorrectError::DictionaryInvalid {
            language: language.as_str().to_string(),
            reason: format!(
                "{} is larger than {} bytes",
                path.display(),
                MAX_DICTIONARY_FILE_BYTES
            ),
        });
    }
    std::fs::read(path).map_err(|_| AutocorrectError::DictionaryInvalid {
        language: language.as_str().to_string(),
        reason: format!("{} could not be read", path.display()),
    })
}

/// A short label for a dictionary file, used in status reports.
fn dictionary_paths_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// SHA-256 of a file, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

/// The encoding declared by the affix file's `SET` directive.
///
/// Hunspell dictionaries are usually UTF-8; the directive exists because older ones
/// are not, so it is read before decoding the word list.
pub fn declared_encoding(aff: &str) -> &str {
    for line in aff.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("SET ") {
            return rest.trim();
        }
    }
    "UTF-8"
}

/// Decodes a dictionary file as UTF-8.
fn decode_text(bytes: &[u8]) -> Result<String, AutocorrectError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) => Err(AutocorrectError::DictionaryInvalid {
            language: "unknown".to_string(),
            reason: "the affix file is not valid UTF-8".to_string(),
        }),
    }
}

/// Decodes the word list using the encoding declared in the affix file.
///
/// UTF-8 and ISO-8859-1 are supported, which covers the dictionaries this project
/// documents. Anything else is refused with a clear reason instead of being decoded
/// wrongly, because a mis-decoded dictionary would report every word as a typo.
pub fn decode_with(bytes: &[u8], encoding: &str) -> Result<String, AutocorrectError> {
    let normalized = encoding.trim().to_ascii_uppercase().replace('_', "-");
    match normalized.as_str() {
        "" | "UTF-8" | "UTF8" => decode_text(bytes),
        "ISO8859-1" | "ISO-8859-1" | "LATIN1" | "LATIN-1" => {
            Ok(bytes.iter().map(|byte| *byte as char).collect())
        }
        other => Err(AutocorrectError::DictionaryInvalid {
            language: "unknown".to_string(),
            reason: format!("unsupported dictionary encoding: {other}"),
        }),
    }
}

/// Number of stems a `.dic` file lists, when the header count is usable.
fn count_entries(dic: &str) -> Option<usize> {
    let mut lines = dic.lines();
    let declared = lines.next()?.trim().parse::<usize>().ok()?;
    let listed = lines.filter(|line| !line.trim().is_empty()).count();
    if declared == 0 {
        return Some(listed);
    }
    // The header count is normally the number of following lines; a mismatch means the
    // file was edited, so the real count is reported instead.
    Some(listed.max(declared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// A minimal but valid Hunspell pair, written by the tests themselves.
    const FIXTURE_AFF: &str = "SET UTF-8\n";
    const FIXTURE_DIC: &str = "4\nhello\nworld\nпривет\nёж\n";

    fn fixture_dir() -> tempfile::TempDir {
        let directory = tempdir().unwrap();
        write_pair(
            directory.path(),
            Language::English,
            FIXTURE_AFF,
            FIXTURE_DIC,
        );
        write_pair(
            directory.path(),
            Language::Russian,
            FIXTURE_AFF,
            FIXTURE_DIC,
        );
        directory
    }

    fn write_pair(dir: &Path, language: Language, aff: &str, dic: &str) {
        fs::write(dir.join(language.aff_file()), aff).unwrap();
        fs::write(dir.join(language.dic_file()), dic).unwrap();
    }

    #[test]
    fn a_missing_folder_reports_the_expected_paths() {
        let directory = tempdir().unwrap();
        let manager = DictionaryManager::new(directory.path());
        let state = manager.state(Language::English);
        match state {
            DictionaryState::Missing { expected, .. } => {
                assert_eq!(expected.len(), 2);
                assert!(expected[0].ends_with("en_US.aff"));
                assert!(expected[1].ends_with("en_US.dic"));
            }
            other => panic!("expected a missing dictionary, got {other:?}"),
        }
        assert!(!manager.is_ready(Language::English));
        let error = manager.load(Language::Russian).unwrap_err();
        assert_eq!(error.code(), "dictionary_missing");
        assert_eq!(error.expected_paths().len(), 2);
    }

    #[test]
    fn a_fixture_pair_loads_and_checks_words() {
        let directory = fixture_dir();
        let manager = DictionaryManager::new(directory.path());
        let state = manager.state(Language::English);
        assert!(state.is_ready(), "{state:?}");
        let dictionary = manager.load(Language::English).unwrap();
        assert!(dictionary.check("hello"));
        assert!(dictionary.check("world"));
        assert!(!dictionary.check("helo"));
        assert!(dictionary.word_count().unwrap_or(0) >= 4);
        // Nothing is reloaded per call: the same instance comes back.
        let again = manager.load(Language::English).unwrap();
        assert!(Arc::ptr_eq(&dictionary, &again));
    }

    #[test]
    fn suggestions_come_from_the_dictionary_and_respect_the_limit() {
        let directory = fixture_dir();
        let manager = DictionaryManager::new(directory.path());
        let dictionary = manager.load(Language::English).unwrap();
        let suggestions = dictionary.suggestions("helo", 5);
        assert!(
            suggestions.iter().any(|word| word == "hello"),
            "expected hello in {suggestions:?}"
        );
        assert!(dictionary.suggestions("helo", 1).len() <= 1);
        assert!(dictionary.suggestions("helo", 0).is_empty());
    }

    #[test]
    fn a_broken_pair_is_reported_as_invalid_not_missing() {
        let directory = tempdir().unwrap();
        write_pair(
            directory.path(),
            Language::English,
            FIXTURE_AFF,
            "not a dictionary at all\n",
        );
        let manager = DictionaryManager::new(directory.path());
        // A file that exists but cannot be parsed is `Invalid`, never `Missing`.
        let state = manager.state(Language::English);
        assert!(
            matches!(state, DictionaryState::Invalid { .. }),
            "{state:?}"
        );
        assert!(!manager.is_ready(Language::English));
    }

    #[test]
    fn an_unsupported_encoding_is_refused_instead_of_guessed() {
        let directory = tempdir().unwrap();
        write_pair(
            directory.path(),
            Language::English,
            "SET KOI8-R\n",
            FIXTURE_DIC,
        );
        let manager = DictionaryManager::new(directory.path());
        let error = manager.load(Language::English).unwrap_err();
        assert_eq!(error.code(), "dictionary_invalid");
        assert!(error.to_string().contains("KOI8-R"));
    }

    #[test]
    fn latin1_dictionaries_are_decoded() {
        let decoded = decode_with(b"caf\xe9\n", "ISO8859-1").unwrap();
        assert_eq!(decoded, "café\n");
        assert_eq!(declared_encoding("SET ISO8859-1\n"), "ISO8859-1");
        assert_eq!(declared_encoding("SET UTF-8\n"), "UTF-8");
        assert_eq!(declared_encoding("no directive"), "UTF-8");
        assert!(decode_with(b"\xff\xfe", "UTF-8").is_err());
    }

    #[test]
    fn a_manifest_verifies_the_installed_files() {
        let directory = tempdir().unwrap();
        write_pair(
            directory.path(),
            Language::English,
            FIXTURE_AFF,
            FIXTURE_DIC,
        );
        let manifest = DictionaryManifest {
            entries: vec![DictionaryManifestEntry {
                language: Language::English,
                aff_sha256: sha256_hex(FIXTURE_AFF.as_bytes()),
                dic_sha256: sha256_hex(FIXTURE_DIC.as_bytes()),
                source: Some("fixture".to_string()),
                version: Some("1".to_string()),
                license: Some("test".to_string()),
            }],
        };
        fs::write(
            directory.path().join(DICTIONARY_MANIFEST_FILE),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let manager = DictionaryManager::new(directory.path());
        match manager.state(Language::English) {
            DictionaryState::Ready { source, .. } => {
                assert_eq!(source.as_deref(), Some("fixture"));
            }
            other => panic!("expected ready, got {other:?}"),
        }

        // Replacing a file without updating the manifest is detected.
        fs::write(
            directory.path().join(Language::English.dic_file()),
            "1\nhello\n",
        )
        .unwrap();
        manager.reload();
        match manager.state(Language::English) {
            DictionaryState::Invalid { reason, .. } => {
                assert!(reason.contains("checksum"), "{reason}");
            }
            other => panic!("expected an invalid checksum state, got {other:?}"),
        }
    }

    #[test]
    fn a_directory_in_place_of_a_dictionary_is_refused() {
        let directory = tempdir().unwrap();
        // A folder named like a dictionary file is not a dictionary.
        fs::create_dir_all(directory.path().join(Language::English.aff_file())).unwrap();
        fs::write(
            directory.path().join(Language::English.dic_file()),
            FIXTURE_DIC,
        )
        .unwrap();
        let manager = DictionaryManager::new(directory.path());
        let error = manager.load(Language::English).unwrap_err();
        assert_eq!(error.code(), "dictionary_invalid");
        assert!(error.to_string().contains("not a file"));
        match manager.state(Language::English) {
            DictionaryState::Invalid { .. } => {}
            other => panic!("expected an invalid dictionary, got {other:?}"),
        }
        // Another language is still reported as missing, not as invalid.
        assert!(matches!(
            manager.state(Language::Russian),
            DictionaryState::Missing { .. }
        ));
    }

    #[test]
    fn a_broken_manifest_is_ignored() {
        let directory = fixture_dir();
        fs::write(
            directory.path().join(DICTIONARY_MANIFEST_FILE),
            "{ not json",
        )
        .unwrap();
        let manager = DictionaryManager::new(directory.path());
        // Without a usable manifest the dictionaries still load.
        assert!(manager.is_ready(Language::English));
    }

    #[test]
    fn reload_picks_up_a_new_install() {
        let directory = tempdir().unwrap();
        let manager = DictionaryManager::new(directory.path());
        assert!(!manager.is_ready(Language::Russian));
        write_pair(
            directory.path(),
            Language::Russian,
            FIXTURE_AFF,
            FIXTURE_DIC,
        );
        assert!(!manager.is_ready(Language::Russian), "states are cached");
        manager.reload();
        assert!(manager.is_ready(Language::Russian));
        assert!(manager.load(Language::Russian).unwrap().check("привет"));
    }

    #[test]
    fn the_manager_reports_every_language() {
        let directory = fixture_dir();
        let manager = DictionaryManager::new(directory.path());
        let states = manager.states();
        assert_eq!(states.len(), 2);
        assert!(states.iter().all(DictionaryState::is_ready));
        assert_eq!(states[0].language(), Language::Russian);
        assert_eq!(states[1].language(), Language::English);
    }
}
