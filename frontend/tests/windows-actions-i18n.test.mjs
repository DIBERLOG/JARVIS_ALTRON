import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    ACTION_SOURCES,
    ACTION_STATUSES,
    ACTION_TYPES,
    ACTION_RISKS,
    ERROR_CODES,
    SCREENSHOT_TARGETS,
    SCHEDULED_KINDS,
    SCHEDULED_STATUSES,
    TOOL_REASONS,
    VOICE_REASONS,
    VOLUME_DIRECTIONS,
    WINDOW_OPERATIONS,
    WINDOW_STATES,
    actionTypeKey,
    directionKey,
    errorKey,
    riskKey,
    scheduledKindKey,
    scheduledStatusKey,
    screenshotTargetKey,
    sourceKey,
    statusKey,
    toolAvailabilityKey,
    voiceReasonKey,
    windowOperationKey,
    windowStateKey
} from "../src/lib/windows-actions-model.ts"

const LOCALES = ["en", "ru", "ua"]
const MODEL_FILE = fileURLToPath(new URL("../src/lib/windows-actions-model.ts", import.meta.url))
const API_FILE = fileURLToPath(new URL("../src/lib/windows-actions.ts", import.meta.url))
const PANEL_FILE = fileURLToPath(
    new URL("../src/components/windows/WindowsActionsPanel.svelte", import.meta.url)
)
const DIALOG_FILE = fileURLToPath(
    new URL("../src/components/windows/ConfirmationDialog.svelte", import.meta.url)
)
const SETTINGS_ROUTE = fileURLToPath(
    new URL("../src/routes/settings/index.svelte", import.meta.url)
)

function sources() {
    return [MODEL_FILE, API_FILE, PANEL_FILE, DIALOG_FILE].filter((path) => existsSync(path))
}

/**
 * The labels the *core* puts into an `ActionPreview`.
 *
 * They are not built by this interface, so they cannot be found by reading its sources; they
 * are listed here because a missing one would show the user a raw key in the dialog.
 */
const CORE_KEYS = [
    "windows-confirm-action",
    "windows-confirm-volume",
    "windows-confirm-screenshot",
    "windows-confirm-launch",
    "windows-confirm-close",
    "windows-confirm-lock",
    "windows-confirm-reminder",
    "windows-consequence-launch",
    "windows-consequence-screenshot",
    "windows-consequence-close",
    "windows-consequence-lock",
    "windows-consequence-reminder"
]

const FIELD_KEYS = [
    "windows-field-action",
    "windows-field-application",
    "windows-field-executable",
    "windows-field-arguments",
    "windows-field-target",
    "windows-field-screenshot-folder",
    "windows-field-operation",
    "windows-field-window",
    "windows-field-delay",
    "windows-field-reminder",
    "windows-field-direction",
    "windows-field-step"
]

/**
 * The keys the model builds from a value list.
 *
 * A key built at run time cannot be found by reading the sources, so the lists are walked
 * instead: a key that is missing from a locale would be shown to the user as itself.
 */
const FAMILY_KEYS = [
    ...ACTION_SOURCES.map(sourceKey),
    ...ACTION_STATUSES.map(statusKey),
    ...ACTION_RISKS.map(riskKey),
    ...ACTION_TYPES.map(actionTypeKey),
    ...WINDOW_OPERATIONS.map(windowOperationKey),
    ...WINDOW_STATES.map(windowStateKey),
    ...SCHEDULED_KINDS.map(scheduledKindKey),
    ...SCHEDULED_STATUSES.map(scheduledStatusKey),
    ...VOLUME_DIRECTIONS.map(directionKey),
    ...SCREENSHOT_TARGETS.map((target) => screenshotTargetKey({ target })),
    ...ERROR_CODES.map(errorKey),
    ...VOICE_REASONS.map((reason) => voiceReasonKey(`windows-voice-${reason}`)),
    ...TOOL_REASONS.map((reason) =>
        toolAvailabilityKey({ available: "unavailable", reason: `windows-ai-tools-${reason}` })
    ),
    "windows-actions-error-unknown",
    "windows-actions-voice-not-an-action",
    "windows-actions-tools-available",
    "windows-actions-error-timer-range",
    "windows-actions-error-reminder-range",
    "windows-actions-error-reminder-empty",
    "windows-actions-error-reminder-too-long",
    "windows-actions-error-name-empty"
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
        // Action names and error codes contain underscores, so the key pattern allows them.
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

/** Literal `t('windows-actions-...')` and `t("windows-field-...")` usages in every source. */
function literalKeys() {
    const keys = new Set()
    // The core chooses these keys and the dialog renders them verbatim.
    for (const key of FIELD_KEYS) keys.add(key)
    for (const key of CORE_KEYS) keys.add(key)
    // The settings page names the tab, and it is the only place that does.
    const settingsRoute = stripMarkupNoise(readFileSync(SETTINGS_ROUTE, "utf8"))
    for (const match of settingsRoute.matchAll(/\bt\(\s*['"](windows-(?:actions|field)-[a-z0-9_-]+)['"]/g)) {
        keys.add(match[1])
    }
    for (const path of sources()) {
        const copy = stripMarkupNoise(readFileSync(path, "utf8"))
        for (const match of copy.matchAll(/\bt\(\s*['"](windows-(?:actions|field)-[a-z0-9_-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    return keys
}

/** Keys the model returns as string literals. */
function modelLiteralKeys() {
    const source = readFileSync(MODEL_FILE, "utf8")
    const keys = new Set()
    for (const match of source.matchAll(/"(windows-(?:actions|field)-[a-z0-9_-]+)"/g)) {
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

test("the safe-command interface exists and is wired into the settings page", () => {
    for (const path of [PANEL_FILE, DIALOG_FILE, API_FILE, MODEL_FILE]) {
        assert.ok(existsSync(path), `${path} must exist`)
    }
    const settings = readFileSync(SETTINGS_ROUTE, "utf8")
    assert.ok(settings.includes("windows-actions-tab"), "the settings page needs a tab")
    assert.ok(settings.includes("WindowsActionsPanel"), "the tab must render the panel")
})

test("every key the interface asks for exists in all three locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 140, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing windows-actions messages`)
    }
})

test("the three locales stay in sync for windows-actions messages", () => {
    const reference = messageKeys("en")
    const keys = [...reference].filter((key) => key.startsWith("windows-actions-"))
    assert.ok(keys.length > 130, `expected windows-actions messages, got ${keys.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = keys.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing windows-actions messages`)
    }
    // The dialog's field labels are part of the same surface.
    const fields = [...reference].filter(
        (key) =>
            key.startsWith("windows-field-") ||
            key.startsWith("windows-confirm-") ||
            key.startsWith("windows-consequence-")
    )
    assert.ok(fields.length >= 24, `expected the dialog's own wording, got ${fields.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = fields.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing windows-field messages`)
    }
})

test("no windows-actions message is left unused", () => {
    const required = requiredKeys()
    const english = messageKeys("en")
    const keys = [...english].filter(
        (key) =>
            key.startsWith("windows-actions-") ||
            key.startsWith("windows-field-") ||
            key.startsWith("windows-confirm-") ||
            key.startsWith("windows-consequence-")
    )
    const unused = keys.filter((key) => !required.has(key)).sort()
    assert.deepEqual(unused, [], "these windows-actions messages are defined but never used")
})

test("the interface never builds a translation key in a template", () => {
    for (const path of [PANEL_FILE, DIALOG_FILE]) {
        const source = readFileSync(path, "utf8")
        assert.equal(/t\(\s*`/.test(source), false, `${path} builds a translation key at run time`)
    }
})

test("the interface keeps nothing in browser storage and opens no browser dialog", () => {
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
                `${path} must not use ${forbidden}: a pending approval expires and is not stored`
            )
        }
    }
})

test("the interface cannot name a program, a shell, or a command line", () => {
    for (const path of sources()) {
        const source = stripMarkupNoise(readFileSync(path, "utf8"))
        for (const forbidden of [
            "Command(",
            "shell",
            "powershell",
            "cmd.exe",
            "exec(",
            "spawn(",
            "executable_path",
            "canonical_executable_path"
        ]) {
            assert.equal(
                source.includes(forbidden),
                false,
                `${path} must not mention ${forbidden}: only the core may start a program`
            )
        }
        // The API surface may not offer a command that takes a path.
        assert.equal(
            /addAllowedApplication\(\s*[^)]*path/.test(source),
            false,
            `${path} must not accept a path from the interface`
        )
    }
    const api = readFileSync(API_FILE, "utf8")
    // The picker runs in the core, on a click; the interface only sends a display name.
    assert.ok(api.includes("windows_actions_add_allowed_application"))
    assert.equal(/invoke<[^>]*>\(\s*"windows_actions_[a-z_]*exec/.test(api), false)
})
