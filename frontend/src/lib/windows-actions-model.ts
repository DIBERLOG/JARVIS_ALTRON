/**
 * Interface-side logic for the safe Windows commands.
 *
 * Everything here is free of Tauri and Svelte dependencies, so it can be unit tested with the
 * Node test runner (`npm run test:ui`). It holds the shapes the core sends, the small checks
 * that give instant feedback, and the formatting the panels render.
 *
 * Three rules are encoded here on purpose:
 *
 * * the interface never decides whether an action may run. It offers buttons, and the answer
 *   of the core (`executed`, `awaiting_confirmation`, or an error) is what the user sees;
 * * a confirmation is shown from `ActionPreview` only. No browser `confirm()`, no window
 *   prompt, and nothing about the pending action is written to browser storage: a reload
 *   forgets it, which is the honest behaviour for an approval that expires;
 * * no function here builds a command line, a path, or an executable path. The action types
 *   below cannot express one, and the only way a program enters the allowlist is the native
 *   file dialog the core opens.
 */

export type ActionSource = "direct_gui" | "voice" | "local_ai" | "internal_timer"
export type ActionRisk = "safe" | "confirm" | "forbidden"
export type ActionStatus =
    | "requested"
    | "confirmed"
    | "cancelled"
    | "expired"
    | "executed"
    | "rejected"
    | "failed"

export type WindowOperation = "minimize" | "maximize" | "restore" | "close" | "move"
export type ScheduledKind = "timer" | "reminder"
export type ScheduledStatus = "pending" | "fired" | "cancelled"
export type ScreenshotTarget =
    | { target: "primary_monitor" }
    | { target: "all_monitors" }
    | { target: "selected_monitor"; monitor: number }
    | { target: "selected_window"; window_id: string }

/** The action shapes, exactly as `WindowsAction` serializes in the core. */
export type WindowsAction =
    | { action: "get_volume" }
    | { action: "set_volume"; percent: number }
    | { action: "change_volume"; direction: "up" | "down"; step: number }
    | { action: "mute_volume"; muted: boolean }
    | { action: "launch_allowed_application"; application_id: string }
    | { action: "take_screenshot"; target: ScreenshotTarget }
    | { action: "create_timer"; duration_seconds: number }
    | { action: "cancel_timer"; timer_id: string }
    | { action: "create_reminder"; delay_seconds: number; message: string }
    | { action: "cancel_reminder"; timer_id: string }
    | { action: "list_windows" }
    | { action: "window"; window_id: string; operation: WindowOperation }
    | { action: "lock_workstation" }

export interface PreviewField {
    label_key: string
    value: string
}

/**
 * What the interface shows before a confirmed action runs.
 *
 * `title_key` and `consequences` are Fluent keys the core chose, so the dialog renders the
 * description the core wrote and never one this window invented.
 */
export interface ActionPreview {
    token: string
    action_kind: string
    risk: ActionRisk
    source: ActionSource
    title_key: string
    fields: PreviewField[]
    consequences: string[]
    expires_in_seconds: number
    cancellable: boolean
}

export interface ActionResult {
    action_id: string
    action_kind: string
    status: ActionStatus
    source: ActionSource
    value: ActionValue
    detail: string | null
    duration_ms: number
}

export type ActionValue =
    | { value: "none" }
    | { value: "volume"; percent: number; muted: boolean }
    | { value: "screenshot_path"; path: string; bytes: number }
    | { value: "launched"; application: string; process_id: number }
    | { value: "windows"; windows: WindowSummary[] }
    | { value: "timer"; timer_id: string; fires_in_seconds: number }
    | { value: "cancelled"; timer_id: string }
    | { value: "locked" }

export type ActionRequestOutcome =
    | { outcome: "executed"; result: ActionResult }
    | { outcome: "awaiting_confirmation"; preview: ActionPreview }
    | { outcome: "rejected"; detail: string }

export interface WindowSummary {
    id: string
    title: string
    process: string
    state: "normal" | "minimized" | "maximized"
    monitor: number
    sensitive: boolean
    foreground: boolean
}

export interface ScheduledView {
    id: string
    kind: ScheduledKind
    status: ScheduledStatus
    source: ActionSource
    created_at: string
    fires_at: string
    remaining_seconds: number
    message: string | null
    message_unreadable: boolean
}

export interface AuditEntry {
    timestamp: string
    action_type: string
    source: ActionSource
    risk: ActionRisk
    decision: ActionStatus
    result: string
    duration_ms: number
    error_category: string | null
    target: string | null
}

export interface AllowedApplicationView {
    id: string
    display_name: string
    executable_file_name: string
    path: string
    fixed_arguments: string[]
    working_directory: string | null
    enabled: boolean
    created_at: string
    updated_at: string
    identity_changed: boolean
    identity_reason: string | null
}

export interface PolicyRowView {
    action_type: string
    risk: ActionRisk
    note: string
    requires_confirmation: boolean
    forbidden: boolean
}

export interface Capabilities {
    platform_supported: boolean
    volume: boolean
    screenshots: boolean
    windows: boolean
    lock_workstation: boolean
    notifications: boolean
    ai_tools: boolean
    notes: string[]
}

export type ToolAvailability =
    | { available: "available" }
    | { available: "unavailable"; reason: string }

export interface ScreenshotSettings {
    directory: string
    block_sensitive_windows: boolean
}

export interface WindowsActionSettings {
    ai_tools_enabled: boolean
    voice_actions_enabled: boolean
    confirm_ttl_seconds: number
    screenshots: ScreenshotSettings
    schema_version: number
}

export interface WindowsActionsOverview {
    capabilities: Capabilities
    settings: WindowsActionSettings
    tools: ToolAvailability
    allowed_applications: AllowedApplicationView[]
    screenshots_directory: string
    audit_entries: number
    policy: PolicyRowView[]
}

/**
 * The answer to one phrase sent to the local model.
 *
 * `answer` is prose, and prose is never read as an action; `requested` means the model asked
 * for a tool and the call was decoded against that tool's own schema.
 */
export type AiActionOutcome =
    | { kind: "answer"; text: string }
    | { kind: "requested"; outcome: ActionRequestOutcome }
    | { kind: "unavailable"; reason: string }

export type VoiceRoute =
    | { result: "requested"; outcome: ActionRequestOutcome }
    | { result: "ambiguous"; reason: string }
    | { result: "not_an_action" }
    | { result: "disabled" }

/** The shortest and longest a timer and a reminder may be, matching the core. */
export const MIN_TIMER_SECONDS = 5
export const MAX_TIMER_SECONDS = 86_400
export const MIN_REMINDER_SECONDS = 30
export const MAX_REMINDER_SECONDS = 2_592_000
export const MAX_REMINDER_CHARS = 500
/** The largest relative volume change one action may ask for. */
export const MAX_VOLUME_STEP_PERCENT = 25
/** The step the quick volume buttons use. */
export const POLICY_VOLUME_STEP = 10
export const MIN_CONFIRM_TTL_SECONDS = 30
export const MAX_CONFIRM_TTL_SECONDS = 120

/**
 * The value lists the interface builds a key from.
 *
 * They are exported because the translation test walks them: a key built from a list must exist
 * in every locale, or the user would see the raw key.
 */
export const ACTION_SOURCES: readonly ActionSource[] = [
    "direct_gui",
    "voice",
    "local_ai",
    "internal_timer"
]
export const ACTION_RISKS: readonly ActionRisk[] = ["safe", "confirm", "forbidden"]
export const ACTION_STATUSES: readonly ActionStatus[] = [
    "requested",
    "confirmed",
    "cancelled",
    "expired",
    "executed",
    "rejected",
    "failed"
]
/** Every action type of the policy table, including the per-operation window rows. */
export const ACTION_TYPES: readonly string[] = [
    "get_volume",
    "set_volume",
    "change_volume",
    "mute_volume",
    "launch_allowed_application",
    "take_screenshot",
    "create_timer",
    "cancel_timer",
    "create_reminder",
    "cancel_reminder",
    "list_windows",
    "window",
    "lock_workstation",
    "minimize_window",
    "maximize_window",
    "restore_window",
    "close_window",
    "move_window",
    "add_allowed_application",
    "remove_allowed_application"
]
export const WINDOW_OPERATIONS: readonly WindowOperation[] = [
    "minimize",
    "maximize",
    "restore",
    "close",
    "move"
]
export const WINDOW_STATES: readonly WindowSummary["state"][] = [
    "normal",
    "minimized",
    "maximized"
]
export const SCHEDULED_KINDS: readonly ScheduledKind[] = ["timer", "reminder"]
export const SCHEDULED_STATUSES: readonly ScheduledStatus[] = ["pending", "fired", "cancelled"]
export const VOLUME_DIRECTIONS: readonly ("up" | "down")[] = ["up", "down"]
export const SCREENSHOT_TARGETS: readonly string[] = [
    "primary_monitor",
    "all_monitors",
    "selected_monitor",
    "selected_window"
]
/** The error codes the core answers with, and nothing else. */
export const ERROR_CODES: readonly string[] = [
    "unsupported_platform",
    "capability_unavailable",
    "invalid_arguments",
    "forbidden_action",
    "confirmation_required",
    "confirmation_expired",
    "confirmation_mismatch",
    "application_not_allowed",
    "executable_changed",
    "window_not_found",
    "window_expired",
    "timer_not_found",
    "screenshot_failed",
    "sensitive_window",
    "windows_api_error",
    "storage_error",
    "busy",
    "cancelled"
]
/** The reasons the local voice router reports, without its own prefix. */
export const VOICE_REASONS: readonly string[] = [
    "application-ambiguous",
    "application-not-allowed",
    "duration-unclear",
    "launch-unspecified",
    "move-unclear",
    "no-foreground-window",
    "reminder-text-missing",
    "volume-unclear",
    "window-ambiguous",
    "window-not-found",
    "window-unspecified"
]
/** The reasons the tool catalogue is unavailable, without the core's own prefix. */
export const TOOL_REASONS: readonly string[] = [
    "disabled",
    "unavailable-platform",
    "unavailable-template"
]

/** The quick actions the panel offers, in the order they are shown. */
export const QUICK_VOLUME_STEPS: readonly number[] = [10, 5]

/**
 * The default screenshot target.
 *
 * A full-screen capture is the useful default for a button, and the core refuses it while a
 * window that looks like it may show credentials has the focus.
 */
export const DEFAULT_SCREENSHOT: ScreenshotTarget = { target: "primary_monitor" }

export function volumeUp(step: number = POLICY_VOLUME_STEP): WindowsAction {
    return { action: "change_volume", direction: "up", step: clampVolumeStep(step) }
}

export function volumeDown(step: number = POLICY_VOLUME_STEP): WindowsAction {
    return { action: "change_volume", direction: "down", step: clampVolumeStep(step) }
}

export function mute(muted: boolean): WindowsAction {
    return { action: "mute_volume", muted }
}

export function lockWorkstation(): WindowsAction {
    return { action: "lock_workstation" }
}

export function screenshot(target: ScreenshotTarget = DEFAULT_SCREENSHOT): WindowsAction {
    return { action: "take_screenshot", target }
}

/** A window capture, which the core also refuses for a window that looks sensitive. */
export function captureWindow(windowId: string): WindowsAction {
    return { action: "take_screenshot", target: { target: "selected_window", window_id: windowId } }
}

export function windowOperation(windowId: string, operation: WindowOperation): WindowsAction {
    return { action: "window", window_id: windowId, operation }
}

export function createTimer(seconds: number): WindowsAction {
    return { action: "create_timer", duration_seconds: clampTimerSeconds(seconds) }
}

export function createReminder(seconds: number, message: string): WindowsAction {
    return {
        action: "create_reminder",
        delay_seconds: clampReminderSeconds(seconds),
        message: message.slice(0, MAX_REMINDER_CHARS)
    }
}

export function cancelScheduled(kind: ScheduledKind, id: string): WindowsAction {
    return kind === "timer" ? { action: "cancel_timer", timer_id: id } : { action: "cancel_reminder", timer_id: id }
}

export function clampVolumeStep(step: number): number {
    if (!Number.isFinite(step)) return POLICY_VOLUME_STEP
    return Math.min(MAX_VOLUME_STEP_PERCENT, Math.max(1, Math.round(step)))
}

export function clampTimerSeconds(seconds: number): number {
    if (!Number.isFinite(seconds)) return MIN_TIMER_SECONDS
    return Math.min(MAX_TIMER_SECONDS, Math.max(MIN_TIMER_SECONDS, Math.round(seconds)))
}

export function clampReminderSeconds(seconds: number): number {
    if (!Number.isFinite(seconds)) return MIN_REMINDER_SECONDS
    return Math.min(MAX_REMINDER_SECONDS, Math.max(MIN_REMINDER_SECONDS, Math.round(seconds)))
}

/** Minutes to seconds, for the two number inputs the panels use. */
export function minutesToSeconds(minutes: number): number {
    return Math.round(minutes * 60)
}

/**
 * The local check of a reminder before it is sent.
 *
 * It is deliberately only a range check: the core repeats it, and it is the core that decides.
 */
export function reminderInputProblem(minutes: number, message: string): string | null {
    const seconds = minutesToSeconds(minutes)
    if (!Number.isFinite(seconds) || seconds < MIN_REMINDER_SECONDS || seconds > MAX_REMINDER_SECONDS) {
        return "windows-actions-error-reminder-range"
    }
    if (message.trim().length === 0) {
        return "windows-actions-error-reminder-empty"
    }
    if (message.length > MAX_REMINDER_CHARS) {
        return "windows-actions-error-reminder-too-long"
    }
    return null
}

export function timerInputProblem(minutes: number): string | null {
    const seconds = minutesToSeconds(minutes)
    if (!Number.isFinite(seconds) || seconds < MIN_TIMER_SECONDS || seconds > MAX_TIMER_SECONDS) {
        return "windows-actions-error-timer-range"
    }
    return null
}

/**
 * The Fluent key for an error code.
 *
 * The core answers with a stable code and never a message, so an unknown code still produces
 * something a person can read instead of an empty box.
 */
export function errorKey(code: string): string {
    return ERROR_CODES.includes(code) ? `windows-actions-error-${code}` : "windows-actions-error-unknown"
}

/** The Fluent key of a voice-router reason; the core names them in full. */
export function voiceReasonKey(reason: string): string {
    const short = reason.replace(/^windows-voice-/, "")
    return VOICE_REASONS.includes(short) ? `windows-actions-voice-${short}` : "windows-actions-voice-not-an-action"
}

/** The Fluent key of a volume direction. */
export function directionKey(direction: "up" | "down"): string {
    return `windows-actions-direction-${direction}`
}

/** The Fluent key of a screenshot target, without its numeric or identifier argument. */
export function screenshotTargetKey(target: ScreenshotTarget): string {
    return `windows-actions-target-${target.target}`
}

/** The Fluent key of a risk level. */
export function riskKey(risk: ActionRisk): string {
    return `windows-actions-risk-${risk}`
}

/** The Fluent key of a status or decision. */
export function statusKey(status: ActionStatus): string {
    return `windows-actions-status-${status}`
}

export function actionTypeKey(actionType: string): string {
    return `windows-actions-action-${actionType}`
}

export function windowStateKey(state: WindowSummary["state"]): string {
    return `windows-actions-window-state-${state}`
}

export function windowOperationKey(operation: WindowOperation): string {
    return `windows-actions-window-operation-${operation}`
}

export function scheduledKindKey(kind: ScheduledKind): string {
    return `windows-actions-kind-${kind}`
}

export function scheduledStatusKey(status: ScheduledStatus): string {
    return `windows-actions-timer-status-${status}`
}

export function sourceKey(source: ActionSource): string {
    return `windows-actions-source-${source}`
}

export function toolAvailabilityKey(tools: ToolAvailability): string {
    if (tools.available === "available") return "windows-actions-tools-available"
    const short = tools.reason.replace(/^windows-ai-tools-/, "")
    return TOOL_REASONS.includes(short)
        ? `windows-actions-tools-${short}`
        : "windows-actions-tools-unavailable-platform"
}

/**
 * A reminder or timer that fired, as a notification line.
 *
 * The text of a reminder is only available while the storage can open it; when it cannot, the
 * line says so instead of showing an empty message.
 */
export function firedNotification(view: ScheduledView): {
    title_key: string
    body_key: string
    message: string | null
} {
    const bodyKey = view.message_unreadable
        ? "windows-actions-fired-unreadable"
        : view.kind === "timer"
          ? "windows-actions-fired-timer-body"
          : "windows-actions-fired-reminder-body"
    return {
        title_key: scheduledKindKey(view.kind),
        body_key: bodyKey,
        message: view.message
    }
}

/** Seconds as a short label: "45 s", "10 min", "2 h". */
export function formatSeconds(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return "0 s"
    if (seconds >= 3600 && seconds % 3600 === 0) return `${seconds / 3600} h`
    if (seconds >= 60 && seconds % 60 === 0) return `${seconds / 60} min`
    if (seconds >= 60) return `${Math.floor(seconds / 60)} min ${seconds % 60} s`
    return `${seconds} s`
}

/** The remaining time of a scheduled item, and whether it still counts down. */
export function remainingLabel(view: ScheduledView): string {
    if (view.status !== "pending") return ""
    return formatSeconds(view.remaining_seconds)
}

/** Trims a window title for a list row without cutting a word in half awkwardly. */
export function shortenTitle(title: string, limit: number = 64): string {
    if (title.length <= limit) return title
    return `${title.slice(0, limit - 1)}…`
}

/**
 * The fields the confirmation dialog shows.
 *
 * They come from the core's preview; this function only puts them in a stable order and never
 * invents a value.
 */
export function previewFields(preview: ActionPreview): PreviewField[] {
    return [...preview.fields]
}

/**
 * The Fluent key of a field *value*, when the value is one of the small vocabularies.
 *
 * A value that is free text — a window title, a reminder, a path the user chose — is shown as
 * it is: it is what the user is being asked to approve, and inventing a label for it would be
 * worse than showing it.
 */
export function previewValueKey(labelKey: string, value: string): string | null {
    if (labelKey === "windows-field-operation") {
        return WINDOW_OPERATIONS.includes(value as WindowOperation)
            ? windowOperationKey(value as WindowOperation)
            : null
    }
    if (labelKey === "windows-field-direction") {
        return VOLUME_DIRECTIONS.includes(value as "up" | "down")
            ? directionKey(value as "up" | "down")
            : null
    }
    if (labelKey === "windows-field-target") {
        const base = value.split(":")[0]
        return SCREENSHOT_TARGETS.includes(base) ? `windows-actions-target-${base}` : null
    }
    return null
}

/** Whether the confirm button should be shown as the dangerous one. */
export function isDangerous(preview: ActionPreview): boolean {
    return preview.action_kind === "lock_workstation" || preview.action_kind === "window"
}

/**
 * The title of the confirmation dialog.
 *
 * The core sends the key, so the wording is written once, in the core's own vocabulary.
 */
export function confirmationTitleKey(preview: ActionPreview): string {
    return preview.title_key
}

/** A one-line summary of a finished action, for the "last action" strip. */
export function resultSummary(result: ActionResult): {
    key: string
    value: string | null
    path: string | null
} {
    switch (result.value.value) {
        case "volume":
            return {
                key: "windows-actions-result-volume",
                value: `${result.value.percent}%${result.value.muted ? " (mute)" : ""}`,
                path: null
            }
        case "screenshot_path":
            return { key: "windows-actions-result-screenshot", value: null, path: result.value.path }
        case "locked":
            return { key: "windows-actions-result-locked", value: null, path: null }
        case "timer":
            return {
                key: "windows-actions-result-scheduled",
                value: formatSeconds(result.value.fires_in_seconds),
                path: null
            }
        case "cancelled":
            return { key: "windows-actions-result-cancelled", value: null, path: null }
        case "launched":
            return {
                key: "windows-actions-result-started",
                value: `PID ${result.value.process_id}`,
                path: null
            }
        case "windows":
            return {
                key: "windows-actions-result-windows",
                value: String(result.value.windows.length),
                path: null
            }
        default:
            return { key: "windows-actions-result-done", value: null, path: null }
    }
}

/** The settings as they are sent back, with the lifetime inside the range the core accepts. */
export function normalizedSettings(settings: WindowsActionSettings): WindowsActionSettings {
    return {
        ...settings,
        confirm_ttl_seconds: Math.min(
            MAX_CONFIRM_TTL_SECONDS,
            Math.max(MIN_CONFIRM_TTL_SECONDS, Math.round(settings.confirm_ttl_seconds))
        )
    }
}
