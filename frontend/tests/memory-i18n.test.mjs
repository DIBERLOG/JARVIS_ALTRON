import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync, readdirSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    CANDIDATE_STATES,
    MEMORY_CATEGORIES,
    MEMORY_SCOPES,
    MEMORY_SOURCES,
    MESSAGE_STATUSES,
    SECRET_KINDS,
    categoryLabelKey,
    candidateStateLabelKey,
    messageStatusLabelKey,
    scopeLabelKey,
    secretKindLabelKey,
    sourceLabelKey,
    storageStateLabelKey
} from "../src/lib/memory-model.ts"

const LOCALES = ["en", "ru", "ua"]
const ROUTE_FILE = fileURLToPath(new URL("../src/routes/memory/index.svelte", import.meta.url))
const COMPONENT_DIR = fileURLToPath(new URL("../src/components/memory/", import.meta.url))
const HEADER_FILE = fileURLToPath(new URL("../src/components/Header.svelte", import.meta.url))
const CHAT_FILE = fileURLToPath(new URL("../src/components/ai/LocalChat.svelte", import.meta.url))
const MODEL_FILE = fileURLToPath(new URL("../src/lib/memory-model.ts", import.meta.url))
const API_FILE = fileURLToPath(new URL("../src/lib/memory.ts", import.meta.url))

/** Keys the model builds at run time from a fixed set of values. */
const FAMILY_KEYS = [
    ...MEMORY_SCOPES.map(scopeLabelKey),
    ...MEMORY_CATEGORIES.map(categoryLabelKey),
    ...MEMORY_SOURCES.map(sourceLabelKey),
    ...CANDIDATE_STATES.map(candidateStateLabelKey),
    ...MESSAGE_STATUSES.map(messageStatusLabelKey),
    ...SECRET_KINDS.map(secretKindLabelKey),
    "memory-secret-unknown",
    "memory-storage-uninitialized",
    "memory-storage-locked",
    "memory-storage-unlocked",
    "memory-storage-key-missing"
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
        const match = /^([A-Za-z0-9-]+)\s*=/.exec(trimmed)
        if (!match) continue
        keys.add(match[1])
    }
    return keys
}

/** Every memory-related source the interface has. */
function memorySources() {
    const files = [ROUTE_FILE, CHAT_FILE, MODEL_FILE, API_FILE]
    if (existsSync(COMPONENT_DIR)) {
        for (const name of readdirSync(COMPONENT_DIR)) {
            if (name.endsWith(".svelte")) files.push(`${COMPONENT_DIR}${name}`)
        }
    }
    return files
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

/** Literal `t('memory-...')` usages in the memory page and in the chat panel. */
function literalKeys() {
    const keys = new Set()
    for (const path of memorySources()) {
        if (!existsSync(path)) continue
        const copy = stripMarkupNoise(readFileSync(path, "utf8"))
        for (const match of copy.matchAll(/\bt\(\s*['"](memory-[a-z0-9-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    // The navigation button lives in the header.
    const header = stripMarkupNoise(readFileSync(HEADER_FILE, "utf8"))
    for (const match of header.matchAll(/\bt\(\s*['"]((?:memory|header-memory)[a-z0-9-]*)['"]/g)) {
        keys.add(match[1])
    }
    return keys
}

/** Keys the model returns as string literals. */
function modelLiteralKeys() {
    const source = readFileSync(MODEL_FILE, "utf8")
    const keys = new Set()
    for (const match of source.matchAll(/"(memory-[a-z0-9-]+)"/g)) {
        keys.add(match[1])
    }
    return keys
}

function requiredKeys() {
    const keys = new Set(literalKeys())
    for (const key of modelLiteralKeys()) keys.add(key)
    for (const key of FAMILY_KEYS) keys.add(key)
    return keys
}

test("the memory interface exists and is wired into the shell", () => {
    assert.ok(existsSync(ROUTE_FILE), "the /memory route must exist")
    const header = readFileSync(HEADER_FILE, "utf8")
    assert.ok(header.includes("header-memory"), "the header must link to the memory page")
    assert.ok(header.includes("/memory"), "the header button must navigate to /memory")
})

test("every key used by the memory interface exists in all locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 100, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing memory messages`)
    }
})

test("the three locales stay in sync for memory messages", () => {
    const reference = messageKeys("en")
    const memoryKeys = [...reference].filter((key) => key.startsWith("memory-"))
    assert.ok(memoryKeys.length > 100, `expected memory messages, got ${memoryKeys.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = memoryKeys.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing memory messages`)
    }
})

test("no memory message is left unused by the interface", () => {
    const required = requiredKeys()
    const english = messageKeys("en")
    const memoryKeys = [...english].filter((key) => key.startsWith("memory-"))
    const unused = memoryKeys.filter((key) => !required.has(key)).sort()
    assert.deepEqual(unused, [], "these memory messages are defined but never used")
})

test("memory components never build message keys by string concatenation", () => {
    for (const path of memorySources()) {
        if (!existsSync(path)) continue
        const source = readFileSync(path, "utf8")
        assert.equal(
            /t\(\s*`/.test(source),
            false,
            `${path} builds a translation key at run time`
        )
    }
})

test("the memory interface never persists anything in the browser", () => {
    const forbidden = [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "window.history",
        "location.hash",
        "location.search",
        "document.cookie"
    ]
    for (const path of memorySources()) {
        if (!existsSync(path)) continue
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of forbidden) {
            assert.equal(
                code.includes(needle),
                false,
                `${path} must not use ${needle}: history lives in the encrypted database`
            )
        }
    }
})

test("the memory interface never talks to a model or a database itself", () => {
    // The gateway owns the model and Rust owns SQLite; a webview that called either
    // directly would bypass the secret filter, the budget, and the encryption.
    const forbidden = ["fetch(", "XMLHttpRequest", "WebSocket", "EventSource", "http://", "https://", "sqlite", "SQL"]
    for (const path of memorySources()) {
        if (!existsSync(path)) continue
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of forbidden) {
            assert.equal(code.includes(needle), false, `${path} must not use ${needle}`)
        }
    }
})

test("the memory interface never receives reasoning output", () => {
    // Reasoning is never stored and never sent, so nothing in the interface may even
    // name it.
    for (const path of memorySources()) {
        if (!existsSync(path)) continue
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of ["reasoning_content", "chain_of_thought", "chain-of-thought"]) {
            assert.equal(code.includes(needle), false, `${path} must not reference ${needle}`)
        }
    }
})

test("the system prompt never reaches the memory interface", () => {
    // A component must not mention it at all: the profile prompt is built in Rust,
    // added by the gateway, and never part of a context plan.
    for (const path of memorySources().filter((file) => file.endsWith(".svelte"))) {
        if (!existsSync(path)) continue
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of ["system_prompt", "systemPrompt"]) {
            assert.equal(code.includes(needle), false, `${path} must not reference ${needle}`)
        }
    }
    // The model may mirror the *size* of the prompt as a budget number, and nothing
    // else: it is declared once as a number and only ever read as that number.
    const model = readFileSync(MODEL_FILE, "utf8")
    assert.match(model, /system_prompt:\s*number/)
    assert.equal(/system_prompt:\s*string/.test(model), false, "the prompt text must never be typed")
    assert.equal(
        /system_prompt_text|systemPromptText|system_prompt_content/.test(model),
        false,
        "no field may carry the prompt text"
    )
    const uses = [...model.matchAll(/system_prompt/g)].length
    const reads = [...model.matchAll(/budget\.system_prompt/g)].length
    assert.equal(uses, reads + 1, "the field is declared once and only read as a number")
})

test("the memory interface cannot reach the password vault", () => {
    const forbidden = ["@/lib/vault", "@/lib/notes", "vaultApi", "notesApi", "vault-model", "notes-model"]
    for (const path of memorySources()) {
        if (!existsSync(path)) continue
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of forbidden) {
            assert.equal(code.includes(needle), false, `${path} must not reference ${needle}`)
        }
    }
})
