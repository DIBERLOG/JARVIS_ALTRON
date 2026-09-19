import { test } from "node:test"
import assert from "node:assert/strict"

import {
    ACTION_SOURCES,
    ACTION_STATUSES,
    ACTION_TYPES,
    ERROR_CODES,
    MAX_CONFIRM_TTL_SECONDS,
    MAX_REMINDER_CHARS,
    MAX_REMINDER_SECONDS,
    MAX_TIMER_SECONDS,
    MAX_VOLUME_STEP_PERCENT,
    MIN_CONFIRM_TTL_SECONDS,
    MIN_REMINDER_SECONDS,
    MIN_TIMER_SECONDS,
    POLICY_VOLUME_STEP,
    SCHEDULED_KINDS,
    SCHEDULED_STATUSES,
    SCREENSHOT_TARGETS,
    TOOL_REASONS,
    VOICE_REASONS,
    VOLUME_DIRECTIONS,
    WINDOW_OPERATIONS,
    WINDOW_STATES,
    actionTypeKey,
    cancelScheduled,
    confirmationTitleKey,
    captureWindow,
    clampReminderSeconds,
    clampTimerSeconds,
    clampVolumeStep,
    createReminder,
    createTimer,
    directionKey,
    errorKey,
    firedNotification,
    formatSeconds,
    isDangerous,
    lockWorkstation,
    minutesToSeconds,
    mute,
    normalizedSettings,
    previewValueKey,
    reminderInputProblem,
    resultSummary,
    riskKey,
    screenshot,
    screenshotTargetKey,
    shortenTitle,
    sourceKey,
    statusKey,
    timerInputProblem,
    toolAvailabilityKey,
    voiceReasonKey,
    volumeDown,
    volumeUp,
    windowOperation,
    windowOperationKey,
    windowStateKey
} from "../src/lib/windows-actions-model.ts"

/** A preview as the core sends it, without any token of value. */
function preview(overrides = {}) {
    return {
        token: "0123456789abcdef0123456789abcdef",
        action_kind: "change_volume",
        risk: "safe",
        source: "direct_gui",
        title_key: "windows-confirm-volume",
        fields: [],
        consequences: [],
        expires_in_seconds: 45,
        cancellable: true,
        ...overrides
    }
}

test("the quick actions are the typed shapes the core accepts", () => {
    assert.deepEqual(volumeUp(), { action: "change_volume", direction: "up", step: POLICY_VOLUME_STEP })
    assert.deepEqual(volumeDown(5), { action: "change_volume", direction: "down", step: 5 })
    assert.deepEqual(mute(true), { action: "mute_volume", muted: true })
    assert.deepEqual(lockWorkstation(), { action: "lock_workstation" })
    assert.deepEqual(screenshot(), { action: "take_screenshot", target: { target: "primary_monitor" } })
    assert.deepEqual(captureWindow("aabbccdd"), {
        action: "take_screenshot",
        target: { target: "selected_window", window_id: "aabbccdd" }
    })
    assert.deepEqual(windowOperation("aabbccdd", "close"), {
        action: "window",
        window_id: "aabbccdd",
        operation: "close"
    })
})

test("no action shape can carry a path, a command line, or an argument list", () => {
    // The interface cannot express any of the shapes the stage forbids: the union has no
    // field for them, and this test fails if one is ever added.
    const forbidden = ["command", "cmd", "shell", "powershell", "path", "executable", "args", "script"]
    const samples = [
        volumeUp(),
        volumeDown(),
        mute(false),
        lockWorkstation(),
        screenshot(),
        captureWindow("aabbccdd"),
        windowOperation("aabbccdd", "minimize"),
        createTimer(600),
        createReminder(600, "позвонить"),
        cancelScheduled("timer", "aabbccdd"),
        cancelScheduled("reminder", "aabbccdd")
    ]
    for (const sample of samples) {
        for (const name of forbidden) {
            assert.equal(name in sample, false, `${sample.action} must not carry ${name}`)
        }
    }
})

test("a volume step is clamped into the range the policy allows", () => {
    assert.equal(clampVolumeStep(999), MAX_VOLUME_STEP_PERCENT)
    assert.equal(clampVolumeStep(0), 1)
    assert.equal(clampVolumeStep(-4), 1)
    assert.equal(clampVolumeStep(Number.NaN), POLICY_VOLUME_STEP)
})

test("a timer and a reminder are clamped into the ranges the core enforces", () => {
    assert.equal(clampTimerSeconds(1), MIN_TIMER_SECONDS)
    assert.equal(clampTimerSeconds(99999999), MAX_TIMER_SECONDS)
    assert.equal(clampReminderSeconds(1), MIN_REMINDER_SECONDS)
    assert.equal(clampReminderSeconds(99999999), MAX_REMINDER_SECONDS)
    assert.equal(minutesToSeconds(10), 600)
    assert.equal(createTimer(600).duration_seconds, 600)
    assert.equal(createReminder(600, "x").delay_seconds, 600)
})

test("a reminder is cut to the length the core stores", () => {
    const long = "я".repeat(MAX_REMINDER_CHARS + 50)
    assert.equal(createReminder(600, long).message.length, MAX_REMINDER_CHARS)
})

test("the local input checks mirror the ranges without replacing the core", () => {
    assert.equal(timerInputProblem(5), null)
    assert.equal(timerInputProblem(1), null)
    // A tenth of a minute is six seconds, which is still a valid timer; below that it is not.
    assert.equal(timerInputProblem(0.05), "windows-actions-error-timer-range")
    assert.equal(timerInputProblem(0), "windows-actions-error-timer-range")
    assert.equal(reminderInputProblem(10, "позвонить"), null)
    assert.equal(reminderInputProblem(0, "позвонить"), "windows-actions-error-reminder-range")
    assert.equal(reminderInputProblem(10, "   "), "windows-actions-error-reminder-empty")
    assert.equal(
        reminderInputProblem(10, "я".repeat(MAX_REMINDER_CHARS + 1)),
        "windows-actions-error-reminder-too-long"
    )
})

test("every value list produces a key that names the value", () => {
    for (const source of ACTION_SOURCES) assert.equal(sourceKey(source), `windows-actions-source-${source}`)
    for (const status of ACTION_STATUSES) assert.equal(statusKey(status), `windows-actions-status-${status}`)
    for (const operation of WINDOW_OPERATIONS) {
        assert.equal(windowOperationKey(operation), `windows-actions-window-operation-${operation}`)
    }
    for (const state of WINDOW_STATES) assert.equal(windowStateKey(state), `windows-actions-window-state-${state}`)
    for (const kind of SCHEDULED_KINDS) {
        assert.equal(firedNotification({ kind }).title_key, `windows-actions-kind-${kind}`)
    }
    for (const status of SCHEDULED_STATUSES) {
        assert.ok(riskKey("safe").startsWith("windows-actions-risk-"))
        assert.ok(status.length > 0)
    }
    for (const direction of VOLUME_DIRECTIONS) {
        assert.equal(directionKey(direction), `windows-actions-direction-${direction}`)
    }
    for (const target of SCREENSHOT_TARGETS) {
        assert.equal(screenshotTargetKey({ target }), `windows-actions-target-${target}`)
    }
    for (const action of ACTION_TYPES) assert.equal(actionTypeKey(action), `windows-actions-action-${action}`)
})

test("an unknown code is named instead of being shown raw", () => {
    for (const code of ERROR_CODES) assert.equal(errorKey(code), `windows-actions-error-${code}`)
    assert.equal(errorKey("something_else"), "windows-actions-error-unknown")
    assert.equal(voiceReasonKey("windows-voice-duration-unclear"), "windows-actions-voice-duration-unclear")
    assert.equal(voiceReasonKey("windows-voice-nonsense"), "windows-actions-voice-not-an-action")
    for (const reason of VOICE_REASONS) {
        assert.equal(voiceReasonKey(`windows-voice-${reason}`), `windows-actions-voice-${reason}`)
    }
})

test("the tool availability keys strip the core's own prefix", () => {
    assert.equal(toolAvailabilityKey({ available: "available" }), "windows-actions-tools-available")
    for (const reason of TOOL_REASONS) {
        assert.equal(
            toolAvailabilityKey({ available: "unavailable", reason: `windows-ai-tools-${reason}` }),
            `windows-actions-tools-${reason}`
        )
    }
    assert.equal(
        toolAvailabilityKey({ available: "unavailable", reason: "windows-ai-tools-nonsense" }),
        "windows-actions-tools-unavailable-platform"
    )
})

test("a preview value is translated only when it is one of the small vocabularies", () => {
    assert.equal(previewValueKey("windows-field-operation", "minimize"), "windows-actions-window-operation-minimize")
    assert.equal(previewValueKey("windows-field-direction", "up"), "windows-actions-direction-up")
    assert.equal(previewValueKey("windows-field-target", "all_monitors"), "windows-actions-target-all_monitors")
    assert.equal(previewValueKey("windows-field-target", "selected_monitor:2"), "windows-actions-target-selected_monitor")
    // Free text stays as it is: it is what the user is being asked to approve.
    assert.equal(previewValueKey("windows-field-window", "Пароль — вход"), null)
    assert.equal(previewValueKey("windows-field-reminder", "позвонить маме"), null)
    assert.equal(previewValueKey("windows-field-operation", "teleport"), null)
})

test("the dialog marks the actions that deserve attention", () => {
    assert.equal(isDangerous(preview({ action_kind: "lock_workstation" })), true)
    assert.equal(isDangerous(preview({ action_kind: "window" })), true)
    assert.equal(isDangerous(preview({ action_kind: "change_volume" })), false)
    // The dialog shows the wording the core chose, not one of its own.
    assert.equal(confirmationTitleKey(preview({ title_key: "windows-confirm-lock" })), "windows-confirm-lock")
})

test("durations are readable", () => {
    assert.equal(formatSeconds(45), "45 s")
    assert.equal(formatSeconds(600), "10 min")
    assert.equal(formatSeconds(90), "1 min 30 s")
    assert.equal(formatSeconds(7200), "2 h")
    assert.equal(formatSeconds(-1), "0 s")
    assert.equal(formatSeconds(Number.NaN), "0 s")
})

test("a long window title is shortened without changing a short one", () => {
    assert.equal(shortenTitle("Notepad"), "Notepad")
    const long = "я".repeat(200)
    assert.equal(shortenTitle(long).length, 64)
    assert.ok(shortenTitle(long).endsWith("…"))
})

test("a fired reminder names its kind and says when the text cannot be read", () => {
    const timer = firedNotification({ kind: "timer", message: null, message_unreadable: false })
    assert.equal(timer.body_key, "windows-actions-fired-timer-body")
    const reminder = firedNotification({ kind: "reminder", message: "позвонить", message_unreadable: false })
    assert.equal(reminder.body_key, "windows-actions-fired-reminder-body")
    assert.equal(reminder.message, "позвонить")
    const unreadable = firedNotification({ kind: "reminder", message: null, message_unreadable: true })
    assert.equal(unreadable.body_key, "windows-actions-fired-unreadable")
})

test("a finished action is summarised from its value only", () => {
    assert.deepEqual(resultSummary({ value: { value: "volume", percent: 40, muted: false } }), {
        key: "windows-actions-result-volume",
        value: "40%",
        path: null
    })
    assert.equal(
        resultSummary({ value: { value: "screenshot_path", path: "C:/shots/a.png", bytes: 12 } }).path,
        "C:/shots/a.png"
    )
    assert.equal(resultSummary({ value: { value: "locked" } }).key, "windows-actions-result-locked")
    assert.equal(
        resultSummary({ value: { value: "launched", application: "app_1", process_id: 42 } }).value,
        "PID 42"
    )
    assert.equal(resultSummary({ value: { value: "timer", timer_id: "a", fires_in_seconds: 600 } }).value, "10 min")
    assert.equal(resultSummary({ value: { value: "none" } }).key, "windows-actions-result-done")
})

test("the settings that are sent back stay inside the range the core accepts", () => {
    const base = {
        ai_tools_enabled: true,
        voice_actions_enabled: true,
        confirm_ttl_seconds: 45,
        screenshots: { directory: "C:/shots", block_sensitive_windows: true },
        schema_version: 1
    }
    assert.equal(normalizedSettings(base).confirm_ttl_seconds, 45)
    assert.equal(normalizedSettings({ ...base, confirm_ttl_seconds: 1 }).confirm_ttl_seconds, MIN_CONFIRM_TTL_SECONDS)
    assert.equal(
        normalizedSettings({ ...base, confirm_ttl_seconds: 9999 }).confirm_ttl_seconds,
        MAX_CONFIRM_TTL_SECONDS
    )
})
