import { test } from "node:test"
import assert from "node:assert/strict"
import { readFileSync, readdirSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    CONFLICT_RESOLUTIONS,
    NOTE_SORTS,
    TRASH_FILTERS,
    conflictOptionKey,
    saveIndicatorKey,
    sortOptionKey,
    timeUnitKey,
    trashOptionKey
} from "../src/lib/notes-model.ts"

const LOCALES = ["en", "ru", "ua"]
const COMPONENT_DIR = fileURLToPath(new URL("../src/components/notes/", import.meta.url))
const ROUTE_FILE = fileURLToPath(new URL("../src/routes/notes/index.svelte", import.meta.url))
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

function notesSources() {
    const files = [
        ...readdirSync(COMPONENT_DIR)
            .filter((name) => name.endsWith(".svelte"))
            .map((name) => `${COMPONENT_DIR}${name}`),
        ROUTE_FILE,
        // The header links to the notes page.
        HEADER_FILE
    ]
    return files.map((path) => readFileSync(path, "utf8"))
}

/** Literal `t('key')` usages plus the keys the model builds at run time. */
function requiredKeys() {
    const keys = new Set()
    for (const source of notesSources()) {
        for (const match of source.matchAll(/\bt\(\s*['"]([a-z0-9-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    for (const sort of NOTE_SORTS) keys.add(sortOptionKey(sort))
    for (const filter of TRASH_FILTERS) keys.add(trashOptionKey(filter))
    for (const resolution of CONFLICT_RESOLUTIONS) keys.add(conflictOptionKey(resolution))
    for (const unit of ["now", "minute", "hour", "day", "date"]) keys.add(timeUnitKey(unit))
    for (const state of ["idle", "dirty", "saving", "saved", "error"]) {
        keys.add(saveIndicatorKey(state, true))
        keys.add(saveIndicatorKey(state, false))
    }
    return keys
}

test("every key used by the notes interface exists in all locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 40, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing notes messages`)
    }
})

test("the three locales stay in sync with each other", () => {
    const [english, ...others] = LOCALES.map((language) => messageKeys(language))
    for (const [index, language] of others.entries()) {
        const name = LOCALES[index + 1]
        const missing = [...english].filter((key) => !language.has(key)).sort()
        assert.deepEqual(missing, [], `${name}.ftl is missing messages that exist in en.ftl`)
    }
})

test("no notes message is left unused by the interface", () => {
    const required = requiredKeys()
    const english = messageKeys("en")
    const notesKeys = [...english].filter(
        (key) => key.startsWith("notes-") || key === "header-notes"
    )
    assert.ok(notesKeys.length > 40, `expected notes messages in en.ftl, got ${notesKeys.length}`)
    const unused = notesKeys.filter((key) => !required.has(key)).sort()
    assert.deepEqual(unused, [], "these messages are defined but never used")
})

test("component sources do not build message keys by string concatenation", () => {
    // Dynamic keys must come from the model helpers so this test can see them.
    for (const source of notesSources()) {
        assert.equal(
            /t\(\s*`/.test(source),
            false,
            "template-literal translation keys cannot be verified"
        )
    }
})
