/**
 * Interface-side logic for the local autocorrect feature.
 *
 * Like `memory-model.ts` and `local-ai-model.ts`, everything here is free of Tauri and
 * Svelte dependencies, so it can be unit tested with the Node test runner
 * (`npm run test:ui`). It holds the shape of what the backend sends, the decisions the
 * interface must make consistently, and the label keys every view needs.
 *
 * Four rules are encoded here on purpose:
 *
 * * **nothing is applied by a view.** `correctionsFromIssues` only builds the payload the
 *   user asked for; the text changes in Rust, under a version guard, and the view shows
 *   what came back;
 * * **a selection is always expressed in the range the backend reported**, never in a
 *   range the view recomputed, so an emoji or a combining mark cannot shift a highlight;
 * * **an AI change is a preview first.** `isPreviewApplicable` is the only gate, and it
 *   refuses a preview that changed nothing or was cancelled;
 * * **nothing is persisted in the browser**: no storage, no URL, no cookie. The word list
 *   lives in the encrypted database and is read through the backend.
 */

import type { StorageState } from "./memory-model"

/** Languages the checker knows. */
export type Language = "russian" | "english"

/** Which dictionaries a check uses. */
export type LanguageMode = "russian" | "english" | "auto" | "mixed"

/** Why a word was reported. */
export type IssueReason =
    | "unknown_word"
    | "double_capital"
    | "repeated_space"
    | "space_before_punctuation"
    | "user_rule"
    | "ai_improvement"

/** Where a suggestion came from. */
export type SuggestionSource =
    | "dictionary"
    | "user_dictionary"
    | "user_rule"
    | "text_rule"
    | "yo_variant"

/** What happened to one requested correction. */
export type CorrectionOutcome = "applied" | "mismatched" | "overlapped"

/** What the user asked the model to do. */
export type ImprovementMode =
    | "correct_spelling"
    | "correct_grammar"
    | "make_clearer"
    | "make_shorter"
    | "make_formal"
    | "custom_instruction"

/** Something the user should know before confirming a preview. */
export type ImprovementWarning =
    | "unchanged"
    | "large_change"
    | "truncated_diff"
    | "instruction_like_source"
    | "longer_answer"

/** Which part of the interface asked for a check. */
export type CheckOrigin = "notes" | "chat"

/** Kind of a difference segment. */
export type DiffKind = "same" | "added" | "removed"

export interface TextRange {
    start: number
    end: number
}

/** The same range in UTF-16 code units, which is what a textarea uses. */
export interface Utf16Range {
    start: number
    end: number
}

export interface Suggestion {
    text: string
    source: SuggestionSource
}

export interface SpellingIssue {
    word: string
    range: TextRange
    utf16: Utf16Range
    language: Language | null
    suggestions: Suggestion[]
    reason: IssueReason
    auto_fixable: boolean
}

export interface CheckReport {
    version: string
    enabled: boolean
    issues: SpellingIssue[]
    auto_fixable: number
    words_checked: number
    words_skipped: number
    words_unverified: number
    unavailable: Language[]
    truncated: boolean
    timed_out: boolean
    elapsed_ms: number
}

export interface Correction {
    range: TextRange
    original: string
    replacement: string
    reason: IssueReason | null
}

export interface AppliedCorrection {
    id: string
    before: string
    after: string
    range: TextRange
    version_before: string
    version_after: string
    applied_at: string
    reason: IssueReason
}

export interface SkippedCorrection {
    range: TextRange
    original: string
    outcome: CorrectionOutcome
}

export interface CorrectionBatch {
    before: string
    after: string
    applied: AppliedCorrection[]
    skipped: SkippedCorrection[]
    version_before: string
    version_after: string
}

export interface UndoOutcome {
    text: string
    restored: AppliedCorrection[]
    version_before: string
    version_after: string
    remaining: number
}

export interface DiffSegment {
    kind: DiffKind
    text: string
}

export interface TextDiff {
    segments: DiffSegment[]
    added_words: number
    removed_words: number
    truncated: boolean
}

export interface TextImprovementPreview {
    mode: ImprovementMode
    source: string
    suggestion: string
    diff: TextDiff
    warnings: ImprovementWarning[]
    version_before: string
    version_after: string
    provider: string
    model: string | null
    duration_ms: number
    cancelled: boolean
    applied: boolean
}

export interface CustomRule {
    pattern: string
    replacement: string
    auto_apply: boolean
}

export interface AutocorrectSettings {
    enabled: boolean
    check_notes: boolean
    check_chat: boolean
    language: LanguageMode
    mixed_mode: boolean
    max_suggestions: number
    max_issues: number
    timeout_ms: number
    debounce_ms: number
    safe_autocorrect: boolean
    ai_improvement: boolean
    require_preview: boolean
    dictionary_dir: string | null
    custom_rules: CustomRule[]
    schema_version: number
}

export interface UserDictionaryEntry {
    id: string
    revision: number
    word: string
    language: Language
    imported: boolean
    created_at: string
}

export interface UserDictionaryStats {
    words: number
    russian: number
    english: number
    imported: number
    ignored: number
    unreadable: number
    decrypted_in_memory: boolean
}

export interface DictionaryState {
    state: "ready" | "missing" | "invalid"
    language: Language
    aff?: string
    dic?: string
    words?: number | null
    source?: string | null
    expected?: string[]
    reason?: string
}

export interface AutocorrectStatus {
    unlocked: boolean
    has_stored_words: boolean
    settings: AutocorrectSettings
    dictionaries: DictionaryState[]
    dictionary_dir: string
    unavailable: Language[]
    user: UserDictionaryStats
    ai_improvement_available: boolean
}

export interface AutocorrectStatusView {
    storage: {
        state: StorageState
        dpapi_available: boolean
        backup_available: boolean
        has_stored_data: boolean
        data_dir: string
    }
    autocorrect: AutocorrectStatus
    ai_running: boolean
}

export interface CheckView {
    report: CheckReport
    auto_corrections: Correction[]
    enabled_for_origin: boolean
}

export interface UndoStatus {
    can_undo: boolean
    batches: number
    words: string[]
    capacity: number
}

export interface WordExportResult {
    path: string
    words: number
    plaintext: boolean
}

export interface DictionaryExportResult {
    path: string
    records: number
}

export interface ImportOutcome {
    added: number
    duplicates: number
    invalid: number
    limit_reached: boolean
}

/** Settings bounds, mirroring `AutocorrectSettings` in the core. */
export const AUTOCORRECT_SETTINGS_SCHEMA_VERSION = 1
export const MIN_MAX_SUGGESTIONS = 1
export const MAX_MAX_SUGGESTIONS = 8
export const MIN_MAX_ISSUES = 10
export const MAX_MAX_ISSUES = 500
export const MIN_TIMEOUT_MS = 100
export const MAX_TIMEOUT_MS = 30000
export const MIN_DEBOUNCE_MS = 0
export const MAX_DEBOUNCE_MS = 5000

export const DEFAULT_MAX_SUGGESTIONS = 5
export const DEFAULT_MAX_ISSUES = 200
export const DEFAULT_TIMEOUT_MS = 3000
export const DEFAULT_DEBOUNCE_MS = 500

/** Limits the interface shows instead of discovering them by failing. */
export const MAX_DICTIONARY_WORD_CHARS = 64
export const MAX_USER_DICTIONARY_ENTRIES = 20000
export const MAX_IMPROVE_CHARS = 20000
export const MAX_CHECK_CHARS = 200000
export const MAX_JOURNAL_BATCHES = 20

export const LANGUAGES: readonly Language[] = ["russian", "english"]
export const LANGUAGE_MODES: readonly LanguageMode[] = ["russian", "english", "auto", "mixed"]
export const ISSUE_REASONS: readonly IssueReason[] = [
    "unknown_word",
    "double_capital",
    "repeated_space",
    "space_before_punctuation",
    "user_rule",
    "ai_improvement"
]
export const SUGGESTION_SOURCES: readonly SuggestionSource[] = [
    "dictionary",
    "user_dictionary",
    "user_rule",
    "text_rule",
    "yo_variant"
]
export const CORRECTION_OUTCOMES: readonly CorrectionOutcome[] = [
    "applied",
    "mismatched",
    "overlapped"
]
export const IMPROVEMENT_MODES: readonly ImprovementMode[] = [
    "correct_spelling",
    "correct_grammar",
    "make_clearer",
    "make_shorter",
    "make_formal",
    "custom_instruction"
]
export const IMPROVEMENT_WARNINGS: readonly ImprovementWarning[] = [
    "unchanged",
    "large_change",
    "truncated_diff",
    "instruction_like_source",
    "longer_answer"
]
export const DIFF_KINDS: readonly DiffKind[] = ["same", "added", "removed"]

/** The modes that change the wording rather than the spelling. */
export const REWRITE_MODES: readonly ImprovementMode[] = [
    "make_clearer",
    "make_shorter",
    "make_formal"
]

/** Defaults the settings form starts from, matching the core. */
export function defaultSettings(): AutocorrectSettings {
    return {
        enabled: true,
        check_notes: true,
        check_chat: true,
        language: "auto",
        mixed_mode: false,
        max_suggestions: DEFAULT_MAX_SUGGESTIONS,
        max_issues: DEFAULT_MAX_ISSUES,
        timeout_ms: DEFAULT_TIMEOUT_MS,
        debounce_ms: DEFAULT_DEBOUNCE_MS,
        safe_autocorrect: false,
        ai_improvement: false,
        require_preview: true,
        dictionary_dir: null,
        custom_rules: [],
        schema_version: AUTOCORRECT_SETTINGS_SCHEMA_VERSION
    }
}

function clamp(value: number, min: number, max: number, fallback: number): number {
    if (!Number.isFinite(value)) return fallback
    return Math.min(Math.max(Math.trunc(value), min), max)
}

/**
 * Repairs a settings object that came from the backend or from an older build.
 *
 * The core clamps as well; doing it here means the form shows the values that will be
 * stored instead of jumping after a save.
 */
export function normalizeSettings(input: Partial<AutocorrectSettings> | null): AutocorrectSettings {
    const base = { ...defaultSettings(), ...(input ?? {}) }
    return {
        enabled: Boolean(base.enabled),
        check_notes: Boolean(base.check_notes),
        check_chat: Boolean(base.check_chat),
        language: LANGUAGE_MODES.includes(base.language) ? base.language : "auto",
        mixed_mode: Boolean(base.mixed_mode),
        max_suggestions: clamp(base.max_suggestions, MIN_MAX_SUGGESTIONS, MAX_MAX_SUGGESTIONS, DEFAULT_MAX_SUGGESTIONS),
        max_issues: clamp(base.max_issues, MIN_MAX_ISSUES, MAX_MAX_ISSUES, DEFAULT_MAX_ISSUES),
        timeout_ms: clamp(base.timeout_ms, MIN_TIMEOUT_MS, MAX_TIMEOUT_MS, DEFAULT_TIMEOUT_MS),
        debounce_ms: clamp(base.debounce_ms, MIN_DEBOUNCE_MS, MAX_DEBOUNCE_MS, DEFAULT_DEBOUNCE_MS),
        // The core never accepts either of these in a different state.
        safe_autocorrect: Boolean(base.safe_autocorrect),
        ai_improvement: Boolean(base.ai_improvement),
        require_preview: true,
        dictionary_dir: base.dictionary_dir && String(base.dictionary_dir).trim().length > 0
            ? String(base.dictionary_dir).trim()
            : null,
        custom_rules: Array.isArray(base.custom_rules)
            ? base.custom_rules.filter((rule) => rule && rule.pattern && rule.replacement)
            : [],
        schema_version: AUTOCORRECT_SETTINGS_SCHEMA_VERSION
    }
}



// ------------------------------------------------------------------ label keys

/**
 * The part of a key that comes from a value list.
 *
 * Fluent identifiers and this project's message keys use hyphens, while the values the
 * backend sends use underscores (`unknown_word`), so the underscore is translated here in
 * one place instead of in every component.
 */
function keyPart(value: string): string {
    return value.replace(/_/g, "-")
}

export function languageLabelKey(language: Language): string {
    return `autocorrect-language-${keyPart(language)}`
}

export function languageModeLabelKey(mode: LanguageMode): string {
    return `autocorrect-mode-${keyPart(mode)}`
}

export function reasonLabelKey(reason: IssueReason): string {
    return `autocorrect-reason-${keyPart(reason)}`
}

export function sourceLabelKey(source: SuggestionSource): string {
    return `autocorrect-source-${keyPart(source)}`
}

export function outcomeLabelKey(outcome: CorrectionOutcome): string {
    return `autocorrect-outcome-${keyPart(outcome)}`
}

export function improvementModeLabelKey(mode: ImprovementMode): string {
    return `autocorrect-improve-${keyPart(mode)}`
}

export function improvementWarningLabelKey(warning: ImprovementWarning): string {
    return `autocorrect-warning-${keyPart(warning)}`
}

export function diffKindLabelKey(kind: DiffKind): string {
    return `autocorrect-diff-${keyPart(kind)}`
}

// ------------------------------------------------------------------- decisions

/** How many issues of a report are worth showing before a "more" hint. */
export const VISIBLE_ISSUE_LIMIT = 40

/** Whether a check found anything at all. */
export function reportIsClean(report: CheckReport | null): boolean {
    return !report || report.issues.length === 0
}

/** Whether a report carries an issue that a user could accept. */
export function hasActionableIssue(report: CheckReport | null): boolean {
    if (!report) return false
    return report.issues.some((issue) => issue.suggestions.length > 0)
}

/**
 * Whether the text moved since the check that produced the report.
 *
 * The version the backend reports is a hash the interface cannot recompute, so the view
 * keeps the text it checked and compares it: the same text means the same version, and a
 * correction is only offered for the text it was computed on.
 */
export function textChangedSinceCheck(checkedText: string, currentText: string): boolean {
    return checkedText !== currentText
}

/** How a report reads in one line: issues, checked words, missing dictionaries. */
export function reportSummaryKey(report: CheckReport | null): string {
    if (!report) return "autocorrect-summary-idle"
    if (!report.enabled) return "autocorrect-summary-off"
    if (report.issues.length === 0) return "autocorrect-summary-clean"
    if (report.unavailable.length > 0 && report.words_checked === report.words_unverified) {
        return "autocorrect-summary-no-dictionary"
    }
    if (report.timed_out) return "autocorrect-summary-timeout"
    if (report.truncated) return "autocorrect-summary-truncated"
    return "autocorrect-summary-issues"
}

/** A dictionary state in one word, for the settings page. */
export function dictionaryStateKey(state: DictionaryState): string {
    return `autocorrect-dictionary-${state.state}`
}

/** Whether a language can be checked with the dictionaries that are installed. */
export function languageIsReady(dictionaries: DictionaryState[], language: Language): boolean {
    return dictionaries.some((entry) => entry.language === language && entry.state === "ready")
}

/** Whether the status says a dictionary is missing for at least one language. */
export function missesAnyDictionary(status: AutocorrectStatus | null): boolean {
    return Boolean(status && status.dictionaries.some((entry) => entry.state !== "ready"))
}

/** Whether the storage lock is what is stopping the word list from being used. */
export function wordListLocked(view: AutocorrectStatusView | null): boolean {
    return Boolean(view && !view.autocorrect.unlocked)
}

/**
 * Builds the corrections for the issues the user selected.
 *
 * A correction always carries the text the backend reported at that range, so the core
 * can refuse a stale one instead of replacing the wrong characters.
 */
export function correctionsFromIssues(
    issues: SpellingIssue[],
    selected: ReadonlySet<number>,
    suggestionIndex: ReadonlyMap<number, number>
): Correction[] {
    const corrections: Correction[] = []
    issues.forEach((issue, index) => {
        if (!selected.has(index)) return
        const suggestion = issue.suggestions[suggestionIndex.get(index) ?? 0]
        if (!suggestion) return
        if (suggestion.text === issue.word) return
        corrections.push({
            range: issue.range,
            original: issue.word,
            replacement: suggestion.text,
            reason: issue.reason
        })
    })
    return corrections
}

/** Every suggestion index that may be applied for an issue. */
export function applicableSuggestions(issue: SpellingIssue): number[] {
    return issue.suggestions
        .map((suggestion, index) => (suggestion.text === issue.word ? -1 : index))
        .filter((index) => index >= 0)
}

/** A short description of what an undo would put back. */
export function undoLabel(words: string[]): string {
    const visible = words.filter((word) => word.length > 0)
    if (visible.length === 0) return ""
    if (visible.length === 1) return visible[0]
    return `${visible[0]} +${visible.length - 1}`
}

/** Whether a preview may be applied: it must change something and not be cancelled. */
export function isPreviewApplicable(preview: TextImprovementPreview | null): boolean {
    if (!preview) return false
    if (preview.cancelled || preview.applied) return false
    if (preview.suggestion.trim().length === 0) return false
    return preview.diff.added_words > 0 || preview.diff.removed_words > 0
}

/** Whether a preview is a rewrite rather than a correction, for a stronger warning. */
export function previewIsRewrite(preview: TextImprovementPreview | null): boolean {
    if (!preview) return false
    return preview.warnings.includes("large_change") || REWRITE_MODES.includes(preview.mode)
}

/** The change counts of a preview, as one line. */
export function previewStatsLine(preview: TextImprovementPreview | null): string {
    if (!preview) return ""
    return `+${preview.diff.added_words} / -${preview.diff.removed_words}`
}

/** The custom instruction is required, and the core refuses an empty one. */
export function instructionIsUsable(mode: ImprovementMode, instruction: string): boolean {
    if (mode !== "custom_instruction") return true
    const trimmed = instruction.trim()
    return trimmed.length > 0 && trimmed.length <= 500
}

/** Whether the text is too long for one improvement. */
export function textTooLongForImprovement(text: string): boolean {
    return [...text].length > MAX_IMPROVE_CHARS
}

/** Whether a word list entry may be added. */
export function wordIsAcceptable(word: string): boolean {
    const trimmed = word.trim()
    if (trimmed.length === 0) return false
    if ([...trimmed].length > MAX_DICTIONARY_WORD_CHARS) return false
    if (/\s/.test(trimmed)) return false
    return /\p{L}/u.test(trimmed)
}

/** The language a word most likely belongs to, by its script. */
export function languageOfWord(word: string): Language | null {
    let cyrillic = 0
    let latin = 0
    for (const character of word) {
        if (!/\p{L}/u.test(character)) continue
        if (/[\u0400-\u04FF\u0500-\u052F]/u.test(character)) cyrillic += 1
        else if (/[A-Za-z]/.test(character)) latin += 1
    }
    if (cyrillic === 0 && latin === 0) return null
    if (cyrillic === 0) return "english"
    if (latin === 0) return "russian"
    return cyrillic >= latin ? "russian" : "english"
}

/** Whether an issue is one the interface may offer to fix in one click. */
export function issueIsOneClickFix(issue: SpellingIssue): boolean {
    if (!issue.auto_fixable) return false
    return applicableSuggestions(issue).length > 0
}

/** The scope label one editor uses for its undo journal. */
export function scopeForNote(noteId: string): string {
    return `note:${noteId}`
}

/** The scope label the chat draft uses for its undo journal. */
export const CHAT_SCOPE = "chat-draft"

/** Whether a scope label is acceptable to the backend. */
export function scopeIsAcceptable(scope: string): boolean {
    const trimmed = scope.trim()
    return trimmed.length > 0 && trimmed.length <= 96
}
