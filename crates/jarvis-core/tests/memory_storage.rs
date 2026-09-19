//! End-to-end storage tests for the encrypted AI memory on a real SQLite file.
//!
//! The unit tests in `crates/jarvis-core/src/memory/tests.rs` use the in-memory
//! repository, so they prove the record, revision, conflict, and payload rules but
//! never touch a database file. This file proves the same feature against
//! `ai-memory.sqlite3` on disk, where write-ahead logging, checkpoints, a second
//! connection, a foreign key, and a missing key file are real:
//!
//! * a conversation, its messages, its summary, and a fact survive a close and a
//!   reopen under the same master key, with the same entity revisions;
//! * notes (`sync.sqlite3`), the password vault (`vault.sqlite3`), and the AI
//!   memory (`ai-memory.sqlite3`) are three files, three derived keys, and three
//!   journals, and none of them can read another's record;
//! * no stored text reaches the database, its write-ahead log, its shared-memory
//!   file, or the journal tables in the clear;
//! * locking a `VaultSession` drops the memory key and the decrypted cache, and
//!   the session still knows that undecypherable memory is on disk;
//! * a foreign master key, a tampered payload, and a database locked by another
//!   writer all become controlled `MemoryError` values instead of panics, hangs,
//!   or a silently empty listing.
//!
//! Every value is fictional and prefixed `FICTIONAL_`. No test opens a network
//! connection, starts a model, or sleeps; every loop is bounded by a page limit
//! and every database has a short busy timeout, so the whole binary stays fast.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use jarvis_core::ai::{ChatRole, Persona};
use jarvis_core::memory::{
    database_has_records, database_path, open_memory_store, summarize_conversation,
    CandidateRequest, CandidateSuggestion, ConversationQuery, ConversationView,
    EncryptedMemoryStore, ExportedMemoryRecord, FactDraft, FactQuery, MemoryCategory, MemoryError,
    MemoryScope, MemorySettings, MemoryStore, MessageStatus, SummaryProvider, SummaryRequest,
    AI_MEMORY_DB_FILE,
};
use jarvis_core::notes::{NoteDraft, StorageState};
use jarvis_core::sync::crypto::{random_master_key, KeyPurpose, MasterKey, PurposeKeyProvider};
use jarvis_core::sync::sqlite::SqliteSyncRepository;
use jarvis_core::sync::{
    CryptoProvider, DeviceId, PayloadContext, SyncCursor, SyncEntityType, SyncError,
};
use jarvis_core::vault::{VaultItemDraft, VaultSession, VAULT_DB_FILE};
use rusqlite::{params, Connection};
use tempfile::tempdir;
use uuid::Uuid;

const PASSWORD: &str = "FICTIONAL_MASTER_PASSWORD";
const WRONG_PASSWORD: &str = "FICTIONAL_WRONG_PASSWORD";

const TITLE: &str = "FICTIONAL_CONVERSATION_TITLE";
const QUESTION: &str = "FICTIONAL_USER_QUESTION";
const ANSWER: &str = "FICTIONAL_ASSISTANT_ANSWER";
const SUMMARY: &str = "FICTIONAL_CONVERSATION_SUMMARY";
const FACT: &str = "FICTIONAL_STORED_FACT";
const NOTE_TITLE: &str = "FICTIONAL_NOTE_TITLE";
const NOTE_BODY: &str = "FICTIONAL_NOTE_BODY";
const ITEM_NAME: &str = "FICTIONAL_VAULT_ITEM";
const ITEM_PASSWORD: &str = "FICTIONAL_VAULT_PASSWORD";
const MALFORMED_PAYLOAD: &[u8] = b"FICTIONAL_MALFORMED_PAYLOAD";

/// Text that must only ever exist inside an encrypted payload.
const CONTENT_MARKERS: [&str; 5] = [TITLE, QUESTION, ANSWER, SUMMARY, FACT];

/// A conversation large enough to page through several times, small enough that
/// the whole binary stays far below a minute.
const LARGE_MESSAGES: usize = 240;
/// Page size used while walking the large conversation.
const PAGE_SIZE: usize = 50;
/// Busy timeout for the "locked by another writer" test. Short on purpose: the
/// test must fail fast rather than wait out the production five seconds.
const SHORT_BUSY_TIMEOUT: Duration = Duration::from_millis(25);

// ---------------------------------------------------------------------- fixtures

/// A fresh random master key. It is never rendered, not even by `Debug`.
fn master_key() -> MasterKey {
    random_master_key().expect("a random master key")
}

/// The memory provider: a key derived for `JARVIS/ai-memory/v1`, never the master
/// key itself, exactly as the session builds it.
fn memory_provider(master: &MasterKey) -> PurposeKeyProvider {
    PurposeKeyProvider::derive(master, KeyPurpose::AiMemory).expect("derive the ai-memory key")
}

/// A device identifier. It is technical metadata stored in the clear, not content.
fn device_id() -> DeviceId {
    DeviceId::new("FICTIONAL_DEVICE_ID").expect("a valid device identifier")
}

/// Opens the real memory database with the derived memory key.
fn open_memory(database: &Path, master: &MasterKey) -> EncryptedMemoryStore {
    open_memory_store(database, memory_provider(master), device_id())
        .expect("open the memory store")
}

/// A manual fact, the way the interface creates one.
fn fact_draft() -> FactDraft {
    FactDraft::new(MemoryScope::Jarvis, MemoryCategory::Project, FACT)
}

/// A note draft for the cross-database separation test.
fn note_draft() -> NoteDraft {
    NoteDraft {
        title: NOTE_TITLE.to_string(),
        body: NOTE_BODY.to_string(),
        folder_id: None,
        tags: Vec::new(),
    }
}

/// A vault item draft for the cross-database separation test.
fn vault_draft() -> VaultItemDraft {
    VaultItemDraft {
        name: ITEM_NAME.to_string(),
        username: ITEM_NAME.to_string(),
        password: ITEM_PASSWORD.to_string(),
        urls: Vec::new(),
        notes: String::new(),
        tags: Vec::new(),
        favorite: false,
    }
}

/// A message body that names its own position, so ordering can be checked from the
/// text alone and not only from the stored sequence number.
fn message_text(index: usize) -> String {
    format!("FICTIONAL_MESSAGE_{index:03}")
}

/// Writes a conversation with alternating messages and returns the conversation.
fn write_conversation(
    store: &mut EncryptedMemoryStore,
    title: &str,
    count: usize,
) -> ConversationView {
    let conversation = store
        .create_conversation(title, Persona::Jarvis)
        .expect("create a conversation");
    for index in 1..=count {
        let text = message_text(index);
        if index % 2 == 1 {
            store
                .append_user_message(conversation.id, &text)
                .expect("append a user message");
        } else {
            store
                .append_assistant_message(conversation.id, &text, MessageStatus::Completed, false)
                .expect("append an assistant message");
        }
    }
    store
        .conversation(conversation.id)
        .expect("read the conversation")
        .expect("the conversation exists")
}

/// A summarizer that never touches a model: it answers with one fixed fictional
/// sentence, so the summary-writing path can be exercised end to end.
struct ScriptedSummaryProvider;

impl SummaryProvider for ScriptedSummaryProvider {
    fn summarize(&self, _request: &SummaryRequest<'_>) -> Result<String, MemoryError> {
        Ok(SUMMARY.to_string())
    }

    fn suggest_candidates(
        &self,
        _request: &CandidateRequest<'_>,
    ) -> Result<Vec<CandidateSuggestion>, MemoryError> {
        // Candidate extraction is opt-in and irrelevant to storage: an empty answer
        // is a valid answer.
        Ok(Vec::new())
    }
}

// -------------------------------------------------------------- file-level helpers

/// Whether `needle` occurs anywhere in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// `path` with a SQLite sibling suffix (`-wal`, `-shm`) appended.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// The database plus the write-ahead log and shared-memory file, so a scan cannot
/// miss data that has not been checkpointed yet.
fn database_family_bytes(database: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(database).unwrap_or_default();
    bytes.extend(std::fs::read(sibling(database, "-wal")).unwrap_or_default());
    bytes.extend(std::fs::read(sibling(database, "-shm")).unwrap_or_default());
    bytes
}

/// Copies a database with its siblings, so a "the file was carried to another
/// machine" fixture cannot lose pages that still live in the write-ahead log.
fn copy_database_family(source: &Path, target: &Path) {
    std::fs::copy(source, target).expect("copy the database");
    for suffix in ["-wal", "-shm"] {
        let from = sibling(source, suffix);
        if from.is_file() {
            std::fs::copy(&from, sibling(target, suffix)).expect("copy a database sibling");
        }
    }
}

/// Every text column of the journal, metadata, and device tables, concatenated.
/// These are the only values the storage contract allows SQLite to keep in the
/// clear, so a hit here means plaintext leaked out of the encrypted payload.
fn journal_text(connection: &Connection) -> String {
    let mut text = String::new();
    for (table, column) in [
        ("sync_entities", "entity_id"),
        ("sync_entities", "entity_type"),
        ("sync_entities", "device_id"),
        ("sync_entities", "updated_at"),
        ("sync_entities", "last_operation_id"),
        ("sync_operations", "operation_id"),
        ("sync_operations", "entity_id"),
        ("sync_operations", "entity_type"),
        ("sync_operations", "device_id"),
        ("sync_operations", "operation_kind"),
        ("sync_operations", "updated_at"),
        ("sync_operations", "outcome"),
        ("sync_operations", "conflict_id"),
        ("sync_conflicts", "conflict_id"),
        ("sync_conflicts", "entity_id"),
        ("sync_conflicts", "entity_type"),
        ("sync_conflicts", "reason"),
        ("sync_conflicts", "detected_at"),
        ("sync_conflicts", "device_id"),
        ("sync_conflicts", "operation_id"),
        ("sync_metadata", "key"),
        ("sync_metadata", "value"),
        ("sync_devices", "device_id"),
    ] {
        let sql = format!("SELECT {column} FROM {table}");
        let mut statement = connection
            .prepare(&sql)
            .expect("prepare a journal text read");
        let mut rows = statement.query([]).expect("query a journal text column");
        while let Some(row) = rows.next().expect("read a journal text row") {
            if let Some(value) = row.get::<_, Option<String>>(0).expect("a text value") {
                text.push_str(&value);
                text.push('\n');
            }
        }
    }
    text
}

/// Every payload blob of the three journal tables.
fn journal_payloads(connection: &Connection) -> Vec<Vec<u8>> {
    let mut payloads = Vec::new();
    for sql in [
        "SELECT payload FROM sync_entities WHERE payload IS NOT NULL",
        "SELECT payload FROM sync_operations WHERE payload IS NOT NULL",
        "SELECT current_payload FROM sync_conflicts WHERE current_payload IS NOT NULL",
        "SELECT incoming_payload FROM sync_conflicts WHERE incoming_payload IS NOT NULL",
    ] {
        let mut statement = connection.prepare(sql).expect("prepare a payload read");
        let mut rows = statement.query([]).expect("query payload blobs");
        while let Some(row) = rows.next().expect("read a payload row") {
            if let Some(blob) = row.get::<_, Option<Vec<u8>>>(0).expect("a payload blob") {
                payloads.push(blob);
            }
        }
    }
    payloads
}

/// What the journal tables hold in the clear, read through SQL rather than through
/// the file bytes: the plaintext scan has to hit the tables themselves.
fn read_journal(database: &Path) -> (String, Vec<Vec<u8>>) {
    let connection = Connection::open(database).expect("open the memory database for reading");
    (journal_text(&connection), journal_payloads(&connection))
}

/// Flips one byte inside a stored ciphertext blob. It simulates on-disk corruption
/// without changing the schema, the revision, or the tombstone flag, so the record
/// still looks perfectly valid to the storage layer until it is decrypted.
fn tamper_payload(database: &Path, entity_id: Uuid) {
    let connection = Connection::open(database).expect("open the memory database");
    let mut payload: Vec<u8> = connection
        .query_row(
            "SELECT payload FROM sync_entities WHERE entity_id = ?1",
            [entity_id.to_string()],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .expect("read the stored payload")
        .expect("the record carries a payload");
    let last = payload.len() - 1;
    payload[last] ^= 0x5a;
    connection
        .execute(
            "UPDATE sync_entities SET payload = ?1 WHERE entity_id = ?2",
            params![payload, entity_id.to_string()],
        )
        .expect("write the tampered payload");
}

// ------------------------------------------------------- 1. real file lifecycle

#[test]
fn a_memory_database_survives_a_reopen_with_the_same_key() {
    let directory = tempdir().unwrap();
    let database = database_path(directory.path());
    let master = master_key();
    assert!(!database.is_file(), "the database starts absent");

    let mut store = open_memory(&database, &master);
    let conversation = write_conversation(&mut store, TITLE, 2);
    let conversation_revision = conversation.revision;
    let page = store.list_messages(conversation.id, 0, 10).unwrap();
    let summary = store
        .save_summary(conversation.id, SUMMARY, page.messages[1].id, 2)
        .unwrap();
    let fact = store.create_fact(&fact_draft()).unwrap();

    // The text really is readable through the store before it is closed, so the
    // reopen below cannot pass by reading an empty database.
    assert_eq!(conversation.title, TITLE);
    assert_eq!(page.messages[0].content, message_text(1));
    assert_eq!(page.messages[1].content, message_text(2));
    assert_eq!(summary.summary, SUMMARY);
    assert_eq!(fact.content, FACT);
    assert!(store.holds_decrypted_entries());
    store.into_repository().checkpoint_and_close().unwrap();
    assert!(database.is_file(), "closing must leave a database file");
    // Detecting stored memory needs no key at all.
    assert!(database_has_records(&database).unwrap());

    // Same master key: the deterministic derivation produces the same memory key.
    let mut reopened = open_memory(&database, &master);
    let reopened_conversation = reopened
        .conversation(conversation.id)
        .unwrap()
        .expect("the conversation survives the reopen");
    assert_eq!(reopened_conversation.title, TITLE);
    assert_eq!(reopened_conversation.revision, conversation_revision);
    assert_eq!(reopened_conversation.message_count, 2);

    let reopened_page = reopened.list_messages(conversation.id, 0, 10).unwrap();
    assert_eq!(reopened_page.total, 2);
    assert_eq!(reopened_page.messages.len(), page.messages.len());
    for (stored, read_back) in page.messages.iter().zip(&reopened_page.messages) {
        assert_eq!(read_back.id, stored.id);
        assert_eq!(read_back.revision, stored.revision);
        assert_eq!(read_back.content, stored.content);
        assert_eq!(read_back.role, stored.role);
        assert_eq!(read_back.status, stored.status);
        assert_eq!(read_back.sequence, stored.sequence);
        assert_eq!(read_back.created_at, stored.created_at);
    }

    let reopened_summary = reopened.summary(conversation.id).unwrap().unwrap();
    assert_eq!(reopened_summary.summary, SUMMARY);
    assert_eq!(reopened_summary.revision, summary.revision);
    assert_eq!(reopened_summary.covered_messages, 2);
    assert!(!reopened_summary.stale);

    let reopened_fact = reopened.fact(fact.id).unwrap().unwrap();
    assert_eq!(reopened_fact.content, FACT);
    assert_eq!(reopened_fact.revision, fact.revision);
    assert_eq!(reopened_fact.scope, MemoryScope::Jarvis);

    let stats = reopened.stats().unwrap();
    assert_eq!(stats.conversations, 1);
    assert_eq!(stats.messages, 2);
    assert_eq!(stats.facts, 1);
    assert_eq!(stats.unreadable, 0);

    let history = reopened.chat_history(conversation.id).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].role, ChatRole::User);
    assert_eq!(history[1].role, ChatRole::Assistant);
}

#[test]
fn a_summary_written_through_the_summarizer_survives_a_reopen() {
    let directory = tempdir().unwrap();
    let database = database_path(directory.path());
    let master = master_key();

    // Only a model-free fixture is used: the summarizer must never be a hidden
    // dependency of storing memory.
    let provider = ScriptedSummaryProvider;
    let settings = MemorySettings {
        summary_trigger_messages: 4,
        summary_keep_recent: 2,
        ..MemorySettings::default()
    };

    let mut store = open_memory(&database, &master);
    let conversation = write_conversation(&mut store, TITLE, 6);
    let written = summarize_conversation(
        &mut store,
        &provider,
        &settings,
        Persona::Jarvis,
        conversation.id,
    )
    .unwrap();
    assert_eq!(written.as_deref(), Some(SUMMARY));

    let stored = store.summary(conversation.id).unwrap().unwrap();
    assert_eq!(stored.summary, SUMMARY);
    assert_eq!(stored.covered_messages, 4, "the newest two stay verbatim");
    assert!(!stored.stale);
    assert_eq!(store.uncovered_messages(conversation.id).unwrap().len(), 2);
    let revision = stored.revision;
    store.into_repository().checkpoint_and_close().unwrap();

    let mut reopened = open_memory(&database, &master);
    let summary = reopened.summary(conversation.id).unwrap().unwrap();
    assert_eq!(summary.summary, SUMMARY);
    assert_eq!(summary.revision, revision);
    assert_eq!(summary.covered_messages, 4);
    assert_eq!(
        reopened.uncovered_messages(conversation.id).unwrap().len(),
        2
    );
}

// ------------------------------------------------------- 2. database separation

#[test]
fn memory_notes_and_vault_live_in_separate_files_with_separate_keys() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();

    // One record per feature, all written through the same unlocked session.
    let conversation = session
        .with_memory(|store| store.create_conversation(TITLE, Persona::Jarvis))
        .unwrap();
    let note_id = session
        .with_storage(|notes| notes.with_store(|store| Ok(store.create_note(&note_draft())?.id)))
        .unwrap();
    let item = session
        .with_store(|store| store.create_item(&vault_draft()))
        .unwrap();

    let notes_database = session.paths().database.clone();
    let vault_database = session.database_path().to_path_buf();
    let memory_database = session.memory_database_path().to_path_buf();
    assert_eq!(notes_database.file_name().unwrap(), "sync.sqlite3");
    assert_eq!(vault_database.file_name().unwrap(), VAULT_DB_FILE);
    assert_eq!(memory_database.file_name().unwrap(), AI_MEMORY_DB_FILE);
    assert_eq!(memory_database, database_path(directory.path()));
    assert_ne!(notes_database, vault_database);
    assert_ne!(notes_database, memory_database);
    assert_ne!(vault_database, memory_database);
    assert!(notes_database.is_file() && vault_database.is_file() && memory_database.is_file());

    // A record written by one feature is not readable through another, even with
    // the identifier in hand: the journals are separate files to begin with.
    assert!(SqliteSyncRepository::open(&memory_database)
        .unwrap()
        .record(conversation.id)
        .unwrap()
        .is_some());
    assert!(
        SqliteSyncRepository::open(&notes_database)
            .unwrap()
            .record(conversation.id)
            .unwrap()
            .is_none(),
        "the notes database must not hold a memory conversation"
    );
    assert!(
        SqliteSyncRepository::open(&vault_database)
            .unwrap()
            .record(note_id)
            .unwrap()
            .is_none(),
        "the vault database must not hold a note"
    );
    assert!(session
        .with_store(|store| store.get_item(conversation.id))
        .unwrap()
        .is_none());
    assert!(session
        .with_memory(|store| store.conversation(item.id))
        .unwrap()
        .is_none());
    assert!(session
        .with_storage(
            |notes| notes.with_store(|store| Ok(store.get_note(conversation.id)?.is_none()))
        )
        .unwrap());

    // The memory journal only ever names AI-memory entity types, and it never names
    // a note or a vault record.
    let repository = SqliteSyncRepository::open(&memory_database).unwrap();
    let page = repository.page_after(SyncCursor(0), 100).unwrap();
    assert!(!page.operations.is_empty());
    assert!(
        page.operations
            .iter()
            .all(|operation| operation.mutation.entity_type.is_ai_memory()),
        "the memory journal must only carry AI-memory entity types"
    );
    let memory_bytes = database_family_bytes(&memory_database);
    assert!(contains(&memory_bytes, b"ai_memory_conversation"));
    assert!(!contains(&memory_bytes, b"vault_record"));
    assert!(!contains(&memory_bytes, b"note_folder"));
    let notes_bytes = database_family_bytes(&notes_database);
    assert!(contains(&notes_bytes, b"note"));
    assert!(!contains(&notes_bytes, b"ai_memory_conversation"));

    // Three purposes, three keys: a ciphertext written by one provider cannot be
    // read by another, even though all three come from the same master key.
    let master = master_key();
    let notes_provider = PurposeKeyProvider::derive(&master, KeyPurpose::Notes).unwrap();
    let vault_provider = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();
    let memory_provider = PurposeKeyProvider::derive(&master, KeyPurpose::AiMemory).unwrap();
    assert_eq!(notes_provider.purpose(), KeyPurpose::Notes);
    assert_eq!(vault_provider.purpose(), KeyPurpose::Vault);
    assert_eq!(memory_provider.purpose(), KeyPurpose::AiMemory);
    let context = PayloadContext::new(SyncEntityType::AiMemoryFact, Uuid::new_v4());
    let ciphertext = memory_provider.encrypt(&context, FACT.as_bytes()).unwrap();
    assert_eq!(
        memory_provider.decrypt(&context, &ciphertext).unwrap(),
        FACT.as_bytes()
    );
    assert!(
        notes_provider.decrypt(&context, &ciphertext).is_err(),
        "the notes key must not open a memory payload"
    );
    assert!(
        vault_provider.decrypt(&context, &ciphertext).is_err(),
        "the vault key must not open a memory payload"
    );
}

// ------------------------------------------------- 3. no plaintext at rest

#[test]
fn no_memory_plaintext_reaches_the_database_the_wal_or_the_journal() {
    let directory = tempdir().unwrap();
    let database = database_path(directory.path());
    let master = master_key();

    let mut store = open_memory(&database, &master);
    let conversation = write_conversation(&mut store, TITLE, 2);
    let messages = store
        .list_messages(conversation.id, 0, 10)
        .unwrap()
        .messages;
    store
        .save_summary(conversation.id, SUMMARY, messages[1].id, 2)
        .unwrap();
    store.create_fact(&fact_draft()).unwrap();
    // Prove the marker really is stored, so the scan below cannot pass trivially.
    assert_eq!(
        store.conversation(conversation.id).unwrap().unwrap().title,
        TITLE
    );
    store.into_repository().checkpoint_and_close().unwrap();

    // The main file must be authoritative after a truncating checkpoint, but the
    // siblings are scanned too in case the platform keeps them around.
    let family = database_family_bytes(&database);
    assert!(!family.is_empty(), "the memory database must exist");
    for marker in CONTENT_MARKERS {
        assert!(
            !contains(&family, marker.as_bytes()),
            "{marker} reached the memory database or one of its sibling files"
        );
    }

    // The same claim at the SQL level: the journal tables hold ciphertext and
    // technical metadata only.
    let (text, payloads) = read_journal(&database);
    assert!(
        payloads.len() >= 4,
        "the journal must actually hold encrypted payloads for this check to mean anything"
    );
    assert!(text.contains("ai_memory_conversation"));
    for marker in CONTENT_MARKERS {
        assert!(
            !text.contains(marker),
            "{marker} reached a text column of a journal table"
        );
        assert!(
            !payloads
                .iter()
                .any(|payload| contains(payload, marker.as_bytes())),
            "{marker} is readable inside a stored payload"
        );
    }
}

// ------------------------------------------------------------ 4. session lifecycle

#[test]
fn a_session_locks_memory_drops_its_keys_and_finds_the_data_again() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    assert!(session.initialize(PASSWORD).unwrap().is_unlocked());

    let conversation = session
        .with_memory(|store| store.create_conversation(TITLE, Persona::Jarvis))
        .unwrap();
    session
        .with_memory(|store| {
            store.append_user_message(conversation.id, QUESTION)?;
            store.append_assistant_message(
                conversation.id,
                ANSWER,
                MessageStatus::Completed,
                false,
            )?;
            Ok(())
        })
        .unwrap();
    let fact = session
        .with_memory(|store| store.create_fact(&fact_draft()))
        .unwrap();
    // Appending a message touches the conversation, so the header revision is read
    // after the messages are written.
    let stored_conversation = session
        .with_memory(|store| store.conversation(conversation.id))
        .unwrap()
        .expect("the conversation exists");
    assert_eq!(stored_conversation.message_count, 2);
    let conversation_revision = stored_conversation.revision;
    assert!(conversation_revision > conversation.revision);
    // While unlocked the decrypted cache really is held in memory.
    assert!(session
        .with_memory(|store| Ok(store.holds_decrypted_entries()))
        .unwrap());

    session.lock();
    assert!(!session.is_unlocked());
    // The derived memory key and the decrypted cache are gone with the master key.
    assert_eq!(
        session.memory_store().err().unwrap(),
        MemoryError::StorageLocked
    );
    assert_eq!(
        session.with_memory(|store| store.stats()).err().unwrap(),
        MemoryError::StorageLocked
    );

    // Locked, nothing is decrypted, but the interface still learns that memory
    // exists on disk.
    let locked = session.memory_status(MemorySettings::default()).unwrap();
    assert!(!locked.unlocked);
    assert!(locked.has_stored_data);
    assert_eq!(locked.stats.conversations, 0);
    assert_eq!(locked.stats.messages, 0);
    assert_eq!(locked.stats.facts, 0);
    assert!(!locked.is_active());
    assert!(session.has_memory_records());

    // A wrong master password changes nothing and leaves the memory key absent.
    assert!(session.unlock_with_password(WRONG_PASSWORD).is_err());
    assert!(!session.is_unlocked());
    assert_eq!(
        session.memory_store().err().unwrap(),
        MemoryError::StorageLocked
    );

    // Unlocking again finds the same records with the same revisions.
    assert!(session
        .unlock_with_password(PASSWORD)
        .unwrap()
        .is_unlocked());
    let reopened = session
        .with_memory(|store| store.conversation(conversation.id))
        .unwrap()
        .expect("the conversation is still there");
    assert_eq!(reopened.title, TITLE);
    assert_eq!(reopened.revision, conversation_revision);
    assert_eq!(reopened.message_count, 2);

    let history = session
        .with_memory(|store| store.chat_history(conversation.id))
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, QUESTION);
    assert_eq!(history[1].content, ANSWER);

    let reopened_fact = session
        .with_memory(|store| store.fact(fact.id))
        .unwrap()
        .expect("the fact is still there");
    assert_eq!(reopened_fact.content, FACT);
    assert_eq!(reopened_fact.revision, fact.revision);
}

#[test]
fn storage_status_reports_key_missing_when_only_memory_holds_records() {
    let source = tempdir().unwrap();
    let mut session = VaultSession::open(source.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    let conversation = session
        .with_memory(|store| store.create_conversation(TITLE, Persona::Jarvis))
        .unwrap();
    session.lock();
    drop(session);

    // A new machine: the memory database is carried over and no key file is.
    let target = tempdir().unwrap();
    let target_memory = database_path(target.path());
    copy_database_family(&source.path().join(AI_MEMORY_DB_FILE), &target_memory);
    assert!(target_memory.is_file());
    assert!(!target.path().join("key.backup.json").is_file());
    assert!(!target.path().join("sync.sqlite3").is_file());

    let mut fresh = VaultSession::open(target.path()).unwrap();
    let storage = fresh.storage_status().unwrap();
    assert_eq!(
        storage.state,
        StorageState::KeyMissing,
        "data exists but no key file remains"
    );
    assert!(storage.has_stored_data);
    assert!(!storage.is_unlocked());

    // The memory gate agrees, and refuses to produce anything.
    let status = fresh.memory_status(MemorySettings::default()).unwrap();
    assert!(!status.unlocked);
    assert!(status.has_stored_data);
    assert_eq!(status.stats.conversations, 0);
    assert!(fresh
        .with_memory(|store| store.conversation(conversation.id))
        .is_err());
}

// --------------------------------------------------------------- 5. wrong key

#[test]
fn a_different_master_key_reports_every_record_as_unreadable() {
    let directory = tempdir().unwrap();
    let database = database_path(directory.path());
    let owner = master_key();

    let mut store = open_memory(&database, &owner);
    let conversation = write_conversation(&mut store, TITLE, 1);
    store.create_fact(&fact_draft()).unwrap();
    store.into_repository().checkpoint_and_close().unwrap();

    // Same database, a different master key: nothing decrypts, and nothing is
    // silently presented as "no memory".
    let mut foreign = open_memory(&database, &master_key());
    assert!(database_has_records(&database).unwrap());
    let stats = foreign.stats().unwrap();
    assert_eq!(stats.conversations, 0);
    assert_eq!(stats.messages, 0);
    assert_eq!(stats.facts, 0);
    assert_eq!(
        stats.unreadable, 3,
        "one conversation, one message, and one fact must be counted as unreadable"
    );
    assert!(foreign
        .list_conversations(&ConversationQuery::default())
        .unwrap()
        .is_empty());
    assert!(foreign
        .list_facts(&FactQuery::default())
        .unwrap()
        .is_empty());
    assert!(foreign
        .list_messages(conversation.id, 0, 10)
        .unwrap()
        .messages
        .is_empty());
    // Asking for the record by identifier is a miss, not a different error, but the
    // count above is what keeps the omission visible.
    assert!(foreign.conversation(conversation.id).unwrap().is_none());
    assert_eq!(foreign.stats().unwrap().unreadable, 3);
}

// -------------------------------------------------------------- 6. corruption

#[test]
fn a_tampered_or_malformed_payload_is_reported_unreadable_without_breaking_the_listing() {
    let directory = tempdir().unwrap();
    let database = database_path(directory.path());
    let master = master_key();

    let mut store = open_memory(&database, &master);
    let conversation = write_conversation(&mut store, TITLE, 6);
    let messages = store
        .list_messages(conversation.id, 0, 10)
        .unwrap()
        .messages;
    assert_eq!(messages.len(), 6);
    let tampered_id = messages[2].id;
    store.into_repository().checkpoint_and_close().unwrap();

    // Tamper with one stored payload; the record itself still looks valid.
    tamper_payload(&database, tampered_id);

    let mut reopened = open_memory(&database, &master);
    let stats = reopened.stats().unwrap();
    assert_eq!(
        stats.unreadable, 1,
        "the tampered record must be counted, not hidden"
    );
    assert_eq!(stats.messages, 5, "the healthy messages stay readable");
    assert_eq!(stats.conversations, 1);

    // The listing still works: the tampered message is simply absent.
    let page = reopened.list_messages(conversation.id, 0, 10).unwrap();
    assert_eq!(page.total, 5);
    assert!(!page
        .messages
        .iter()
        .any(|message| message.id == tampered_id));
    assert_eq!(page.messages[0].content, message_text(1));
    assert_eq!(page.messages[4].content, message_text(6));
    assert_eq!(reopened.chat_history(conversation.id).unwrap().len(), 5);
    assert_eq!(
        reopened
            .list_conversations(&ConversationQuery::default())
            .unwrap()
            .len(),
        1
    );

    // A record whose payload is not ciphertext at all is reported the same way,
    // and never becomes a fabricated entry.
    let malformed = ExportedMemoryRecord {
        entity_id: Uuid::new_v4(),
        entity_type: SyncEntityType::AiMemoryFact.as_str().to_string(),
        base_revision: 0,
        tombstone: false,
        payload: Some(MALFORMED_PAYLOAD.to_vec()),
    };
    let outcome = reopened.import_records(&[malformed]).unwrap();
    assert_eq!(outcome.applied, 1);
    let stats = reopened.stats().unwrap();
    assert_eq!(stats.unreadable, 2);
    assert_eq!(stats.facts, 0, "a malformed payload is not a fact");
    assert!(reopened.list_facts(&Default::default()).unwrap().is_empty());
}

// ------------------------------------------------------------- 7. busy database

#[test]
fn a_busy_database_fails_with_a_controlled_error_and_recovers() {
    let directory = tempdir().unwrap();
    let database = database_path(directory.path());
    let master = master_key();

    let mut store = open_memory(&database, &master);
    let conversation = write_conversation(&mut store, TITLE, 1);
    store.into_repository().checkpoint_and_close().unwrap();

    // A second connection with a very short busy timeout, exactly as a diagnostic
    // or a second window would open it.
    let repository =
        SqliteSyncRepository::open_with_busy_timeout(&database, SHORT_BUSY_TIMEOUT).unwrap();
    let mut second = MemoryStore::new(repository, memory_provider(&master), device_id());
    // Reading is never blocked in WAL mode.
    assert_eq!(second.stats().unwrap().conversations, 1);

    // A first connection now holds the write lock for the whole test body.
    let writer = Connection::open(&database).unwrap();
    writer.busy_timeout(SHORT_BUSY_TIMEOUT).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE;").unwrap();

    let started = Instant::now();
    let blocked = second.append_user_message(conversation.id, QUESTION);
    let elapsed = started.elapsed();
    assert_eq!(
        blocked.err(),
        Some(MemoryError::Storage(SyncError::StorageBusy)),
        "a locked database must surface as a controlled memory error"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the short busy timeout must not turn into a hang (took {elapsed:?})"
    );

    // Opening a third connection while the lock is held also fails cleanly rather
    // than waiting out the production timeout.
    let blocked_open = SqliteSyncRepository::open_with_busy_timeout(&database, SHORT_BUSY_TIMEOUT)
        .map_err(MemoryError::from);
    assert_eq!(
        blocked_open.err(),
        Some(MemoryError::Storage(SyncError::StorageBusy)),
        "opening a locked database must be a controlled memory error"
    );

    // Releasing the lock lets the same store write again: the failure was a lock,
    // not corruption.
    drop(writer);
    second
        .append_user_message(conversation.id, QUESTION)
        .expect("the write succeeds once the lock is gone");
    assert_eq!(second.stats().unwrap().messages, 2);
    assert!(second.into_repository().checkpoint_and_close().is_ok());
}

// -------------------------------------------------------- 8. large conversation

#[test]
fn a_large_conversation_stays_ordered_pages_and_stays_consistent() {
    let directory = tempdir().unwrap();
    let database = database_path(directory.path());
    let master = master_key();

    let mut store = open_memory(&database, &master);
    let conversation = write_conversation(&mut store, TITLE, LARGE_MESSAGES);
    assert_eq!(conversation.message_count, LARGE_MESSAGES);
    assert_eq!(store.stats().unwrap().messages, LARGE_MESSAGES);

    // Page through the conversation: every page keeps the written order, and the
    // bounded loop proves paging terminates.
    let mut sequences = Vec::with_capacity(LARGE_MESSAGES);
    let mut offset = 0usize;
    for _ in 0..=(LARGE_MESSAGES / PAGE_SIZE) {
        let page = store
            .list_messages(conversation.id, offset, PAGE_SIZE)
            .unwrap();
        assert_eq!(page.total, LARGE_MESSAGES);
        assert_eq!(page.offset, offset);
        if page.messages.is_empty() {
            break;
        }
        for message in &page.messages {
            assert_eq!(message.content, message_text(message.sequence as usize));
            sequences.push(message.sequence);
        }
        offset += page.messages.len();
    }
    assert_eq!(offset, LARGE_MESSAGES);
    assert_eq!(
        sequences,
        (1..=LARGE_MESSAGES as u64).collect::<Vec<u64>>(),
        "the stored sequence must be the conversation order, on every page"
    );

    // A window in the middle of the conversation is a contiguous slice.
    let window = store
        .list_messages(conversation.id, 123, 7)
        .unwrap()
        .messages;
    assert_eq!(window.len(), 7);
    assert_eq!(window[0].sequence, 124);
    assert_eq!(window[6].sequence, 130);

    // Without a summary every message is uncovered.
    assert_eq!(
        store.uncovered_messages(conversation.id).unwrap().len(),
        LARGE_MESSAGES
    );

    // A summary covering the first hundred messages leaves the rest verbatim.
    let boundary = store
        .list_messages(conversation.id, 99, 1)
        .unwrap()
        .messages[0]
        .id;
    store
        .save_summary(conversation.id, SUMMARY, boundary, 100)
        .unwrap();
    let uncovered = store.uncovered_messages(conversation.id).unwrap();
    assert_eq!(uncovered.len(), LARGE_MESSAGES - 100);
    assert_eq!(uncovered[0].sequence, 101);

    let history = store.chat_history(conversation.id).unwrap();
    assert_eq!(history.len(), LARGE_MESSAGES);
    assert_eq!(history[0].role, ChatRole::User);
    assert_eq!(history[0].content, message_text(1));
    assert_eq!(
        history[LARGE_MESSAGES - 1].content,
        message_text(LARGE_MESSAGES)
    );
    let revision = conversation.revision;
    store.into_repository().checkpoint_and_close().unwrap();

    // Order, paging, and the summary boundary all survive the reopen.
    let mut reopened = open_memory(&database, &master);
    let page = reopened
        .list_messages(conversation.id, LARGE_MESSAGES - 40, PAGE_SIZE)
        .unwrap();
    assert_eq!(page.total, LARGE_MESSAGES);
    assert_eq!(page.messages.len(), 40);
    assert_eq!(page.messages[0].sequence, (LARGE_MESSAGES - 40 + 1) as u64);
    assert_eq!(page.messages[39].sequence, LARGE_MESSAGES as u64);
    assert_eq!(
        reopened.chat_history(conversation.id).unwrap().len(),
        LARGE_MESSAGES
    );
    assert_eq!(
        reopened.uncovered_messages(conversation.id).unwrap().len(),
        LARGE_MESSAGES - 100
    );
    let summary = reopened.summary(conversation.id).unwrap().unwrap();
    assert_eq!(summary.summary, SUMMARY);
    assert_eq!(summary.covered_messages, 100);
    assert_eq!(
        reopened
            .conversation(conversation.id)
            .unwrap()
            .unwrap()
            .revision,
        revision
    );
}

// ------------------------------------------- 9. locked or missing session storage

#[test]
fn locked_or_missing_storage_reports_zero_counts_and_fails_cleanly() {
    // Nothing on disk at all.
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    let status = session.memory_status(MemorySettings::default()).unwrap();
    assert!(!status.unlocked);
    assert!(!status.has_stored_data, "no memory database exists yet");
    assert_eq!(status.stats.conversations, 0);
    assert_eq!(status.stats.messages, 0);
    assert_eq!(status.stats.unreadable, 0);
    assert_eq!(
        session.memory_store().err().unwrap(),
        MemoryError::StorageLocked
    );
    assert_eq!(
        session.with_memory(|store| store.stats()).err().unwrap(),
        MemoryError::StorageLocked
    );
    assert!(!database_has_records(&database_path(directory.path())).unwrap());

    // Records on disk, storage locked: zero counts, but the file is reported.
    session.initialize(PASSWORD).unwrap();
    let conversation = session
        .with_memory(|store| store.create_conversation(TITLE, Persona::Jarvis))
        .unwrap();
    session
        .with_memory(|store| store.append_user_message(conversation.id, QUESTION))
        .unwrap();
    session.lock();

    assert!(database_has_records(&database_path(directory.path())).unwrap());
    let locked = session.memory_status(MemorySettings::default()).unwrap();
    assert!(!locked.unlocked);
    assert!(locked.has_stored_data);
    assert_eq!(locked.stats.conversations, 0);
    assert_eq!(locked.stats.messages, 0);
    assert!(!locked.is_active());
    assert!(locked.linear_search_cost_warning.is_none());
    assert_eq!(
        session.with_memory(|store| store.stats()).err().unwrap(),
        MemoryError::StorageLocked
    );

    // Unlocked but switched off: still no counts, and nothing is decrypted.
    session.unlock_with_password(PASSWORD).unwrap();
    let disabled = session
        .memory_status(MemorySettings {
            enabled: false,
            ..MemorySettings::default()
        })
        .unwrap();
    assert!(disabled.unlocked);
    assert!(disabled.has_stored_data);
    assert!(!disabled.is_active());
    assert_eq!(disabled.stats.conversations, 0);
    assert_eq!(disabled.stats.messages, 0);
    assert!(disabled.linear_search_cost_warning.is_none());

    // Switched back on, the same session finds both records again.
    let enabled = session.memory_status(MemorySettings::default()).unwrap();
    assert!(enabled.is_active());
    assert!(enabled.has_stored_data);
    assert_eq!(enabled.stats.conversations, 1);
    assert_eq!(enabled.stats.messages, 1);
    assert_eq!(enabled.stats.unreadable, 0);
}
