import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import { failureKeyOf, isBusy, stageKeyOf } from "../src/lib/conversation.ts"

/**
 * The conversation route, checked where a static check can reach it:
 *
 * * the commands exist in the window's command layer and are registered;
 * * the panel stores nothing, executes nothing, and shows the stages;
 * * the microphone belongs to one request at a time;
 * * the route cannot reach a command — not in the core, not in the host, not here;
 * * every key the panel needs exists in all three locales.
 */

const PANEL = fileURLToPath(new URL("../src/components/ai/ConversationPanel.svelte", import.meta.url))
const HOME = fileURLToPath(new URL("../src/routes/index.svelte", import.meta.url))
const API = fileURLToPath(new URL("../src/lib/conversation.ts", import.meta.url))
const IPC = fileURLToPath(new URL("../src/lib/ipc.ts", import.meta.url))
const RUST = fileURLToPath(
    new URL("../../crates/jarvis-gui/src/tauri_commands/conversation.rs", import.meta.url)
)
const RUST_MAIN = fileURLToPath(new URL("../../crates/jarvis-gui/src/main.rs", import.meta.url))
const APP = fileURLToPath(new URL("../../crates/jarvis-app/src/app.rs", import.meta.url))
const CORE = fileURLToPath(new URL("../../crates/jarvis-core/src/conversation/mod.rs", import.meta.url))
const LOCALES = ["en", "ru", "ua"]

const COMMANDS = [
    "conversation_status",
    "conversation_ask",
    "conversation_cancel",
    "conversation_stop",
    "conversation_clear",
    "conversation_set_profile"
]

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

test("every conversation command exists and is registered", () => {
    const rust = readFileSync(RUST, "utf8")
    const main = readFileSync(RUST_MAIN, "utf8")
    const api = readFileSync(API, "utf8")
    for (const command of COMMANDS) {
        assert.ok(rust.includes(`pub async fn ${command}(`), `${command} must exist`)
        assert.ok(main.includes(`tauri_commands::${command},`), `${command} must be registered`)
        assert.ok(api.includes(command), `the interface must call ${command}`)
    }
})

test("the route uses the one provider interface this build has", () => {
    const rust = readFileSync(RUST, "utf8")
    // The local gateway and the disabled provider, not a second trait of its own.
    assert.ok(rust.includes("jarvis_core::ai::{"), "it must use the AI module's types")
    assert.ok(rust.includes("ChatRequest"), "a question is a ChatRequest")
    assert.ok(rust.includes("DisabledProvider"), "nothing configured is a typed refusal")
    assert.ok(rust.includes("pub fn use_local_ai"), "the local server is the shared gateway")
    assert.ok(rust.includes("local_ai.shared()"), "and it is the gateway, not a second server")
    const core = readFileSync(CORE, "utf8")
    assert.equal(core.includes("pub trait ChatProvider"), false, "no second provider trait")
    assert.ok(core.includes("pub use crate::ai::ChatProvider;"), "the one trait, re-exported")
})

test("nothing in the route can execute a command", () => {
    const rust = readFileSync(RUST, "utf8")
    const core = readFileSync(CORE, "utf8")
    for (const forbidden of [
        "execute_command",
        "WindowsAction",
        "windows_actions",
        "SafetyGate",
        "Command::new",
        "std::process",
        "CommandPack"
    ]) {
        assert.equal(rust.includes(forbidden), false, `the route must not reach ${forbidden}`)
        assert.equal(core.includes(forbidden), false, `the core route must not reach ${forbidden}`)
    }
    // And the host decides the conversation before the actions and before the packs,
    // which is where an answer could otherwise become one.
    const app = readFileSync(APP, "utf8")
    const conversation = app.indexOf("conversation::intent_of(&normalized)")
    const actions = app.indexOf("try_windows_action(&normalized, &mut unclear)")
    assert.ok(conversation > 0 && conversation < actions, "the conversation goes first")
})

test("one question at a time, and the microphone goes back", () => {
    const rust = readFileSync(RUST, "utf8")
    // One request at a time, whatever the trigger.
    assert.ok(rust.includes("compare_exchange"), "the runtime must refuse a second question")
    assert.ok(rust.includes("ConversationError::Busy"), "and say so with a code")
    // The listener is restored by the dictation engine's guard, which is the one
    // piece of this that already had that property; the route reuses it.
    assert.ok(rust.includes("voice_input.ask()"), "the question is recorded by that engine")
    const dictation = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-core/src/dictation/session.rs", import.meta.url)),
        "utf8"
    )
    assert.ok(dictation.includes("struct ListenerGuard"), "the guard still exists")
    assert.ok(dictation.includes("keep_text"), "and the text can be kept for a question")
})

test("the panel is on the home page and stores nothing", () => {
    assert.ok(existsSync(PANEL), "the panel must exist")
    const panel = readFileSync(PANEL, "utf8")
    const home = readFileSync(HOME, "utf8")
    assert.ok(home.includes("<ConversationPanel />"), "the home page must show it")
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
    for (const stage of ["recording", "transcribing", "thinking", "answering"]) {
        assert.ok(panel.includes("stageKeyOf"), "the panel shows the stage")
        assert.ok(stageKeyOf(stage).startsWith("conversation-stage-"))
    }
    assert.ok(panel.includes("conversation-provider"), "the panel names the model")
    assert.ok(panel.includes("conversation-profile"), "the panel names the profile")
    assert.ok(panel.includes("answer"), "the panel shows the answer")
})

test("the events the host sends are handled and nothing else is started", () => {
    const ipc = readFileSync(IPC, "utf8")
    assert.ok(ipc.includes('case "conversation_requested"'), "the handover must be handled")
    assert.ok(ipc.includes('case "conversation_stopped"'), "a stop must be handled")
    assert.ok(ipc.includes("await invoke(\"conversation_ask\")"), "the window asks")
    assert.ok(ipc.includes("await invoke(\"conversation_stop\")"), "and can stop")
    // The event carries an intent, never a transcript, and no command is run from it.
    const events = readFileSync(
        fileURLToPath(new URL("../../crates/jarvis-core/src/ipc/events.rs", import.meta.url)),
        "utf8"
    )
    assert.ok(events.includes("ConversationRequested {"), "the handover event must exist")
    assert.ok(events.includes("intent: String"), "it carries an intent")
    assert.ok(!/ConversationRequested \{[\s\S]{0,80}text:/.test(events), "no transcript crosses the channel")
})

test("the stage and failure helpers behave", () => {
    assert.equal(stageKeyOf("recording"), "conversation-stage-recording")
    assert.equal(failureKeyOf("server_not_running"), "conversation-error-server_not_running")
    assert.equal(failureKeyOf(null), "conversation-error-unknown")
    assert.ok(isBusy("recording"))
    assert.ok(isBusy("thinking"))
    assert.equal(isBusy("idle"), false)
    assert.equal(isBusy("finished"), false)
})

test("every conversation key exists in all three locales", () => {
    const required = new Set()
    for (const key of [
        "conversation-title",
        "conversation-description",
        "conversation-ask",
        "conversation-stop",
        "conversation-clear",
        "conversation-stage",
        "conversation-provider",
        "conversation-profile",
        "conversation-provider-unavailable",
        "conversation-cloud-warning",
        "conversation-question",
        "conversation-answer",
        "conversation-characters"
    ]) {
        required.add(key)
    }
    for (const stage of [
        "idle",
        "recording",
        "transcribing",
        "thinking",
        "answering",
        "speaking",
        "finished",
        "cancelled",
        "failed"
    ]) {
        required.add(stageKeyOf(stage))
    }
    for (const profile of ["jarvis", "altron"]) {
        required.add(`conversation-profile-${profile}`)
    }
    for (const code of [
        "server_not_running",
        "server_not_ready",
        "server_stopped",
        "provider_disabled",
        "provider_not_configured",
        "provider_busy",
        "provider_timeout",
        "provider_failed",
        "record_timeout",
        "transcribe_timeout",
        "answer_timeout",
        "cancelled",
        "question_too_long",
        "conversation_busy",
        "conversation_wrong_stage",
        "unknown"
    ]) {
        required.add(failureKeyOf(code))
    }
    assert.ok(required.size > 30, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = localeKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a conversation message`)
    }
})
