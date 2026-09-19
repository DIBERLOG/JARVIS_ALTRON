import { test } from "node:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    CHAT_SCOPE,
    DEFAULT_MAX_ISSUES,
    DEFAULT_MAX_SUGGESTIONS,
    IMPROVEMENT_MODES,
    ISSUE_REASONS,
    LANGUAGES,
    LANGUAGE_MODES,
    MAX_IMPROVE_CHARS,
    SUGGESTION_SOURCES,
    applicableSuggestions,
    correctionsFromIssues,
    defaultSettings,
    dictionaryStateKey,
    instructionIsUsable,
    isPreviewApplicable,
    issueIsOneClickFix,
    languageOfWord,
    missesAnyDictionary,
    normalizeSettings,
    previewIsRewrite,
    previewStatsLine,
    reportIsClean,
    reportSummaryKey,
    scopeForNote,
    scopeIsAcceptable,
    textChangedSinceCheck,
    textTooLongForImprovement,
    undoLabel,
    wordIsAcceptable,
    wordListLocked
} from "../src/lib/autocorrect-model.ts"

const MODEL_FILE = fileURLToPath(new URL("../src/lib/autocorrect-model.ts", import.meta.url))

/** A minimal report, shaped like the one the backend sends. */
function report(overrides = {}) {
    return {
        version: "0123456789abcdef",
        enabled: true,
        issues: [],
        auto_fixable: 0,
        words_checked: 3,
        words_skipped: 1,
        words_unverified: 0,
        unavailable: [],
        truncated: false,
        timed_out: false,
        elapsed_ms: 4,
        ...overrides
    }
}

function issue(overrides = {}) {
    return {
        word: "привт",
        range: { start: 0, end: 5 },
        utf16: { start: 0, end: 5 },
        language: "russian",
        suggestions: [
            { text: "привет", source: "dictionary" },
            { text: "привит", source: "user_dictionary" }
        ],
        reason: "unknown_word",
        auto_fixable: false,
        ...overrides
    }
}

function preview(overrides = {}) {
    return {
        mode: "correct_spelling",
        source: "привт мир",
        suggestion: "привет мир",
        diff: {
            segments: [
                { kind: "removed", text: "привт " },
                { kind: "added", text: "привет " },
                { kind: "same", text: "мир" }
            ],
            added_words: 1,
            removed_words: 1,
            truncated: false
        },
        warnings: [],
        version_before: "aaaa",
        version_after: "bbbb",
        provider: "local-ai",
        model: null,
        duration_ms: 1200,
        cancelled: false,
        applied: false,
        ...overrides
    }
}

test("the defaults keep every automatic behaviour off", () => {
    const settings = defaultSettings()
    assert.equal(settings.enabled, true)
    assert.equal(settings.check_notes, true)
    assert.equal(settings.check_chat, true)
    // Nothing is rewritten without the user.
    assert.equal(settings.safe_autocorrect, false)
    assert.equal(settings.ai_improvement, false)
    // And a preview can never be switched off.
    assert.equal(settings.require_preview, true)
    assert.equal(settings.max_suggestions, DEFAULT_MAX_SUGGESTIONS)
    assert.equal(settings.max_issues, DEFAULT_MAX_ISSUES)
})

test("out of range settings are clamped and a preview cannot be disabled", () => {
    const repaired = normalizeSettings({
        max_suggestions: 99,
        max_issues: 1,
        timeout_ms: 1,
        debounce_ms: 999999,
        require_preview: false,
        language: "klingon",
        dictionary_dir: "   ",
        custom_rules: [
            { pattern: "этта", replacement: "это", auto_apply: true },
            { pattern: "", replacement: "x", auto_apply: true }
        ]
    })
    assert.equal(repaired.max_suggestions, 8)
    assert.equal(repaired.max_issues, 10)
    assert.equal(repaired.timeout_ms, 100)
    assert.equal(repaired.debounce_ms, 5000)
    assert.equal(repaired.require_preview, true)
    assert.equal(repaired.language, "auto")
    assert.equal(repaired.dictionary_dir, null)
    assert.equal(repaired.custom_rules.length, 1)
})

test("an empty or damaged settings object yields the defaults", () => {
    assert.deepEqual(normalizeSettings(null), defaultSettings())
    assert.deepEqual(normalizeSettings(undefined), defaultSettings())
    const partial = normalizeSettings({ safe_autocorrect: true })
    assert.equal(partial.safe_autocorrect, true)
    assert.equal(partial.enabled, true)
})

test("a correction carries the range and the original text the backend reported", () => {
    const issues = [issue(), issue({ word: "мир", range: { start: 6, end: 9 }, auto_fixable: true })]
    const corrections = correctionsFromIssues(
        issues,
        new Set([1]),
        new Map([[1, 1]])
    )
    assert.equal(corrections.length, 1)
    assert.deepEqual(corrections[0].range, { start: 6, end: 9 })
    assert.equal(corrections[0].original, "мир")
    assert.equal(corrections[0].replacement, "привит")
    assert.equal(corrections[0].reason, "unknown_word")
})

test("nothing is corrected for an issue that was not selected", () => {
    const issues = [issue()]
    assert.deepEqual(correctionsFromIssues(issues, new Set(), new Map()), [])
    // A suggestion equal to the word itself is not a change.
    const same = issue({ suggestions: [{ text: "привт", source: "dictionary" }] })
    assert.deepEqual(correctionsFromIssues([same], new Set([0]), new Map()), [])
})

test("only suggestions that change the word are offered", () => {
    const same = issue({ suggestions: [{ text: "привт", source: "dictionary" }] })
    assert.deepEqual(applicableSuggestions(same), [])
    assert.deepEqual(applicableSuggestions(issue()), [0, 1])
    // An unknown word is never a one-click fix, even with suggestions.
    assert.equal(issueIsOneClickFix(issue()), false)
    // A double capital is.
    assert.equal(issueIsOneClickFix(issue({ reason: "double_capital", auto_fixable: true })), true)
})

test("a report is summarised by what it means, not by its size", () => {
    assert.equal(reportSummaryKey(null), "autocorrect-summary-idle")
    assert.equal(reportSummaryKey(report({ enabled: false })), "autocorrect-summary-off")
    assert.equal(reportSummaryKey(report()), "autocorrect-summary-clean")
    assert.equal(
        reportSummaryKey(report({ issues: [issue()] })),
        "autocorrect-summary-issues"
    )
    assert.equal(
        reportSummaryKey(report({ issues: [issue()], truncated: true })),
        "autocorrect-summary-truncated"
    )
    assert.equal(
        reportSummaryKey(report({ issues: [issue()], timed_out: true })),
        "autocorrect-summary-timeout"
    )
    assert.equal(
        reportSummaryKey(
            report({
                issues: [issue()],
                unavailable: ["russian", "english"],
                words_checked: 2,
                words_unverified: 2
            })
        ),
        "autocorrect-summary-no-dictionary"
    )
    assert.equal(reportIsClean(report()), true)
    assert.equal(reportIsClean(report({ issues: [issue()] })), false)
})

test("a changed text invalidates the check that was made for the old one", () => {
    assert.equal(textChangedSinceCheck("привт мир", "привт мир"), false)
    assert.equal(textChangedSinceCheck("привт мир", "привт мир!"), true)
})

test("only a good preview may be applied", () => {
    assert.equal(isPreviewApplicable(preview()), true)
    assert.equal(isPreviewApplicable(null), false)
    // Unchanged text is not a change.
    assert.equal(
        isPreviewApplicable(
            preview({
                suggestion: "привт мир",
                diff: { segments: [], added_words: 0, removed_words: 0, truncated: false }
            })
        ),
        false
    )
    // A cancelled generation is not applicable either.
    assert.equal(isPreviewApplicable(preview({ cancelled: true })), false)
    // Neither is a preview that was already applied.
    assert.equal(isPreviewApplicable(preview({ applied: true })), false)
    // Nor an empty answer.
    assert.equal(isPreviewApplicable(preview({ suggestion: "   " })), false)
    assert.equal(previewStatsLine(preview()), "+1 / -1")
    assert.equal(previewIsRewrite(preview()), false)
    assert.equal(previewIsRewrite(preview({ mode: "make_shorter" })), true)
    assert.equal(previewIsRewrite(preview({ warnings: ["large_change"] })), true)
})

test("a custom instruction is required and bounded", () => {
    assert.equal(instructionIsUsable("correct_spelling", ""), true)
    assert.equal(instructionIsUsable("custom_instruction", ""), false)
    assert.equal(instructionIsUsable("custom_instruction", "   "), false)
    assert.equal(instructionIsUsable("custom_instruction", "сделай короче"), true)
    assert.equal(instructionIsUsable("custom_instruction", "я".repeat(501)), false)
})

test("a text that is too long for one improvement is refused before the round trip", () => {
    assert.equal(textTooLongForImprovement("привет"), false)
    assert.equal(textTooLongForImprovement("я".repeat(MAX_IMPROVE_CHARS)), false)
    assert.equal(textTooLongForImprovement("я".repeat(MAX_IMPROVE_CHARS + 1)), true)
    // The check counts characters, not code units, so an emoji is one.
    assert.equal(textTooLongForImprovement("😀".repeat(MAX_IMPROVE_CHARS)), false)
})

test("a word is only accepted when the backend would accept it", () => {
    assert.equal(wordIsAcceptable("Джарвис"), true)
    assert.equal(wordIsAcceptable("  Джарвис  "), true)
    assert.equal(wordIsAcceptable(""), false)
    assert.equal(wordIsAcceptable("   "), false)
    assert.equal(wordIsAcceptable("два слова"), false)
    assert.equal(wordIsAcceptable("12345"), false)
    assert.equal(wordIsAcceptable("я".repeat(65)), false)
    assert.equal(wordIsAcceptable("я".repeat(64)), true)
})

test("the language of a word follows its script", () => {
    assert.equal(languageOfWord("привет"), "russian")
    assert.equal(languageOfWord("hello"), "english")
    assert.equal(languageOfWord("ЯRVIS"), "english")
    assert.equal(languageOfWord("12345"), null)
    assert.equal(languageOfWord("—"), null)
})

test("a dictionary state and a missing dictionary are visible", () => {
    assert.equal(dictionaryStateKey({ state: "ready", language: "russian" }), "autocorrect-dictionary-ready")
    assert.equal(dictionaryStateKey({ state: "missing", language: "english" }), "autocorrect-dictionary-missing")
    assert.equal(dictionaryStateKey({ state: "invalid", language: "english" }), "autocorrect-dictionary-invalid")
    const status = {
        unlocked: true,
        has_stored_words: false,
        settings: defaultSettings(),
        dictionaries: [
            { state: "ready", language: "russian" },
            { state: "missing", language: "english" }
        ],
        dictionary_dir: "C:/data/jarvis/dictionaries",
        unavailable: ["english"],
        user: {
            words: 0,
            russian: 0,
            english: 0,
            imported: 0,
            ignored: 0,
            unreadable: 0,
            decrypted_in_memory: false
        },
        ai_improvement_available: false
    }
    assert.equal(missesAnyDictionary(status), true)
    assert.equal(
        missesAnyDictionary({ ...status, dictionaries: [{ state: "ready", language: "russian" }] }),
        false
    )
    assert.equal(missesAnyDictionary(null), false)
    assert.equal(wordListLocked({ storage: {}, autocorrect: { ...status, unlocked: false }, ai_running: false }), true)
    assert.equal(wordListLocked({ storage: {}, autocorrect: status, ai_running: false }), false)
})

test("the undo label and the editor scopes are stable", () => {
    assert.equal(undoLabel([]), "")
    assert.equal(undoLabel(["привет"]), "привет")
    assert.equal(undoLabel(["привет", "мир", "дела"]), "привет +2")
    assert.equal(scopeForNote("abc"), "note:abc")
    assert.equal(CHAT_SCOPE, "chat-draft")
    assert.equal(scopeIsAcceptable(CHAT_SCOPE), true)
    assert.equal(scopeIsAcceptable("  "), false)
    assert.equal(scopeIsAcceptable("x".repeat(97)), false)
})

test("every label family is enumerable, so no key can be missed", () => {
    // The i18n test builds the same key set from these lists; keeping them complete is
    // what makes a missing translation a test failure rather than a blank label.
    assert.deepEqual(LANGUAGES, ["russian", "english"])
    assert.equal(LANGUAGE_MODES.length, 4)
    assert.equal(ISSUE_REASONS.length, 6)
    assert.equal(SUGGESTION_SOURCES.length, 5)
    assert.equal(IMPROVEMENT_MODES.length, 6)
    assert.ok(ISSUE_REASONS.includes("ai_improvement"))
})

test("the model file holds no browser storage, no network, and no vault access", () => {
    const source = readFileSync(MODEL_FILE, "utf8")
    for (const needle of [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "document.cookie",
        "fetch(",
        "XMLHttpRequest",
        "WebSocket",
        "http://",
        "https://",
        "@/lib/vault",
        "vaultApi",
        "vault-model"
    ]) {
        assert.equal(source.includes(needle), false, `the model must not use ${needle}`)
    }
})

test("the model layer never talks to Tauri directly", () => {
    const source = readFileSync(MODEL_FILE, "utf8")
    assert.equal(source.includes("@tauri-apps/api"), false, "only the api wrapper may invoke commands")
    assert.equal(source.includes("invoke("), false, "the model is pure logic")
})
