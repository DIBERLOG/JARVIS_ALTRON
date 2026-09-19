import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    LANGUAGES,
    NOTE_CODES,
    RECORDER_CODES,
    candidateSourceKey,
    errorKey,
    modelKindKey,
    noteKey,
    readinessKey,
    stateKey
} from "../src/lib/whisper-model.ts"

/** The codes `RecorderError::code()` can produce, read from its own arms. */
function coreRecorderCodes() {
    const source = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-core/src/recorder/error.rs", import.meta.url)),
        "utf8"
    )
    // Only the `code()` function: the stage names are lowercase too, and they
    // are not codes.
    const start = source.indexOf("pub fn code(")
    assert.ok(start >= 0, "RecorderError must still have a code()")
    const rest = source.slice(start)
    const end = rest.indexOf("\n    pub fn ")
    const body = end >= 0 ? rest.slice(0, end) : rest
    const codes = new Set()
    for (const match of body.matchAll(/=>\s*"([a-z_]+)"/g)) {
        codes.add(match[1])
    }
    return codes
}

/** The error codes the core can produce, read from its own match arms. */
function coreNoteCodes() {
    const source = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-core/src/whisper/error.rs", import.meta.url)),
        "utf8"
    )
    const codes = new Set()
    for (const match of source.matchAll(/=> "([a-z_]+)",/g)) {
        codes.add(match[1])
    }
    return codes
}

const LOCALES = ["en", "ru", "ua"]
const MODEL_FILE = fileURLToPath(new URL("../src/lib/whisper-model.ts", import.meta.url))
const API_FILE = fileURLToPath(new URL("../src/lib/whisper.ts", import.meta.url))
const PANEL_FILE = fileURLToPath(
    new URL("../src/components/settings/WhisperSettings.svelte", import.meta.url)
)
const SETTINGS_ROUTE = fileURLToPath(
    new URL("../src/routes/settings/index.svelte", import.meta.url)
)

function sources() {
    return [MODEL_FILE, API_FILE, PANEL_FILE].filter((path) => existsSync(path))
}

/** The keys the model builds from a value list. */
const FAMILY_KEYS = [
    "whisper-state-idle",
    "whisper-state-recording",
    "whisper-state-transcribing",
    ...[
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
    ].map(modelKindKey),
    ...[
        "disabled",
        "not_configured",
        "invalid_configuration",
        "binary_unavailable",
        "model_unavailable",
        "model_unknown",
        "wrong_architecture",
        "audio_unavailable",
        "audio_empty",
        "busy",
        "process_unavailable",
        "process_failed",
        "timed_out",
        "invalid_response",
        "cancelled",
        "unsupported_language",
        "storage"
    ].map(errorKey),
    "whisper-error-unknown",
    "whisper-error-threads",
    // The recorder's own codes, so a refused microphone is never shown as a
    // generic "unknown error".
    ...RECORDER_CODES.map(errorKey),
    "whisper-error-seconds",
    "whisper-error-silence",
    "whisper-error-timeout",
    "whisper-error-language",
    "whisper-readiness-disabled",
    "whisper-readiness-ready",
    "whisper-readiness-empty",
    "whisper-readiness-binary",
    "whisper-readiness-model",
    noteKey("windows-whisper-note-no-binary") ?? "",
    noteKey("windows-whisper-note-no-model") ?? "",
    noteKey("windows-whisper-note-disabled") ?? "",
    // Every note the status can build, one per error code in the core.
    ...NOTE_CODES.map((code) => `whisper-note-${code.replace(/_/g, "-")}`),
    // The discovery result lines and the places a candidate can come from.
    "whisper-discovery-idle",
    "whisper-discovery-nothing",
    "whisper-discovery-nothing-usable",
    "whisper-discovery-one",
    "whisper-discovery-choose",
    // The core sends `bundled_runtime`/`known_directory`/`path`; the label keys
    // are the short spelling that `candidateSourceKey` produces.
    ...["bundled_runtime", "known_directory", "path"].map((source) => candidateSourceKey(source))
]

function localePath(language) {
    return fileURLToPath(
        new URL(`../../crates/jarvis-core/src/i18n/locales/${language}.ftl`, import.meta.url)
    )
}

function messageKeys(language) {
    const source = readFileSync(localePath(language), "utf8")
    const keys = new Set()
    for (const line of source.split("\n")) {
        const trimmed = line.trim()
        if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("-")) continue
        const match = /^([A-Za-z0-9_-]+)\s*=/.exec(trimmed)
        if (!match) continue
        keys.add(match[1])
    }
    return keys
}

function stripComments(source) {
    return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "")
}

function stripMarkupNoise(source) {
    return stripComments(source)
        .replace(/<style[\s\S]*?<\/style>/g, "")
        .replace(/class="[^"]*"/g, "")
        .replace(/class:([a-z0-9-]+)/g, "")
}

function literalKeys() {
    const keys = new Set()
    // The settings page names the tab, and it is the only place that does.
    const settingsRoute = stripMarkupNoise(readFileSync(SETTINGS_ROUTE, "utf8"))
    for (const match of settingsRoute.matchAll(/\bt\(\s*['"](whisper-[a-z0-9_-]+)['"]/g)) {
        keys.add(match[1])
    }
    for (const path of sources()) {
        const copy = stripMarkupNoise(readFileSync(path, "utf8"))
        for (const match of copy.matchAll(/\bt\(\s*['"](whisper-[a-z0-9_-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    return keys
}

function modelLiteralKeys() {
    const source = readFileSync(MODEL_FILE, "utf8")
    const keys = new Set()
    for (const match of source.matchAll(/"(whisper-[a-z0-9_-]+)"/g)) {
        // A prefix used to *build* a key is not a key: the model has the string
        // `whisper-note-` as the replacement prefix, and no message is named that.
        if (match[1].endsWith("-")) continue
        keys.add(match[1])
    }
    return keys
}

function requiredKeys() {
    const keys = new Set(literalKeys())
    for (const key of modelLiteralKeys()) keys.add(key)
    for (const key of FAMILY_KEYS) if (key) keys.add(key)
    return keys
}

test("the dictation settings panel exists and is wired into the settings page", () => {
    for (const path of [PANEL_FILE, API_FILE, MODEL_FILE]) {
        assert.ok(existsSync(path), `${path} must exist`)
    }
    const settings = readFileSync(SETTINGS_ROUTE, "utf8")
    assert.ok(settings.includes("whisper-tab"), "the settings page needs a tab")
    assert.ok(settings.includes("WhisperSettingsPanel"), "the tab must render the panel")
})

test("every key the panel asks for exists in all three locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 80, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing whisper messages`)
    }
})

test("the three locales stay in sync for whisper messages", () => {
    const reference = messageKeys("en")
    const keys = [...reference].filter((key) => key.startsWith("whisper-"))
    assert.ok(keys.length > 60, `expected whisper messages, got ${keys.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = keys.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing whisper messages`)
    }
})

test("no whisper message is left unused", () => {
    const required = requiredKeys()
    const english = messageKeys("en")
    const keys = [...english].filter((key) => key.startsWith("whisper-"))
    const unused = keys.filter((key) => !required.has(key)).sort()
    assert.deepEqual(unused, [], "these whisper messages are defined but never used")
})

test("a key is never built in a template inside the panel", () => {
    const source = readFileSync(PANEL_FILE, "utf8")
    assert.equal(/t\(\s*`/.test(source), false, "the panel builds a translation key at run time")
    // The one dynamic case goes through the model's own function.
    assert.ok(source.includes("noteKey("))
    assert.ok(source.includes("modelKindKey("))
    assert.ok(source.includes("stateKey("))
})

test("the recorder codes the panel translates are the core's own", () => {
    // The defect this guards: a recorder failure was flattened into
    // `audio_unavailable`, so the window could not tell "never initialised"
    // from "no input device". The list has to follow the core.
    const core = coreRecorderCodes()
    assert.ok(core.size >= 6, `expected the recorder codes, got ${core.size}`)
    const missing = [...core].filter((code) => !RECORDER_CODES.includes(code)).sort()
    assert.deepEqual(missing, [], "the panel does not know every recorder code")
    const extra = RECORDER_CODES.filter((code) => !core.has(code)).sort()
    assert.deepEqual(extra, [], "the panel invents a recorder code the core cannot send")
})

test("every recorder code has a sentence in all three locales", () => {
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = RECORDER_CODES.map(errorKey)
            .filter((key) => !available.has(key))
            .sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a recorder message`)
    }
})

test("the interface keeps no transcript and opens no browser dialog", () => {
    for (const path of sources()) {
        const source = stripComments(readFileSync(path, "utf8"))
        for (const forbidden of [
            "localStorage",
            "sessionStorage",
            "indexedDB",
            "window.history",
            "location.hash",
            "document.cookie",
            "window.confirm",
            "window.prompt",
            "window.alert"
        ]) {
            assert.equal(
                source.includes(forbidden),
                false,
                `${path} must not use ${forbidden}: a transcript stays in memory`
            )
        }
    }
})

test("the interface cannot name a program or a command line", () => {
    const api = readFileSync(API_FILE, "utf8")
    for (const forbidden of ["Command(", "spawn(", "shell", "powershell", "cmd.exe"]) {
        assert.equal(api.includes(forbidden), false, `the API must not mention ${forbidden}`)
    }
    assert.ok(api.includes("whisper_select_binary"))
    assert.ok(api.includes("whisper_select_model"))
    // The dictation is the only thing that can start a recording, and it takes
    // no arguments from the window.
    assert.ok(api.includes('invoke<Transcript>("whisper_dictate")'))
})
