//! Encrypted local AI memory.
//!
//! ```text
//! conversations + messages + summaries + facts   (payloads, encrypted)
//!        |
//! MemoryStore  ->  SyncEngine  ->  SqliteSyncRepository  ->  ai-memory.sqlite3
//!        |                 |
//!        |                 `-- PurposeKeyProvider(JARVIS/ai-memory/v1)
//!        |
//!        +-- ContextBuilder   bounded, injection-resistant context
//!        `-- SecretFilter     keeps credentials out of memory
//! ```
//!
//! The memory reuses the application's existing storage stack: the same SQLite
//! repository, the same revision, cursor, conflict, and tombstone rules, the same
//! master key, and a **different derived key and database file** than notes and the
//! password vault. It never reads a vault record, never receives a vault handle,
//! and never stores a secret it was asked to keep out.
//!
//! What is stored, and what is not, is documented in `docs/AI_MEMORY.md`:
//!
//! * stored: the conversation header, the visible user question, the visible final
//!   answer, summaries, and facts the user created or approved;
//! * never stored: reasoning output, the system prompt, HTTP events, server stderr,
//!   passwords, keys, tokens, and vault content.

pub mod config;
pub mod context;
pub mod error;
pub mod model;
pub mod redaction;
pub mod session;
pub mod store;

pub use config::{
    linear_search_warning, load_settings, MemorySettings, DEFAULT_MAX_RECENT_MESSAGES,
    DEFAULT_MEMORY_TOKEN_BUDGET, DEFAULT_SUMMARY_KEEP_RECENT, DEFAULT_SUMMARY_TRIGGER_MESSAGES,
    MEMORY_SETTINGS_SCHEMA_VERSION, SETTINGS_KEY,
};
pub use context::{
    build_context, estimate_tokens, now, plan_budget, rank_facts, BudgetRequest, ContextBudget,
    ContextPlan, ContextRequest, ContextSection, ContextWarning, UsedFact, SAFETY_RESERVE_TOKENS,
};
pub use error::{MemoryError, SecretKind};
pub use model::{
    CandidateState, ConversationDetails, ConversationDraft, ConversationPayload, ConversationQuery,
    ConversationView, ExportedMemoryRecord, FactDraft, FactPayload, FactQuery, FactView,
    MemoryCategory, MemoryExportEnvelope, MemoryScope, MemorySource, MemoryStats, MessagePage,
    MessagePayload, MessageRole, MessageStatus, MessageView, SummaryPayload, SummaryView,
    CONVERSATION_PAYLOAD_SCHEMA_VERSION, DEFAULT_PAGE_SIZE, FACT_PAYLOAD_SCHEMA_VERSION,
    MAX_FACT_CHARS, MAX_MESSAGE_BYTES, MAX_PAGE_SIZE, MAX_SUMMARY_BYTES,
    MESSAGE_PAYLOAD_SCHEMA_VERSION, SUMMARY_PAYLOAD_SCHEMA_VERSION,
};
pub use redaction::{scan as scan_for_secrets, SecretFinding, SecretScan};
pub use session::{
    database_has_records, database_path, open_store as open_memory_store, MemoryStatus,
    AI_MEMORY_DB_FILE,
};
pub use store::{
    EncryptedMemoryStore, MemoryConflictResolution, MemoryConflictView, MemoryImportOutcome,
    MemoryStore,
};
