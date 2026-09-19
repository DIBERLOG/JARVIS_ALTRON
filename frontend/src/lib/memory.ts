/**
 * Typed wrappers around the AI-memory commands.
 *
 * The interface never reads SQLite, never receives a key, and never decrypts a
 * payload: it asks for views and sends drafts. Everything heavy (SQLite, decryption,
 * summarization) happens in Rust on a worker thread.
 *
 * Nothing here persists anything in the browser.
 */

import { invoke } from "@tauri-apps/api/core"

import type {
    BudgetView,
    CandidateState,
    ContextPlanView,
    ConversationDetails,
    ConversationQuery,
    ConversationView,
    FactDraft,
    FactQuery,
    FactView,
    MemoryConflictView,
    MemoryExportResult,
    MemoryImportView,
    MemoryScope,
    MemorySettings,
    MemoryStatusView,
    MessageStatus,
    MessageView,
    Persona,
    SummaryView
} from "./memory-model"

/** Hard cap used when a caller asks for "everything" from a listing. */
export const MEMORY_PAGE_LIMIT = 200

export const memoryApi = {
    // storage and settings
    status: () => invoke<MemoryStatusView>("memory_status"),
    getSettings: () => invoke<MemorySettings>("memory_get_settings"),
    updateSettings: (settings: MemorySettings) =>
        invoke<MemoryStatusView>("memory_update_settings", { settings }),
    /**
     * Locks the shared encrypted storage.
     *
     * The memory key is dropped with the master key, and so are the notes and the
     * password vault: one master password protects one encrypted storage.
     */
    lock: () => invoke<MemoryStatusView>("memory_lock"),

    // conversations
    listConversations: (query: ConversationQuery) =>
        invoke<ConversationView[]>("memory_list_conversations", { query }),
    createConversation: (profile: Persona, title: string) =>
        invoke<ConversationView>("memory_create_conversation", { profile, title }),
    openConversation: (id: string, offset: number, limit: number) =>
        invoke<ConversationDetails>("memory_open_conversation", { id, offset, limit }),
    renameConversation: (id: string, title: string) =>
        invoke<ConversationView>("memory_rename_conversation", { id, title }),
    archiveConversation: (id: string, archived: boolean) =>
        invoke<ConversationView>("memory_archive_conversation", { id, archived }),
    deleteConversation: (id: string) => invoke<unknown>("memory_delete_conversation", { id }),
    clearConversation: (id: string) => invoke<unknown>("memory_clear_conversation", { id }),
    /** Requires an explicit confirmation from the interface. */
    clearHistory: (confirmed: boolean) => invoke<unknown>("memory_clear_history", { confirmed }),

    // messages
    appendUserMessage: (conversationId: string, content: string) =>
        invoke<MessageView>("memory_append_user_message", { conversationId, content }),
    appendAssistantMessage: (
        conversationId: string,
        content: string,
        status: MessageStatus,
        partial: boolean
    ) =>
        invoke<MessageView>("memory_append_assistant_message", {
            conversationId,
            content,
            status,
            partial
        }),
    deleteMessage: (id: string) => invoke<void>("memory_delete_message", { id }),

    // facts
    listFacts: (query: FactQuery) => invoke<FactView[]>("memory_list_facts", { query }),
    createFact: (draft: FactDraft) => invoke<FactView>("memory_create_fact", { draft }),
    updateFact: (id: string, draft: FactDraft) => invoke<FactView>("memory_update_fact", { id, draft }),
    setFactUsage: (id: string, pinned: boolean, disabled: boolean) =>
        invoke<FactView>("memory_set_fact_usage", { id, pinned, disabled }),
    deleteFact: (id: string) => invoke<FactView>("memory_delete_fact", { id }),
    restoreFact: (id: string) => invoke<FactView>("memory_restore_fact", { id }),
    purgeFact: (id: string) => invoke<void>("memory_purge_fact", { id }),

    // candidates
    listCandidates: (conversationId: string | null) =>
        invoke<FactView[]>("memory_list_candidates", { conversationId }),
    approveCandidate: (id: string, draft: FactDraft | null) =>
        invoke<FactView>("memory_approve_candidate", { id, draft }),
    rejectCandidate: (id: string) => invoke<FactView>("memory_reject_candidate", { id }),

    // context
    buildContext: (
        conversationId: string,
        prompt: string,
        useMemory: boolean,
        useHistory: boolean
    ) =>
        invoke<ContextPlanView>("memory_build_context", {
            conversationId,
            prompt,
            useMemory,
            useHistory
        }),
    contextBudget: () => invoke<BudgetView>("memory_context_budget"),

    // summaries and candidates from the model
    summarize: (conversationId: string) => invoke<string | null>("memory_summarize", { conversationId }),
    suggestCandidates: (conversationId: string) =>
        invoke<FactView[]>("memory_suggest_candidates", { conversationId }),

    // encrypted export and import
    exportBackup: (confirmed: boolean) =>
        invoke<MemoryExportResult>("memory_export_backup", { confirmed }),
    importBackup: () => invoke<MemoryImportView>("memory_import_backup"),

    // conflicts
    conflicts: () => invoke<MemoryConflictView[]>("memory_conflicts"),
    resolveConflict: (conflict: string, resolution: "keep_current" | "accept_incoming") =>
        invoke<boolean>("memory_resolve_conflict", { conflict, resolution })
}

/** Convenience for the memory page: every approved fact of one scope. */
export function scopeQuery(scope: MemoryScope | null): FactQuery {
    return {
        search: "",
        scope,
        category: null,
        state: "approved" as CandidateState,
        include_deleted: false,
        limit: MEMORY_PAGE_LIMIT,
        offset: 0
    }
}

/** The summary of a conversation, or `null` when it has none. */
export function summaryOrNull(details: ConversationDetails | null): SummaryView | null {
    return details ? details.summary : null
}
