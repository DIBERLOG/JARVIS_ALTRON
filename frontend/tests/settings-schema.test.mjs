import { test } from "node:test"
import assert from "node:assert/strict"
import { readFileSync, readdirSync, statSync } from "node:fs"
import { fileURLToPath } from "node:url"

/**
 * The settings document owns the keys; the interface only names them.
 *
 * A `db_write` with a key the schema does not know is refused with
 * "unknown setting: '…'" — the value silently stays out of the document and the
 * log fills with warnings. That is what happened to three keys on the settings
 * page and two on the statistics bar: they were read and written under names
 * that had been renamed in the schema long before. This test is the guard that
 * keeps the two sides together.
 */

const STRUCTS = fileURLToPath(
    new URL("../../crates/jarvis-core/src/db/structs.rs", import.meta.url)
)
const SOURCE_ROOT = fileURLToPath(new URL("../src", import.meta.url))

/** The match arms of one function, as a set of keys. */
function armsOf(source, signature) {
    const start = source.indexOf(signature)
    assert.ok(start >= 0, `the schema must still declare ${signature}`)
    const rest = source.slice(start + signature.length)
    const end = rest.indexOf("\n    pub fn ")
    const body = end >= 0 ? rest.slice(0, end) : rest
    const keys = new Set()
    // The arms are the ones at the match's own indentation: a nested
    // `"true" => …` is a value, not a setting name.
    for (const match of body.matchAll(/^ {12}"([a-z_]+)"\s*=>/gm)) {
        keys.add(match[1])
    }
    return keys
}

function frontendFiles(directory) {
    const files = []
    for (const entry of readdirSync(directory)) {
        const path = `${directory}/${entry}`
        if (statSync(path).isDirectory()) {
            files.push(...frontendFiles(path))
        } else if (/\.(svelte|ts)$/.test(entry)) {
            files.push(path)
        }
    }
    return files
}

/** Every key the interface names literally, per command. */
function namedKeys() {
    const writes = new Map()
    const reads = new Map()
    for (const path of frontendFiles(SOURCE_ROOT)) {
        const source = readFileSync(path, "utf8")
        for (const match of source.matchAll(
            /invoke(?:<[^>]*>)?\(\s*"(db_write|db_read)"\s*,\s*\{\s*key:\s*"([^"]+)"/g
        )) {
            const [, command, key] = match
            const table = command === "db_write" ? writes : reads
            if (!table.has(key)) table.set(key, [])
            table.get(key).push(path)
        }
    }
    return { writes, reads }
}

test("every setting the interface writes is one the schema knows", () => {
    const source = readFileSync(STRUCTS, "utf8")
    const writable = armsOf(source, "pub fn set(&mut self, key: &str, val: &str)")
    assert.ok(writable.size >= 10, `expected a real schema, got ${writable.size} keys`)
    const { writes } = namedKeys()
    assert.ok(writes.size >= 10, `expected the page to write settings, got ${writes.size}`)
    const unknown = [...writes.keys()].filter((key) => !writable.has(key)).sort()
    assert.deepEqual(unknown, [], "the interface writes settings the schema refuses")
})

test("every setting the interface reads is one the schema serves", () => {
    const source = readFileSync(STRUCTS, "utf8")
    const readable = armsOf(source, "pub fn get(&self, key: &str)")
    const { reads } = namedKeys()
    assert.ok(reads.size >= 10, `expected the interface to read settings, got ${reads.size}`)
    const unknown = [...reads.keys()].filter((key) => !readable.has(key)).sort()
    assert.deepEqual(unknown, [], "the interface reads settings the schema does not serve")
})

test("the enumerated keys cover every writable one", () => {
    const source = readFileSync(STRUCTS, "utf8")
    const writable = armsOf(source, "pub fn set(&mut self, key: &str, val: &str)")
    const listStart = source.indexOf("pub fn keys()")
    assert.ok(listStart >= 0, "the schema must list its keys")
    const list = source.slice(listStart)
    const enumerated = new Set()
    for (const match of list.matchAll(/"([a-z_]+)"/g)) {
        enumerated.add(match[1])
    }
    const missing = [...writable].filter((key) => !enumerated.has(key)).sort()
    assert.deepEqual(missing, [], "a writable key is missing from the enumeration")
})

test("the stale names really are gone", () => {
    // These five were the defect: they read and wrote keys that no longer
    // existed, so the settings page and the statistics bar showed defaults and
    // the log filled with "unknown setting" warnings.
    for (const stale of [
        "selected_intent_recognition_engine",
        "selected_slot_extraction_engine",
        "selected_stt_engine",
        `key: "vad"`
    ]) {
        for (const path of frontendFiles(SOURCE_ROOT)) {
            // Comments are allowed to name the old keys: the panel explains them.
            const source = readFileSync(path, "utf8")
                .replace(/\/\*[\s\S]*?\*\//g, "")
                .replace(/^\s*\/\/.*$/gm, "")
            assert.equal(
                source.includes(stale),
                false,
                `${path} still names the stale setting ${stale}`
            )
        }
    }
})
