import { test } from "node:test"
import assert from "node:assert/strict"

import {
    CANDIDATE_STATES,
    DEFAULT_MAX_RECENT_MESSAGES,
    DEFAULT_MEMORY_TOKEN_BUDGET,
    DEFAULT_SUMMARY_KEEP_RECENT,
    DEFAULT_SUMMARY_TRIGGER_MESSAGES,
    KEEP_PARTIAL_ANSWER_DEFAULT,
    LINEAR_SEARCH_WARNING_FACTS,
    LINEAR_SEARCH_WARNING_MESSAGES,
    MAX_MAX_RECENT_MESSAGES,
    MAX_MEMORY_TOKEN_BUDGET,
    MAX_SUMMARY_KEEP_RECENT,
    MAX_SUMMARY_TRIGGER_MESSAGES,
    MEMORY_CATEGORIES,
    MEMORY_SCOPES,
    MEMORY_SOURCES,
    MESSAGE_STATUSES,
    MIN_MAX_RECENT_MESSAGES,
    MIN_SUMMARY_KEEP_RECENT,
    MIN_SUMMARY_TRIGGER_MESSAGES,
    SECRET_KINDS,
    answerNoteKey,
    budgetBreakdown,
    bulkDeleteConfirmationKey,
    canUseFacts,
    canWriteHistory,
    candidateStateLabelKey,
    categoryLabelKey,
    contextSectionLabelKey,
    contextWarningKey,
    conversationTitle,
    defaultConversationQuery,
    defaultFactQuery,
    defaultScopeFor,
    defaultSettings,
    emptyStats,
    exportWarningKeys,
    formatTokens,
    importWarningKeys,
    isKeyMissing,
    isSecretConfirmation,
    isSummarizing,
    largeMemoryWarningKey,
    matchesFact,
    memoryIsUsable,
    messageStatusLabelKey,
    needsSetup,
    needsUnlock,
    normalizeSettings,
    profileSwitchRequiresNewConversation,
    scopeLabelKey,
    secretKindLabelKey,
    secretWarningKeys,
    sortFacts,
    sourceLabelKey,
    statsRowKeys,
    statsRowValue,
    storageStateLabelKey,
    validateSettings
} from "../src/lib/memory-model.ts"

/** A fact as the backend sends it. */
function fact(overrides = {}) {
    return {
        id: "00000000-0000-0000-0000-000000000001",
        revision: 1,
        scope: "global",
        category: "preference",
        content: "Предпочитает тёмную тему",
        source: "manual",
        confidence: 1,
        state: "approved",
        pinned: false,
        disabled: false,
        created_at: "2026-09-01T10:00:00+00:00",
        updated_at: "2026-09-01T10:00:00+00:00",
        last_used_at: null,
        deleted_at: null,
        source_conversation_id: null,
        source_message_id: null,
        instruction_like: false,
        ...overrides
    }
}

/** A status view as `memory_status` returns it. */
function statusView(overrides = {}) {
    const settings = defaultSettings()
    return {
        storage: {
            state: "unlocked",
            dpapi_available: true,
            backup_available: true,
            has_stored_data: true,
            data_dir: "C:/data"
        },
        memory: {
            unlocked: true,
            has_stored_data: true,
            settings,
            stats: emptyStats(),
            linear_search_cost_warning: null
        },
        summarizing: [],
        ...overrides
    }
}

// ------------------------------------------------------------------ settings

test("the default settings keep automatic extraction off", () => {
    const settings = defaultSettings()
    assert.equal(settings.enabled, true)
    assert.equal(settings.save_history, true)
    assert.equal(settings.use_long_term_memory, true)
    // The only behaviour that writes memory from model output is opt-in.
    assert.equal(settings.suggest_facts, false)
    assert.equal(settings.auto_summaries, true)
    assert.equal(settings.max_recent_messages, DEFAULT_MAX_RECENT_MESSAGES)
    assert.equal(settings.memory_token_budget, DEFAULT_MEMORY_TOKEN_BUDGET)
    assert.equal(settings.summary_trigger_messages, DEFAULT_SUMMARY_TRIGGER_MESSAGES)
    assert.equal(settings.summary_keep_recent, DEFAULT_SUMMARY_KEEP_RECENT)
    assert.deepEqual(validateSettings(settings), [])
})

test("settings from the backend are repaired rather than trusted", () => {
    const repaired = normalizeSettings({
        enabled: "yes",
        suggest_facts: "yes",
        max_recent_messages: 0,
        memory_token_budget: 999_999,
        summary_trigger_messages: Number.NaN,
        summary_keep_recent: -4
    })
    // Only a real boolean may switch something on.
    assert.equal(repaired.suggest_facts, false)
    assert.equal(repaired.max_recent_messages, MIN_MAX_RECENT_MESSAGES)
    assert.equal(repaired.memory_token_budget, MAX_MEMORY_TOKEN_BUDGET)
    assert.equal(repaired.summary_trigger_messages, DEFAULT_SUMMARY_TRIGGER_MESSAGES)
    assert.equal(repaired.summary_keep_recent, MIN_SUMMARY_KEEP_RECENT)
    assert.deepEqual(validateSettings(repaired), [])
})

test("a missing settings object falls back to the defaults", () => {
    for (const value of [null, undefined, {}, 42]) {
        assert.deepEqual(normalizeSettings(value), defaultSettings())
    }
    // A zero budget is meaningful and stays zero.
    assert.equal(normalizeSettings({ memory_token_budget: 0 }).memory_token_budget, 0)
})

test("the local settings checks explain each problem", () => {
    const broken = {
        ...defaultSettings(),
        max_recent_messages: MAX_MAX_RECENT_MESSAGES + 1,
        memory_token_budget: -1,
        summary_trigger_messages: MAX_SUMMARY_TRIGGER_MESSAGES + 1,
        summary_keep_recent: MAX_SUMMARY_KEEP_RECENT + 1
    }
    const fields = validateSettings(broken).map((issue) => issue.field)
    assert.ok(fields.includes("max_recent_messages"))
    assert.ok(fields.includes("memory_token_budget"))
    assert.ok(fields.includes("summary_trigger_messages"))
    assert.ok(fields.includes("summary_keep_recent"))

    // A summary must always leave the newest messages verbatim.
    const inverted = {
        ...defaultSettings(),
        summary_trigger_messages: 6,
        summary_keep_recent: 6
    }
    assert.deepEqual(
        validateSettings(inverted).map((issue) => issue.key),
        ["memory-issue-summary-order"]
    )

    // Suggestions need memory to be on.
    const suggestions = { ...defaultSettings(), enabled: false, suggest_facts: true }
    assert.deepEqual(
        validateSettings(suggestions).map((issue) => issue.key),
        ["memory-issue-suggestions-need-memory"]
    )
})

// ------------------------------------------------------------------- queries

test("the default queries show approved memory only", () => {
    const facts = defaultFactQuery()
    assert.equal(facts.state, "approved")
    assert.equal(facts.include_deleted, false)
    assert.equal(facts.scope, null)
    assert.equal(facts.category, null)
    assert.equal(facts.search, "")

    const conversations = defaultConversationQuery()
    assert.equal(conversations.include_archived, false)
    assert.ok(conversations.limit > 0)
})

test("the local filter mirrors the store filter", () => {
    const approved = fact()
    const pending = fact({ id: "2", state: "pending" })
    const deleted = fact({ id: "3", deleted_at: "2026-09-10T00:00:00+00:00" })
    const otherScope = fact({ id: "4", scope: "altron" })
    const otherCategory = fact({ id: "5", category: "project" })

    const query = defaultFactQuery()
    assert.equal(matchesFact(approved, query), true)
    assert.equal(matchesFact(pending, query), false)
    assert.equal(matchesFact(deleted, query), false)
    assert.equal(matchesFact(otherScope, query), true)

    assert.equal(matchesFact(otherScope, { ...query, scope: "altron" }), true)
    assert.equal(matchesFact(otherScope, { ...query, scope: "jarvis" }), false)
    assert.equal(matchesFact(otherCategory, { ...query, category: "project" }), true)
    assert.equal(matchesFact(otherCategory, { ...query, category: "other" }), false)
    assert.equal(matchesFact(approved, { ...query, search: "ТЁМНУЮ" }), true)
    assert.equal(matchesFact(approved, { ...query, search: "rust" }), false)
    assert.equal(matchesFact(deleted, { ...query, include_deleted: true }), true)
})

test("pinned memory sorts first and the rest by recency", () => {
    const old = fact({ id: "a", updated_at: "2026-09-01T00:00:00+00:00" })
    const recent = fact({ id: "b", updated_at: "2026-09-18T00:00:00+00:00" })
    const pinned = fact({ id: "c", pinned: true, updated_at: "2026-08-01T00:00:00+00:00" })
    const sorted = sortFacts([old, recent, pinned])
    assert.deepEqual(
        sorted.map((entry) => entry.id),
        ["c", "b", "a"]
    )
})

// -------------------------------------------------------------------- labels

test("every enum value maps to its own label key", () => {
    const scopeKeys = MEMORY_SCOPES.map(scopeLabelKey)
    assert.deepEqual(scopeKeys, ["memory-scope-global", "memory-scope-jarvis", "memory-scope-altron"])

    const categoryKeys = MEMORY_CATEGORIES.map(categoryLabelKey)
    assert.equal(categoryKeys.length, 6)
    assert.ok(categoryKeys.includes("memory-category-personal-fact"))
    assert.equal(new Set(categoryKeys).size, 6)

    const sourceKeys = MEMORY_SOURCES.map(sourceLabelKey)
    assert.ok(sourceKeys.includes("memory-source-suggested-from-conversation"))
    assert.equal(new Set(sourceKeys).size, 3)

    assert.deepEqual(CANDIDATE_STATES.map(candidateStateLabelKey), [
        "memory-candidate-pending",
        "memory-candidate-approved",
        "memory-candidate-rejected"
    ])
    assert.deepEqual(MESSAGE_STATUSES.map(messageStatusLabelKey), [
        "memory-message-completed",
        "memory-message-cancelled",
        "memory-message-failed"
    ])
    assert.equal(contextSectionLabelKey("question"), "memory-context-question")
    assert.equal(contextSectionLabelKey("memory"), "memory-context-memory")
    assert.equal(storageStateLabelKey("key_missing"), "memory-storage-key-missing")
})

test("every secret kind maps to a description key", () => {
    const keys = SECRET_KINDS.map(secretKindLabelKey)
    assert.equal(keys.length, 7)
    assert.equal(new Set(keys).size, 7)
    assert.ok(keys.includes("memory-secret-password-assignment"))
    // Kinds and their messages are content-free by construction.
    for (const key of keys) {
        assert.ok(key.startsWith("memory-secret-"))
        assert.ok(!key.includes("FICTIONAL"))
    }
})

// -------------------------------------------------------------------- states

test("the storage gate is read from the shared state", () => {
    const unlocked = statusView()
    assert.equal(needsUnlock(unlocked), false)
    assert.equal(needsSetup(unlocked), false)
    assert.equal(isKeyMissing(unlocked), false)
    assert.equal(memoryIsUsable(unlocked), true)
    assert.equal(canWriteHistory(unlocked), true)
    assert.equal(canUseFacts(unlocked), true)

    const locked = statusView({
        storage: { ...unlocked.storage, state: "locked" },
        memory: { ...unlocked.memory, unlocked: false, stats: emptyStats() }
    })
    assert.equal(needsUnlock(locked), true)
    assert.equal(memoryIsUsable(locked), false)
    assert.equal(canWriteHistory(locked), false)
    assert.equal(canUseFacts(locked), false)

    const uninitialized = statusView({ storage: { ...unlocked.storage, state: "uninitialized" } })
    assert.equal(needsSetup(uninitialized), true)

    const missing = statusView({ storage: { ...unlocked.storage, state: "key_missing" } })
    assert.equal(isKeyMissing(missing), true)

    assert.equal(needsUnlock(null), false)
    assert.equal(memoryIsUsable(null), false)
    assert.equal(canWriteHistory(null), false)
    assert.equal(canUseFacts(null), false)
})

test("switching memory off stops writes and reads without deleting anything", () => {
    const off = statusView()
    off.memory = { ...off.memory, settings: { ...off.memory.settings, enabled: false } }
    assert.equal(memoryIsUsable(off), false)
    assert.equal(canWriteHistory(off), false)
    assert.equal(canUseFacts(off), false)
    // The storage is still unlocked, so the settings remain editable.
    assert.equal(needsUnlock(off), false)
})

test("history and long-term memory can be switched separately", () => {
    const historyOnly = statusView()
    historyOnly.memory = {
        ...historyOnly.memory,
        settings: { ...historyOnly.memory.settings, use_long_term_memory: false }
    }
    assert.equal(canWriteHistory(historyOnly), true)
    assert.equal(canUseFacts(historyOnly), false)

    const factsOnly = statusView()
    factsOnly.memory = {
        ...factsOnly.memory,
        settings: { ...factsOnly.memory.settings, save_history: false }
    }
    assert.equal(canWriteHistory(factsOnly), false)
    assert.equal(canUseFacts(factsOnly), true)
})

test("a running summary is reported per conversation", () => {
    const view = statusView({ summarizing: ["c1"] })
    assert.equal(isSummarizing(view, "c1"), true)
    assert.equal(isSummarizing(view, "c2"), false)
    assert.equal(isSummarizing(view, null), false)
    assert.equal(isSummarizing(null, "c1"), false)
})

// ------------------------------------------------------------------ warnings

test("a secret warning names kinds and never a value", () => {
    const keys = secretWarningKeys(["private_key", "jwt"])
    assert.deepEqual(keys, ["memory-secret-private-key", "memory-secret-jwt"])
    assert.deepEqual(secretWarningKeys([]), ["memory-secret-unknown"])
    assert.equal(isSecretConfirmation("this looks like it contains an API token; confirm to save it anyway"), true)
    assert.equal(isSecretConfirmation("secret_confirmation_required"), true)
    assert.equal(isSecretConfirmation("the entry does not exist"), false)
})

test("a large memory produces a cost warning", () => {
    assert.equal(largeMemoryWarningKey(emptyStats()), null)
    assert.equal(
        largeMemoryWarningKey({ ...emptyStats(), facts: LINEAR_SEARCH_WARNING_FACTS + 1 }),
        "memory-warning-many-facts"
    )
    assert.equal(
        largeMemoryWarningKey({ ...emptyStats(), messages: LINEAR_SEARCH_WARNING_MESSAGES + 1 }),
        "memory-warning-many-messages"
    )
})

test("context warnings map to translated messages", () => {
    assert.equal(contextWarningKey("instruction_like_fact"), "memory-warning-instruction-like")
    assert.equal(contextWarningKey("summary_stale"), "memory-warning-summary-stale")
    assert.equal(contextWarningKey("summary_dropped"), "memory-warning-summary-dropped")
    assert.equal(contextWarningKey("low_relevance_fact"), "memory-warning-low-relevance")
    assert.equal(contextWarningKey("something-new"), "memory-warning-generic")
})

test("an incomplete answer is labelled by how it ended", () => {
    assert.equal(answerNoteKey("cancelled", true), "memory-answer-cancelled-partial")
    assert.equal(answerNoteKey("cancelled", false), "memory-answer-cancelled")
    assert.equal(answerNoteKey("failed", false), "memory-answer-failed")
    // A completed answer needs no note.
    assert.equal(answerNoteKey("completed", false), null)
    // Keeping a partial answer is always an explicit decision.
    assert.equal(KEEP_PARTIAL_ANSWER_DEFAULT, false)
})

test("destructive actions ask with their own message", () => {
    assert.equal(bulkDeleteConfirmationKey("history"), "memory-confirm-history")
    assert.equal(bulkDeleteConfirmationKey("conversation"), "memory-confirm-conversation")
    assert.equal(bulkDeleteConfirmationKey("fact"), "memory-confirm-fact")
})

// ------------------------------------------------------------------- budgets

test("the budget is split into labelled rows", () => {
    const budget = {
        context_size: 8192,
        system_prompt: 400,
        response_reserve: 1024,
        safety_reserve: 256,
        available: 6512,
        memory: 512,
        summary: 1628,
        recent_messages: 4372
    }
    const rows = budgetBreakdown(budget)
    assert.equal(rows.length, 6)
    assert.deepEqual(
        rows.map((row) => row.key),
        [
            "memory-budget-system",
            "memory-budget-reserve",
            "memory-budget-safety",
            "memory-budget-memory",
            "memory-budget-summary",
            "memory-budget-history"
        ]
    )
    const memory = rows.find((row) => row.key === "memory-budget-memory")
    assert.equal(memory.tokens, 512)
    assert.ok(memory.percent > 6 && memory.percent < 7)
    assert.equal(budgetBreakdown(null).length, 0)
})

test("token counts are compact", () => {
    assert.equal(formatTokens(0), "0")
    assert.equal(formatTokens(512), "512")
    assert.equal(formatTokens(1500), "1.5k")
    assert.equal(formatTokens(Number.NaN), "0")
})

test("the dashboard lists counts and hides empty categories", () => {
    const stats = emptyStats()
    assert.deepEqual(statsRowKeys(stats), [
        "memory-stats-conversations",
        "memory-stats-messages",
        "memory-stats-facts"
    ])

    const full = {
        conversations: 3,
        archived: 1,
        messages: 42,
        facts: 5,
        pending_candidates: 2,
        disabled_facts: 1,
        trashed_facts: 1,
        unreadable: 1
    }
    const keys = statsRowKeys(full)
    assert.equal(keys.length, 7)
    for (const key of keys) {
        assert.ok(statsRowValue(full, key) > 0, `${key} must report a positive count`)
    }
    assert.equal(statsRowValue(full, "memory-stats-messages"), 42)
    assert.equal(statsRowValue(full, "memory-stats-facts"), 5)
    assert.equal(statsRowValue(full, "memory-stats-unknown"), 0)
})

// ------------------------------------------------------------ export/import

test("an export warns that the file is only ciphertext", () => {
    assert.deepEqual(exportWarningKeys({ path: "", records: 0 }, true), ["memory-export-cancelled"])
    assert.deepEqual(exportWarningKeys({ path: "", records: 0 }, false), ["memory-export-cancelled"])
    const keys = exportWarningKeys({ path: "C:/x.json", records: 12 }, false)
    assert.ok(keys.includes("memory-export-encrypted-only"))
    assert.ok(keys.includes("memory-export-needs-key"))
})

test("an import reports what happened, including secret suspects", () => {
    assert.deepEqual(importWarningKeys({ applied: 0, conflicts: 0, skipped: 0, secret_suspects: 0 }), [])
    const keys = importWarningKeys({ applied: 4, conflicts: 1, skipped: 2, secret_suspects: 1 })
    assert.deepEqual(keys, [
        "memory-import-applied",
        "memory-import-conflicts",
        "memory-import-skipped",
        "memory-import-secret-suspects"
    ])
})

// ------------------------------------------------------------- presentation

test("conversation titles fall back to a placeholder", () => {
    assert.equal(conversationTitle("  Мой проект ", "New conversation"), "Мой проект")
    assert.equal(conversationTitle("   ", "New conversation"), "New conversation")
})

test("changing the assistant starts a new conversation", () => {
    // Memory is scoped by profile; rewriting an existing conversation under another
    // assistant would mix the two private areas.
    assert.equal(profileSwitchRequiresNewConversation(), true)
    assert.equal(defaultScopeFor("jarvis"), "jarvis")
    assert.equal(defaultScopeFor("altron"), "altron")
})
