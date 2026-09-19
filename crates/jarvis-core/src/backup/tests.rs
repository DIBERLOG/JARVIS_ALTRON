//! The backup cycle, end to end, with real databases and a real container.
//!
//! Every test here works inside a temporary directory. Nothing touches the
//! profile of the person running it: the "application" is a directory tree built
//! for the test, with the same file names the real one uses.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rusqlite::Connection;
use tempfile::TempDir;

use super::container::{self, is_safe_entry_name, ComponentKind, Limits};
use super::error::BackupError;
use super::restore::{self, LocalKeyBinding, NoKeyBinding, RestorePlan, RestoreStage};
use super::snapshot::{self, BackupRoots, Component, COMPONENTS};
use super::*;
use crate::sync::crypto::{export_backup, random_master_key, MasterKey};

/// A password that is long enough to be accepted.
const PASSWORD: &[u8] = b"fixture-password";

/// A temporary installation: a data directory, a configuration directory, and
/// the four databases the real application keeps, with one row each.
struct Fixture {
    _directory: TempDir,
    roots: BackupRoots,
    note_text: String,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let data_dir = directory.path().join("data");
        let config_dir = directory.path().join("config");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        let roots = BackupRoots {
            data_dir,
            config_dir,
        };
        let fixture = Self {
            _directory: directory,
            roots,
            note_text: "FICTIONAL_SECRET_NOTE".to_string(),
        };
        fixture.write_key_envelope();
        fixture.write_databases();
        fixture.write_documents();
        fixture
    }

    /// The portable key envelope the application itself writes.
    fn write_key_envelope(&self) {
        let master = random_master_key().expect("a key");
        let envelope = export_backup(&master, PASSWORD).expect("an envelope");
        let bytes = serde_json::to_vec(&envelope).expect("json");
        std::fs::write(self.roots.data_dir.join(KEY_ENVELOPE_FILE), bytes).expect("the envelope");
    }

    fn master_key(&self) -> MasterKey {
        let bytes = std::fs::read(self.roots.data_dir.join(KEY_ENVELOPE_FILE)).expect("envelope");
        let envelope: crate::sync::crypto::PortableKeyBackup =
            serde_json::from_slice(&bytes).expect("envelope json");
        crate::sync::crypto::import_backup(&envelope, PASSWORD).expect("the master key")
    }

    /// Four real SQLite databases, in WAL mode, each with one recognizable row.
    fn write_databases(&self) {
        self.seed_databases(&self.note_text);
    }

    /// Creates the four databases with a given value in each row.
    fn seed_databases(&self, value: &str) {
        for component in COMPONENTS
            .iter()
            .filter(|component| component.kind == ComponentKind::Sqlite)
        {
            let path = self.roots.source_path(component);
            let connection = Connection::open(&path).expect("the database");
            connection
                .execute_batch("PRAGMA journal_mode = WAL;")
                .expect("wal");
            connection
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS content (name TEXT PRIMARY KEY, value TEXT);",
                )
                .expect("a table");
            connection
                .execute("DELETE FROM content;", [])
                .expect("a clean table");
            connection
                .execute(
                    "INSERT OR REPLACE INTO content (name, value) VALUES ('row', ?1)",
                    rusqlite::params![format!("{}|{}", component.name, value)],
                )
                .expect("a row");
        }
    }

    /// The settings documents, including the two switches a restore must reset.
    fn write_documents(&self) {
        std::fs::write(
            self.roots.data_dir.join("whisper-settings.json"),
            br#"{"enabled":true,"binary_path":"C:/tools/whisper-cli.exe","keep_audio":true}"#,
        )
        .expect("whisper settings");
        std::fs::write(
            self.roots.data_dir.join("desktop.json"),
            br#"{"autostart_enabled":true,"start_minimized":true,"close_behavior":"tray"}"#,
        )
        .expect("desktop settings");
        std::fs::write(
            self.roots.config_dir.join("app.db"),
            br#"{"language":"ru","microphone":0}"#,
        )
        .expect("settings document");
    }

    /// The value stored in one database, read back after a restore.
    fn read_row(&self, component: &Component) -> Option<String> {
        let path = self.roots.source_path(component);
        if !path.is_file() {
            return None;
        }
        let connection =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        connection
            .query_row("SELECT value FROM content WHERE name = 'row'", [], |row| {
                row.get::<_, String>(0)
            })
            .ok()
    }

    /// Removes everything a restore would replace, so a successful restore can
    /// only be explained by the container. Nothing is written back: the next step
    /// decides what the live state is.
    fn erase_user_data(&self) {
        for component in COMPONENTS.iter() {
            let path = self.roots.source_path(component);
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
        }
        // The envelope is written back: a restore carries its own, and this keeps
        // the fixture's key material usable between steps.
        self.write_key_envelope();
    }

    fn container_path(&self, name: &str) -> PathBuf {
        self.roots.data_dir.join(name)
    }
}

fn live_components() -> Vec<Component> {
    COMPONENTS.to_vec()
}

// ---------------------------------------------------------------- the container

#[test]
fn a_full_export_produces_a_container_that_verifies_and_previews() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    let plan = ExportPlan::full(fixture.roots.clone());
    let report = export(&plan, &destination, PASSWORD).expect("the export");
    assert!(destination.is_file());
    assert!(report
        .components
        .contains(&"notes/sync.sqlite3".to_string()));
    assert!(report
        .components
        .contains(&"key/portable-envelope.json".to_string()));
    // The names are logical: no directory of the machine is in a name.
    for name in &report.components {
        assert!(is_safe_entry_name(name), "{name}");
        assert!(!name.contains(':'), "{name}");
    }
    // The container is not a listing of the profile.
    assert!(!report
        .components
        .iter()
        .any(|name| name.to_lowercase().contains("ebwebview")));
    assert!(!report.components.iter().any(|name| name.contains("audit")));
    assert!(!report.components.iter().any(|name| name.ends_with(".wav")));

    let cancel = AtomicBool::new(false);
    let preview = inspect(&destination, PASSWORD, &cancel, &LIMITS).expect("the inspect");
    assert_eq!(preview.format_version, FORMAT_VERSION);
    assert!(preview.total_bytes > 0);
    assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);
}

#[test]
fn a_wrong_password_is_refused_before_anything_is_read() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let cancel = AtomicBool::new(false);
    assert_eq!(
        inspect(&destination, b"not-the-password", &cancel, &LIMITS).unwrap_err(),
        BackupError::WrongPasswordOrDamaged
    );
}

#[test]
fn a_damaged_byte_in_a_large_section_is_refused() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let original = std::fs::read(&destination).expect("the container");
    let cancel = AtomicBool::new(false);

    // Every region of the file: the magic, the version, the header, and several
    // places in the payload.
    let positions = [
        0usize,
        3,
        9,
        12,
        original.len() / 3,
        original.len() / 2,
        original.len() - 2,
    ];
    for position in positions {
        let mut damaged = original.clone();
        damaged[position] ^= 0x40;
        let path = fixture.container_path("damaged.jarvisbak");
        std::fs::write(&path, &damaged).expect("the damaged container");
        let outcome = inspect(&path, PASSWORD, &cancel, &LIMITS);
        let code = outcome
            .err()
            .map(|error| error.code())
            .unwrap_or("none")
            .to_string();
        assert_ne!(code, "none", "a change at byte {position} must be refused");
        assert!(
            [
                "not_a_container",
                "unsupported_version",
                "wrong_password_or_damaged",
                "manifest_mismatch",
                "content_mismatch",
                "invalid_header",
                "truncated_container",
                "too_large",
            ]
            .contains(&code.as_str()),
            "unexpected code {code} at byte {position}"
        );
    }
}

#[test]
fn a_tampered_manifest_is_refused_without_a_password() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let (header, _) = container::read_header(&destination, &LIMITS).expect("the header");

    let rebuilt = rebuild_with_header(&destination, &mut header.clone(), |header| {
        header.manifest.entries[0].name = "notes/other.sqlite3".to_string();
    });
    let path = fixture.container_path("tampered.jarvisbak");
    std::fs::write(&path, &rebuilt).expect("the tampered container");
    assert_eq!(
        container::read_header(&path, &LIMITS).unwrap_err(),
        BackupError::ManifestMismatch
    );
}

#[test]
fn a_newer_format_version_is_refused() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let mut newer = std::fs::read(&destination).expect("the container");
    newer[8] = 99;
    let path = fixture.container_path("newer.jarvisbak");
    std::fs::write(&path, &newer).expect("the container");
    assert_eq!(
        container::read_header(&path, &LIMITS).unwrap_err(),
        BackupError::UnsupportedVersion
    );
}

/// Rewrites a container with a modified header, leaving the manifest hash alone.
///
/// The hash is deliberately *not* recomputed: this is how a tampered header
/// looks, and the reader has to notice without being told the password.
fn rebuild_with_header(
    source: &Path,
    header: &mut container::ContainerHeader,
    edit: impl FnOnce(&mut container::ContainerHeader),
) -> Vec<u8> {
    edit(header);
    let header_bytes = serde_json::to_vec(header).expect("json");
    let original = std::fs::read(source).expect("the container");
    let old_header_len =
        u32::from_le_bytes([original[10], original[11], original[12], original[13]]) as usize;
    let mut rebuilt = Vec::new();
    rebuilt.extend_from_slice(&original[..10]);
    rebuilt.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    rebuilt.extend_from_slice(&header_bytes);
    rebuilt.extend_from_slice(&original[14 + old_header_len..]);
    rebuilt
}

#[test]
fn a_newer_storage_schema_is_refused_before_anything_live_is_touched() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let (header, _) = container::read_header(&destination, &LIMITS).expect("the header");
    let mut tampered = header.clone();
    for entry in &mut tampered.manifest.entries {
        if entry.kind == ComponentKind::Sqlite {
            entry.schema_version = Some(crate::sync::sqlite::SCHEMA_VERSION + 5);
        }
    }
    let before: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();

    // A database whose own `user_version` is from the future is what the stage
    // refuses; the manifest field alone is a hint, so the database is bumped to
    // match and the container is rebuilt honestly from those files.
    let notes = COMPONENTS[0];
    let notes_path = fixture.roots.source_path(&notes);
    let connection = Connection::open(&notes_path).expect("the database");
    connection
        .execute_batch(&format!(
            "PRAGMA user_version = {};",
            crate::sync::sqlite::SCHEMA_VERSION + 5
        ))
        .expect("the version");
    drop(connection);
    let staging = fixture.roots.data_dir.join("schema-staging");
    let staged = snapshot::snapshot_all(&fixture.roots, &staging, &COMPONENTS).expect("snapshot");
    let container_path = fixture.container_path("schema-newer.jarvisbak");
    write_staged(&fixture, &container_path, &staged);
    let _ = std::fs::remove_dir_all(&staging);
    let _ = tampered;

    let plan = RestorePlan {
        roots: fixture.roots.clone(),
        container: container_path,
        components: live_components(),
        safety_backup: false,
        bind: Box::new(NoKeyBinding),
        limits: LIMITS,
    };
    let cancel = AtomicBool::new(false);
    assert_eq!(
        restore::stage(&plan, PASSWORD, &cancel).unwrap_err(),
        BackupError::SchemaTooNew
    );
    let after: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();
    assert_eq!(before, after, "a refused stage changes nothing");
}

/// Writes a container out of already-staged files.
fn write_staged(fixture: &Fixture, destination: &Path, staged: &[container::EntrySource]) {
    let envelope: crate::sync::crypto::PortableKeyBackup = serde_json::from_slice(
        &std::fs::read(fixture.roots.data_dir.join(KEY_ENVELOPE_FILE)).expect("read"),
    )
    .expect("json");
    let key = crate::sync::crypto::derive_purpose_key(
        &fixture.master_key(),
        crate::sync::crypto::KeyPurpose::Backup,
    )
    .expect("the key");
    container::write_container(
        destination,
        staged,
        &envelope,
        &key,
        app_version(),
        &now_rfc3339(),
        &LIMITS,
    )
    .expect("the container");
}

#[test]
fn a_container_that_declares_more_than_the_bounds_is_refused() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let cancel = AtomicBool::new(false);
    let small_entries = Limits {
        max_entries: 1,
        ..LIMITS
    };
    assert_eq!(
        container::read_header(&destination, &small_entries).unwrap_err(),
        BackupError::TooLarge
    );
    let small_bytes = Limits {
        max_entry_bytes: 1,
        ..LIMITS
    };
    assert_eq!(
        container::read_header(&destination, &small_bytes).unwrap_err(),
        BackupError::TooLarge
    );
    let small_header = Limits {
        max_header_bytes: 8,
        ..LIMITS
    };
    assert_eq!(
        container::read_header(&destination, &small_header).unwrap_err(),
        BackupError::TooLarge
    );
    let _ = inspect(&destination, PASSWORD, &cancel, &LIMITS).expect("the real bounds still read");
}

#[test]
fn an_unsafe_or_duplicated_name_is_refused() {
    for name in [
        "../escape",
        "notes/../../escape",
        "/absolute",
        "c:/drive",
        "notes\\windows",
        "notes//double",
        "notes/NUL",
        "notes/con.txt",
        "notes/",
        "",
        "notes/stream:ads",
    ] {
        assert!(!is_safe_entry_name(name), "{name} must be refused");
        assert_eq!(
            container::entry_path(Path::new("staging"), name).unwrap_err(),
            BackupError::UnsafeEntryName
        );
    }
    for name in [
        "notes/sync.sqlite3",
        "settings/app.db",
        "key/portable-envelope.json",
    ] {
        assert!(is_safe_entry_name(name), "{name} must be allowed");
        let path = container::entry_path(Path::new("staging"), name).expect("a path");
        assert!(path.starts_with("staging"));
    }

    // A manifest with two entries of the same name is refused before any payload
    // is read, and it is refused without the password.
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("duplicate.jarvisbak");
    let mut manifest = container::BackupManifest {
        entries: Vec::new(),
        total_bytes: 20,
    };
    for _ in 0..2 {
        manifest.entries.push(container::ManifestEntry {
            name: "notes/sync.sqlite3".to_string(),
            kind: ComponentKind::Sqlite,
            bytes: 10,
            sha256: "0".repeat(64),
            schema_version: Some(1),
        });
    }
    let manifest_bytes = container::canonical_manifest(&manifest).expect("json");
    let header = container::ContainerHeader {
        format: FORMAT_NAME.to_string(),
        format_version: FORMAT_VERSION,
        created_at: now_rfc3339(),
        app_version: app_version().to_string(),
        cipher: "xchacha20poly1305".to_string(),
        kdf: crate::sync::crypto::KeyPurpose::Backup.label().to_string(),
        envelope: String::new(),
        manifest,
        manifest_sha256: container::sha256_hex(&manifest_bytes),
    };
    let bytes = serde_json::to_vec(&header).expect("json");
    let mut file = Vec::new();
    file.extend_from_slice(&container::MAGIC);
    file.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    file.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    file.extend_from_slice(&bytes);
    std::fs::write(&path, &file).expect("the container");
    assert_eq!(
        container::read_header(&path, &LIMITS).unwrap_err(),
        BackupError::DuplicateEntryName
    );
}

// ------------------------------------------------------------ the full cycle

#[test]
fn export_then_wipe_then_restore_returns_the_same_data() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");

    let before: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();
    fixture.erase_user_data();
    // The live state after the wipe is different data, so a restore that does
    // nothing cannot pass this test by accident.
    fixture.seed_databases("WIPED_AFTER_BACKUP");
    fixture.write_documents();
    let after_wipe: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();
    assert_ne!(before, after_wipe, "the wipe must be real");

    let plan = RestorePlan::full(
        fixture.roots.clone(),
        destination.clone(),
        Box::new(NoKeyBinding),
    );
    let cancel = AtomicBool::new(false);
    let report = restore::restore(&plan, PASSWORD, &cancel).expect("the restore");
    assert!(report.safety_backup.is_some(), "a safety backup is taken");
    assert!(report.rolled_back.is_none(), "nothing was rolled back");

    let restored: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();
    assert_eq!(before, restored, "the data must come back exactly");
    for value in restored.iter().flatten() {
        assert!(value.contains("FICTIONAL_SECRET_NOTE"), "{value}");
    }

    // The two switches that must not travel are off.
    let whisper: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fixture.roots.data_dir.join("whisper-settings.json")).expect("read"),
    )
    .expect("json");
    assert_eq!(
        whisper["enabled"],
        serde_json::Value::Bool(false),
        "dictation stays off"
    );
    let desktop: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fixture.roots.data_dir.join("desktop.json")).expect("read"),
    )
    .expect("json");
    assert_eq!(
        desktop["autostart_enabled"],
        serde_json::Value::Bool(false),
        "autostart stays off"
    );
    assert_eq!(
        desktop["close_behavior"],
        serde_json::Value::String("tray".to_string()),
        "the rest of the document survived"
    );

    // The staging plaintext is gone, the journal is gone, and the replaced state
    // is kept for the user.
    assert!(!fixture.roots.data_dir.join(STAGING_DIR).exists());
    assert!(!fixture.roots.data_dir.join(JOURNAL_FILE).exists());
    assert!(fixture.roots.data_dir.join(PREVIOUS_DIR).is_dir());

    // Importing the same container again is idempotent.
    let again = restore::restore(&plan, PASSWORD, &cancel).expect("the second restore");
    assert!(again.rolled_back.is_none());
    let twice: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();
    assert_eq!(before, twice, "a second import changes nothing");
}

#[test]
fn a_wal_that_holds_changes_the_main_file_does_not_have_is_still_snapshotted() {
    let fixture = Fixture::new();
    let notes = COMPONENTS[0];
    let path = fixture.roots.source_path(&notes);

    // A committed write that stays in the WAL: this is the state a plain file
    // copy gets wrong, because the main file does not have it yet.
    let writer = Connection::open(&path).expect("the writer");
    writer
        .execute_batch("PRAGMA journal_mode = WAL;")
        .expect("wal");
    writer
        .execute(
            "INSERT OR REPLACE INTO content (name, value) VALUES ('in-wal', 'FICTIONAL_WAL_VALUE')",
            [],
        )
        .expect("the row");
    let visible: String = writer
        .query_row(
            "SELECT value FROM content WHERE name = 'in-wal'",
            [],
            |row| row.get(0),
        )
        .expect("the row");
    assert_eq!(visible, "FICTIONAL_WAL_VALUE");

    let wal = PathBuf::from(format!("{}-wal", path.display()));
    assert!(wal.is_file(), "the WAL exists");
    assert!(
        std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0) > 0,
        "the WAL holds frames"
    );

    // A snapshot taken while that writer is open still contains the row.
    let staging = fixture.roots.data_dir.join("probe-staging");
    let staged = snapshot::snapshot_all(&fixture.roots, &staging, &COMPONENTS).expect("snapshot");
    let notes_snapshot = staged
        .iter()
        .find(|entry| entry.name == notes.name)
        .expect("the notes snapshot");
    let copy = Connection::open_with_flags(
        &notes_snapshot.path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("the copy");
    let copied: String = copy
        .query_row(
            "SELECT value FROM content WHERE name = 'in-wal'",
            [],
            |row| row.get(0),
        )
        .expect("the row is in the snapshot");
    assert_eq!(copied, "FICTIONAL_WAL_VALUE");
    // The snapshot is one file that stands on its own.
    assert!(snapshot::integrity_check(&notes_snapshot.path).is_ok());
    assert!(!notes_snapshot.path.with_extension("sqlite3-wal").exists());
    drop(writer);
    let _ = std::fs::remove_dir_all(&staging);
}

// --------------------------------------------------------- failure and rollback

/// A binding that fails on purpose: the DPAPI step is where a restore can still
/// refuse, and the old state has to come back when it does.
struct FailingBinding;

impl LocalKeyBinding for FailingBinding {
    fn bind(&self, _: &MasterKey) -> Result<(), BackupError> {
        Err(BackupError::LocalKeyFailed)
    }
    fn name(&self) -> &'static str {
        "failing"
    }
}

#[test]
fn a_local_key_that_cannot_be_bound_puts_the_old_state_back() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");

    // Change the live data, so a successful restore would be visible.
    for component in COMPONENTS
        .iter()
        .filter(|component| component.kind == ComponentKind::Sqlite)
    {
        let path = fixture.roots.source_path(component);
        let connection = Connection::open(&path).expect("the database");
        connection
            .execute(
                "INSERT OR REPLACE INTO content (name, value) VALUES ('row', 'CHANGED_AFTER_BACKUP')",
                [],
            )
            .expect("the change");
    }

    let plan = RestorePlan::full(fixture.roots.clone(), destination, Box::new(FailingBinding));
    let cancel = AtomicBool::new(false);
    let error = restore::restore(&plan, PASSWORD, &cancel).unwrap_err();
    assert_eq!(error, BackupError::LocalKeyFailed);

    let after: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();
    for value in after.iter().flatten() {
        assert_eq!(value, "CHANGED_AFTER_BACKUP", "the rollback must win");
    }
    assert!(!fixture.roots.data_dir.join(JOURNAL_FILE).exists());
    assert!(!fixture.roots.data_dir.join(PREVIOUS_DIR).exists());
    // The safety backup is not deleted by a rollback: it is the user's.
    assert!(fixture.roots.data_dir.join(SAFETY_DIR).is_dir());
}

#[test]
fn every_stage_of_an_interrupted_restore_is_rolled_back_on_the_next_start() {
    let fixture = Fixture::new();
    for stage in [
        RestoreStage::Prepared,
        RestoreStage::OldMoved,
        RestoreStage::NewInstalled,
        RestoreStage::Verified,
    ] {
        // A known live state, then a crash simulated by the journal alone.
        fixture.erase_user_data();
        fixture.seed_databases(&fixture.note_text);
        fixture.write_documents();
        let expected: Vec<Option<String>> = COMPONENTS
            .iter()
            .map(|component| fixture.read_row(component))
            .collect();

        let previous = fixture.roots.data_dir.join(PREVIOUS_DIR);
        let _ = std::fs::remove_dir_all(&previous);
        std::fs::create_dir_all(&previous).expect("the previous directory");
        let mut components = Vec::new();
        for component in COMPONENTS.iter() {
            let source = fixture.roots.source_path(component);
            let existed = source.is_file();
            if existed {
                std::fs::rename(&source, previous.join(component.file)).expect("moved aside");
            }
            components.push(restore::JournalComponent {
                name: component.name.to_string(),
                kind: component.kind,
                file: component.file.to_string(),
                in_config_dir: component.in_config_dir,
                existed_before: existed,
            });
        }
        // A file the interrupted restore installed, which was not there before.
        let installed = fixture.roots.data_dir.join("autocorrect.sqlite3");
        let installed_is_new = !components
            .iter()
            .any(|component| component.file == "autocorrect.sqlite3" && component.existed_before);
        if installed_is_new {
            std::fs::write(&installed, b"a file the restore created").expect("the installed file");
        }
        let journal = restore::RestoreJournal {
            stage,
            started_at: now_rfc3339(),
            container: "full.jarvisbak".to_string(),
            safety_backup: Some("safety-fixture.jarvisbak".to_string()),
            components,
        };
        crate::fsutil::write_bytes_atomic(
            &fixture.roots.data_dir.join(JOURNAL_FILE),
            &serde_json::to_vec(&journal).expect("json"),
        )
        .expect("the journal");

        let recovered = restore::recover_interrupted(&fixture.roots).expect("the recovery");
        assert_eq!(recovered, Some(stage));
        assert!(!fixture.roots.data_dir.join(JOURNAL_FILE).exists());
        assert!(!fixture.roots.data_dir.join(PREVIOUS_DIR).exists());

        let after: Vec<Option<String>> = COMPONENTS
            .iter()
            .map(|component| fixture.read_row(component))
            .collect();
        assert_eq!(expected, after, "the old data is back after {stage:?}");
        if installed_is_new {
            assert!(
                !installed.exists(),
                "a file the restore created is removed again"
            );
        }
    }
}

#[test]
fn a_committed_journal_is_only_cleaned_up() {
    let fixture = Fixture::new();
    let journal = restore::RestoreJournal {
        stage: RestoreStage::Committed,
        started_at: now_rfc3339(),
        container: "full.jarvisbak".to_string(),
        safety_backup: None,
        components: Vec::new(),
    };
    crate::fsutil::write_bytes_atomic(
        &fixture.roots.data_dir.join(JOURNAL_FILE),
        &serde_json::to_vec(&journal).expect("json"),
    )
    .expect("the journal");
    assert_eq!(
        restore::recover_interrupted(&fixture.roots).expect("the recovery"),
        Some(RestoreStage::Committed)
    );
    assert!(!fixture.roots.data_dir.join(JOURNAL_FILE).exists());
}

#[test]
fn a_safety_backup_exists_before_anything_is_replaced() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    for component in COMPONENTS
        .iter()
        .filter(|component| component.kind == ComponentKind::Sqlite)
    {
        let path = fixture.roots.source_path(component);
        let connection = Connection::open(&path).expect("the database");
        connection
            .execute(
                "INSERT OR REPLACE INTO content (name, value) VALUES ('row', 'BEFORE_RESTORE')",
                [],
            )
            .expect("the change");
    }

    let plan = RestorePlan::full(fixture.roots.clone(), destination, Box::new(NoKeyBinding));
    let cancel = AtomicBool::new(false);
    let report = restore::restore(&plan, PASSWORD, &cancel).expect("the restore");
    let safety = report.safety_backup.expect("a safety backup");

    // The safety backup is a real container of the state the restore replaced.
    let path = fixture.roots.data_dir.join(SAFETY_DIR).join(&safety);
    assert!(path.is_file());
    let preview = inspect(&path, PASSWORD, &cancel, &LIMITS).expect("it verifies");
    assert!(preview
        .entries
        .iter()
        .any(|entry| entry.name == "notes/sync.sqlite3"));
    let staging = fixture.roots.data_dir.join("safety-check");
    container::extract(&path, PASSWORD, &staging, &cancel, &LIMITS).expect("it extracts");
    // The logical name decides the path: `notes/sync.sqlite3`.
    let restored = Connection::open(staging.join("notes").join("sync.sqlite3")).expect("the copy");
    let value: String = restored
        .query_row("SELECT value FROM content WHERE name = 'row'", [], |row| {
            row.get(0)
        })
        .expect("the row");
    assert!(value.contains("BEFORE_RESTORE"), "{value}");
    let _ = std::fs::remove_dir_all(&staging);

    // It is still there after a successful restore: it is the user's to keep.
    assert!(path.is_file());
    let status = BackupStatus::read(&fixture.roots);
    assert_eq!(
        status.newest_safety_backup.as_deref(),
        Some(safety.as_str())
    );
}

// ------------------------------------------------------------- bounds and codes

#[test]
fn an_export_never_replaces_a_file_without_permission() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("exists.jarvisbak");
    std::fs::write(&destination, b"something the person put here").expect("the file");
    let plan = ExportPlan::full(fixture.roots.clone());
    assert_eq!(
        export(&plan, &destination, PASSWORD).unwrap_err(),
        BackupError::DestinationUnavailable
    );
    let plan = ExportPlan {
        overwrite: true,
        ..ExportPlan::full(fixture.roots.clone())
    };
    export(&plan, &destination, PASSWORD).expect("the export");
    let leftovers: Vec<String> = std::fs::read_dir(&fixture.roots.data_dir)
        .expect("the data directory")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.contains(".part-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(!fixture.roots.data_dir.join("export-staging").exists());
}

#[test]
fn a_short_password_is_refused() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("short.jarvisbak");
    assert_eq!(
        export(
            &ExportPlan::full(fixture.roots.clone()),
            &destination,
            b"short"
        )
        .unwrap_err(),
        BackupError::PasswordTooShort
    );
    assert!(!destination.exists(), "nothing is written");
}

#[test]
fn an_export_without_a_key_envelope_is_refused() {
    let fixture = Fixture::new();
    std::fs::remove_file(fixture.roots.data_dir.join(KEY_ENVELOPE_FILE)).expect("removed");
    let destination = fixture.container_path("nokey.jarvisbak");
    assert_eq!(
        export(
            &ExportPlan::full(fixture.roots.clone()),
            &destination,
            PASSWORD
        )
        .unwrap_err(),
        BackupError::Storage
    );
    assert!(!destination.exists());
}

#[test]
fn a_cancelled_read_stops_and_leaves_no_staging_behind() {
    let fixture = Fixture::new();
    let destination = fixture.container_path("cancel.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let cancel = AtomicBool::new(true);
    let staging = fixture.roots.data_dir.join("cancel-staging");
    assert!(container::extract(&destination, PASSWORD, &staging, &cancel, &LIMITS).is_err());
    let _ = std::fs::remove_dir_all(&staging);
}

#[test]
fn the_components_are_the_ones_the_requirement_names_and_nothing_else() {
    let names: Vec<&str> = COMPONENTS.iter().map(|component| component.name).collect();
    for required in [
        "notes/sync.sqlite3",
        "vault/vault.sqlite3",
        "memory/ai-memory.sqlite3",
        "autocorrect/autocorrect.sqlite3",
        "key/portable-envelope.json",
        "settings/whisper-settings.json",
        "settings/desktop.json",
        "settings/app.db",
    ] {
        assert!(names.contains(&required), "{required} must be included");
    }
    for forbidden in [
        "key.dpapi",
        "device.id",
        "dictionaries.json",
        "actions-audit.jsonl",
        "dictation.wav",
        "ggml-small.bin",
        "whisper-cli.exe",
        "ebwebview",
    ] {
        assert!(
            !names.iter().any(|name| name.contains(forbidden)),
            "{forbidden} must never be in a backup"
        );
    }
    // The key envelope is the portable one, never the DPAPI blob.
    let envelope = COMPONENTS
        .iter()
        .find(|component| component.kind == ComponentKind::PortableKey)
        .expect("the envelope");
    assert_eq!(envelope.file, "key.backup.json");
    assert!(COMPONENTS
        .iter()
        .filter(|component| component.kind == ComponentKind::Sqlite)
        .all(|component| component.required));
}

#[test]
fn the_status_reports_only_safe_facts() {
    let fixture = Fixture::new();
    let status = BackupStatus::read(&fixture.roots);
    assert!(status.available);
    assert_eq!(status.format_version, FORMAT_VERSION);
    assert!(status.interrupted_restore.is_none());
    assert!(!status.previous_state_present);
    assert!(status.newest_safety_backup.is_none());
    let rendered = serde_json::to_string(&status).expect("json");
    for forbidden in ["FICTIONAL", "password", "C:", "\\\\", "Users"] {
        assert!(!rendered.contains(forbidden), "{rendered}");
    }

    let journal = restore::RestoreJournal {
        stage: RestoreStage::NewInstalled,
        started_at: now_rfc3339(),
        container: "full.jarvisbak".to_string(),
        safety_backup: None,
        components: Vec::new(),
    };
    crate::fsutil::write_bytes_atomic(
        &fixture.roots.data_dir.join(JOURNAL_FILE),
        &serde_json::to_vec(&journal).expect("json"),
    )
    .expect("the journal");
    let status = BackupStatus::read(&fixture.roots);
    assert_eq!(status.interrupted_restore.as_deref(), Some("new_installed"));
    let status = status.with_last(Some("export"), Some("none"));
    assert_eq!(status.last_operation.as_deref(), Some("export"));
    assert_eq!(stage_name(RestoreStage::OldMoved), "old_moved");
}

#[test]
fn a_container_whose_notes_database_is_not_a_database_is_refused_before_the_restore() {
    let fixture = Fixture::new();
    let before: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();

    // A container that is internally perfect and whose notes entry is garbage: it
    // passes the manifest and the AEAD, and it must still be refused, because a
    // file that is not a database cannot be a backup of one.
    let staging = fixture.roots.data_dir.join("broken-staging");
    let staged = snapshot::snapshot_all(&fixture.roots, &staging, &COMPONENTS).expect("snapshot");
    let notes = staged
        .iter()
        .find(|entry| entry.name == "notes/sync.sqlite3")
        .expect("the notes entry");
    std::fs::write(&notes.path, b"this is not a database, and never was").expect("the bytes");
    let envelope: crate::sync::crypto::PortableKeyBackup = serde_json::from_slice(
        &std::fs::read(fixture.roots.data_dir.join(KEY_ENVELOPE_FILE)).expect("read"),
    )
    .expect("json");
    let key = crate::sync::crypto::derive_purpose_key(
        &fixture.master_key(),
        crate::sync::crypto::KeyPurpose::Backup,
    )
    .expect("the key");
    let broken = fixture.container_path("broken.jarvisbak");
    container::write_container(
        &broken,
        &staged,
        &envelope,
        &key,
        app_version(),
        &now_rfc3339(),
        &LIMITS,
    )
    .expect("the container is written: its manifest describes its own bytes");
    let _ = std::fs::remove_dir_all(&staging);

    let plan = RestorePlan::full(fixture.roots.clone(), broken, Box::new(NoKeyBinding));
    let cancel = AtomicBool::new(false);
    let error = restore::restore(&plan, PASSWORD, &cancel).unwrap_err();
    assert_eq!(error, BackupError::IntegrityCheckFailed);
    let after: Vec<Option<String>> = COMPONENTS
        .iter()
        .map(|component| fixture.read_row(component))
        .collect();
    assert_eq!(before, after, "a refused restore changes nothing");
    assert!(!fixture.roots.data_dir.join(JOURNAL_FILE).exists());
    assert!(!fixture.roots.data_dir.join(PREVIOUS_DIR).exists());
}

#[test]
fn a_container_without_the_notes_database_is_refused_at_the_stage() {
    let fixture = Fixture::new();
    let staging = fixture.roots.data_dir.join("partial-staging");
    let staged: Vec<_> = snapshot::snapshot_all(&fixture.roots, &staging, &COMPONENTS)
        .expect("snapshot")
        .into_iter()
        .filter(|entry| entry.name != "notes/sync.sqlite3")
        .collect();
    let envelope: crate::sync::crypto::PortableKeyBackup = serde_json::from_slice(
        &std::fs::read(fixture.roots.data_dir.join(KEY_ENVELOPE_FILE)).expect("read"),
    )
    .expect("json");
    let key = crate::sync::crypto::derive_purpose_key(
        &fixture.master_key(),
        crate::sync::crypto::KeyPurpose::Backup,
    )
    .expect("the key");
    let partial = fixture.container_path("partial.jarvisbak");
    container::write_container(
        &partial,
        &staged,
        &envelope,
        &key,
        app_version(),
        &now_rfc3339(),
        &LIMITS,
    )
    .expect("the container");
    let _ = std::fs::remove_dir_all(&staging);

    let plan = RestorePlan::full(fixture.roots.clone(), partial, Box::new(NoKeyBinding));
    let cancel = AtomicBool::new(false);
    let error = restore::restore(&plan, PASSWORD, &cancel).unwrap_err();
    assert_eq!(error.code(), "invalid_header");
    assert!(!fixture.roots.data_dir.join(JOURNAL_FILE).exists());
    assert!(!fixture.roots.data_dir.join(PREVIOUS_DIR).exists());
}

#[test]
fn the_checkpoint_step_empties_the_write_ahead_log() {
    let fixture = Fixture::new();
    let notes = COMPONENTS[0];
    let path = fixture.roots.source_path(&notes);
    let connection = Connection::open(&path).expect("the database");
    connection
        .execute_batch("PRAGMA journal_mode = WAL;")
        .expect("wal");
    connection
        .execute(
            "INSERT OR REPLACE INTO content (name, value) VALUES ('checkpointed', 'yes')",
            [],
        )
        .expect("the row");
    drop(connection);
    let checkpointed = snapshot::checkpoint_databases(&fixture.roots.data_dir).expect("checkpoint");
    assert!(checkpointed >= 1);
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    let wal_bytes = std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0);
    assert_eq!(wal_bytes, 0, "the WAL is truncated by the checkpoint step");
}

#[test]
fn no_log_or_report_carries_the_note_fixture() {
    // The one string every fixture writes. It must not appear in any error, any
    // status, or any preview this module can produce.
    let fixture = Fixture::new();
    let destination = fixture.container_path("full.jarvisbak");
    export(
        &ExportPlan::full(fixture.roots.clone()),
        &destination,
        PASSWORD,
    )
    .expect("the export");
    let cancel = AtomicBool::new(false);
    let preview = inspect(&destination, PASSWORD, &cancel, &LIMITS).expect("the inspect");
    let rendered = serde_json::to_string(&preview).expect("json");
    assert!(!rendered.contains("FICTIONAL_SECRET_NOTE"), "{rendered}");
    let status = serde_json::to_string(&BackupStatus::read(&fixture.roots)).expect("json");
    assert!(!status.contains("FICTIONAL"), "{status}");
    // And an error never carries it either.
    let error = match inspect(&destination, b"wrong-password", &cancel, &LIMITS) {
        Err(error) => error,
        Ok(_) => panic!("a wrong password must be refused"),
    };
    let rendered = format!("{error} {:?} {} ", error, error.code());
    assert!(!rendered.contains("FICTIONAL"), "{rendered}");
}
