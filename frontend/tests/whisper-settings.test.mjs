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
    microphoneLevel,
    microphoneLevelKey,
    numericBounds,
    numericDrafts,
    parseNumericDraft,
    resultPhase,
    resultPhaseKey,
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
    assert.ok(panel.includes("whisper-result-title"), "the transcript must be shown")
    // The text is inserted, not sent: no command is dispatched from here.
    for (const forbidden of ["sendTextCommand", "sendAction", "invoke(\"send"]) {
        assert.equal(
            panel.includes(forbidden),
            false,
            `the panel must not send a command (${forbidden})`
        )
    }
})

// ------------------------------------------------- the result block at the top

test("the result block is always there, in every phase", () => {
    // The five answers the block gives, and nothing else can be one of them.
    assert.equal(resultPhase(null, false, false), "empty")
    assert.equal(resultPhase("recording", false, false), "recording")
    assert.equal(resultPhase("transcribing", false, false), "transcribing")
    assert.equal(resultPhase("idle", true, false), "ready")
    assert.equal(resultPhase("idle", false, true), "failed")
    // A state that is neither recording nor transcribing, with no text: empty.
    assert.equal(resultPhase("idle", false, false), "empty")
    // The text wins over an error: a failure must never take the last text away.
    assert.equal(resultPhase("idle", true, true), "ready")
    for (const phase of ["empty", "recording", "transcribing", "ready", "failed"]) {
        assert.match(resultPhaseKey(phase), /^whisper-result-[a-z]+$/)
    }
})

test("the result block sits under the state panel and above the enable switch", () => {
    const panel = readFileSync(PANEL, "utf8")
    const block = panel.indexOf('class="result"')
    const statePanel = panel.indexOf("{#if status}")
    const enable = panel.indexOf('t("whisper-enabled")')
    assert.ok(statePanel >= 0 && block > statePanel, "the block comes after the state panel")
    assert.ok(enable > block, "and before the switch that turns dictation on")
    // It is not inside the state panel's condition: the array of buttons and
    // sentences must be reachable with no status at all.
    const between = panel.slice(statePanel, panel.indexOf("{/if}", statePanel))
    assert.equal(
        between.includes("whisper-result-title"),
        false,
        "the block must not be hidden behind `{#if status}`"
    )
    // Every phase has its own branch, so no state renders an empty box.
    for (const phase of ["recording", "transcribing", "ready", "failed", "empty"]) {
        assert.ok(
            panel.includes(`phase === "${phase}"`) || phase === "empty",
            `the block needs a branch for ${phase}`
        )
    }
    assert.ok(panel.includes("{:else}"), "the empty phase is the fallback")
})

test("the block offers the text, the numbers, and the three buttons", () => {
    const panel = readFileSync(PANEL, "utf8")
    const block = panel.slice(
        panel.indexOf('class="result"'),
        panel.indexOf('t("whisper-enabled")')
    )
    for (const key of [
        "whisper-insert-command",
        "whisper-copy",
        "whisper-clear",
        "whisper-characters",
        "whisper-audio-length"
    ]) {
        assert.ok(block.includes(key), `the result block needs ${key}`)
    }
    // The language is shown as the core reported it, and the text itself.
    assert.ok(block.includes("transcript.language"))
    assert.ok(block.includes("transcript.text"))
})

test("no poll and no page switch can remove the result", () => {
    const panel = readFileSync(PANEL, "utf8")
    // `load()` is the poll, and the reload after a page switch. It only reads.
    const load = panel.slice(panel.indexOf("async function load()"), panel.indexOf("function describe("))
    for (const forbidden of ["clearLast", "forget", "last = null"]) {
        assert.equal(
            load.includes(forbidden),
            false,
            `the poll must not clear the transcript (${forbidden})`
        )
    }
    // Clearing happens in exactly one place, and a person does it.
    const clearCalls = panel.match(/whisperApi\.clearLast\(\)/g) ?? []
    assert.equal(clearCalls.length, 1, "clearing is one explicit call")
    const forget = panel.slice(panel.indexOf("async function forget()"))
    assert.ok(
        forget.slice(0, 300).includes("clearLast"),
        "and it lives in the function the Clear button uses"
    )
    // The transcript is never written to the log or to browser storage.
    for (const forbidden of [
        "console.log",
        "console.error",
        "console.warn",
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "document.cookie",
        "location.hash"
    ]) {
        assert.equal(
            panel.includes(forbidden),
            false,
            `the panel must not use ${forbidden}`
        )
    }
})

// ------------------------------------------------- the microphone check level

test("the microphone level is a word, not a rounded percentage", () => {
    // 0.001 of full scale printed as "0 %", which told a person nothing.
    assert.equal(microphoneLevel(0), "none")
    assert.equal(microphoneLevel(0.001), "quiet")
    assert.equal(microphoneLevel(0.019), "quiet")
    assert.equal(microphoneLevel(0.02), "normal")
    assert.equal(microphoneLevel(0.5), "normal")
    assert.equal(microphoneLevel(0.9), "normal")
    assert.equal(microphoneLevel(0.95), "loud")
    assert.equal(microphoneLevel(Number.NaN), "none")
    for (const level of ["none", "quiet", "normal", "loud"]) {
        assert.equal(microphoneLevelKey(level), `whisper-mic-level-${level}`)
    }
})

test("the check lasts about a second", () => {
    // The core is asked for thirty frames of 512 samples at 16 kHz: 0.96 s.
    // Three frames (96 ms) missed the beginning of a word.
    const rust = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-gui/src/desktop.rs", import.meta.url)),
        "utf8"
    )
    const call = /check_microphone\((\d+)\)/.exec(rust)
    assert.ok(call, "the check must call the core")
    const frames = Number(call[1])
    const seconds = (frames * 512) / 16_000
    assert.ok(
        seconds >= 0.8 && seconds <= 1.6,
        `the check must last about a second, got ${seconds.toFixed(2)} s`
    )
})

test("the panel shows the level as a word and keeps the number behind it", () => {
    const panel = readFileSync(PANEL, "utf8")
    assert.ok(panel.includes("microphoneLevelKey"), "the word comes from the model")
    assert.ok(panel.includes("levelLabel(microphone.level)"), "the number is still shown")
    // A value under one percent must not print as a bare "0".
    assert.ok(panel.includes("tenths"), "a very small level needs a fraction")
})
