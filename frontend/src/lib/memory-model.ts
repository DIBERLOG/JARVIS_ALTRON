/**
 * Interface-side logic for the encrypted AI memory.
 *
 * Like `local-ai-model.ts`, everything here is free of Tauri and Svelte
 * dependencies so it can be unit tested with the Node test runner
 * (`npm run test:ui`). It holds the shape of what the backend sends, the local
 * checks the memory page can make before a round trip, the label keys every view
 * needs, and the small decisions the interface must make consistently:
 * memory off, storage locked, a suspicious secret, a partial answer.
 *
 * Two rules are encoded here on purpose:
 *
 * * a detected secret is reported by **kind**, never by value: there is no field in
 *   this module that carries the matched text, so a warning cannot leak it;
 * * nothing is persisted in the browser: no browser storage, no URL, no file. The
 *   history lives in the encrypted database and is read through the backend.
 */

export type Persona = "jarvis" | "altron"

export type MemoryScope = "global" | "jarvis" | "altron"
export type MemoryCategory =
    | "preference"
    | "personal_fact"
    | "project"
    | "instruction"
    | "correction"
    | "other"
export type MemorySource = "manual" | "suggested_from_conversation" | "imported"
export type CandidateState = "pending" | "approved" | "rejected"
export type MessageRole = "user" | "assistant"
export type MessageStatus = "completed" | "cancelled" | "failed"
export type StorageState = "uninitialized" | "locked" | "unlocked" | "key_missing"

export type SecretKind =
    | "private_key"
    | "api_token"
    | "jwt"
    | "recovery_code"
    | "password_assignment"
    | "high_entropy_token"
    | "payment_card"

/** Settings bounds, mirroring `MemorySettings` in the core. */
export const MEMORY_SETTINGS_SCHEMA_VERSION = 1
export const MIN_MAX_RECENT_MESSAGES = 2
export const MAX_MAX_RECENT_MESSAGES = 100
export const MIN_MEMORY_TOKEN_BUDGET = 0
export const MAX_MEMORY_TOKEN_BUDGET = 4096
export const MIN_SUMMARY_TRIGGER_MESSAGES = 4
export const MAX_SUMMARY_TRIGGER_MESSAGES = 200
export const MIN_SUMMARY_KEEP_RECENT = 2
export const MAX_SUMMARY_KEEP_RECENT = 50

export const DEFAULT_MAX_RECENT_MESSAGES = 12
export const DEFAULT_MEMORY_TOKEN_BUDGET = 512
export const DEFAULT_SUMMARY_TRIGGER_MESSAGES = 12
export const DEFAULT_SUMMARY_KEEP_RECENT = 6

/** Above these counts the linear search cost is worth mentioning. */
export const LINEAR_SEARCH_WARNING_FACTS = 500
export const LINEAR_SEARCH_WARNING_MESSAGES = 5000

export const MEMORY_SCOPES: readonly MemoryScope[] = ["global", "jarvis", "altron"]
export const MEMORY_CATEGORIES: readonly MemoryCategory[] = [
    "preference",
    "personal_fact",
    "project",
    "instruction",
    "correction",
    "other"
]
export const MEMORY_SOURCES: readonly MemorySource[] = [
    "manual",
    "suggested_from_conversation",
    "imported"
]
export const CANDIDATE_STATES: readonly CandidateState[] = ["pending", "approved", "rejected"]
export const MESSAGE_STATUSES: readonly MessageStatus[] = ["completed", "cancelled", "failed"]
export const SECRET_KINDS: readonly SecretKind[] = [
    "private_key",
    "api_token",
    "jwt",
    "recovery_code",
    "password_assignment",
    "high_entropy_token",
    "payment_card"
]

export interface MemorySettings {
    enabled: boolean
    save_history: boolean
    use_long_term_memory: boolean
    suggest_facts: boolean
    auto_summaries: boolean
    max_recent_messages: number
    memory_token_budget: number
    summary_trigger_messages: number
    summary_keep_recent: number
    schema_version: number
}

export interface MemoryStats {
    conversations: number
    archived: number
    messages: number
    facts: number
    pending_candidates: number
    disabled_facts: number
    trashed_facts: number
    unreadable: number
}

export interface StorageStatus {
    state: StorageState
    dpapi_available: boolean
    backup_available: boolean
    has_stored_data: boolean
    data_dir: string
}

export interface MemoryStatus {
    unlocked: boolean
    has_stored_data: boolean
    settings: MemorySettings
    stats: MemoryStats
    linear_search_cost_warning: string | null
}

export interface MemoryStatusView {
    storage: StorageStatus
    memory: MemoryStatus
    summarizing: string[]
}

export interface ConversationView {
    id: string
    revision: number
    title: string
    profile: Persona
    created_at: string
    updated_at: string
    archived_at: string | null
    message_count: number
    first_message_at: string | null
    last_message_at: string | null
}

export interface MessageView {
    id: string
    revision: number
    role: MessageRole
    content: string
    status: MessageStatus
    partial: boolean
    sequence: number
    created_at: string
}

export interface MessagePage {
    messages: MessageView[]
    total: number
    offset: number
}

export interface SummaryView {
    id: string
    revision: number
    summary: string
    covers_until_message: string
    covered_messages: number
    stale: boolean
    created_at: string
    updated_at: string
}

export interface FactView {
    id: string
    revision: number
    scope: MemoryScope
    category: MemoryCategory
    content: string
    source: MemorySource
    confidence: number
    state: CandidateState
    pinned: boolean
    disabled: boolean
    created_at: string
    updated_at: string
    last_used_at: string | null
    deleted_at: string | null
    source_conversation_id: string | null
    source_message_id: string | null
    instruction_like: boolean
}

export interface ConversationDetails {
    conversation: ConversationView
    page: MessagePage
    summary: SummaryView | null
    candidates: FactView[]
}

export interface FactDraft {
    scope: MemoryScope
    category: MemoryCategory
    content: string
    pinned: boolean
    disabled: boolean
    accept_secret_warning: boolean
}

export interface ConversationQuery {
    include_archived: boolean
    limit: number
    offset: number
}

export interface FactQuery {
    search: string
    scope: MemoryScope | null
    category: MemoryCategory | null
    state: CandidateState | null
    include_deleted: boolean
    limit: number
    offset: number
}

export interface BudgetView {
    context_size: number
    system_prompt: number
    response_reserve: number
    safety_reserve: number
    available: number
    memory: number
    summary: number
    recent_messages: number
}

export interface UsedFactView {
    id: string
    scope: MemoryScope
    category: MemoryCategory
    excerpt: string
    score: number
}

export interface ContextWarningView {
    code: string
    fact_id: string | null
    message: string
}

export type ContextSection = "memory" | "summary" | "history" | "question"

export interface ChatMessage {
    role: "system" | "user" | "assistant"
    content: string
}

export interface ContextPlanView {
    messages: ChatMessage[]
    sections: ContextSection[]
    used_facts: UsedFactView[]
    summary_used: boolean
    dropped_facts: number
    dropped_messages: number
    estimated_tokens: number
    budget: BudgetView
    memory_enabled: boolean
    history_enabled: boolean
    warnings: ContextWarningView[]
}

export interface MemoryConflictView {
    conflict_id: string
    entity_id: string
    entity_type: string
    current_revision: number
    incoming_revision: number
    incoming_available: boolean
}

export interface MemoryImportView {
    applied: number
    conflicts: number
    skipped: number
    secret_suspects: number
}

export interface MemoryExportResult {
    path: string
    records: number
}

// ------------------------------------------------------------------ settings

export function defaultSettings(): MemorySettings {
    return {
        enabled: true,
        save_history: true,
        use_long_term_memory: true,
        suggest_facts: false,
        auto_summaries: true,
        max_recent_messages: DEFAULT_MAX_RECENT_MESSAGES,
        memory_token_budget: DEFAULT_MEMORY_TOKEN_BUDGET,
        summary_trigger_messages: DEFAULT_SUMMARY_TRIGGER_MESSAGES,
        summary_keep_recent: DEFAULT_SUMMARY_KEEP_RECENT,
        schema_version: MEMORY_SETTINGS_SCHEMA_VERSION
    }
}

export function emptyStats(): MemoryStats {
    return {
        conversations: 0,
        archived: 0,
        messages: 0,
        facts: 0,
        pending_candidates: 0,
        disabled_facts: 0,
        trashed_facts: 0,
        unreadable: 0
    }
}

function isFiniteNumber(value: unknown): value is number {
    return typeof value === "number" && Number.isFinite(value)
}

function clampInteger(value: unknown, fallback: number, min: number, max: number): number {
    if (!isFiniteNumber(value)) return fallback
    const rounded = Math.round(value)
    if (rounded < min) return min
    if (rounded > max) return max
    return rounded
}

/**
 * Repairs a settings object that came from the backend or from a form field.
 *
 * A value the user has not finished typing must never become `NaN` or a number the
 * core would refuse for a reason the interface could have explained.
 */
export function normalizeSettings(input: Partial<MemorySettings> | null | undefined): MemorySettings {
    const fallback = defaultSettings()
    if (!input || typeof input !== "object") return fallback
    return {
        enabled: input.enabled !== false,
        save_history: input.save_history !== false,
        use_long_term_memory: input.use_long_term_memory !== false,
        // Automatic extraction stays off unless it is explicitly true.
        suggest_facts: input.suggest_facts === true,
        auto_summaries: input.auto_summaries !== false,
        max_recent_messages: clampInteger(
            input.max_recent_messages,
            fallback.max_recent_messages,
            MIN_MAX_RECENT_MESSAGES,
            MAX_MAX_RECENT_MESSAGES
        ),
        memory_token_budget: clampInteger(
            input.memory_token_budget,
            fallback.memory_token_budget,
            MIN_MEMORY_TOKEN_BUDGET,
            MAX_MEMORY_TOKEN_BUDGET
        ),
        summary_trigger_messages: clampInteger(
            input.summary_trigger_messages,
            fallback.summary_trigger_messages,
            MIN_SUMMARY_TRIGGER_MESSAGES,
            MAX_SUMMARY_TRIGGER_MESSAGES
        ),
        summary_keep_recent: clampInteger(
            input.summary_keep_recent,
            fallback.summary_keep_recent,
            MIN_SUMMARY_KEEP_RECENT,
            MAX_SUMMARY_KEEP_RECENT
        ),
        schema_version: MEMORY_SETTINGS_SCHEMA_VERSION
    }
}

/** The field name and message key of one local problem. */
export interface SettingsIssue {
    field: string
    key: string
}

/**
 * Local checks for the settings form.
 *
 * The core normalizes and validates as well; these exist so the form can explain a
 * problem while the user types.
 */
export function validateSettings(settings: MemorySettings): SettingsIssue[] {
    const issues: SettingsIssue[] = []
    if (settings.max_recent_messages < MIN_MAX_RECENT_MESSAGES || settings.max_recent_messages > MAX_MAX_RECENT_MESSAGES) {
        issues.push({ field: "max_recent_messages", key: "memory-issue-recent-messages" })
    }
    if (settings.memory_token_budget < MIN_MEMORY_TOKEN_BUDGET || settings.memory_token_budget > MAX_MEMORY_TOKEN_BUDGET) {
        issues.push({ field: "memory_token_budget", key: "memory-issue-token-budget" })
    }
    if (
        settings.summary_trigger_messages < MIN_SUMMARY_TRIGGER_MESSAGES ||
        settings.summary_trigger_messages > MAX_SUMMARY_TRIGGER_MESSAGES
    ) {
        issues.push({ field: "summary_trigger_messages", key: "memory-issue-summary-trigger" })
    }
    if (settings.summary_keep_recent < MIN_SUMMARY_KEEP_RECENT || settings.summary_keep_recent > MAX_SUMMARY_KEEP_RECENT) {
        issues.push({ field: "summary_keep_recent", key: "memory-issue-summary-keep" })
    }
    // A summary must always leave the newest messages untouched, so the trigger has
    // to be larger than the window that stays verbatim.
    if (
        settings.auto_summaries &&
        settings.summary_trigger_messages <= settings.summary_keep_recent
    ) {
        issues.push({ field: "summary_trigger_messages", key: "memory-issue-summary-order" })
    }
    if (settings.suggest_facts && !settings.enabled) {
        issues.push({ field: "suggest_facts", key: "memory-issue-suggestions-need-memory" })
    }
    return issues
}

// -------------------------------------------------------------------- queries

export function defaultConversationQuery(): ConversationQuery {
    return { include_archived: false, limit: 50, offset: 0 }
}

export function defaultFactQuery(): FactQuery {
    return {
        search: "",
        scope: null,
        category: null,
        // Only approved memory is shown by default: candidates have their own list.
        state: "approved",
        include_deleted: false,
        limit: 50,
        offset: 0
    }
}

/** Client-side mirror of the store filter, for instant filtering of a loaded page. */
export function matchesFact(fact: FactView, query: FactQuery): boolean {
    if (!query.include_deleted && fact.deleted_at !== null) return false
    if (query.scope && fact.scope !== query.scope) return false
    if (query.category && fact.category !== query.category) return false
    if (query.state && fact.state !== query.state) return false
    const needle = query.search.trim().toLowerCase()
    if (needle.length > 0 && !fact.content.toLowerCase().includes(needle)) return false
    return true
}

/** Pinned first, then most recently updated. Mirrors the store's order. */
export function sortFacts(facts: readonly FactView[]): FactView[] {
    return facts.slice().sort((left, right) => {
        if (left.pinned !== right.pinned) return left.pinned ? -1 : 1
        if (left.updated_at !== right.updated_at) return left.updated_at < right.updated_at ? 1 : -1
        return left.id < right.id ? -1 : left.id > right.id ? 1 : 0
    })
}

// --------------------------------------------------------------------- labels

export function scopeLabelKey(scope: MemoryScope): string {
    return `memory-scope-${scope}`
}

export function categoryLabelKey(category: MemoryCategory): string {
    return `memory-category-${category.replace(/_/g, "-")}`
}

export function sourceLabelKey(source: MemorySource): string {
    return `memory-source-${source.replace(/_/g, "-")}`
}

export function candidateStateLabelKey(state: CandidateState): string {
    return `memory-candidate-${state}`
}

export function messageStatusLabelKey(status: MessageStatus): string {
    return `memory-message-${status}`
}

export function contextSectionLabelKey(section: ContextSection): string {
    return `memory-context-${section}`
}

export function secretKindLabelKey(kind: SecretKind): string {
    return `memory-secret-${kind.replace(/_/g, "-")}`
}

/** Storage states the memory page renders, reusing the shared storage gate keys. */
export function storageStateLabelKey(state: StorageState): string {
    return `memory-storage-${state.replace(/_/g, "-")}`
}

/** Whether the interface must ask to unlock the shared storage first. */
export function needsUnlock(status: MemoryStatusView | null): boolean {
    if (!status) return false
    return status.storage.state === "locked" && !status.memory.unlocked
}

/** Whether a master password has to be created before anything can be stored. */
export function needsSetup(status: MemoryStatusView | null): boolean {
    if (!status) return false
    return status.storage.state === "uninitialized"
}

/** Whether the key material is gone and only a backup import can recover. */
export function isKeyMissing(status: MemoryStatusView | null): boolean {
    if (!status) return false
    return status.storage.state === "key_missing"
}

/** Whether the memory layer can read and write right now. */
export function memoryIsUsable(status: MemoryStatusView | null): boolean {
    return Boolean(status && status.memory.unlocked && status.memory.settings.enabled)
}

/** Whether conversation history may be written. */
export function canWriteHistory(status: MemoryStatusView | null): boolean {
    if (!status || !status.memory.unlocked) return false
    return status.memory.settings.enabled && status.memory.settings.save_history
}

/** Whether approved facts may be used in a request. */
export function canUseFacts(status: MemoryStatusView | null): boolean {
    if (!status || !status.memory.unlocked) return false
    return status.memory.settings.enabled && status.memory.settings.use_long_term_memory
}

/** Whether a summarization is running for this conversation. */
export function isSummarizing(status: MemoryStatusView | null, conversationId: string | null): boolean {
    if (!status || !conversationId) return false
    return status.summarizing.includes(conversationId)
}

// ------------------------------------------------------------------- warnings

/**
 * The label keys of a secret warning.
 *
 * The backend reports kinds only, so this maps kinds to messages; the matched text
 * never reaches the interface.
 */
export function secretWarningKeys(kinds: readonly SecretKind[]): string[] {
    const keys = kinds.map(secretKindLabelKey)
    return keys.length > 0 ? keys : ["memory-secret-unknown"]
}

/** Whether a failed call was the secret gate asking for a confirmation. */
export function isSecretConfirmation(error: string): boolean {
    return error.includes("confirm to save") || error.includes("secret_confirmation_required")
}

/** A linear-cost warning for a memory that grew large. */
export function largeMemoryWarningKey(stats: MemoryStats): string | null {
    if (stats.facts > LINEAR_SEARCH_WARNING_FACTS) return "memory-warning-many-facts"
    if (stats.messages > LINEAR_SEARCH_WARNING_MESSAGES) return "memory-warning-many-messages"
    return null
}

/** The label key of a context warning the backend reported. */
export function contextWarningKey(code: string): string {
    switch (code) {
        case "instruction_like_fact":
            return "memory-warning-instruction-like"
        case "summary_stale":
            return "memory-warning-summary-stale"
        case "summary_dropped":
            return "memory-warning-summary-dropped"
        case "low_relevance_fact":
            return "memory-warning-low-relevance"
        default:
            return "memory-warning-generic"
    }
}

/** How an answer that was not completed should be labelled. */
export function answerNoteKey(status: MessageStatus, partial: boolean): string | null {
    if (status === "cancelled") {
        return partial ? "memory-answer-cancelled-partial" : "memory-answer-cancelled"
    }
    if (status === "failed") return "memory-answer-failed"
    return null
}

/** Whether a partial answer may be kept; the user decides, and the default is no. */
export const KEEP_PARTIAL_ANSWER_DEFAULT = false

/** Message key shown before a destructive bulk action. */
export function bulkDeleteConfirmationKey(scope: "history" | "conversation" | "fact"): string {
    return `memory-confirm-${scope}`
}

// -------------------------------------------------------------------- budgets

export interface BudgetRow {
    key: string
    tokens: number
    percent: number
}

/** The budget split, as rows the page can render. */
export function budgetBreakdown(budget: BudgetView | null): BudgetRow[] {
    if (!budget) return []
    const total = Math.max(1, budget.context_size)
    const rows: BudgetRow[] = [
        { key: "memory-budget-system", tokens: budget.system_prompt, percent: percentOf(budget.system_prompt, total) },
        { key: "memory-budget-reserve", tokens: budget.response_reserve, percent: percentOf(budget.response_reserve, total) },
        { key: "memory-budget-safety", tokens: budget.safety_reserve, percent: percentOf(budget.safety_reserve, total) },
        { key: "memory-budget-memory", tokens: budget.memory, percent: percentOf(budget.memory, total) },
        { key: "memory-budget-summary", tokens: budget.summary, percent: percentOf(budget.summary, total) },
        { key: "memory-budget-history", tokens: budget.recent_messages, percent: percentOf(budget.recent_messages, total) }
    ]
    return rows
}

function percentOf(value: number, total: number): number {
    if (!isFiniteNumber(value) || total <= 0) return 0
    return Math.round((value / total) * 1000) / 10
}

/** A compact token count for the interface. */
export function formatTokens(tokens: number): string {
    if (!isFiniteNumber(tokens) || tokens <= 0) return "0"
    if (tokens < 1000) return String(Math.round(tokens))
    return `${(tokens / 1000).toFixed(1)}k`
}

/** Counts in the dashboard, with a translated label key. */
export function statsRowKeys(stats: MemoryStats): string[] {
    const keys = ["memory-stats-conversations", "memory-stats-messages", "memory-stats-facts"]
    if (stats.pending_candidates > 0) keys.push("memory-stats-candidates")
    if (stats.disabled_facts > 0) keys.push("memory-stats-disabled")
    if (stats.trashed_facts > 0) keys.push("memory-stats-trashed")
    if (stats.unreadable > 0) keys.push("memory-stats-unreadable")
    return keys
}

/** The number a stats row shows. */
export function statsRowValue(stats: MemoryStats, key: string): number {
    switch (key) {
        case "memory-stats-conversations":
            return stats.conversations
        case "memory-stats-messages":
            return stats.messages
        case "memory-stats-facts":
            return stats.facts
        case "memory-stats-candidates":
            return stats.pending_candidates
        case "memory-stats-disabled":
            return stats.disabled_facts
        case "memory-stats-trashed":
            return stats.trashed_facts
        case "memory-stats-unreadable":
            return stats.unreadable
        default:
            return 0
    }
}

// ------------------------------------------------------------------ export/import

/** What the interface must warn about before an export or an import. */
export function exportWarningKeys(result: MemoryExportResult, cancelling: boolean): string[] {
    if (cancelling || result.path.length === 0) return ["memory-export-cancelled"]
    return ["memory-export-encrypted-only", "memory-export-needs-key"]
}

/** What an import outcome should tell the user. */
export function importWarningKeys(outcome: MemoryImportView): string[] {
    const keys: string[] = []
    if (outcome.applied > 0) keys.push("memory-import-applied")
    if (outcome.conflicts > 0) keys.push("memory-import-conflicts")
    if (outcome.skipped > 0) keys.push("memory-import-skipped")
    if (outcome.secret_suspects > 0) keys.push("memory-import-secret-suspects")
    return keys
}

/** A conversation title placeholder, for an empty or whitespace-only title. */
export function conversationTitle(title: string, placeholder: string): string {
    const trimmed = title.trim()
    return trimmed.length > 0 ? trimmed : placeholder
}

/** Whether a conversation can be switched to another profile. */
export function profileSwitchRequiresNewConversation(): boolean {
    // The profile of a conversation is fixed when it is created, so the interface
    // must start a new conversation instead of changing an existing one. Memory is
    // scoped by profile, and rewriting history under another profile would mix the
    // two areas.
    return true
}

/** The scope a new fact defaults to for a profile. */
export function defaultScopeFor(profile: Persona): MemoryScope {
    return profile === "altron" ? "altron" : "jarvis"
}
