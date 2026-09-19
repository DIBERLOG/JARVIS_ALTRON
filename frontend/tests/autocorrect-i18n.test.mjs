import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    CHAT_SCOPE,
    DIFF_KINDS,
    IMPROVEMENT_MODES,
    IMPROVEMENT_WARNINGS,
    ISSUE_REASONS,
    LANGUAGES,
    LANGUAGE_MODES,
    SUGGESTION_SOURCES,
    CORRECTION_OUTCOMES,
    diffKindLabelKey,
    improvementModeLabelKey,
    improvementWarningLabelKey,
    languageLabelKey,
    languageModeLabelKey,
    outcomeLabelKey,
    reasonLabelKey,
    sourceLabelKey
} from "../src/lib/autocorrect-model.ts"

const LOCALES = ["en", "ru", "ua"]
const MODEL_FILE = fileURLToPath(new URL("../src/lib/autocorrect-model.ts", import.meta.url))
const API_FILE = fileURLToPath(new URL("../src/lib/autocorrect.ts", import.meta.url))
const PANEL_FILE = fileURLToPath(
    new URL("../src/components/notes/SpellingPanel.svelte", import.meta.url)
)
const IMPROVE_FILE = fileURLToPath(
    new URL("../src/components/ai/TextImprovementPanel.svelte", import.meta.url)
)
const SETTINGS_FILE = fileURLToPath(
    new URL("../src/components/settings/AutocorrectSettings.svelte", import.meta.url)
)
const NOTES_FILE = fileURLToPath(new URL("../src/routes/notes/index.svelte", import.meta.url))
const CHAT_FILE = fileURLToPath(new URL("../src/components/ai/LocalChat.svelte", import.meta.url))
const SETTINGS_ROUTE = fileURLToPath(
    new URL("../src/routes/settings/index.svelte", import.meta.url)
)

/** Every interface source of the autocorrect feature. */
function sources() {
    return [
        MODEL_FILE,
        API_FILE,
        PANEL_FILE,
        IMPROVE_FILE,
        SETTINGS_FILE,
        NOTES_FILE,
        CHAT_FILE,
        SETTINGS_ROUTE
    ].filter((path) => existsSync(path))
}

/** Keys the model builds at run time from a fixed set of values. */
const FAMILY_KEYS = [
    ...LANGUAGES.map(languageLabelKey),
    ...LANGUAGE_MODES.map(languageModeLabelKey),
    ...ISSUE_REASONS.map(reasonLabelKey),
    ...SUGGESTION_SOURCES.map(sourceLabelKey),
    ...CORRECTION_OUTCOMES.map(outcomeLabelKey),
    ...IMPROVEMENT_MODES.map(improvementModeLabelKey),
    ...IMPROVEMENT_WARNINGS.map(improvementWarningLabelKey),
    ...DIFF_KINDS.map(diffKindLabelKey),
    "autocorrect-dictionary-ready",
    "autocorrect-dictionary-missing",
    "autocorrect-dictionary-invalid"
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

function stripComments(source) {
    return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "")
}

function stripMarkupNoise(source) {
    return stripComments(source)
        .replace(/<style[\s\S]*?<\/style>/g, "")
        .replace(/class="[^"]*"/g, "")
        .replace(/class:([a-z0-9-]+)/g, "")
}

/** Literal `t('autocorrect-...')` usages in every autocorrect source. */
function literalKeys() {
    const keys = new Set()
    for (const path of sources()) {
        const copy = stripMarkupNoise(readFileSync(path, "utf8"))
        for (const match of copy.matchAll(/\bt\(\s*['"](autocorrect-[a-z0-9-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    return keys
}

/** Keys the model returns as string literals. */
function modelLiteralKeys() {
    const source = readFileSync(MODEL_FILE, "utf8")
    const keys = new Set()
    for (const match of source.matchAll(/"(autocorrect-[a-z0-9-]+)"/g)) {
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

test("the spelling interface exists and is wired into the shell", () => {
    for (const path of [PANEL_FILE, IMPROVE_FILE, SETTINGS_FILE, API_FILE, MODEL_FILE]) {
        assert.ok(existsSync(path), `${path} must exist`)
    }
    const settingsRoute = readFileSync(SETTINGS_ROUTE, "utf8")
    assert.ok(settingsRoute.includes("autocorrect-settings-tab"), "the settings page needs a tab")
    assert.ok(settingsRoute.includes("AutocorrectSettings"), "the tab must render the settings")
    // Notes and chat are the two places the feature is integrated.
    for (const path of [NOTES_FILE, CHAT_FILE]) {
        const source = readFileSync(path, "utf8")
        assert.ok(source.includes("SpellingPanel"), `${path} must show the spelling panel`)
        assert.ok(source.includes("autocorrectApi"), `${path} must use the autocorrect API`)
    }
    assert.ok(
        readFileSync(CHAT_FILE, "utf8").includes("CHAT_SCOPE"),
        "the chat uses its own undo scope"
    )
})

test("every key used by the spelling interface exists in all locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 90, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing autocorrect messages`)
    }
})

test("the three locales stay in sync for autocorrect messages", () => {
    const reference = messageKeys("en")
    const keys = [...reference].filter((key) => key.startsWith("autocorrect-"))
    assert.ok(keys.length > 90, `expected autocorrect messages, got ${keys.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = keys.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing autocorrect messages`)
    }
})

test("no autocorrect message is left unused by the interface", () => {
    const required = requiredKeys()
    const english = messageKeys("en")
    const keys = [...english].filter((key) => key.startsWith("autocorrect-"))
    const unused = keys.filter((key) => !required.has(key)).sort()
    assert.deepEqual(unused, [], "these autocorrect messages are defined but never used")
})

test("no locale is missing the messages the panels ask for at run time", () => {
    // A key built from a value list must exist for every value in that list, in every
    // locale: a missing one would render the raw key to the user.
    for (const language of LOCALES) {
        const available = messageKeys(language)
        for (const key of FAMILY_KEYS) {
            assert.ok(available.has(key), `${language}.ftl is missing ${key}`)
        }
    }
})

test("the spelling interface never builds a translation key in a template", () => {
    for (const path of sources()) {
        const source = readFileSync(path, "utf8")
        assert.equal(/t\(\s*`/.test(source), false, `${path} builds a translation key at run time`)
    }
})

test("the spelling interface never persists anything in the browser", () => {
    const forbidden = [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "window.history",
        "location.hash",
        "location.search",
        "document.cookie"
    ]
    for (const path of sources()) {
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of forbidden) {
            assert.equal(
                code.includes(needle),
                false,
                `${path} must not use ${needle}: the word list lives in the encrypted database`
            )
        }
    }
})

test("the spelling interface never talks to a model, a network, or a database itself", () => {
    // Only the files this feature adds are scanned: the settings route is a pre-existing
    // page that legitimately links to documentation on the web.
    for (const path of [MODEL_FILE, API_FILE, PANEL_FILE, IMPROVE_FILE, SETTINGS_FILE]) {
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of ["fetch(", "XMLHttpRequest", "WebSocket", "EventSource", "http://", "https://", "sqlite", "SQL"]) {
            assert.equal(code.includes(needle), false, `${path} must not use ${needle}`)
        }
        // The Tauri bridge belongs to the api wrapper and nowhere else.
        if (path !== API_FILE) {
            assert.equal(
                code.includes("@tauri-apps/api"),
                false,
                `${path} must use the autocorrect api wrapper`
            )
        }
    }
})

test("the spelling interface cannot reach the password vault", () => {
    // The vault is deliberately outside this feature: no page offers to check a stored
    // password, and no component may even hold a handle to it.
    const forbidden = ["@/lib/vault", "vaultApi", "vault-model", "revealSecret", "vault_reveal"]
    for (const path of sources()) {
        const code = stripComments(readFileSync(path, "utf8"))
        for (const needle of forbidden) {
            assert.equal(code.includes(needle), false, `${path} must not reference ${needle}`)
        }
    }
})

test("an AI change is only ever applied through a confirmed preview", () => {
    const api = readFileSync(API_FILE, "utf8")
    // The two steps are separate commands.
    assert.ok(api.includes("autocorrect_improve_text"), "the preview command must exist")
    assert.ok(api.includes("autocorrect_apply_improvement"), "the apply command must exist")
    assert.ok(api.includes("autocorrect_cancel_improvement"), "a running generation can be stopped")

    for (const path of [NOTES_FILE, CHAT_FILE]) {
        const code = stripComments(readFileSync(path, "utf8"))
        // Each editor calls the preview command and the apply command, and the apply
        // handler refuses to run without a preview it holds.
        assert.ok(code.includes("improveText"), `${path} must ask for a preview`)
        assert.ok(code.includes("applyImprovement"), `${path} must apply only a preview`)
        assert.ok(
            code.includes("!improvePreview) return"),
            `${path} must refuse to apply without a preview`
        )
        assert.ok(
            code.includes("improvePreview.version_before"),
            `${path} must send the version the preview was made from`
        )
    }

    // The preview component renders the whole difference, so a user always sees what
    // would change before it changes.
    const improve = readFileSync(IMPROVE_FILE, "utf8")
    assert.ok(improve.includes("preview.diff.segments"), "the preview must show the difference")
    assert.ok(improve.includes("isPreviewApplicable"), "the preview must be validated")
    assert.ok(improve.includes("previewStatsLine"), "the change counts must be visible")
})

test("a correction is applied only with the version the check reported", () => {
    for (const path of [NOTES_FILE, CHAT_FILE]) {
        const code = stripComments(readFileSync(path, "utf8"))
        assert.ok(
            code.includes("spellReport.version"),
            `${path} must send the checked version with a correction`
        )
        assert.ok(
            code.includes("spellCheckedText !== draft") || code.includes("spellCheckedText !== draft.body"),
            `${path} must refuse a correction for text that changed`
        )
    }
})

test("the panels never apply a suggestion the backend did not report", () => {
    const panel = stripComments(readFileSync(PANEL_FILE, "utf8"))
    // Every correction is built from the issue the backend sent: the range, the word,
    // and a suggestion from its list. The panel never computes a range of its own.
    assert.ok(panel.includes("original: issue.word"), "the original text must come from the issue")
    assert.ok(panel.includes("range: issue.range"), "the range must come from the issue")
    assert.equal(
        /utf16\s*:/.test(panel),
        false,
        "the panel must not build a range from UTF-16 offsets"
    )
})

test("the settings page never offers to disable the preview", () => {
    const settings = stripComments(readFileSync(SETTINGS_FILE, "utf8"))
    assert.equal(
        /bind:checked=\{settings\.require_preview\}/.test(settings),
        false,
        "require_preview must not be switchable"
    )
    assert.ok(settings.includes("require-preview"), "the page explains why a preview is required")
})
