import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import { phraseReasonKey } from "../src/lib/voice-input.ts"

/**
 * The command diagnostics and the phrase checker, checked where a static check can
 * reach them:
 *
 * * the checker command exists in the window's command layer and is registered;
 * * the answer carries an identifier, never a path, an argument or a transcript;
 * * the voice host writes every stage the core names, and no stage contains a
 *   phrase;
 * * the phrase is nowhere in a log line any more;
 * * the panel keeps the phrase out of storage and out of the console;
 * * every key the panel and the reason codes need exist in all three locales.
 */

const RUST_CHECKER = fileURLToPath(
    new URL("../../crates/jarvis-gui/src/tauri_commands/commands.rs", import.meta.url)
)
const RUST_MAIN = fileURLToPath(new URL("../../crates/jarvis-gui/src/main.rs", import.meta.url))
const CORE_COMMANDS = fileURLToPath(new URL("../../crates/jarvis-core/src/commands.rs", import.meta.url))
const APP = fileURLToPath(new URL("../../crates/jarvis-app/src/app.rs", import.meta.url))
const DIAG = fileURLToPath(new URL("../../crates/jarvis-app/src/diag.rs", import.meta.url))
const PANEL = fileURLToPath(new URL("../src/components/settings/PhraseCheck.svelte", import.meta.url))
const SETTINGS_PANEL = fileURLToPath(
    new URL("../src/components/settings/VoiceInputSettings.svelte", import.meta.url)
)
const API = fileURLToPath(new URL("../src/lib/voice-input.ts", import.meta.url))
const LOCALES = ["en", "ru", "ua"]

/** The stages the core names, in the order a phrase passes them. */
const STAGES = [
    "listener_received_phrase",
    "wake_word_detected",
    "normalized_length",
    "command_match",
    "rejection_code",
    "execution_started",
    "execution_result"
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

test("the checker command exists, is registered and never executes", () => {
    assert.ok(existsSync(RUST_CHECKER), "the command layer must exist")
    const rust = readFileSync(RUST_CHECKER, "utf8")
    const main = readFileSync(RUST_MAIN, "utf8")
    assert.ok(rust.includes("pub fn check_phrase_without_running("), "the checker must exist")
    assert.ok(main.includes("tauri_commands::check_phrase_without_running,"), "it must be registered")
    // It is the core's own matcher, not a second one.
    assert.ok(rust.includes("commands::check_phrase("), "it must use the core matcher")
    for (const forbidden of ["std::process", "Command::new", "execute_command", "powershell"]) {
        assert.equal(rust.includes(forbidden), false, `the checker must not contain ${forbidden}`)
    }
    // And the answer carries no path and no argument.
    const start = rust.indexOf("pub struct PhraseCheckView")
    const end = rust.indexOf("\n}", start)
    const view = rust.slice(start, end)
    for (const forbidden of ["path", "cli_", "exe_", "args"]) {
        assert.equal(view.includes(forbidden), false, `the view must not carry ${forbidden}`)
    }
})

test("one normalizer serves the microphone and the checker", () => {
    const core = readFileSync(CORE_COMMANDS, "utf8")
    assert.ok(core.includes("pub fn normalize_phrase("), "the normalizer must be public")
    assert.ok(core.includes("pub fn check_phrase("), "the checker must be in the core")
    assert.ok(core.includes("fn fetch_command_in"), "the one matcher must be reusable")
    assert.ok(core.includes("fetch_command_in(phrase, commands"), "fetch_command must delegate to it")
    // The matcher folds a slot placeholder out of both sides: a phrase written with
    // `{city}` cannot be compared literally, which is why the weather command was
    // unreachable by voice.
    assert.ok(core.includes("placeholder"), "the matcher must expect a slot placeholder")
})

test("the voice host writes every stage the core names, and no stage holds a phrase", () => {
    const core = readFileSync(CORE_COMMANDS, "utf8")
    const diag = readFileSync(DIAG, "utf8")
    const app = readFileSync(APP, "utf8")
    for (const stage of STAGES) {
        assert.ok(core.includes(`"${stage}"`), `the core must name ${stage}`)
    }
    // The host names the stages through the core's own enum, and writes each one.
    for (const variant of [
        "CommandStage::ListenerReceivedPhrase",
        "CommandStage::WakeWordDetected",
        "CommandStage::NormalizedLength",
        "CommandStage::CommandMatch",
        "CommandStage::RejectionCode",
        "CommandStage::ExecutionStarted",
        "CommandStage::ExecutionResult"
    ]) {
        assert.ok(diag.includes(variant), `the diagnostics must write ${variant}`)
    }
    // Each stage has its own function, and each one goes through the single writer.
    for (const call of ["received", "wake_word", "normalized", "matched", "rejection", "started", "finished"]) {
        assert.ok(diag.includes(`pub fn ${call}(`), `the diagnostics need ${call}`)
        assert.ok(app.includes(`diag::${call}(`), `the host must call diag::${call}`)
    }
    // A transcript is measured, never written.
    assert.equal(/\btext\b/.test(diag.slice(diag.indexOf("use "), diag.indexOf("/// One stage"))), false)
    assert.ok(diag.includes("length"), "a stage carries a length")
})

test("no log line quotes a recognized phrase any more", () => {
    const app = readFileSync(APP, "utf8")
    const core = readFileSync(CORE_COMMANDS, "utf8")
    for (const removed of [
        'info!("Recognized voice: {}", recognized_voice)',
        "Wake word + command during chaining",
        'debug!("Ignoring too short recognition:',
        'info!("Processing text command: {}", text)',
        'info!("No command found for: {}", text)',
        'message: format!("Command not found: {}", text)'
    ]) {
        assert.equal(app.includes(removed), false, `the host must not log or echo ${removed}`)
    }
    for (const removed of [
        "Fuzzy match: '{}' -> cmd",
        "No match for '{}'",
        "Perfect match: '{}' -> '{}'"
    ]) {
        assert.equal(core.includes(removed), false, `the matcher must not log ${removed}`)
    }
})

test("an unclear action falls through to the configured commands", () => {
    const app = readFileSync(APP, "utf8")
    // The router records its reason and returns "not handled", so the command packs
    // get their turn: this is the defect that made ordinary commands look broken.
    assert.ok(
        app.includes("fn try_windows_action(text: &str, unclear: &mut Option<&'static str>)"),
        "the router must report why it could not decide"
    )
    const ambiguous = app.indexOf("Ok(VoiceRoute::Ambiguous { reason })")
    assert.ok(ambiguous > 0, "the ambiguous arm must exist")
    const arm = app.slice(ambiguous, app.indexOf("Ok(VoiceRoute::NotAnAction)", ambiguous))
    assert.ok(arm.includes("*unclear = Some("), "the reason must be kept")
    assert.ok(arm.includes("None"), "the ambiguous arm must not answer for the packs")
    assert.equal(arm.includes("Some(false)"), false, "it must not end the search")
    // A confident intent that names nothing no longer ends the search either.
    assert.ok(
        app.includes("commands::fetch_command(&normalized, commands_list)"),
        "the phrase matcher must be the fallback"
    )
})

test("the panel checks a phrase without storing it or running anything", () => {
    assert.ok(existsSync(PANEL), "the checker panel must exist")
    const panel = readFileSync(PANEL, "utf8")
    const settings = readFileSync(SETTINGS_PANEL, "utf8")
    const api = readFileSync(API, "utf8")
    assert.ok(settings.includes("<PhraseCheck />"), "the settings tab must render the checker")
    assert.ok(api.includes("check_phrase_without_running"), "the interface must call the checker")
    for (const forbidden of [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "document.cookie",
        "location.hash",
        "window.history",
        "console.log"
    ]) {
        assert.equal(panel.includes(forbidden), false, `the checker must not use ${forbidden}`)
    }
    for (const key of [
        "phrase-check-run",
        "phrase-check-normalized",
        "phrase-check-matched",
        "phrase-check-nothing",
        "phrase-check-slots",
        "phrase-check-hint"
    ]) {
        assert.ok(panel.includes(key), `the checker needs ${key}`)
    }
})

test("every phrase check key and reason exists in all three locales", () => {
    const panel = readFileSync(PANEL, "utf8")
    const required = new Set()
    for (const match of panel.matchAll(/["'`](phrase-check-[a-z0-9_-]+)["'`]/g)) {
        required.add(match[1])
    }
    for (const reason of ["empty", "no_commands", "no_match"]) {
        required.add(phraseReasonKey(reason))
    }
    assert.ok(required.size >= 12, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = localeKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a phrase check message`)
    }
    assert.equal(phraseReasonKey(null), "phrase-check-reason-no_match")
})
