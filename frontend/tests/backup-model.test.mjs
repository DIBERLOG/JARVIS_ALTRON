import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    MIN_PASSWORD_BYTES,
    componentLabelKey,
    containerIsRestorable,
    formatBytes,
    operationKey,
    passwordProblem,
    previewWarnings,
    warningKey
} from "../src/lib/backup-model.ts"

/**
 * What these tests protect:
 *
 * * the interface never shows a path, a key, or the content of a record;
 * * the interface never stores the password;
 * * a restore is never one click;
 * * every component of the container and every code the core can send has a
 *   translated message in all three locales.
 */

const PANEL = fileURLToPath(
    new URL("../src/components/settings/BackupSettings.svelte", import.meta.url)
)
const MODEL = fileURLToPath(new URL("../src/lib/backup-model.ts", import.meta.url))
const API = fileURLToPath(new URL("../src/lib/backup.ts", import.meta.url))
const SETTINGS_ROUTE = fileURLToPath(
    new URL("../src/routes/settings/index.svelte", import.meta.url)
)
const LOCALES = ["en", "ru", "ua"]

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
        if (match) keys.add(match[1])
    }
    return keys
}

/** The error codes the core can produce, read from its own match arms. */
function coreErrorCodes() {
    const source = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-core/src/backup/error.rs", import.meta.url)),
        "utf8"
    )
    const start = source.indexOf("pub fn code(")
    assert.ok(start >= 0, "the module must still have a code()")
    const body = source.slice(start, source.indexOf("\n    pub fn ", start))
    const codes = new Set()
    for (const match of body.matchAll(/=>\s*"([a-z_]+)"/g)) codes.add(match[1])
    return codes
}

/** The logical component names the core can put in a container. */
function coreComponentNames() {
    const source = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-core/src/backup/snapshot.rs", import.meta.url)),
        "utf8"
    )
    const names = new Set()
    for (const match of source.matchAll(/name:\s*"([a-z0-9/._-]+)"/g)) names.add(match[1])
    return names
}

test("the panel is wired into the settings page and the API", () => {
    for (const path of [PANEL, MODEL, API]) {
        assert.ok(existsSync(path), `${path} must exist`)
    }
    const settings = readFileSync(SETTINGS_ROUTE, "utf8")
    assert.ok(settings.includes("BackupSettingsPanel"), "the section must be rendered")
    assert.ok(settings.includes("backup-tab"), "and it must have its own tab")
    const api = readFileSync(API, "utf8")
    for (const command of [
        "backup_status",
        "backup_export",
        "backup_inspect",
        "backup_restore",
        "backup_discard_previous",
        "backup_delete_safety"
    ]) {
        assert.ok(api.includes(command), `the API must call ${command}`)
    }
})

test("a component name becomes a feature label, never a file name", () => {
    assert.equal(componentLabelKey("notes/sync.sqlite3"), "backup-component-notes")
    assert.equal(componentLabelKey("vault/vault.sqlite3"), "backup-component-vault")
    assert.equal(componentLabelKey("memory/ai-memory.sqlite3"), "backup-component-memory")
    assert.equal(
        componentLabelKey("autocorrect/autocorrect.sqlite3"),
        "backup-component-autocorrect"
    )
    assert.equal(componentLabelKey("key/portable-envelope.json"), "backup-component-key")
    assert.equal(componentLabelKey("settings/app.db"), "backup-component-settings")
    assert.equal(componentLabelKey("something/unknown"), "backup-component-other")
    // No label ever carries a file name or an extension.
    for (const name of coreComponentNames()) {
        const key = componentLabelKey(name)
        assert.match(key, /^backup-component-[a-z]+$/)
        assert.equal(key.includes(".json"), false)
        assert.equal(key.includes(".db"), false)
    }
})

test("every error code of the core has a message in all three locales", () => {
    const codes = coreErrorCodes()
    assert.ok(codes.size >= 15, `expected a real code list, got ${codes.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...codes]
            .map((code) => `backup-error-${code}`)
            .filter((key) => !available.has(key))
            .sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a backup error message`)
    }
})

test("every component, warning and operation has a message in all three locales", () => {
    const required = new Set()
    for (const name of coreComponentNames()) {
        required.add(componentLabelKey(name))
    }
    required.add("backup-component-other")
    for (const code of ["no_vault", "no_notes", "no_key_envelope"]) {
        required.add(warningKey(code))
    }
    required.add(warningKey("something-else"))
    for (const operation of ["export", "inspect", "restore"]) {
        required.add(operationKey({ last_operation: operation, last_error_code: null }))
        required.add(operationKey({ last_operation: operation, last_error_code: "storage" }))
    }
    // With no operation yet, there is one message whatever the error code is: a
    // failure of an operation that never ran does not exist.
    required.add(operationKey({ last_operation: null, last_error_code: null }))
    assert.equal(
        operationKey({ last_operation: null, last_error_code: "storage" }),
        "backup-operation-none"
    )
    // The panel's own keys.
    const panel = readFileSync(PANEL, "utf8")
    for (const match of panel.matchAll(/t\(\s*['"](backup-[a-z0-9_-]+)['"]/g)) {
        required.add(match[1])
    }
    for (const match of readFileSync(MODEL, "utf8").matchAll(/"(backup-[a-z0-9_-]+)"/g)) {
        required.add(match[1])
    }
    assert.ok(required.size > 40, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a backup message`)
    }
})

test("the three locales stay in sync for backup messages", () => {
    const reference = [...messageKeys("en")].filter((key) => key.startsWith("backup-"))
    assert.ok(reference.length > 70, `expected backup messages, got ${reference.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = reference.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a backup message`)
    }
})

test("the password check matches the rule the core enforces", () => {
    assert.equal(MIN_PASSWORD_BYTES, 8)
    assert.equal(passwordProblem(""), "backup-password-empty")
    assert.equal(passwordProblem("short"), "backup-password-short")
    assert.equal(passwordProblem("long-enough"), null)
    // The core counts bytes, so the interface has to as well: two Cyrillic
    // characters are four bytes, which is under the minimum.
    assert.equal(passwordProblem("да"), "backup-password-short")
    assert.equal(passwordProblem("пароль"), null)
    assert.equal(passwordProblem("пароль-подлиннее"), null)
})

test("a size is readable and never a path", () => {
    assert.equal(formatBytes(0), "0")
    assert.equal(formatBytes(512), "512")
    assert.equal(formatBytes(2048), "2.0 KiB")
    assert.equal(formatBytes(5 * 1024 * 1024), "5.0 MiB")
    assert.equal(formatBytes(Number.NaN), "0")
})

test("a container without notes or a key envelope is not offered for restore", () => {
    const full = {
        entries: [
            { name: "notes/sync.sqlite3", kind: "sqlite", bytes: 1 },
            { name: "key/portable-envelope.json", kind: "portable_key", bytes: 1 }
        ]
    }
    assert.equal(containerIsRestorable(full), true)
    assert.equal(
        containerIsRestorable({ entries: [{ name: "key/portable-envelope.json" }] }),
        false
    )
    assert.equal(containerIsRestorable({ entries: [] }), false)
    assert.deepEqual(
        previewWarnings({ warnings: ["no_vault", "no_notes"] }),
        ["backup-warning-no_vault", "backup-warning-no_notes"]
    )
})

test("the panel never stores the password and never restores in one click", () => {
    const panel = readFileSync(PANEL, "utf8")
    for (const forbidden of [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "document.cookie",
        "location.hash",
        "window.history",
        "console.log"
    ]) {
        assert.equal(panel.includes(forbidden), false, `the panel must not use ${forbidden}`)
    }
    // The password lives in one variable, is passed once per call, and is never
    // written anywhere.
    assert.ok(panel.includes('type="password"'), "the field must be a password field")
    // A restore only exists after a container was opened and verified: the
    // confirm button is inside the preview branch, and nothing before that branch
    // is wired to it.
    const markup = panel.slice(panel.indexOf("<Text weight={700}"))
    const previewBranch = markup.slice(
        markup.indexOf("{#if preview}"),
        markup.indexOf("{#if restored}")
    )
    assert.ok(previewBranch.includes("backup-restore"), "the restore button follows the preview")
    assert.equal(
        markup.slice(0, markup.indexOf("{#if preview}")).includes("on:click={restoreContainer}"),
        false,
        "no button may restore before a container is shown"
    )
    assert.ok(panel.includes("backupApi.restore(password, true)"), "the core is told the person confirmed")
    assert.equal(panel.includes("sendTextCommand"), false)
})

test("the panel says nothing about a path", () => {
    const panel = readFileSync(PANEL, "utf8")
    // A full path in the interface would be a path the person did not ask to see,
    // and it is exactly what the core refuses to hand over.
    assert.equal(/[A-Za-z]:\\\\/.test(panel), false, "no Windows path in the panel")
    assert.equal(panel.includes("data_dir"), false, "the panel does not know a directory")
})

test("no backup module keeps a secret in a DTO or a log", () => {
    // The Rust side: the DTOs and reports must not carry the password, the key,
    // or a note. This is a structural check of the shapes, not of a runtime value.
    const files = [
        "../../crates/jarvis-core/src/backup/mod.rs",
        "../../crates/jarvis-core/src/backup/container.rs",
        "../../crates/jarvis-core/src/backup/restore.rs"
    ]
    for (const file of files) {
        const source = readFileSync(fileURLToPath(new URL(file, import.meta.url)), "utf8")
        // A `Password` field in a serializable struct would travel to the window.
        assert.equal(
            /pub\s+password\s*:/.test(source),
            false,
            `${file} must not carry a password in a value`
        )
        assert.equal(
            /log::(info|warn|error)!\([^)]*password/.test(source),
            false,
            `${file} must not log a password`
        )
        assert.equal(
            /log::(info|warn|error)!\([^)]*master_key|log::(info|warn|error)!\([^)]*envelope/.test(
                source
            ),
            false,
            `${file} must not log key material`
        )
    }
})
