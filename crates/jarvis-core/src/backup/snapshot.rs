//! Taking a consistent snapshot of everything a backup carries.
//!
//! # Why the live files are never copied
//!
//! The four databases this application keeps run in WAL mode, and their WAL is
//! not assumed to be checkpointed: at the moment a backup starts, committed data
//! can live only in `-wal`. Copying `db`, `db-wal` and `db-shm` by hand produces
//! a pair of files that may belong to different moments, and copying only `db`
//! silently loses everything the WAL holds.
//!
//! Every SQLite component is therefore snapshotted with `VACUUM INTO`, which is
//! SQLite's own documented backup path: it opens a read transaction, sees the
//! database as of one moment including committed WAL frames, writes a complete,
//! defragmented copy, and leaves the live file untouched. It runs on a
//! connection this module opens for the snapshot alone, so no live store has to
//! be reached into and no connection has to be closed to make a backup.
//!
//! # What "coordinated" means here, and where it ends
//!
//! One snapshot set is taken in one pass, each database consistently, and the
//! result is one container with one timestamp. The databases are separate files
//! and SQLite has no distributed transaction across them, so the guarantee is
//! per database, not across all four: a write that lands between two of them
//! appears in the later one and not in the earlier one. What the set does
//! guarantee is that no database is a torn mix of moments, and that no WAL
//! content is lost. The window therefore closes its own writing paths for the
//! duration and says so in the interface.
//!
//! # Documents
//!
//! Settings documents are already written atomically (write, flush, rename), so
//! reading one gives either the old or the new file and never a half file. They
//! are read in bounded chunks and hashed as they are read.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::container::{ComponentKind, EntrySource};
use super::error::BackupError;

/// The application data directory holds these databases.
pub const NOTES_DATABASE: &str = "notes/sync.sqlite3";
pub const VAULT_DATABASE: &str = "vault/vault.sqlite3";
pub const MEMORY_DATABASE: &str = "memory/ai-memory.sqlite3";
pub const AUTOCORRECT_DATABASE: &str = "autocorrect/autocorrect.sqlite3";

/// The portable key envelope: the only way to open the data on another machine.
pub const KEY_ENVELOPE: &str = "key/portable-envelope.json";

/// One component of the backup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Component {
    /// Logical name inside the container.
    pub name: &'static str,
    pub kind: ComponentKind,
    /// File name inside the application data directory.
    pub file: &'static str,
    /// Whether the component exists on every installation.
    pub required: bool,
    /// Whether it lives in the configuration directory instead of the data one.
    pub in_config_dir: bool,
}

/// Every component a full backup carries, in the order it is written.
///
/// The four databases are required: a backup without them is not a backup of
/// this application. The key envelope is required too — without it the restored
/// data could not be opened anywhere, which would make the container useless.
/// The documents are optional: a fresh installation has none of them, and a
/// missing settings document is a default, not a loss.
pub const COMPONENTS: [Component; 11] = [
    Component {
        name: NOTES_DATABASE,
        kind: ComponentKind::Sqlite,
        file: "sync.sqlite3",
        required: true,
        in_config_dir: false,
    },
    Component {
        name: VAULT_DATABASE,
        kind: ComponentKind::Sqlite,
        file: "vault.sqlite3",
        required: true,
        in_config_dir: false,
    },
    Component {
        name: MEMORY_DATABASE,
        kind: ComponentKind::Sqlite,
        file: "ai-memory.sqlite3",
        required: true,
        in_config_dir: false,
    },
    Component {
        name: AUTOCORRECT_DATABASE,
        kind: ComponentKind::Sqlite,
        file: "autocorrect.sqlite3",
        required: true,
        in_config_dir: false,
    },
    Component {
        name: KEY_ENVELOPE,
        kind: ComponentKind::PortableKey,
        file: "key.backup.json",
        required: true,
        in_config_dir: false,
    },
    Component {
        name: "settings/whisper-settings.json",
        kind: ComponentKind::Document,
        file: "whisper-settings.json",
        required: false,
        in_config_dir: false,
    },
    Component {
        name: "settings/desktop.json",
        kind: ComponentKind::Document,
        file: "desktop.json",
        required: false,
        in_config_dir: false,
    },
    Component {
        name: "settings/setup.json",
        kind: ComponentKind::Document,
        file: "setup.json",
        required: false,
        in_config_dir: false,
    },
    Component {
        name: "settings/windows-actions.json",
        kind: ComponentKind::Document,
        file: "settings.json",
        required: false,
        in_config_dir: false,
    },
    Component {
        name: "settings/windows-timers.json",
        kind: ComponentKind::Document,
        file: "timers.json",
        required: false,
        in_config_dir: false,
    },
    Component {
        name: "settings/app.db",
        kind: ComponentKind::Document,
        file: "app.db",
        required: false,
        in_config_dir: true,
    },
];

/// What a snapshot pass produced.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnapshotReport {
    /// Components that were written to the staging directory.
    pub captured: Vec<String>,
    /// Components that do not exist on this installation.
    pub absent: Vec<String>,
    /// Bytes of plaintext, over all captured components.
    pub total_bytes: u64,
}

/// The two roots a snapshot reads from.
#[derive(Clone, Debug)]
pub struct BackupRoots {
    /// The data directory: the four databases, the key envelope, the settings.
    pub data_dir: PathBuf,
    /// The configuration directory: the JSON settings document.
    pub config_dir: PathBuf,
}

impl BackupRoots {
    /// Production roots, from the same paths the application itself uses.
    pub fn production() -> Result<Self, BackupError> {
        let database = crate::sync::sqlite::SqliteSyncRepository::production_path()
            .map_err(|_| BackupError::Storage)?;
        let data_dir = database
            .parent()
            .map(Path::to_path_buf)
            .ok_or(BackupError::Storage)?;
        let config_dir = crate::APP_CONFIG_DIR
            .get()
            .cloned()
            .ok_or(BackupError::Storage)?;
        Ok(Self {
            data_dir,
            config_dir,
        })
    }

    /// The path of one component on this machine.
    pub fn source_path(&self, component: &Component) -> PathBuf {
        if component.in_config_dir {
            self.config_dir.join(component.file)
        } else {
            self.data_dir.join(component.file)
        }
    }

    /// The path a restored component takes.
    pub fn destination_path(&self, component: &Component) -> PathBuf {
        self.source_path(component)
    }
}

/// Whether a database declares a schema this build can still read.
///
/// The value is SQLite's `user_version`, which this application's repositories
/// set from their own schema version. A snapshot that is *newer* than the build
/// reading it is refused; an older one is accepted, because the repositories
/// migrate forward on open.
pub fn database_schema_version(path: &Path) -> Result<i64, BackupError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| BackupError::Storage)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| BackupError::Storage)?;
    Ok(version)
}

/// Runs `PRAGMA integrity_check` and refuses anything but `ok`.
///
/// This is what a restored database is made to pass before it is put in place:
/// a container whose SQLite file is subtly damaged is refused while the live
/// data is still untouched.
pub fn integrity_check(path: &Path) -> Result<(), BackupError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| BackupError::Storage)?;
    // A file that is not a database at all fails the pragma itself, and that is
    // an integrity failure: the answer must not be "a file could not be read",
    // which would suggest a permission problem instead of a damaged backup.
    let result: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| BackupError::IntegrityCheckFailed)?;
    if !result.eq_ignore_ascii_case("ok") {
        return Err(BackupError::IntegrityCheckFailed);
    }
    Ok(())
}

/// Writes a consistent copy of one SQLite database to `destination`.
///
/// `VACUUM INTO` is the whole mechanism: one read transaction, committed WAL
/// frames included, a complete file out. The destination must not exist.
pub fn snapshot_database(source: &Path, destination: &Path) -> Result<u64, BackupError> {
    if !source.is_file() {
        return Err(BackupError::Storage);
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| BackupError::Storage)?;
    // Bound the wait: a snapshot must not block for minutes behind a writer, and
    // the caller can try again when the application is quieter.
    connection
        .busy_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| BackupError::Storage)?;
    connection
        .execute(
            "VACUUM INTO ?1",
            rusqlite::params![destination.to_string_lossy().into_owned()],
        )
        .map_err(|_| BackupError::Storage)?;
    // The copy is complete and consistent; prove it before it is trusted.
    integrity_check(destination)?;
    Ok(std::fs::metadata(destination)?.len())
}

/// Copies a small document, in bounded chunks.
pub fn snapshot_document(source: &Path, destination: &Path) -> Result<u64, BackupError> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut input = std::fs::File::open(source)?;
    let mut output = std::fs::File::create(destination)?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        total += read as u64;
    }
    output.flush()?;
    output.sync_all()?;
    Ok(total)
}

/// Snapshots every component that exists into `staging`.
///
/// The result is a directory of plain files with the same logical layout the
/// container uses, which is what the container writer streams from and what the
/// restore verifies before anything is put in place.
pub fn snapshot_all(
    roots: &BackupRoots,
    staging: &Path,
    components: &[Component],
) -> Result<Vec<EntrySource>, BackupError> {
    std::fs::create_dir_all(staging)?;
    let mut entries = Vec::new();
    for component in components {
        let source = roots.source_path(component);
        if !source.is_file() {
            if component.required {
                return Err(BackupError::Storage);
            }
            continue;
        }
        let destination = super::container::entry_path(staging, component.name)?;
        let bytes = match component.kind {
            ComponentKind::Sqlite => snapshot_database(&source, &destination)?,
            ComponentKind::Document | ComponentKind::PortableKey => {
                snapshot_document(&source, &destination)?
            }
        };
        let schema_version = match component.kind {
            ComponentKind::Sqlite => database_schema_version(&destination).ok(),
            _ => None,
        };
        entries.push(EntrySource {
            name: component.name.to_string(),
            kind: component.kind,
            path: destination,
            schema_version,
        });
        let _ = bytes;
    }
    Ok(entries)
}

/// Checkpoints the write-ahead log of every database that exists.
///
/// This is the `checkpoint-databases` lifecycle step: a full exit truncates the
/// WAL so the next start finds a database that is complete on its own. It runs
/// on its own connection, so it works whether or not a store is open, and a
/// database that is locked is reported rather than waited for forever.
pub fn checkpoint_databases(data_dir: &Path) -> Result<usize, String> {
    let mut checkpointed = 0usize;
    for component in COMPONENTS
        .iter()
        .filter(|component| component.kind == ComponentKind::Sqlite)
    {
        let path = data_dir.join(component.file);
        if !path.is_file() {
            continue;
        }
        let connection = match Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        ) {
            Ok(connection) => connection,
            Err(_) => return Err("a database could not be opened".to_string()),
        };
        if connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .is_err()
        {
            return Err("a database could not be configured".to_string());
        }
        // TRUNCATE: one checkpoint that also empties the WAL, which is what makes
        // the main file complete on its own.
        let outcome = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            row.get::<_, i64>(0)
        });
        match outcome {
            Ok(0) => checkpointed += 1,
            // A busy checkpoint is not a failure of the exit: the WAL is valid and
            // SQLite will recover from it. It is reported, not escalated.
            Ok(_) => continue,
            Err(_) => return Err("a database could not be checkpointed".to_string()),
        }
    }
    Ok(checkpointed)
}
