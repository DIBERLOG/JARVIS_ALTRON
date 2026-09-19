import { test } from "node:test"
import assert from "node:assert/strict"
import { readFileSync, readdirSync } from "node:fs"
import { fileURLToPath } from "node:url"

import { fieldLabelKey, PROFILES, THINKING_MODES } from "../src/lib/local-ai-model.ts"

const LOCALES = ["en", "ru", "ua"]
const COMPONENT_DIR = fileURLToPath(new URL("../src/components/ai/", import.meta.url))
const MODEL_FILE = fileURLToPath(new URL("../src/lib/local-ai-model.ts", import.meta.url))
const API_FILE = fileURLToPath(new URL("../src/lib/local-ai.ts", import.meta.url))
const ROUTE_FILES = [
    fileURLToPath(new URL("../src/routes/index.svelte", import.meta.url)),
    fileURLToPath(new URL("../src/routes/settings/index.svelte", import.meta.url))
]

/** Keys the model builds at run time from a fixed set of values. */
const STATE_KEYS = ["stopped", "starting", "ready", "generating", "stopping", "failed"].map(
    (state) => `ai-chat-state-${state}`
)
const REPORT_LEVEL_KEYS = ["ok", "warning", "blocked"].map((level) => `ai-settings-report-${level}`)
const FIELD_KEYS = [
    "server_path",
    "model_path",
    "host",
    "port",
    "context_size",
    "cpu_threads",
    "gpu_layers",
    "startup_timeout_seconds",
    "temperature",
    "top_p",
    "max_tokens",
    "allow_lan",
    "schema_version",
    "generic"
].map((field) => fieldLabelKey(field))

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
        // An empty value (`key =`) is still a defined message.
        const match = /^([A-Za-z0-9-]+)\s*=/.exec(trimmed)
        if (!match) continue
        keys.add(match[1])
    }
    return keys
}

function aiSources() {
    const files = [
        ...readdirSync(COMPONENT_DIR)
            .filter((name) => name.endsWith(".svelte"))
            .map((name) => `${COMPONENT_DIR}${name}`),
        MODEL_FILE,
        API_FILE
    ]
    return files.map((path) => ({ path, source: readFileSync(path, "utf8") }))
}

/** Removes comments so documented prohibitions are not mistaken for usage. */
function stripComments(source) {
    return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "")
}

/** Removes style blocks and class attributes, whose names look like keys. */
function stripMarkupNoise(source) {
    return stripComments(source)
        .replace(/<style[\s\S]*?<\/style>/g, "")
        .replace(/class="[^"]*"/g, "")
        .replace(/class:([a-z0-9-]+)/g, "")
}

/** Literal `t('key')` usages in the AI components and their routes. */
function literalKeys() {
    const keys = new Set()
    const components = aiSources().filter((file) => file.path.endsWith(".svelte"))
    for (const file of components) {
        const copy = stripMarkupNoise(file.source)
        for (const match of copy.matchAll(/\bt\(\s*['"]([a-z0-9-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    // The two routes contain unrelated messages from earlier stages; only the
    // local AI keys they reference are this test's business.
    for (const path of ROUTE_FILES) {
        const copy = stripMarkupNoise(readFileSync(path, "utf8"))
        for (const match of copy.matchAll(/\bt\(\s*['"](ai-[a-z0-9-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    return keys
}

/** Keys the model returns as string literals, such as the local issue messages. */
function modelLiteralKeys() {
    const source = readFileSync(MODEL_FILE, "utf8")
    const keys = new Set()
    for (const match of source.matchAll(/"(ai-[a-z0-9-]+)"/g)) {
        keys.add(match[1])
    }
    return keys
}

function requiredKeys() {
    const keys = new Set(literalKeys())
    for (const key of modelLiteralKeys()) keys.add(key)
    for (const key of STATE_KEYS) keys.add(key)
    for (const key of REPORT_LEVEL_KEYS) keys.add(key)
    for (const key of FIELD_KEYS) keys.add(key)
    for (const profile of PROFILES) keys.add(`ai-chat-profile-${profile}`)
    for (const mode of THINKING_MODES) keys.add(`ai-chat-thinking-${mode}`)
    return keys
}

test("every key used by the local AI interface exists in all locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 60, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing local AI messages`)
    }
})

test("the three locales stay in sync for local AI messages", () => {
    const reference = messageKeys("en")
    const aiKeys = [...reference].filter((key) => key.startsWith("ai-"))
    assert.ok(aiKeys.length > 60, `expected local AI messages, got ${aiKeys.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = aiKeys.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing local AI messages`)
    }
})

test("no local AI message is left unused by the interface", () => {
    const required = requiredKeys()
    const english = messageKeys("en")
    const aiKeys = [...english].filter((key) => key.startsWith("ai-"))
    const unused = aiKeys.filter((key) => !required.has(key)).sort()
    assert.deepEqual(unused, [], "these local AI messages are defined but never used")
})

test("local AI components never build message keys by string concatenation", () => {
    for (const file of aiSources()) {
        assert.equal(
            /t\(\s*`/.test(file.source),
            false,
            `${file.path} builds a translation key at run time`
        )
    }
})

test("the local AI interface never persists the conversation in the browser", () => {
    const forbidden = [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "window.history",
        "location.hash",
        "location.search",
        "document.cookie"
    ]
    for (const file of aiSources()) {
        const code = stripComments(file.source)
        for (const needle of forbidden) {
            assert.equal(
                code.includes(needle),
                false,
                `${file.path} must not use ${needle}: AI memory is a later stage`
            )
        }
    }
})

test("the interface never talks to the model server itself", () => {
    // The gateway in Rust owns the process and the HTTP client; a webview that
    // called the endpoint directly would bypass validation and cancellation.
    const forbidden = ["fetch(", "XMLHttpRequest", "WebSocket", "EventSource", "http://", "https://"]
    for (const file of aiSources()) {
        const code = stripComments(file.source)
        for (const needle of forbidden) {
            assert.equal(
                code.includes(needle),
                false,
                `${file.path} must not use ${needle}: the local gateway is the only path`
            )
        }
    }
})

test("the local AI interface never imports the encrypted storage modules", () => {
    // The chat panel and the model settings must stay unable to reach the notes
    // or password storages, exactly like the Rust side.
    const forbidden = ["@/lib/vault", "@/lib/notes", "vaultApi", "notesApi", "vault-model", "notes-model"]
    for (const file of aiSources()) {
        const code = stripComments(file.source)
        for (const needle of forbidden) {
            assert.equal(code.includes(needle), false, `${file.path} must not reference ${needle}`)
        }
    }
})
