import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import { isRunning, stageKey, startProblem } from "../src/lib/voice-input.ts"

/**
 * The global voice input, checked where a static check can reach it:
 *
 * * the six commands (and the seventh, copy again) exist in the Rust command
 *   layer and are registered with Tauri;
 * * the interface types match the Rust DTO field for field;
 * * the pipeline is the clipboard, and nothing in the sources synthesizes a
 *   keystroke, runs a shell, or stores the text;
 * * every key the panel asks for exists in all three locales.
 */

const ROOT = new URL("../../", import.meta.url)
const PANEL = fileURLToPath(new URL("../src/components/settings/VoiceInputSettings.svelte", import.meta.url))
const API = fileURLToPath(new URL("../src/lib/voice-input.ts", import.meta.url))
const SETTINGS_ROUTE = fileURLToPath(new URL("../src/routes/settings/index.svelte", import.meta.url))
const RUST_COMMANDS = fileURLToPath(new URL("../../crates/jarvis-gui/src/tauri_commands/voice_input.rs", import.meta.url))
const RUST_MAIN = fileURLToPath(new URL("../../crates/jarvis-gui/src/main.rs", import.meta.url))
const CORE_SETTINGS = fileURLToPath(new URL("../../crates/jarvis-core/src/dictation/mod.rs", import.meta.url))
const LOCALES = ["en", "ru", "ua"]

function localeKeys(language) {
    const path = fileURLToPath(
        new URL(`../../crates/jarvis-core/src/i18n/locales/${language}.ftl`, import.meta.url)
    )
    const keys = new Set()
    for (const line of readFileSync(path, "utf8").split("\n")) {
        const trimmed = line.trim()
        if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("-")) continue
        const match = /^([A-Za-z0-9_-]+)\s*=/.exec(trimmed)
        if (match) keys.add(match[1])
    }
    return keys
}

const COMMANDS = [
    "voice_input_status",
    "voice_input_start",
    "voice_input_cancel",
    "voice_input_update_settings",
    "voice_input_clear_result",
    "voice_input_preview",
    "voice_input_copy_again"
]

test("every voice input command exists in the core and is registered", () => {
    const commands = readFileSync(RUST_COMMANDS, "utf8")
    const main = readFileSync(RUST_MAIN, "utf8")
    for (const command of COMMANDS) {
        assert.ok(
            commands.includes(`pub async fn ${command}(`),
            `${command} must exist in the command layer`
        )
        assert.ok(
            main.includes(`tauri_commands::${command},`),
            `${command} must be registered with Tauri`
        )
    }
    // And the window's API calls the six the panel uses.
    const api = readFileSync(API, "utf8")
    for (const command of COMMANDS) {
        assert.ok(api.includes(command), `the interface must call ${command}`)
    }
})

test("the interface DTO matches the Rust one field for field", () => {
    const api = readFileSync(API, "utf8")
    const rust = readFileSync(RUST_COMMANDS, "utf8")
    // The settings shape: every field the Rust struct has is in the interface.
    const settingsStart = rust.indexOf("pub struct GlobalDictationSettings")
    const core = readFileSync(CORE_SETTINGS, "utf8")
    const structStart = core.indexOf("pub struct GlobalDictationSettings")
    const structEnd = core.indexOf("}", structStart)
    const fields = [...core.slice(structStart, structEnd).matchAll(/pub ([a-z_]+):/g)].map(
        (match) => match[1]
    )
    assert.ok(fields.length >= 8, `expected the settings fields, got ${fields.length}`)
    for (const field of fields) {
        assert.ok(
            api.includes(`${field}:`) || api.includes(`${field}?`),
            `the interface must carry ${field}`
        )
    }
    // The view carries a stage, a length and codes — and never the text.
    assert.ok(api.includes("stage: VoiceInputStage"))
    assert.ok(api.includes("characters: number"))
    const viewStart = rust.indexOf("pub struct VoiceInputView")
    const viewEnd = rust.indexOf("\n}", viewStart)
    const view = rust.slice(viewStart, viewEnd)
    assert.ok(!/pub text:/.test(view), "the view must not carry the text")
    assert.ok(!/window/.test(view), "the view must not carry a window")
})

test("the production mode is the clipboard and no keystroke is synthesized", () => {
    const rust = readFileSync(RUST_COMMANDS, "utf8")
    const core = readFileSync(CORE_SETTINGS, "utf8")
    assert.ok(
        rust.includes("VoiceInputPreference::Clipboard"),
        "the route must run in the clipboard mode"
    )
    assert.ok(
        core.includes("preference: VoiceInputPreference::Clipboard"),
        "the default preference must be the clipboard"
    )
    for (const forbidden of ["SendInput", "keybd_event", "mouse_event", "Command::new", "powershell"]) {
        assert.equal(rust.includes(forbidden), false, `the route must not contain ${forbidden}`)
    }
    // The clipboard is the vault's own guard, not a second manager.
    assert.ok(rust.includes("ClipboardGuard<SystemClipboard>"))
    // And there is no second recorder or session.
    assert.equal(rust.includes("KeyboardCapture"), false)
    assert.ok(rust.includes("WhisperHandle"), "the dictation is the existing handle")
})

test("the panel and the settings route are wired, and the text stays out of storage", () => {
    assert.ok(existsSync(PANEL), "the panel must exist")
    const panel = readFileSync(PANEL, "utf8")
    const settings = readFileSync(SETTINGS_ROUTE, "utf8")
    assert.ok(settings.includes("VoiceInputSettingsPanel"), "the tab must render the panel")
    assert.ok(settings.includes("voice-input-tab"), "the tab needs a label")
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
    for (const key of [
        "voice-input-start",
        "voice-input-cancel",
        "voice-input-copy-again",
        "voice-input-clear",
        "voice-input-ctrl-v"
    ]) {
        assert.ok(panel.includes(key), `the panel needs ${key}`)
    }
})

test("the tray has the voice input rows and its disabled states", () => {
    const desktop = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-gui/src/desktop.rs", import.meta.url)),
        "utf8"
    )
    assert.ok(desktop.includes('"voice_input_start"'), "a tray row must start the route")
    assert.ok(desktop.includes('"voice_input_stop"'), "a tray row must cancel it")
    assert.ok(desktop.includes("voice_input_stage_label"), "the tray shows a stage")
    // The start row is disabled when the feature is off, when the dictation is
    // not configured, and while one is running.
    assert.ok(desktop.includes("ready && !running"), "the start row needs its disabled states")
    assert.ok(desktop.includes("running,\n        None::<&str>"), "cancelling stays available")
})

test("every voice input key exists in all three locales", () => {
    const panel = readFileSync(PANEL, "utf8")
    const api = readFileSync(API, "utf8")
    const required = new Set()
    for (const source of [panel, api]) {
        for (const match of source.matchAll(/["'`](voice-input-[a-z0-9_-]+)["'`]/g)) {
            // The event name is not a message: it never reaches a translation.
            if (match[1] === "voice-input-notice") continue
            required.add(match[1])
        }
    }
    for (const match of panel.matchAll(/t\(\s*['"](voice-input-[a-z0-9_-]+)['"]/g)) {
        required.add(match[1])
    }
    // The stages and the errors are built from values, so they are listed here.
    for (const stage of [
        "idle",
        "preparing",
        "confirming",
        "handover",
        "recording",
        "transcribing",
        "correcting",
        "inserting",
        "delivered",
        "failed",
        "cancelled"
    ]) {
        required.add(stageKey(stage))
    }
    for (const code of [
        "disabled",
        "busy",
        "microphone-busy",
        "recorder-unavailable",
        "transcription-failed",
        "audio-empty",
        "cancelled",
        "no-delivery-path",
        "voice-host-unavailable",
        "window-changed",
        "target-refused",
        "unavailable"
    ]) {
        required.add(`voice-input-error-${code}`)
    }
    for (const problem of ["disabled", "whisper", "vosk", "busy"]) {
        required.add(`voice-input-problem-${problem}`)
    }
    assert.ok(required.size > 40, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = localeKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a voice input message`)
    }
})

test("the stage helpers say what the panel needs", () => {
    assert.equal(stageKey("recording"), "voice-input-stage-recording")
    assert.equal(isRunning("recording"), true)
    assert.equal(isRunning("transcribing"), true)
    assert.equal(isRunning("idle"), false)
    assert.equal(isRunning("delivered"), false)
    assert.equal(isRunning("cancelled"), false)

    const base = {
        settings: { enabled: true, phrase: "x" },
        status: { stage: "idle", characters: 0, has_text: false },
        whisper_configured: true,
        vosk_available: true
    }
    assert.equal(startProblem(base), null)
    assert.equal(
        startProblem({ ...base, settings: { enabled: false } }),
        "voice-input-problem-disabled"
    )
    assert.equal(
        startProblem({ ...base, whisper_configured: false }),
        "voice-input-problem-whisper"
    )
    assert.equal(startProblem({ ...base, vosk_available: false }), "voice-input-problem-vosk")
    assert.equal(
        startProblem({ ...base, status: { stage: "recording", characters: 0, has_text: false } }),
        "voice-input-problem-busy"
    )
})
