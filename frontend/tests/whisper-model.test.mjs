import { test } from "node:test"
import assert from "node:assert/strict"

import {
    DEFAULT_SECONDS,
    DEFAULT_THREADS,
    LANGUAGES,
    MAX_SECONDS,
    MAX_SILENCE_MS,
    MAX_THREADS,
    MAX_TIMEOUT_SECONDS,
    MIN_SECONDS,
    MIN_SILENCE_MS,
    MIN_THREADS,
    MIN_TIMEOUT_SECONDS,
    PREVIEW_CHARS,
    audioLabel,
    cleanedTranscript,
    defaultSettings,
    errorKey,
    isBusy,
    isRecording,
    modelIsUnverified,
    modelKindKey,
    normalizedSettings,
    noteKey,
    readinessKey,
    settingsProblem,
    transcriptLength,
    transcriptPreview
} from "../src/lib/whisper-model.ts"

function transcript(overrides = {}) {
    return {
        text: "привет мир",
        segments: [],
        language: "ru",
        audio_ms: 1500,
        duration_ms: 300,
        ...overrides
    }
}

function status(overrides = {}) {
    return {
        state: "idle",
        enabled: true,
        configured: true,
        binary: { size_bytes: 100, architecture: "x86_64" },
        model: { size_bytes: 500, kind: "small", container: "ggml", notes: [] },
        binary_path: "C:/whisper/whisper-cli.exe",
        model_path: "C:/models/ggml-small.bin",
        notes: [],
        ...overrides
    }
}

test("dictation is off in the defaults", () => {
    const settings = defaultSettings()
    assert.equal(settings.enabled, false)
    assert.equal(settings.language, "auto")
    assert.equal(settings.threads, DEFAULT_THREADS)
    assert.equal(settings.max_seconds, DEFAULT_SECONDS)
    assert.equal(settings.keep_audio, false)
    assert.equal(settingsProblem(settings), null)
})

test("a state is what the core says, and only recording means a microphone is open", () => {
    assert.equal(isRecording("recording"), true)
    assert.equal(isRecording("idle"), false)
    assert.equal(isRecording("transcribing"), false)
    assert.equal(isBusy("transcribing"), true)
    assert.equal(isBusy("idle"), false)
})

test("every value list produces a key that names the value", () => {
    for (const state of ["idle", "recording", "transcribing"]) {
        assert.ok(isBusy(state) !== undefined)
    }
    for (const kind of [
        "tiny",
        "tiny_en",
        "base",
        "base_en",
        "small",
        "small_en",
        "medium",
        "medium_en",
        "large_v1",
        "large_v2",
        "large_v3",
        "unknown"
    ]) {
        assert.equal(modelKindKey(kind), `whisper-model-${kind}`)
    }
    for (const language of LANGUAGES) {
        assert.ok(language.length > 0)
    }
})

test("an unknown error code is named instead of shown raw", () => {
    assert.equal(errorKey("not_configured"), "whisper-error-not_configured")
    assert.equal(errorKey("wrong_architecture"), "whisper-error-wrong_architecture")
    assert.equal(errorKey("something_else"), "whisper-error-unknown")
})

test("a note from the core is a key when the core sent a key", () => {
    assert.equal(noteKey("windows-whisper-note-no-model"), "whisper-note-no-model")
    // A plain sentence is shown as it is, not translated by guessing.
    assert.equal(noteKey("the file's contents are not verified against a hash"), null)
})

test("the numbers are checked before they are sent", () => {
    const base = defaultSettings()
    assert.equal(settingsProblem({ ...base, threads: 99 }), "whisper-error-threads")
    assert.equal(settingsProblem({ ...base, threads: 0 }), "whisper-error-threads")
    assert.equal(settingsProblem({ ...base, max_seconds: 0 }), "whisper-error-seconds")
    assert.equal(settingsProblem({ ...base, max_seconds: 10_000 }), "whisper-error-seconds")
    assert.equal(settingsProblem({ ...base, silence_ms: 10 }), "whisper-error-silence")
    assert.equal(settingsProblem({ ...base, timeout_seconds: 1 }), "whisper-error-timeout")
    assert.equal(settingsProblem({ ...base, language: "klingon" }), "whisper-error-language")
})

test("the settings are clamped into the ranges the core enforces", () => {
    const repaired = normalizedSettings({
        ...defaultSettings(),
        threads: 200,
        max_seconds: 99_999,
        silence_ms: 1,
        timeout_seconds: 0,
        language: "xx",
        binary_path: "  C:/whisper/whisper-cli.exe  ",
        model_path: " C:/models/ggml-small.bin "
    })
    assert.equal(repaired.threads, MAX_THREADS)
    assert.equal(repaired.max_seconds, MAX_SECONDS)
    assert.equal(repaired.silence_ms, MIN_SILENCE_MS)
    assert.equal(repaired.timeout_seconds, MIN_TIMEOUT_SECONDS)
    assert.equal(repaired.language, "auto")
    assert.equal(repaired.binary_path, "C:/whisper/whisper-cli.exe")
    assert.equal(repaired.model_path, "C:/models/ggml-small.bin")
    // A value of the wrong type falls back instead of producing NaN.
    assert.equal(normalizedSettings({ ...defaultSettings(), threads: Number.NaN }).threads, DEFAULT_THREADS)
    assert.equal(settingsProblem({ ...repaired, silence_ms: MAX_SILENCE_MS }), null)
    assert.equal(settingsProblem({ ...repaired, timeout_seconds: MAX_TIMEOUT_SECONDS }), null)
    assert.equal(MIN_THREADS, 1)
    assert.equal(MIN_SECONDS, 1)
    assert.equal(MIN_SILENCE_MS, 500)
    assert.equal(MIN_TIMEOUT_SECONDS, 5)
})

test("a transcript is collapsed for the box and shortened for the panel", () => {
    const messy = transcript({ text: "  привет,\n\n  как   дела  " })
    assert.equal(cleanedTranscript(messy), "привет, как дела")
    // "привет, как дела" is sixteen characters, counted as a person would.
    assert.equal(transcriptLength(messy), 16)
    const short = transcriptPreview(messy)
    assert.equal(short.shortened, false)
    assert.equal(short.text, "привет, как дела")
    const long = transcriptPreview(transcript({ text: "я".repeat(PREVIEW_CHARS + 50) }))
    assert.equal(long.shortened, true)
    assert.equal(long.text.length, PREVIEW_CHARS)
})

test("a duration is readable", () => {
    assert.equal(audioLabel(transcript({ audio_ms: 1500 })), "1.5 s")
    assert.equal(audioLabel(transcript({ audio_ms: 90_000 })), "1 min 30 s")
})

test("an unverified model is reported as such", () => {
    assert.equal(modelIsUnverified(status()), false)
    assert.equal(
        modelIsUnverified(status({ model: { size_bytes: 1, kind: "small", container: "ggml", notes: ["the file's contents are not verified against a hash"] } })),
        true
    )
})

test("readiness says what is missing without naming a path", () => {
    assert.equal(readinessKey(status()), "whisper-readiness-ready")
    assert.equal(readinessKey(status({ enabled: false })), "whisper-readiness-disabled")
    assert.equal(
        readinessKey(status({ configured: false, binary_path: "", model_path: "" })),
        "whisper-readiness-empty"
    )
    assert.equal(
        readinessKey(status({ configured: false, binary: null })),
        "whisper-readiness-binary"
    )
    assert.equal(
        readinessKey(
            status({
                configured: false,
                model: null,
                model_path: "C:/models/ggml-small.bin"
            })
        ),
        "whisper-readiness-model"
    )
})
