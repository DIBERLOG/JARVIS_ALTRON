/**
 * Interface-side logic for the desktop shell: the close dialog, autostart, the
 * first-run wizard, and the diagnostics page.
 *
 * Everything here is free of Tauri and Svelte dependencies, so it can be unit
 * tested with the Node test runner. It holds the shapes the core sends, the
 * order of the wizard steps, and the labels the panels render.
 *
 * Three rules are encoded here on purpose:
 *
 * * the interface never writes a registry key, never touches the tray, and never
 *   builds a diagnostics document: it asks the core and shows the answer;
 * * a wizard step can always be skipped, and a skipped step is remembered as
 *   skipped rather than as done;
 * * no password and no key is ever part of this state, so nothing here can put
 *   one in browser storage.
 */

export type CloseBehavior = "tray" | "exit" | "ask"
export type AutostartState = "enabled" | "disabled" | "needs_attention" | "unavailable"
export type MicrophoneState =
    | "idle"
    | "vosk_listening"
    | "whisper_dictation"
    | "transcribing_file"
    | "stopping"
    | "failed"
export type AiState = "not_configured" | "stopped" | "ready" | "busy"
export type ComponentState =
    | "ready"
    | "missing"
    | "invalid"
    | "locked"
    | "wrong_architecture"
    | "version_unknown"
    | "incompatible"
    | "unavailable"
    | "permission_denied"
    | "not_configured"
    | "disabled"

export interface SetupState {
    setup_version: number
    completed_at: string | null
    completed_steps: string[]
    skipped_steps: string[]
    language: string | null
}

export interface DesktopSettings {
    close_behavior: CloseBehavior
    autostart_enabled: boolean
    start_minimized: boolean
    start_local_ai: boolean
    start_vosk: boolean
    tray_explained: boolean
    schema_version: number
}

export interface AutostartStatus {
    entry_present: boolean
    path_matches: boolean
    command: string | null
    error: string | null
}

export interface DesktopState {
    settings: DesktopSettings
    setup: SetupState
    microphone: MicrophoneState
    autostart: AutostartStatus
    window_visible: boolean
    tray_available: boolean
    pending_timers: number
    ai_state: AiState
    whisper_configured: boolean
    vault_unlocked: boolean
}

export interface ComponentHealth {
    name: string
    state: ComponentState
    detail: string | null
}

export interface FileSize {
    name: string
    bytes: number
}

export interface HealthCheck {
    name: string
    passed: boolean
    detail: string | null
}

export interface LicenseStatus {
    component: string
    license: string
    status: string
}

export interface DiagnosticReport {
    application_version: string
    operating_system: string
    architecture: string
    components: ComponentHealth[]
    schema_versions: [string, number][]
    recent_errors: { code: string; count: number }[]
    health_checks: HealthCheck[]
    database_sizes: FileSize[]
    model_sizes: FileSize[]
    memory_total_mb: number | null
    memory_free_mb: number | null
    disk_free_mb: number | null
    licenses: LicenseStatus[]
    notes: string[]
}

export interface DiagnosticsView {
    report: DiagnosticReport
    preview: string[]
    screen_passed: boolean
    screen_error: string | null
}

/** The wizard's steps, in the order the core stores them. */
export const SETUP_STEPS: readonly string[] = [
    "language",
    "storage",
    "local_ai",
    "whisper",
    "microphone",
    "vosk",
    "dictionaries",
    "windows_actions",
    "autostart",
    "diagnostics"
]

/** The languages the first step offers, matching the application's own set. */
export const SETUP_LANGUAGES: readonly string[] = ["ru", "en", "ua"]

export const CURRENT_SETUP_VERSION = 1

// ------------------------------------------------------------------ close behaviour

export function closeBehaviorKey(behavior: CloseBehavior): string {
    return `desktop-close-${behavior}`
}

export function closeBehaviorHintKey(behavior: CloseBehavior): string {
    return `desktop-close-${behavior}-hint`
}

/** What the close dialog offers, in the order it shows them. */
export const CLOSE_CHOICES: readonly CloseBehavior[] = ["tray", "exit"]

/** Whether the dialog has to be shown at all. */
export function needsCloseDialog(behavior: CloseBehavior): boolean {
    return behavior === "ask"
}

// ------------------------------------------------------------------------ autostart

export function autostartStateKey(state: AutostartState): string {
    return `desktop-autostart-${state.replace(/_/g, "-")}`
}

/** Whether the entry and the setting agree, and what to offer. */
export function autostartNeedsRepair(status: AutostartStatus): boolean {
    return status.entry_present && !status.path_matches && status.error === null
}

/** The switches that decide what a login starts, in the order they are shown. */
export const AUTOSTART_SWITCHES: readonly (keyof DesktopSettings)[] = [
    "start_minimized",
    "start_vosk",
    "start_local_ai"
]

export function autostartSwitchKey(name: keyof DesktopSettings): string {
    return `desktop-autostart-${String(name).replace(/_/g, "-")}`
}

/**
 * Whether the switches can be edited.
 *
 * They are only meaningful while autostart is on, so they are shown disabled
 * rather than hidden: a person can see what a login would do.
 */
export function autostartSwitchesEnabled(settings: DesktopSettings): boolean {
    return settings.autostart_enabled
}

// ------------------------------------------------------------------- microphone

export function microphoneStateKey(state: MicrophoneState): string {
    return `desktop-mic-${state.replace(/_/g, "-")}`
}

/** Whether the microphone is open right now, whatever the reason. */
export function microphoneHoldsDevice(state: MicrophoneState): boolean {
    return state === "vosk_listening" || state === "whisper_dictation" || state === "stopping"
}

/** Whether a stop button should be offered. */
export function microphoneCanStop(state: MicrophoneState): boolean {
    return state === "vosk_listening" || state === "whisper_dictation" || state === "transcribing_file"
}

export function aiStateKey(state: AiState): string {
    return `desktop-ai-${state.replace(/_/g, "-")}`
}

// --------------------------------------------------------------------- first run

export function setupStepKey(step: string): string {
    return `setup-step-${step.replace(/_/g, "-")}`
}

export function setupStepHintKey(step: string): string {
    return `setup-step-${step.replace(/_/g, "-")}-hint`
}

/** Whether the wizard has to run. */
export function needsWizard(state: SetupState): boolean {
    return state.setup_version < CURRENT_SETUP_VERSION || state.completed_at === null
}

/** The step a person should be shown: the first that is not handled. */
export function nextSetupStep(state: SetupState): string | null {
    return SETUP_STEPS.find((step) => !isStepHandled(state, step)) ?? null
}

export function isStepHandled(state: SetupState, step: string): boolean {
    return state.completed_steps.includes(step) || state.skipped_steps.includes(step)
}

export function isStepSkipped(state: SetupState, step: string): boolean {
    return state.skipped_steps.includes(step)
}

/** The step after `step`, or null at the end. */
export function stepAfter(step: string): string | null {
    const index = SETUP_STEPS.indexOf(step)
    if (index < 0 || index === SETUP_STEPS.length - 1) return null
    return SETUP_STEPS[index + 1]
}

/** How far along the wizard is, as a fraction. */
export function setupProgress(state: SetupState): number {
    const handled = SETUP_STEPS.filter((step) => isStepHandled(state, step)).length
    return handled / SETUP_STEPS.length
}

/** The summary the last step shows. */
export function setupSummary(state: SetupState): {
    completed: string[]
    skipped: string[]
    remaining: string[]
} {
    return {
        completed: SETUP_STEPS.filter((step) => state.completed_steps.includes(step)),
        skipped: SETUP_STEPS.filter((step) => state.skipped_steps.includes(step)),
        remaining: SETUP_STEPS.filter((step) => !isStepHandled(state, step))
    }
}

/** The section a wizard step belongs to, so "open the settings" can jump there. */
export function stepSection(step: string): string {
    switch (step) {
        case "language":
            return "general"
        case "storage":
            return "notes-vault"
        case "local_ai":
            return "local-ai"
        case "whisper":
        case "microphone":
        case "vosk":
            return "voice"
        case "dictionaries":
            return "autocorrect"
        case "windows_actions":
            return "windows-actions"
        case "autostart":
            return "startup-tray"
        default:
            return "diagnostics"
    }
}

// ------------------------------------------------------------------ diagnostics

export function componentStateKey(state: ComponentState): string {
    return `desktop-component-${state.replace(/_/g, "-")}`
}

export function componentNameKey(name: string): string {
    return `desktop-component-name-${name.replace(/_/g, "-")}`
}

/** Whether a component needs the user's attention. */
export function componentNeedsAttention(component: ComponentHealth): boolean {
    return !["ready", "disabled", "not_configured", "locked"].includes(component.state)
}

/** The components that need attention, worst first in the order the core sent. */
export function attentionComponents(report: DiagnosticReport): ComponentHealth[] {
    return report.components.filter(componentNeedsAttention)
}

/** A size, as a short label. */
export function sizeLabel(bytes: number): string {
    const units: [number, string][] = [
        [1024 * 1024 * 1024, "GB"],
        [1024 * 1024, "MB"],
        [1024, "KB"]
    ]
    for (const [scale, unit] of units) {
        if (bytes >= scale) return `${(bytes / scale).toFixed(1)} ${unit}`
    }
    return `${bytes} B`
}

/** The one-line summary of a report, for the page's own status strip. */
export function reportSummaryKey(report: DiagnosticReport): string {
    if (!report.components.some(componentNeedsAttention)) return "desktop-diagnostics-all-ready"
    return "desktop-diagnostics-needs-attention"
}

/** Whether the report may be written to a file. */
export function canExport(view: DiagnosticsView): boolean {
    return view.screen_passed
}

/** The lines the copy button puts on the clipboard: the preview, unchanged. */
export function summaryLines(view: DiagnosticsView): string[] {
    return view.preview
}
