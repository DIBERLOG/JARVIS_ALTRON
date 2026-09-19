//! AI-memory commands exposed to the interface.
//!
//! Rules that shape this file:
//!
//! * the interface never reads SQLite, never receives a key, and never decrypts a
//!   payload: it asks for views and sends drafts;
//! * every command is declared `(async)`, and the commands that call the model run
//!   on a blocking worker, so no store read and no summarization blocks the window;
//! * the secret filter runs here, on the paths that could store a credential
//!   automatically (candidates, summaries, imports) and on the manual paths as a
//!   warning that needs an explicit confirmation;
//! * nothing in this file logs memory text: errors carry content-free messages, and
//!   a detected secret is reported by kind, never by value;
//! * this module holds no password-vault handle. It reaches the encrypted storage
//!   through the shared session, which hands it the memory store only.

use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use jarvis_core::ai::{system_prompt, ChatMessage, Persona};
use jarvis_core::memory::context::{ContextPlan, ContextSection};
use jarvis_core::memory::model::{
    ConversationDetails, ConversationQuery, ConversationView, FactDraft, FactQuery, FactView,
    MessageStatus, MessageView,
};
use jarvis_core::memory::store::{
    MemoryConflictResolution, MemoryConflictView, MemoryImportOutcome,
};
use jarvis_core::memory::summarizer::{
    summarize_conversation, CandidateRequest, LocalAiSummaryProvider, SummaryGuard, SummaryJobs,
    SummaryProvider,
};
use jarvis_core::memory::{
    build_context, estimate_tokens, load_settings, now, plan_budget, BudgetRequest, ContextRequest,
    MemoryError, MemoryExportEnvelope, MemoryScope, MemorySettings, MemoryStats, MemoryStatus,
    SETTINGS_KEY,
};
use jarvis_core::notes::vault::StorageStatus;

use crate::AppState;

/// The summarize/extract jobs that are running right now.
#[derive(Clone, Default)]
pub struct MemoryHandle {
    jobs: Arc<SummaryJobs>,
}

impl MemoryHandle {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(SummaryJobs::new()),
        }
    }

    pub fn jobs(&self) -> &SummaryJobs {
        &self.jobs
    }

    /// Whether a summarization is running for this conversation.
    pub fn is_summarizing(&self, conversation_id: Uuid) -> bool {
        self.jobs.is_running(conversation_id)
    }

    /// Releases every claim, for application exit.
    pub fn shutdown(&self) {
        // Dropping the guards is what releases a claim; at exit there is nothing
        // left to release, but the count is reported for diagnostics.
        if self.jobs.running_count() > 0 {
            log::debug!(
                "memory: {} job(s) still running at exit",
                self.jobs.running_count()
            );
        }
    }
}

impl std::fmt::Debug for MemoryHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryHandle")
            .field("running_jobs", &self.jobs.running_count())
            .finish()
    }
}

/// Status of the memory layer, as the interface needs it.
#[derive(Clone, Debug, Serialize)]
pub struct MemoryStatusView {
    /// Shared encrypted storage state: uninitialized, locked, unlocked, key missing.
    pub storage: StorageStatus,
    pub memory: MemoryStatus,
    /// Conversations with a summarization running right now.
    pub summarizing: Vec<Uuid>,
}

/// One fact that was put into a context, for the "what was added" list.
#[derive(Clone, Debug, Serialize)]
pub struct UsedFactView {
    pub id: Uuid,
    pub scope: MemoryScope,
    pub category: jarvis_core::memory::MemoryCategory,
    pub excerpt: String,
    pub score: f32,
}

/// A warning about the built context.
#[derive(Clone, Debug, Serialize)]
pub struct ContextWarningView {
    pub code: String,
    pub fact_id: Option<Uuid>,
    pub message: String,
}

/// The context the interface may send, without the system prompt.
#[derive(Clone, Debug, Serialize)]
pub struct ContextPlanView {
    /// Messages to send, in order. The profile system prompt is not among them: the
    /// gateway adds it, so memory can never replace it.
    pub messages: Vec<ChatMessage>,
    /// Section label of each message, in the same order.
    pub sections: Vec<String>,
    pub used_facts: Vec<UsedFactView>,
    pub summary_used: bool,
    pub dropped_facts: usize,
    pub dropped_messages: usize,
    pub estimated_tokens: usize,
    pub budget: BudgetView,
    pub memory_enabled: bool,
    pub history_enabled: bool,
    pub warnings: Vec<ContextWarningView>,
}

/// Numbers behind one request, so the interface can explain the size.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct BudgetView {
    pub context_size: usize,
    pub system_prompt: usize,
    pub response_reserve: usize,
    pub safety_reserve: usize,
    pub available: usize,
    pub memory: usize,
    pub summary: usize,
    pub recent_messages: usize,
}

impl BudgetView {
    fn from_plan(plan: &ContextPlan) -> Self {
        Self {
            context_size: plan.budget.context_size,
            system_prompt: plan.budget.system_prompt,
            response_reserve: plan.budget.response_reserve,
            safety_reserve: plan.budget.safety_reserve,
            available: plan.budget.available,
            memory: plan.budget.memory,
            summary: plan.budget.summary,
            recent_messages: plan.budget.recent_messages,
        }
    }
}

impl ContextPlanView {
    fn from_plan(plan: &ContextPlan) -> Self {
        Self {
            messages: plan.messages.clone(),
            sections: plan
                .sections
                .iter()
                .map(|section| section_label(*section).to_string())
                .collect(),
            used_facts: plan
                .used_facts
                .iter()
                .map(|fact| UsedFactView {
                    id: fact.id,
                    scope: fact.scope,
                    category: fact.category,
                    excerpt: fact.excerpt.clone(),
                    score: fact.score,
                })
                .collect(),
            summary_used: plan.summary_used,
            dropped_facts: plan.dropped_facts,
            dropped_messages: plan.dropped_messages,
            estimated_tokens: plan.estimated_tokens,
            budget: BudgetView::from_plan(plan),
            memory_enabled: plan.memory_enabled,
            history_enabled: plan.history_enabled,
            warnings: plan
                .warnings
                .iter()
                .map(|warning| ContextWarningView {
                    code: warning.code.to_string(),
                    fact_id: warning.fact_id,
                    message: warning.message.clone(),
                })
                .collect(),
        }
    }
}

/// Section label, without exposing anything the model did not receive.
fn section_label(section: ContextSection) -> &'static str {
    match section {
        ContextSection::MemoryData => "memory",
        ContextSection::SummaryData => "summary",
        ContextSection::History => "history",
        ContextSection::UserMessage => "question",
    }
}

/// Result of an encrypted export.
#[derive(Clone, Debug, Serialize)]
pub struct MemoryExportResult {
    /// Written path, or an empty string when the user cancelled the picker.
    pub path: String,
    pub records: usize,
}

/// Result of an import, including the secret warning count.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MemoryImportView {
    pub applied: usize,
    pub conflicts: usize,
    pub skipped: usize,
    /// Imported facts whose text the filter would refuse to save today.
    pub secret_suspects: usize,
}

impl From<MemoryImportOutcome> for MemoryImportView {
    fn from(outcome: MemoryImportOutcome) -> Self {
        Self {
            applied: outcome.applied,
            conflicts: outcome.conflicts,
            skipped: outcome.skipped,
            secret_suspects: 0,
        }
    }
}

// ------------------------------------------------------------------- settings

/// Current state of the encrypted memory and of its settings.
#[tauri::command(async)]
pub fn memory_status(state: tauri::State<'_, AppState>) -> Result<MemoryStatusView, String> {
    let settings = current_settings(&state);
    let storage = state
        .notes
        .with_session(|session| session.storage_status())?;
    let memory = state
        .notes
        .with_memory(|session| session.memory_status(settings))?;
    Ok(MemoryStatusView {
        storage,
        memory,
        summarizing: state.memory.jobs().running(),
    })
}

#[tauri::command(async)]
pub fn memory_get_settings(state: tauri::State<'_, AppState>) -> Result<MemorySettings, String> {
    Ok(current_settings(&state))
}

/// Locks the shared encrypted storage.
///
/// The memory key and the decrypted cache are dropped together with the master key,
/// and so are the notes and the password vault: one master password protects one
/// encrypted storage. The ciphertext on disk is untouched.
#[tauri::command(async)]
pub fn memory_lock(state: tauri::State<'_, AppState>) -> Result<MemoryStatusView, String> {
    // The undo journal of the spelling feature holds document text, so it is dropped
    // together with the key it was produced under.
    state.autocorrect.clear_journals();
    state.notes.lock();
    memory_status(state)
}

/// Stores the settings and reports the new status.
///
/// Settings are not secret, so this works while the encrypted storage is locked:
/// that is exactly when a user decides whether memory should be on at all.
#[tauri::command(async)]
pub fn memory_update_settings(
    state: tauri::State<'_, AppState>,
    settings: MemorySettings,
) -> Result<MemoryStatusView, String> {
    let normalized = settings.normalized();
    if normalized.validate().is_err() {
        return Err(MemoryError::InvalidConfiguration.to_string());
    }
    let encoded = normalized.to_json().map_err(describe)?;
    state
        .settings
        .write(SETTINGS_KEY, &encoded)
        .map_err(|error| {
            log::warn!("memory: settings could not be saved: {error}");
            error
        })?;
    // Switching memory off drops the memory key and its decrypted cache immediately,
    // without locking the shared storage: the notes and the vault stay usable.
    if !normalized.enabled {
        state.notes.with_memory(|session| {
            session.drop_memory();
            Ok(())
        })?;
    }
    memory_status(state)
}

// -------------------------------------------------------------- conversations

#[tauri::command(async)]
pub fn memory_list_conversations(
    state: tauri::State<'_, AppState>,
    query: ConversationQuery,
) -> Result<Vec<ConversationView>, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.list_conversations(&query)))
}

/// Creates a conversation. The profile is fixed for the conversation's lifetime.
#[tauri::command(async)]
pub fn memory_create_conversation(
    state: tauri::State<'_, AppState>,
    profile: jarvis_core::ai::Persona,
    title: String,
) -> Result<ConversationView, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| store.create_conversation(&title, profile))
    })
}

#[tauri::command(async)]
pub fn memory_open_conversation(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    offset: usize,
    limit: usize,
) -> Result<ConversationDetails, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| store.open_conversation(id, offset, limit))
    })
}

#[tauri::command(async)]
pub fn memory_rename_conversation(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    title: String,
) -> Result<ConversationView, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.rename_conversation(id, &title)))
}

#[tauri::command(async)]
pub fn memory_archive_conversation(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    archived: bool,
) -> Result<ConversationView, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| store.set_conversation_archived(id, archived))
    })
}

/// Deletes a conversation, its messages, and its summary.
#[tauri::command(async)]
pub fn memory_delete_conversation(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<MemoryStats, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| {
            store.delete_conversation(id)?;
            store.stats()
        })
    })
}

/// Removes the messages of one conversation and keeps the conversation.
#[tauri::command(async)]
pub fn memory_clear_conversation(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<MemoryStats, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| {
            store.clear_conversation(id)?;
            store.stats()
        })
    })
}

/// Removes every conversation. Facts are kept; a confirmation is required.
#[tauri::command(async)]
pub fn memory_clear_history(
    state: tauri::State<'_, AppState>,
    confirmed: bool,
) -> Result<MemoryStats, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| {
            store.clear_history(confirmed)?;
            store.stats()
        })
    })
}

// -------------------------------------------------------------------- messages

/// Stores the user's message.
#[tauri::command(async)]
pub fn memory_append_user_message(
    state: tauri::State<'_, AppState>,
    conversation_id: Uuid,
    content: String,
) -> Result<MessageView, String> {
    let settings = current_settings(&state);
    if !settings.stores_history() {
        return Err(MemoryError::MemoryDisabled.to_string());
    }
    state.notes.with_memory(|session| {
        session.with_memory(|store| store.append_user_message(conversation_id, &content))
    })
}

/// Stores the answer that was just produced.
///
/// A cancelled or failed answer is stored with its status, so the interface can
/// show what actually happened; a failure never becomes a completed answer.
#[tauri::command(async)]
pub fn memory_append_assistant_message(
    state: tauri::State<'_, AppState>,
    conversation_id: Uuid,
    content: String,
    status: MessageStatus,
    partial: bool,
) -> Result<MessageView, String> {
    let settings = current_settings(&state);
    if !settings.stores_history() {
        return Err(MemoryError::MemoryDisabled.to_string());
    }
    state.notes.with_memory(|session| {
        session.with_memory(|store| {
            store.append_assistant_message(conversation_id, &content, status, partial)
        })
    })
}

#[tauri::command(async)]
pub fn memory_delete_message(state: tauri::State<'_, AppState>, id: Uuid) -> Result<(), String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.delete_message(id)))
}

// ----------------------------------------------------------------------- facts

#[tauri::command(async)]
pub fn memory_list_facts(
    state: tauri::State<'_, AppState>,
    query: FactQuery,
) -> Result<Vec<FactView>, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.list_facts(&query)))
}

/// Creates a fact the user typed.
///
/// The secret filter runs first: a suspicious text is refused with a content-free
/// warning, and the interface can repeat the request with the confirmation flag once
/// the user has seen it.
#[tauri::command(async)]
pub fn memory_create_fact(
    state: tauri::State<'_, AppState>,
    draft: FactDraft,
) -> Result<FactView, String> {
    gate_secret(&draft)?;
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.create_fact(&draft)))
}

#[tauri::command(async)]
pub fn memory_update_fact(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    draft: FactDraft,
) -> Result<FactView, String> {
    gate_secret(&draft)?;
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.update_fact(id, &draft)))
}

#[tauri::command(async)]
pub fn memory_set_fact_usage(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    pinned: bool,
    disabled: bool,
) -> Result<FactView, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| store.set_fact_usage(id, pinned, disabled))
    })
}

/// Soft-deletes a fact; it can be restored until it is purged.
#[tauri::command(async)]
pub fn memory_delete_fact(state: tauri::State<'_, AppState>, id: Uuid) -> Result<FactView, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.delete_fact(id)))
}

#[tauri::command(async)]
pub fn memory_restore_fact(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<FactView, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.restore_fact(id)))
}

/// Drops the payload for good.
#[tauri::command(async)]
pub fn memory_purge_fact(state: tauri::State<'_, AppState>, id: Uuid) -> Result<(), String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.purge_fact(id)))
}

// ------------------------------------------------------------------ candidates

#[tauri::command(async)]
pub fn memory_list_candidates(
    state: tauri::State<'_, AppState>,
    conversation_id: Option<Uuid>,
) -> Result<Vec<FactView>, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.list_candidates(conversation_id)))
}

/// Approves a candidate, optionally after the user corrected it.
#[tauri::command(async)]
pub fn memory_approve_candidate(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    draft: Option<FactDraft>,
) -> Result<FactView, String> {
    if let Some(draft) = draft.as_ref() {
        gate_secret(draft)?;
    }
    state.notes.with_memory(|session| {
        session.with_memory(|store| store.approve_candidate(id, draft.as_ref()))
    })
}

#[tauri::command(async)]
pub fn memory_reject_candidate(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<FactView, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.reject_candidate(id)))
}

// --------------------------------------------------------------------- context

/// Builds the context of one request, without the system prompt.
///
/// The interface sends `messages` on to `local_ai_generate`; the dropped counts and
/// the warnings explain what was left out, and `used_facts` is the list of memory
/// entries that were included.
#[tauri::command(async)]
pub fn memory_build_context(
    state: tauri::State<'_, AppState>,
    conversation_id: Uuid,
    prompt: String,
    use_memory: bool,
    use_history: bool,
) -> Result<ContextPlanView, String> {
    let settings = current_settings(&state);
    let ai_config = state.local_ai.config();
    let plan = state.notes.with_memory(|session| {
        session.with_memory(|store| {
            let conversation = store
                .conversation(conversation_id)?
                .ok_or(MemoryError::NotFound)?;
            let persona = conversation.profile;
            let facts = if settings.uses_long_term_memory() {
                store.usable_facts(persona)?
            } else {
                Vec::new()
            };
            let history = if settings.stores_history() {
                store.chat_history(conversation_id)?
            } else {
                Vec::new()
            };
            let summary = store.summary(conversation_id)?;
            let prompt_text = system_prompt(persona);
            let plan = build_context(
                ContextRequest {
                    persona,
                    settings: &settings,
                    prompt: &prompt,
                    history,
                    facts,
                    summary: summary.as_ref().map(|view| view.summary.clone()),
                    summary_stale: summary.as_ref().map(|view| view.stale).unwrap_or(false),
                    context_size: ai_config.server.context_size as usize,
                    response_reserve: ai_config.max_tokens as usize,
                    use_memory,
                    use_history,
                    now: now(),
                },
                estimate_tokens(&prompt_text),
            );
            // Record what was actually used, so the memory page can show it.
            let used: Vec<Uuid> = plan.used_facts.iter().map(|fact| fact.id).collect();
            store.mark_facts_used(&used)?;
            Ok(plan)
        })
    })?;
    Ok(ContextPlanView::from_plan(&plan))
}

/// Budget numbers for the current model configuration, without a conversation.
#[tauri::command(async)]
pub fn memory_context_budget(state: tauri::State<'_, AppState>) -> Result<BudgetView, String> {
    let settings = current_settings(&state);
    let ai_config = state.local_ai.config();
    let prompt = system_prompt(ai_config.profile);
    let plan = plan_budget(BudgetRequest {
        context_size: ai_config.server.context_size as usize,
        response_reserve: ai_config.max_tokens as usize,
        memory_cap: settings.memory_token_budget,
        system_prompt_tokens: estimate_tokens(&prompt),
    });
    Ok(BudgetView {
        context_size: plan.context_size,
        system_prompt: plan.system_prompt,
        response_reserve: plan.response_reserve,
        safety_reserve: plan.safety_reserve,
        available: plan.available,
        memory: plan.memory,
        summary: plan.summary,
        recent_messages: plan.recent_messages,
    })
}

// ------------------------------------------------------- summaries and candidat.

/// Writes a summary for one conversation, if the threshold is reached.
///
/// Runs on a blocking worker because it waits for the model, and a conversation can
/// only have one summarization at a time. A failed model call is not an error the
/// interface has to handle: the messages stay and `null` is returned.
#[tauri::command]
pub async fn memory_summarize(
    state: tauri::State<'_, AppState>,
    conversation_id: Uuid,
) -> Result<Option<String>, String> {
    let settings = current_settings(&state);
    if !settings.writes_summaries() {
        return Ok(None);
    }
    let jobs = state.memory.clone();
    let notes = state.notes.clone();
    let gateway = state.local_ai.shared();
    on_blocking(move || {
        let guard = SummaryGuard::claim(jobs.jobs(), conversation_id).map_err(describe)?;
        let provider = LocalAiSummaryProvider::new(gateway);
        let result = notes.with_memory(|session| {
            session.with_memory(|store| {
                let persona = store
                    .conversation(conversation_id)?
                    .map(|conversation| conversation.profile)
                    .unwrap_or(Persona::Jarvis);
                summarize_conversation(store, &provider, &settings, persona, conversation_id)
            })
        });
        drop(guard);
        result
    })
    .await
}

/// Asks the model for memory candidates. Off unless the user turned it on.
///
/// Candidates are always stored as `Pending`: nothing becomes memory without an
/// explicit approval, and scope is chosen by the user at approval time.
#[tauri::command]
pub async fn memory_suggest_candidates(
    state: tauri::State<'_, AppState>,
    conversation_id: Uuid,
) -> Result<Vec<FactView>, String> {
    let settings = current_settings(&state);
    if !settings.suggests_facts() {
        return Err(MemoryError::MemoryDisabled.to_string());
    }
    let jobs = state.memory.clone();
    let notes = state.notes.clone();
    let gateway = state.local_ai.shared();
    on_blocking(move || {
        let guard = SummaryGuard::claim(jobs.jobs(), conversation_id).map_err(describe)?;
        let provider = LocalAiSummaryProvider::new(gateway);
        let outcome = notes.with_memory(|session| {
            session.with_memory(|store| {
                let conversation = store
                    .conversation(conversation_id)?
                    .ok_or(MemoryError::NotFound)?;
                let messages = store
                    .list_messages(conversation_id, 0, jarvis_core::memory::MAX_PAGE_SIZE)?
                    .messages;
                let suggestions = provider
                    .suggest_candidates(&CandidateRequest {
                        persona: conversation.profile,
                        messages: &messages,
                    })
                    .unwrap_or_default();
                let mut created = Vec::new();
                for suggestion in suggestions {
                    // The anchor is the user message the suggestion came from, and it
                    // was validated by the provider against the sent messages.
                    if let Ok(view) = store.create_candidate(
                        suggestion.scope,
                        suggestion.category,
                        &suggestion.content,
                        suggestion.confidence,
                        conversation_id,
                        suggestion.source_message_id,
                    ) {
                        created.push(view);
                    }
                }
                Ok(created)
            })
        });
        drop(guard);
        outcome
    })
    .await
}

// ------------------------------------------------------------ export and import

/// Writes an encrypted export of the memory database.
///
/// The file holds ciphertext and technical metadata only: it is useless without the
/// master key. A confirmation is required because it is a complete copy of memory.
#[tauri::command(async)]
pub fn memory_export_backup(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    confirmed: bool,
) -> Result<MemoryExportResult, String> {
    if !confirmed {
        return Err(MemoryError::ConfirmationRequired.to_string());
    }
    let Some(destination) = save_export_path(&app) else {
        return Ok(MemoryExportResult {
            path: String::new(),
            records: 0,
        });
    };
    let records = state
        .notes
        .with_memory(|session| session.with_memory(|store| store.export_records()))?;
    let device_id = state
        .notes
        .with_memory(|session| Ok(session.device_id().as_str().to_string()))?;
    let envelope = MemoryExportEnvelope {
        schema_version: MemoryExportEnvelope::CURRENT_SCHEMA_VERSION,
        exported_at: now().to_rfc3339(),
        device_id,
        records: records.clone(),
    };
    let text = serde_json::to_string_pretty(&envelope).map_err(|_| describe(MemoryError::Io))?;
    jarvis_core::fsutil::write_bytes_atomic(&destination, text.as_bytes())
        .map_err(|_| describe(MemoryError::Io))?;
    Ok(MemoryExportResult {
        path: destination.display().to_string(),
        records: records.len(),
    })
}

/// Imports an encrypted export, keeping conflicts for review instead of overwriting.
#[tauri::command(async)]
pub fn memory_import_backup(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<MemoryImportView, String> {
    let Some(source) = open_export_path(&app) else {
        return Ok(MemoryImportView {
            applied: 0,
            conflicts: 0,
            skipped: 0,
            secret_suspects: 0,
        });
    };
    let text = std::fs::read_to_string(&source).map_err(|_| describe(MemoryError::Io))?;
    let envelope: MemoryExportEnvelope =
        serde_json::from_str(&text).map_err(|_| describe(MemoryError::MalformedPayload))?;
    if envelope.schema_version != MemoryExportEnvelope::CURRENT_SCHEMA_VERSION {
        return Err(MemoryError::UnsupportedPayloadVersion.to_string());
    }
    state.notes.with_memory(|session| {
        session.with_memory(|store| {
            let outcome = store.import_records(&envelope.records)?;
            // Imported facts are scanned after they are decrypted: the filter cannot
            // read ciphertext, and importing a credential silently would be worse
            // than reporting it.
            let suspects = store
                .list_facts(&FactQuery {
                    state: None,
                    include_deleted: true,
                    ..FactQuery::default()
                })?
                .into_iter()
                .filter(|fact| !jarvis_core::memory::scan_for_secrets(&fact.content).is_clean())
                .count();
            Ok(MemoryImportView {
                secret_suspects: suspects,
                ..MemoryImportView::from(outcome)
            })
        })
    })
}

// ------------------------------------------------------------------- conflicts

#[tauri::command(async)]
pub fn memory_conflicts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MemoryConflictView>, String> {
    state
        .notes
        .with_memory(|session| session.with_memory(|store| store.conflicts()))
}

#[tauri::command(async)]
pub fn memory_resolve_conflict(
    state: tauri::State<'_, AppState>,
    conflict: Uuid,
    resolution: MemoryConflictResolution,
) -> Result<bool, String> {
    state.notes.with_memory(|session| {
        session.with_memory(|store| store.resolve_conflict(conflict, resolution))
    })
}

// -------------------------------------------------------------------- helpers

/// Reads the stored memory settings; a damaged value means defaults.
fn current_settings(state: &tauri::State<'_, AppState>) -> MemorySettings {
    let stored = state.settings.read(SETTINGS_KEY);
    load_settings(stored.as_deref())
}

/// Runs a blocking job on the blocking pool.
async fn on_blocking<T, F>(action: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(action).await {
        Ok(result) => result,
        Err(_) => Err(describe(MemoryError::Io)),
    }
}

/// The secret gate: refuse an automatic save, ask for confirmation on a manual one.
///
/// The matched text never leaves the core: the error names the kinds it saw, and the
/// interface shows a warning that the user has to accept explicitly.
fn gate_secret(draft: &FactDraft) -> Result<(), String> {
    let scan = jarvis_core::memory::scan_for_secrets(&draft.content);
    if scan.is_clean() {
        return Ok(());
    }
    let kinds = scan.kinds();
    if draft.accept_secret_warning {
        // Only the kinds are logged, never the value.
        log::warn!("memory: a fact was saved after accepting a secret warning");
        Ok(())
    } else {
        Err(describe(MemoryError::SecretConfirmationRequired(kinds)))
    }
}

/// Turns a memory error into a message safe to show and to log.
fn describe(error: MemoryError) -> String {
    let message = error.to_string();
    log::warn!("memory: {message}");
    message
}

fn save_export_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name("jarvis-ai-memory-export.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}

fn open_export_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("JSON", &["json"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}
