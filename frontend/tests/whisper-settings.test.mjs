import { test } from "node:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    MAX_SECONDS,
    MAX_SILENCE_MS,
    MAX_THREADS,
    MAX_TIMEOUT_SECONDS,
    MIN_SECONDS,
    MIN_SILENCE_MS,
    MIN_THREADS,
    MIN_TIMEOUT_SECONDS,
    NUMERIC_FIELDS,
    defaultSettings,
    draftsAfterPoll,
    isDirty,
    numericBounds,
    numericDrafts,
    parseNumericDraft,
    settingsProblem
} from "../src/lib/whisper-model.ts"

/**
 * The defect these tests pin: the numeric inputs were bound straight to the
 * settings the panel re-reads every two seconds. Typing 3000 in the silence
 * field was overwritten by the next poll before anything was saved, and the
 * field snapped back to 1500 — the value was never sent, so nothing could be
 * saved and nothing could be shown.
 */

const PANEL = fileURLToPath(
    new URL("../src/components/settings/WhisperSettings.svelte", import.meta.url)
)

/** The bounds the core enforces, per field, for the comparison below. */
const CORE_BOUNDS = {
    threads: [MIN_THREADS, MAX_THREADS],
    max_seconds: [MIN_SECONDS, MAX_SECONDS],
    silence_ms: [MIN_SILENCE_MS, MAX_SILENCE_MS],
    timeout_seconds: [MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS]
}

test("every numeric setting has bounds, a step, and a message key", () => {
    for (const field of NUMERIC_FIELDS) {
        const bounds = numericBounds(field)
        assert.deepEqual(
            [bounds.minimum, bounds.maximum],
            CORE_BOUNDS[field],
            `${field} must mirror the core's range`
        )
        assert.ok(bounds.step >= 1, `${field} needs a step`)
        assert.match(bounds.problem, /^whisper-error-[a-z_]+$/)
    }
})

test("a draft starts from the stored document", () => {
    const settings = { ...defaultSettings(), threads: 8, silence_ms: 3000 }
    const drafts = numericDrafts(settings)
    assert.equal(drafts.threads, "8")
    assert.equal(drafts.silence_ms, "3000")
    assert.deepEqual(Object.keys(drafts).sort(), [...NUMERIC_FIELDS].sort())
})

test("3000 is accepted by the field that used to reject it", () => {
    const parsed = parseNumericDraft("silence_ms", "3000")
    assert.deepEqual(parsed, { value: 3000 })
})

test("an out-of-range or unfinished value is refused with its own key", () => {
    assert.deepEqual(parseNumericDraft("silence_ms", "10"), {
        problem: "whisper-error-silence"
    })
    assert.deepEqual(parseNumericDraft("silence_ms", ""), {
        problem: "whisper-error-silence"
    })
    assert.deepEqual(parseNumericDraft("threads", "0"), { problem: "whisper-error-threads" })
    assert.deepEqual(parseNumericDraft("threads", "abc"), { problem: "whisper-error-threads" })
    assert.deepEqual(parseNumericDraft("timeout_seconds", "1"), {
        problem: "whisper-error-timeout"
    })
    assert.deepEqual(parseNumericDraft("max_seconds", "9999"), {
        problem: "whisper-error-seconds"
    })
})

test("a poll refreshes every field except the one being edited", () => {
    const stored = { ...defaultSettings(), threads: 4, silence_ms: 1500 }
    // The person typed 3000 and is still in the field.
    const drafts = { ...numericDrafts(stored), silence_ms: "3000" }
    const afterPoll = draftsAfterPoll(drafts, "silence_ms", {
        ...stored,
        threads: 8,
        silence_ms: 1500
    })
    assert.equal(afterPoll.silence_ms, "3000", "the edited field keeps what was typed")
    assert.equal(afterPoll.threads, "8", "the other fields follow the document")
})

test("no poll can take a half-typed value away, on any field", () => {
    let drafts = numericDrafts(defaultSettings())
    for (const field of NUMERIC_FIELDS) {
        drafts = { ...drafts, [field]: "3000" }
        const afterPoll = draftsAfterPoll(drafts, field, {
            ...defaultSettings(),
            [field]: 1500
        })
        assert.equal(afterPoll[field], "3000", `${field} must survive the poll`)
    }
})

test("a field that is not being edited always shows the stored value", () => {
    const stored = { ...defaultSettings(), threads: 12 }
    const drafts = { ...numericDrafts(defaultSettings()), threads: "999" }
    const afterPoll = draftsAfterPoll(drafts, null, stored)
    assert.equal(afterPoll.threads, "12")
})

test("dirtiness is measured against the document, not against the draft", () => {
    const stored = { ...defaultSettings(), silence_ms: 1500 }
    const dirty = { ...numericDrafts(stored), silence_ms: "3000" }
    assert.equal(isDirty("silence_ms", dirty, stored), true)
    assert.equal(isDirty("not_a_field" in stored ? "threads" : "threads", dirty, stored), false)
    const clean = numericDrafts(stored)
    assert.equal(isDirty("silence_ms", clean, stored), false)
})

test("the whole settings object the core accepts still validates", () => {
    const settings = { ...defaultSettings(), threads: 8, max_seconds: 45, silence_ms: 3000 }
    assert.equal(settingsProblem(settings), null)
})

test("the numbers are committed on blur and on Enter, never on every keystroke", () => {
    const panel = readFileSync(PANEL, "utf8")
    // Every numeric field has a draft, a focus marker, a commit on blur, and
    // Enter. The old binding straight to `settings` is gone.
    for (const field of NUMERIC_FIELDS) {
        assert.ok(
            panel.includes(`bind:value={drafts.${field}}`),
            `${field} must be edited in a draft`
        )
        assert.ok(
            panel.includes(`commitNumeric("${field}")`),
            `${field} must be committed on blur`
        )
    }
    assert.equal(
        /bind:value=\{settings\.(threads|max_seconds|silence_ms|timeout_seconds)\}/.test(panel),
        false,
        "an input must not be bound straight to the polled settings"
    )
    assert.ok(panel.includes("on:keydown={(event) => numericKeydown(event,"))
    // And nothing is stored while a key is held down.
    assert.equal(
        /on:input=\{[^}]*store\(/.test(panel),
        false,
        "no value may be sent on every character"
    )
})

test("the panel says where the recognized text goes", () => {
    const panel = readFileSync(PANEL, "utf8")
    assert.ok(panel.includes("whisper-insert-command"), "the insert button must exist")
    assert.ok(panel.includes("commandDraft.set("), "it must fill the command field")
    assert.ok(panel.includes("whisper-transcript"), "the transcript must be shown")
    // The text is inserted, not sent: no command is dispatched from here.
    for (const forbidden of ["sendTextCommand", "sendAction", "invoke(\"send"]) {
        assert.equal(
            panel.includes(forbidden),
            false,
            `the panel must not send a command (${forbidden})`
        )
    }
})
