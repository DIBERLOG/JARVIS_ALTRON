import { test } from "node:test"
import assert from "node:assert/strict"
import { readFileSync, readdirSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    CLIPBOARD_TIMEOUT_OPTIONS,
    CONFLICT_RESOLUTIONS,
    IDLE_TIMEOUT_OPTIONS,
    VAULT_SORTS,
    VAULT_TRASH_FILTERS,
    clipboardOptionKey,
    conflictOptionKey,
    idleOptionKey,
    sortOptionKey,
    trashOptionKey
} from "../src/lib/vault-model.ts"

const LOCALES = ["en", "ru", "ua"]
const COMPONENT_DIR = fileURLToPath(new URL("../src/components/vault/", import.meta.url))
const ROUTE_FILE = fileURLToPath(new URL("../src/routes/vault/index.svelte", import.meta.url))
const HEADER_FILE = fileURLToPath(new URL("../src/components/Header.svelte", import.meta.url))

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
        if (!trimmed.includes(" = ")) continue
        keys.add(trimmed.split(" = ")[0].trim())
    }
    return keys
}

function vaultSources() {
    const files = [
        ...readdirSync(COMPONENT_DIR)
            .filter((name) => name.endsWith(".svelte"))
            .map((name) => `${COMPONENT_DIR}${name}`),
        ROUTE_FILE,
        HEADER_FILE
    ]
    return files.map((path) => readFileSync(path, "utf8"))
}

const MODEL_FILE = fileURLToPath(new URL("../src/lib/vault-model.ts", import.meta.url))

/** Removes comments so documented prohibitions are not mistaken for usage. */
function stripComments(source) {
    return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "")
}

/**
 * Removes style blocks and class attributes, whose names look like message keys
 * (`class="vault-page"`) but are not.
 */
function stripMarkupNoise(source) {
    return stripComments(source)
        .replace(/<style[\s\S]*?<\/style>/g, "")
        .replace(/class="[^"]*"/g, "")
        .replace(/class:([a-z0-9-]+)/g, "")
}

/**
 * Every key the model can produce, taken from the source itself.
 *
 * Option keys and validator error keys are string literals in the model, so
 * reading them keeps this test correct when a new option is added.
 */
function modelKeys() {
    const source = readFileSync(MODEL_FILE, "utf8")
    const keys = new Set()
    for (const match of source.matchAll(/"(vault-[a-z0-9-]+)"/g)) {
        keys.add(match[1])
    }
    return keys
}

/** Literal `t('key')` usages plus every key the model builds at run time. */
function requiredKeys() {
    const keys = new Set()
    for (const source of vaultSources()) {
        const copy = stripMarkupNoise(source)
        for (const match of copy.matchAll(/\bt\(\s*['"]([a-z0-9-]+)['"]/g)) {
            keys.add(match[1])
        }
        // Keys passed indirectly, for example `t(check.errorKey ?? "vault-error")`.
        for (const match of copy.matchAll(/"(vault-[a-z0-9-]+)"/g)) {
            keys.add(match[1])
        }
    }
    for (const key of modelKeys()) keys.add(key)
    for (const sort of VAULT_SORTS) keys.add(sortOptionKey(sort))
    for (const filter of VAULT_TRASH_FILTERS) keys.add(trashOptionKey(filter))
    for (const resolution of CONFLICT_RESOLUTIONS) keys.add(conflictOptionKey(resolution))
    for (const seconds of IDLE_TIMEOUT_OPTIONS) keys.add(idleOptionKey(seconds))
    for (const seconds of CLIPBOARD_TIMEOUT_OPTIONS) keys.add(clipboardOptionKey(seconds))
    return keys
}

test("every key used by the vault interface exists in all locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 90, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing vault messages`)
    }
})

test("the three locales stay in sync for vault messages", () => {
    const reference = messageKeys("en")
    const vaultKeys = [...reference].filter((key) => key.startsWith("vault-"))
    assert.ok(vaultKeys.length > 90, `expected vault messages, got ${vaultKeys.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = vaultKeys.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing vault messages`)
    }
})

test("no vault message is left unused by the interface", () => {
    const required = requiredKeys()
    const english = messageKeys("en")
    const vaultKeys = [...english].filter(
        (key) => key.startsWith("vault-") || key === "header-vault"
    )
    const unused = vaultKeys.filter((key) => !required.has(key)).sort()
    assert.deepEqual(unused, [], "these vault messages are defined but never used")
})

test("vault components never build message keys by string concatenation", () => {
    for (const source of vaultSources()) {
        assert.equal(
            /t\(\s*`/.test(source),
            false,
            "template-literal translation keys cannot be verified"
        )
    }
})

test("the vault interface never persists secrets in browser storage", () => {
    const forbidden = [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "window.history",
        "location.hash",
        "location.search"
    ]
    for (const source of vaultSources()) {
        const code = stripComments(source)
        for (const needle of forbidden) {
            assert.equal(code.includes(needle), false, `a vault component must not use ${needle}`)
        }
    }
    // The vault model and API modules must be equally clean.
    const model = stripComments(readFileSync(MODEL_FILE, "utf8"))
    const api = stripComments(readFileSync(fileURLToPath(new URL("../src/lib/vault.ts", import.meta.url)), "utf8"))
    for (const source of [model, api]) {
        for (const needle of forbidden) {
            assert.equal(source.includes(needle), false, `vault code must not use ${needle}`)
        }
    }
})
