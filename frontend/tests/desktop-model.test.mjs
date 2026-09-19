import { test } from "node:test"
import assert from "node:assert/strict"

import {
    AUTOSTART_SWITCHES,
    CLOSE_CHOICES,
    CURRENT_SETUP_VERSION,
    SETUP_LANGUAGES,
    SETUP_STEPS,
    aiStateKey,
    attentionComponents,
    autostartNeedsRepair,
    autostartStateKey,
    autostartSwitchKey,
    autostartSwitchesEnabled,
    canExport,
    closeBehaviorHintKey,
    closeBehaviorKey,
    componentNameKey,
    componentNeedsAttention,
    componentStateKey,
    isStepHandled,
    isStepSkipped,
    microphoneCanStop,
    microphoneHoldsDevice,
    microphoneStateKey,
    needsCloseDialog,
    needsWizard,
    nextSetupStep,
    reportSummaryKey,
    setupProgress,
    setupStepHintKey,
    setupStepKey,
    setupSummary,
    sizeLabel,
    stepAfter,
    stepSection
} from "../src/lib/desktop-model.ts"

function settings(overrides = {}) {
    return {
        close_behavior: "ask",
        autostart_enabled: false,
        start_minimized: false,
        start_local_ai: false,
        start_vosk: false,
        tray_explained: false,
        schema_version: 1,
        ...overrides
    }
}

function setup(overrides = {}) {
    return {
        setup_version: 0,
        completed_at: null,
        completed_steps: [],
        skipped_steps: [],
        language: null,
        ...overrides
    }
}

function report(components = []) {
    return {
        application_version: "0.0.0",
        operating_system: "windows",
        architecture: "x86_64",
        components,
        schema_versions: [],
        recent_errors: [],
        health_checks: [],
        database_sizes: [],
        model_sizes: [],
        memory_total_mb: null,
        memory_free_mb: null,
        disk_free_mb: null,
        licenses: [],
        notes: []
    }
}

test("the close dialog is only for the asking answer, and it offers both real choices", () => {
    assert.equal(needsCloseDialog("ask"), true)
    assert.equal(needsCloseDialog("tray"), false)
    assert.equal(needsCloseDialog("exit"), false)
    assert.deepEqual([...CLOSE_CHOICES], ["tray", "exit"])
    assert.equal(closeBehaviorKey("tray"), "desktop-close-tray")
    assert.equal(closeBehaviorHintKey("exit"), "desktop-close-exit-hint")
})

test("autostart is off by default and its switches are only meaningful when it is on", () => {
    const off = settings()
    assert.equal(off.autostart_enabled, false)
    assert.equal(autostartSwitchesEnabled(off), false)
    assert.equal(autostartSwitchesEnabled(settings({ autostart_enabled: true })), true)
    assert.deepEqual([...AUTOSTART_SWITCHES], ["start_minimized", "start_vosk", "start_local_ai"])
    assert.equal(autostartSwitchKey("start_local_ai"), "desktop-autostart-start-local-ai")
    // Dictation is not one of them: no switch may start it.
    assert.equal(AUTOSTART_SWITCHES.map(String).includes("start_whisper"), false)
})

test("an autostart entry that points elsewhere is reported and repairable", () => {
    assert.equal(
        autostartNeedsRepair({ entry_present: true, path_matches: false, command: null, error: null }),
        true
    )
    assert.equal(
        autostartNeedsRepair({ entry_present: true, path_matches: true, command: null, error: null }),
        false
    )
    // A refusal is not something to repair: it is something to explain.
    assert.equal(
        autostartNeedsRepair({
            entry_present: false,
            path_matches: false,
            command: null,
            error: "group policy"
        }),
        false
    )
    for (const state of ["enabled", "disabled", "needs_attention", "unavailable"]) {
        assert.equal(autostartStateKey(state), `desktop-autostart-${state.replace(/_/g, "-")}`)
    }
})

test("the microphone state says whether the device is held and whether it can be stopped", () => {
    assert.equal(microphoneHoldsDevice("idle"), false)
    assert.equal(microphoneHoldsDevice("vosk_listening"), true)
    assert.equal(microphoneHoldsDevice("whisper_dictation"), true)
    assert.equal(microphoneHoldsDevice("transcribing_file"), false)
    assert.equal(microphoneHoldsDevice("stopping"), true)
    assert.equal(microphoneHoldsDevice("failed"), false)
    assert.equal(microphoneCanStop("transcribing_file"), true)
    assert.equal(microphoneCanStop("idle"), false)
    assert.equal(microphoneStateKey("whisper_dictation"), "desktop-mic-whisper-dictation")
    assert.equal(aiStateKey("not_configured"), "desktop-ai-not-configured")
})

test("a new profile needs the wizard, and every step can be skipped", () => {
    const fresh = setup()
    assert.equal(needsWizard(fresh), true)
    assert.equal(nextSetupStep(fresh), "language")
    assert.equal(setupProgress(fresh), 0)

    const skipped = setup({
        setup_version: CURRENT_SETUP_VERSION,
        completed_at: "2026-01-01T00:00:00Z",
        skipped_steps: [...SETUP_STEPS]
    })
    assert.equal(needsWizard(skipped), false)
    assert.equal(nextSetupStep(skipped), null)
    assert.equal(setupProgress(skipped), 1)
    for (const step of SETUP_STEPS) {
        assert.equal(isStepHandled(skipped, step), true)
        assert.equal(isStepSkipped(skipped, step), true)
    }
})

test("a finished wizard from an older version runs once more", () => {
    const older = setup({
        setup_version: CURRENT_SETUP_VERSION - 1,
        completed_at: "2025-01-01T00:00:00Z"
    })
    assert.equal(needsWizard(older), true)
    // A finished state without a timestamp is not finished either.
    assert.equal(needsWizard(setup({ setup_version: CURRENT_SETUP_VERSION })), true)
})

test("the wizard resumes at the first step that is not handled", () => {
    const state = setup({ completed_steps: ["language"], skipped_steps: ["storage"] })
    assert.equal(nextSetupStep(state), "local_ai")
    assert.equal(isStepHandled(state, "storage"), true)
    assert.equal(isStepSkipped(state, "language"), false)
    assert.equal(stepAfter("language"), "storage")
    assert.equal(stepAfter("diagnostics"), null)
    assert.equal(stepAfter("nonsense"), null)
})

test("the summary separates what is configured from what was skipped", () => {
    const summary = setupSummary(
        setup({ completed_steps: ["language", "local_ai"], skipped_steps: ["storage"] })
    )
    assert.deepEqual(summary.completed, ["language", "local_ai"])
    assert.deepEqual(summary.skipped, ["storage"])
    assert.equal(summary.remaining.length, SETUP_STEPS.length - 3)
    // The order is the wizard's order, not the order the steps were answered.
    assert.deepEqual(summary.completed, ["language", "local_ai"])
})

test("each step knows which settings section it belongs to", () => {
    assert.equal(stepSection("local_ai"), "local-ai")
    assert.equal(stepSection("whisper"), "voice")
    assert.equal(stepSection("microphone"), "voice")
    assert.equal(stepSection("dictionaries"), "autocorrect")
    assert.equal(stepSection("windows_actions"), "windows-actions")
    assert.equal(stepSection("autostart"), "startup-tray")
    assert.equal(stepSection("storage"), "notes-vault")
    // Every step maps to a section, so "open the settings" always has a target.
    for (const step of SETUP_STEPS) {
        assert.ok(stepSection(step).length > 0, step)
        assert.equal(setupStepKey(step), `setup-step-${step.replace(/_/g, "-")}`)
        assert.equal(setupStepHintKey(step), `setup-step-${step.replace(/_/g, "-")}-hint`)
    }
    assert.deepEqual([...SETUP_LANGUAGES], ["ru", "en", "ua"])
})

test("a component needs attention unless it is ready, off, locked, or not configured", () => {
    assert.equal(componentNeedsAttention({ name: "tray", state: "ready", detail: null }), false)
    assert.equal(componentNeedsAttention({ name: "tray", state: "disabled", detail: null }), false)
    assert.equal(componentNeedsAttention({ name: "tray", state: "locked", detail: null }), false)
    assert.equal(componentNeedsAttention({ name: "tray", state: "not_configured", detail: null }), false)
    assert.equal(componentNeedsAttention({ name: "tray", state: "missing", detail: null }), true)
    assert.equal(componentNeedsAttention({ name: "tray", state: "invalid", detail: null }), true)
    assert.equal(componentNeedsAttention({ name: "tray", state: "unavailable", detail: null }), true)
    assert.equal(
        componentNeedsAttention({ name: "tray", state: "permission_denied", detail: null }),
        true
    )
})

test("the report summary distinguishes a healthy report from one that needs attention", () => {
    const healthy = report([
        { name: "tray", state: "ready", detail: null },
        { name: "dictation", state: "not_configured", detail: null }
    ])
    assert.equal(attentionComponents(healthy).length, 0)
    assert.equal(reportSummaryKey(healthy), "desktop-diagnostics-all-ready")
    const broken = report([
        { name: "tray", state: "ready", detail: null },
        { name: "whisper_binary", state: "missing", detail: null }
    ])
    assert.equal(attentionComponents(broken).length, 1)
    assert.equal(reportSummaryKey(broken), "desktop-diagnostics-needs-attention")
})

test("a report can only be exported when the core's screen passed it", () => {
    const view = {
        report: report(),
        preview: ["JARVIS 0.0.0"],
        screen_passed: true,
        screen_error: null
    }
    assert.equal(canExport(view), true)
    assert.equal(canExport({ ...view, screen_passed: false, screen_error: "path" }), false)
})

test("keys are built from values, never from a template in a component", () => {
    for (const state of [
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
    ]) {
        assert.equal(componentStateKey(state), `desktop-component-${state.replace(/_/g, "-")}`)
    }
    assert.equal(
        componentNameKey("notes_vault_memory"),
        "desktop-component-name-notes-vault-memory"
    )
    assert.equal(componentNameKey("webview2"), "desktop-component-name-webview2")
})

test("a size is readable", () => {
    assert.equal(sizeLabel(512), "512 B")
    assert.equal(sizeLabel(2048), "2.0 KB")
    assert.equal(sizeLabel(5 * 1024 * 1024), "5.0 MB")
    assert.equal(sizeLabel(3 * 1024 * 1024 * 1024), "3.0 GB")
})
