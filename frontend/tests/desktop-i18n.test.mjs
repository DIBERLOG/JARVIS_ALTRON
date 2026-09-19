import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    AUTOSTART_SWITCHES,
    CLOSE_CHOICES,
    SETUP_LANGUAGES,
    SETUP_STEPS,
    aiStateKey,
    autostartStateKey,
    autostartSwitchKey,
    closeBehaviorHintKey,
    closeBehaviorKey,
    componentNameKey,
    componentStateKey,
    microphoneStateKey,
    setupStepHintKey,
    setupStepKey
} from "../src/lib/desktop-model.ts"

const LOCALES = ["en", "ru", "ua"]
const MODEL_FILE = fileURLToPath(new URL("../src/lib/desktop-model.ts", import.meta.url))
const API_FILE = fileURLToPath(new URL("../src/lib/desktop.ts", import.meta.url))
const CLOSE_FILE = fileURLToPath(new URL("../src/components/desktop/CloseDialog.svelte", import.meta.url))
const SETTINGS_FILE = fileURLToPath(
    new URL("../src/components/desktop/DesktopSettings.svelte", import.meta.url)
)
const WIZARD_FILE = fileURLToPath(
    new URL("../src/components/desktop/FirstRunWizard.svelte", import.meta.url)
)
const DIAGNOSTICS_FILE = fileURLToPath(
    new URL("../src/components/desktop/DiagnosticsPanel.svelte", import.meta.url)
)
const SHELL_FILE = fileURLToPath(new URL("../src/routes/index.svelte", import.meta.url))
const SETTINGS_ROUTE = fileURLToPath(new URL("../src/routes/settings/index.svelte", import.meta.url))

function sources() {
    return [MODEL_FILE, API_FILE, CLOSE_FILE, SETTINGS_FILE, WIZARD_FILE, DIAGNOSTICS_FILE].filter(
        (path) => existsSync(path)
    )
}

/** The component names the diagnostics report can carry. */
const COMPONENT_NAMES = [
    "data_directory",
    "sqlite",
    "notes_vault_memory",
    "llama_server",
    "local_model",
    "whisper_binary",
    "whisper_model",
    "vosk_runtime",
    "dictionaries",
    "microphone",
    "windows_actions",
    "core_audio",
    "screenshot_backend",
    "autostart",
    "tray",
    "webview2",
    "windows_notifications",
    "local_ai_state",
    "dictation_ready",
    "microphone_permission",
    "action_scheduler"
]

const FAMILY_KEYS = [
    ...CLOSE_CHOICES.flatMap((choice) => [closeBehaviorKey(choice), closeBehaviorHintKey(choice)]),
    "desktop-close-ask",
    "desktop-close-ask-hint",
    ...["enabled", "disabled", "needs_attention", "unavailable"].map(autostartStateKey),
    ...AUTOSTART_SWITCHES.map(autostartSwitchKey),
    ...["idle", "vosk_listening", "whisper_dictation", "transcribing_file", "stopping", "failed"].map(
        microphoneStateKey
    ),
    ...["not_configured", "stopped", "ready", "busy"].map(aiStateKey),
    ...[
        "ready",
        "missing",
        "invalid",
        "locked",
        "wrong_architecture",
        "version_unknown",
        "incompatible",
        "unavailable",
        "permission_denied",
        "not_configured",
        "disabled"
    ].map(componentStateKey),
    ...COMPONENT_NAMES.map(componentNameKey),
    ...SETUP_STEPS.flatMap((step) => [setupStepKey(step), setupStepHintKey(step)]),
    "desktop-diagnostics-all-ready",
    "desktop-diagnostics-needs-attention",
    "desktop-status-available",
    "desktop-status-unavailable"
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
    for (const path of [...sources(), SHELL_FILE, SETTINGS_ROUTE].filter((path) => existsSync(path))) {
        const copy = stripMarkupNoise(readFileSync(path, "utf8"))
        for (const match of copy.matchAll(/\bt\(\s*['"]((?:desktop|setup)-[a-z0-9_-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    return keys
}

function requiredKeys() {
    const keys = new Set(literalKeys())
    for (const key of FAMILY_KEYS) keys.add(key)
    return keys
}

test("the desktop shell exists and is wired into the shell and the settings page", () => {
    for (const path of [
        MODEL_FILE,
        API_FILE,
        CLOSE_FILE,
        SETTINGS_FILE,
        WIZARD_FILE,
        DIAGNOSTICS_FILE
    ]) {
        assert.ok(existsSync(path), `${path} must exist`)
    }
    const shell = readFileSync(SHELL_FILE, "utf8")
    assert.ok(shell.includes("CloseDialog"), "the shell must show the close dialog")
    assert.ok(shell.includes("FirstRunWizard"), "the shell must show the wizard")
    assert.ok(shell.includes("onCloseRequested"), "the shell must answer the core's close request")
    const settings = readFileSync(SETTINGS_ROUTE, "utf8")
    for (const tab of [
        "desktop-tab",
        "desktop-privacy-tab",
        "desktop-diagnostics-tab",
        "desktop-about-tab"
    ]) {
        assert.ok(settings.includes(tab), `the settings page needs the ${tab} tab`)
    }
})

test("every key the shell asks for exists in all three locales", () => {
    const required = requiredKeys()
    assert.ok(required.size > 120, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messageKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing desktop messages`)
    }
})

test("the three locales stay in sync for the desktop shell", () => {
    const reference = messageKeys("en")
    const keys = [...reference].filter(
        (key) => key.startsWith("desktop-") || key.startsWith("setup-")
    )
    assert.ok(keys.length > 120, `expected desktop messages, got ${keys.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = messageKeys(language)
        const missing = keys.filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing desktop messages`)
    }
})

test("the interface never writes a registry key, touches the tray, or builds a report", () => {
    for (const path of sources()) {
        const source = stripComments(readFileSync(path, "utf8"))
        for (const forbidden of [
            "localStorage",
            "sessionStorage",
            "indexedDB",
            "document.cookie",
            "windows.confirm",
            "require(",
            "reg add",
            "reg.exe",
            "HKEY_",
            "powershell",
            "cmd.exe"
        ]) {
            assert.equal(
                source.includes(forbidden),
                false,
                `${path} must not use ${forbidden}: the core owns the platform`
            )
        }
    }
    const api = readFileSync(API_FILE, "utf8")
    // The autostart and the tray are asked for by name, not driven from here.
    assert.ok(api.includes("autostart_enable"))
    assert.ok(api.includes("desktop_hide_window"))
    // The report is built by the core; the window only asks for it.
    assert.ok(api.includes("diagnostics_run"))
    assert.equal(/JSON\.stringify\([^)]*components/.test(api), false)
})

test("the wizard never handles a password", () => {
    const wizard = readFileSync(WIZARD_FILE, "utf8")
    for (const forbidden of ["password", "passphrase", "master_key", "secret"]) {
        assert.equal(
            stripComments(wizard).toLowerCase().includes(forbidden),
            false,
            `the wizard must not mention ${forbidden}: the storage page owns it`
        )
    }
    // The storage step points at the page that does own it.
    assert.ok(wizard.includes("setup-storage-body"))
    assert.ok(wizard.includes("setup-open-settings"))
})

test("the close dialog offers a way to cancel and to remember", () => {
    const dialog = readFileSync(CLOSE_FILE, "utf8")
    assert.ok(dialog.includes("desktop-close-cancel"))
    assert.ok(dialog.includes("desktop-close-remember"))
    assert.ok(dialog.includes("desktopApi.hideWindow"))
    assert.ok(dialog.includes("desktopApi.requestExit"))
    // The dialog does not decide what a close means: it sends the answer.
    assert.equal(dialog.includes("process.exit"), false)
})
