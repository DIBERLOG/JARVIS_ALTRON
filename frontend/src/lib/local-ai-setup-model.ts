/**
 * Interface-side logic for the managed local AI setup.
 *
 * Like `local-ai-model.ts` and `whisper-model.ts`, everything here is free of
 * Tauri and Svelte dependencies so it can be unit tested with the Node test
 * runner (`npm run test:ui`).
 *
 * Four rules are encoded here on purpose:
 *
 * * the code lists below are the single source of truth — the interface renders
 *   a label for a code and never invents one, so a stage or an error the core
 *   adds later degrades to a generic message instead of an empty line;
 * * no address, no checksum, no file name, and no command line is written here:
 *   the offer comes from the core, and this module only formats what it is
 *   given, because a value the interface made up is a value the core has not
 *   verified;
 * * a payload arriving over IPC is repaired rather than trusted, so a partially
 *   typed document cannot render as `NaN` or crash the panel;
 * * both the binary and the decimal reading of a size are produced, because a
 *   vendor quotes decimal gigabytes while the filesystem reports binary ones,
 *   and a person comparing the two must not think space went missing.
 */

export type ActiveOrigin = "managed" | "user_provided" | "mixed" | "unset"
export type ComponentCode = "runtime" | "model"
export type ComponentState = "absent" | "partial" | "ready" | "damaged" | "updating"
export type StageOutcome = "running" | "ok" | "failed" | "cancelled"

/**
 * The stages a run passes, in the order the core emits them.
 *
 * `idle` is listed first because the backend uses it for "nothing has started",
 * and `stageProgressStages` removes it from a run's list: it is a state, not a
 * step of the run.
 */
export const STAGES: readonly string[] = [
    "idle",
    "preflight",
    "download_runtime",
    "validate_runtime",
    "extract_runtime",
    "activate_runtime",
    "download_model",
    "validate_model",
    "activate_model",
    "configure",
    "launch_test",
    "readiness",
    "test_inference",
    "complete",
    "cancelled",
    "failed"
]

/** The six steps the wizard shows, which `offer.steps` is expected to match. */
export const STEPS: readonly string[] = [
    "preflight",
    "runtime",
    "model",
    "configure",
    "launch_test",
    "done"
]

export const COMPONENTS: readonly ComponentCode[] = ["runtime", "model"]

export const COMPONENT_STATES: readonly ComponentState[] = [
    "absent",
    "partial",
    "ready",
    "damaged",
    "updating"
]

/** The four values `active_origin` can take. */
export const COMPONENT_ORIGINS: readonly ActiveOrigin[] = [
    "managed",
    "user_provided",
    "mixed",
    "unset"
]

export const ERROR_CODES: readonly string[] = [
    "insufficient_space",
    "network",
    "timeout",
    "no_progress",
    "hash_mismatch",
    "size_mismatch",
    "content_length_mismatch",
    "range_mismatch",
    "identity_changed",
    "too_large",
    "refused_url",
    "archive_invalid",
    "archive_unexpected_file",
    "archive_path_traversal",
    "runtime_missing",
    "runtime_architecture_mismatch",
    "model_invalid",
    "model_architecture_mismatch",
    "model_quantization_mismatch",
    "not_same_volume",
    "not_owned",
    "destination_exists",
    "cancelled",
    "already_running",
    "not_running",
    "io",
    "interrupted",
    "test_failed",
    "test_timed_out",
    "process_unavailable",
    "settings_refused"
]

export const WARNING_CODES: readonly string[] = [
    "runtime_pre_release",
    "internet_required_for_download",
    "manual_settings_preserved",
    "download_resumed",
    "staging_reused",
    "previous_version_retained",
    "runtime_already_installed",
    "model_already_installed",
    "space_is_tight",
    "space_unknown",
    "user_model_untouched"
]

/** The three stages that end a run. */
export const TERMINAL_STAGES: readonly string[] = ["complete", "cancelled", "failed"]

// ------------------------------------------------------------------ DTO types

export interface OfferComponent {
    component: string
    display_name: string
    version: string | null
    /** Where the core takes the file from, named by the core. */
    source_label: string
    license: string | null
    download_bytes: number
    installed_bytes: number
    pre_release: boolean
    checksum_verified: boolean
    quantization: string | null
    architecture: string | null
    ram_recommendation_bytes: number | null
    context_recommendation: number | null
}

export interface SetupOffer {
    runtime: OfferComponent
    model: OfferComponent
    install_root: string
    internet_needed_for_download_only: boolean
    works_offline_after_install: boolean
    steps: string[]
    stage_codes: string[]
}

export interface RecoveryState {
    interrupted: boolean
    interrupted_stage: string | null
    previous_error_code: string | null
    partial_download_bytes: number
    staging_ready: boolean
    runtime_installed: boolean
    model_installed: boolean
}

export interface SetupView {
    stage: string
    step: string
    step_index: number
    component: string | null
    downloaded_bytes: number
    total_bytes: number
    required_bytes: number
    available_bytes: number
    missing_bytes: number
    download_bytes: number
    temporary_bytes: number
    installed_bytes: number
    rollback_bytes: number
    safety_reserve_bytes: number
    runtime_state: string
    runtime_version: string | null
    runtime_bytes: number
    model_state: string
    model_bytes: number
    active_origin: ActiveOrigin
    runtime_pre_release: boolean
    running: boolean
    can_start: boolean
    can_cancel: boolean
    can_retry: boolean
    can_cleanup: boolean
    error_code: string | null
    warning_codes: string[]
    recovery: RecoveryState
    offer: SetupOffer
}

/** One message the core streams while a run is in flight. */
export interface SetupEventView {
    stage: string
    component: string | null
    downloaded_bytes: number
    total_bytes: number
}

/** The start request. The core accepts exactly these two fields. */
export interface StartRequest {
    consent_managed_paths: boolean
    resume: boolean
}

export interface CleanupResult {
    removed_bytes: number
    removed_directories: number
    skipped_foreign: number
}

export interface ValidationResult {
    runtime_state: string
    runtime_server_ok: boolean
    runtime_architecture_ok: boolean
    runtime_bytes: number
    model_state: string
    model_size_ok: boolean
    model_hash_ok: boolean
    model_format_ok: boolean
    model_bytes: number
    error_code: string | null
}

export interface TestResult {
    passed: boolean
    server_ready: boolean
    answer_received: boolean
    elapsed_ms: number
    answer_tokens: number | null
}

/** The progress a channel event has accumulated, as the panel holds it. */
export interface ProgressState {
    downloaded: number
    total: number
    startedAt: number
    now: number
}

export interface ProgressReading {
    downloaded: number
    total: number
    bytesPerSecond: number
}

// ------------------------------------------------------------- label builders

/** The key shown for a code the local lists do not know. */
export const UNKNOWN_LABEL_KEY = "ai-setup-unknown"

/** Whether a value is a code this build can label. */
function isKnown(codes: readonly string[], code: unknown): code is string {
    return typeof code === "string" && codes.includes(code)
}

export function stageLabelKey(code: string): string {
    return isKnown(STAGES, code) ? `ai-setup-stage-${code}` : UNKNOWN_LABEL_KEY
}

export function stepLabelKey(code: string): string {
    return isKnown(STEPS, code) ? `ai-setup-step-${code}` : UNKNOWN_LABEL_KEY
}

export function componentLabelKey(code: string): string {
    return isKnown(COMPONENTS, code) ? `ai-setup-component-${code}` : UNKNOWN_LABEL_KEY
}

export function stateLabelKey(code: string): string {
    return isKnown(COMPONENT_STATES, code) ? `ai-setup-state-${code}` : UNKNOWN_LABEL_KEY
}

export function errorLabelKey(code: string): string {
    return isKnown(ERROR_CODES, code) ? `ai-setup-error-${code}` : UNKNOWN_LABEL_KEY
}

export function warningLabelKey(code: string): string {
    return isKnown(WARNING_CODES, code) ? `ai-setup-warning-${code}` : UNKNOWN_LABEL_KEY
}

/** The key for an `active_origin` value, which may also be absent. */
export function originLabelKey(code: string | null | undefined): string {
    return isKnown(COMPONENT_ORIGINS, code) ? `ai-setup-origin-${code}` : UNKNOWN_LABEL_KEY
}

// ---------------------------------------------------------------- formatting

function isFiniteNumber(value: unknown): value is number {
    return typeof value === "number" && Number.isFinite(value)
}

/**
 * A byte count in binary units, the way a filesystem reports it.
 *
 * `GiB` keeps one decimal so two sizes can be compared at a glance; the larger
 * `MiB` and `KiB` values are rounded, because a download that shows a moving
 * fraction of a mebibyte reads as noise.
 */
export function formatBytes(bytes: number): string {
    if (!isFiniteNumber(bytes) || bytes <= 0) return "0 B"
    const kib = 1024
    const mib = kib * 1024
    const gib = mib * 1024
    if (bytes >= gib) return `${(bytes / gib).toFixed(1)} GiB`
    if (bytes >= mib) return `${Math.round(bytes / mib)} MiB`
    if (bytes >= kib) return `${Math.round(bytes / kib)} KiB`
    return `${bytes} B`
}

/**
 * The same byte count in decimal units.
 *
 * This is the second reading on purpose: a vendor's page says "5.03 GB" while
 * the disk says "4.7 GiB", and a panel that shows only one of them makes the
 * difference look like a bug in the download.
 */
export function formatDecimalBytes(bytes: number): string {
    if (!isFiniteNumber(bytes) || bytes <= 0) return "0 B"
    const kb = 1000
    const mb = kb * 1000
    const gb = mb * 1000
    if (bytes >= gb) return `${(bytes / gb).toFixed(1)} GB`
    if (bytes >= mb) return `${(bytes / mb).toFixed(1)} MB`
    if (bytes >= kb) return `${(bytes / kb).toFixed(1)} kB`
    return `${bytes} B`
}

/** A transfer rate, empty when there is no rate to report yet. */
export function formatSpeed(bytesPerSecond: number): string {
    if (!isFiniteNumber(bytesPerSecond) || bytesPerSecond < 0) return ""
    return `${formatDecimalBytes(bytesPerSecond)}/s`
}

/** An integer percentage clamped to 0..100, and 0 whenever it is not derivable. */
export function percent(done: number, total: number): number {
    if (!isFiniteNumber(done) || !isFiniteNumber(total) || total <= 0) return 0
    const value = Math.round((done / total) * 100)
    if (value < 0) return 0
    if (value > 100) return 100
    return value
}

// -------------------------------------------------------------- stage helpers

export function stageOrder(code: string): number {
    return STAGES.indexOf(code) + 1
}

/**
 * Which of the six steps a stage belongs to, as a 1-based index.
 *
 * A run is a sequence of six steps, and the core reports the stage inside the
 * current one; mapping a stage to its step is what lets the wizard mark the
 * earlier steps as done without trusting any extra field.
 */
export function stepOfStage(code: string): number {
    switch (code) {
        case "idle":
        case "preflight":
            return 1
        case "download_runtime":
        case "validate_runtime":
        case "extract_runtime":
        case "activate_runtime":
            return 2
        case "download_model":
        case "validate_model":
        case "activate_model":
            return 3
        case "configure":
            return 4
        case "launch_test":
        case "readiness":
            return 5
        case "test_inference":
        case "complete":
        case "cancelled":
        case "failed":
            return 6
        default:
            return 1
    }
}

export function isTerminalStage(code: string): boolean {
    return TERMINAL_STAGES.includes(code)
}

/** A stage with work in it: not the idle state and not a terminal one. */
export function isActiveStage(code: string): boolean {
    return code !== "idle" && !isTerminalStage(code)
}

export function stageOutcome(stage: string): StageOutcome {
    if (stage === "complete") return "ok"
    if (stage === "cancelled") return "cancelled"
    if (stage === "failed") return "failed"
    return "running"
}

/**
 * The ordered sub-list of `stage_codes` that belongs to a run.
 *
 * The core sends the whole stage list with the offer; `idle` is dropped from it
 * because a stage list that starts with "nothing has started" reads like a step
 * of the installation. Unknown codes are dropped rather than rendered blank.
 */
export function stageProgressStages(stageCodes: readonly string[] | null | undefined): string[] {
    if (!Array.isArray(stageCodes)) return []
    const stages: string[] = []
    for (const code of stageCodes) {
        if (!isKnown(STAGES, code)) continue
        if (code === "idle") continue
        if (stages.includes(code)) continue
        stages.push(code)
    }
    return stages
}

// -------------------------------------------------------------- normalization

function toNumber(value: unknown, fallback = 0): number {
    if (typeof value === "number") return Number.isFinite(value) ? value : fallback
    if (typeof value === "string") {
        const parsed = Number(value)
        return Number.isFinite(parsed) ? parsed : fallback
    }
    return fallback
}

function toNullableNumber(value: unknown): number | null {
    if (value === null || value === undefined) return null
    const parsed = toNumber(value, Number.NaN)
    return Number.isFinite(parsed) ? parsed : null
}

function toText(value: unknown, fallback = ""): string {
    return typeof value === "string" ? value : fallback
}

/** An absent string and an empty one are the same thing on the wire. */
function toNullableText(value: unknown): string | null {
    if (typeof value !== "string") return null
    return value.length > 0 ? value : null
}

function toTextList(value: unknown, fallback: readonly string[]): string[] {
    if (!Array.isArray(value)) return fallback.length > 0 ? [...fallback] : []
    return value.filter((entry): entry is string => typeof entry === "string")
}

function toBoolean(value: unknown): boolean {
    return value === true
}

function normalizeOfferComponent(raw: unknown, component: ComponentCode): OfferComponent {
    const source = (raw ?? {}) as Record<string, unknown>
    return {
        component: toText(source.component, component),
        display_name: toText(source.display_name),
        version: toNullableText(source.version),
        source_label: toText(source.source_label),
        license: toNullableText(source.license),
        download_bytes: toNumber(source.download_bytes),
        installed_bytes: toNumber(source.installed_bytes),
        pre_release: toBoolean(source.pre_release),
        checksum_verified: toBoolean(source.checksum_verified),
        quantization: toNullableText(source.quantization),
        architecture: toNullableText(source.architecture),
        ram_recommendation_bytes: toNullableNumber(source.ram_recommendation_bytes),
        context_recommendation: toNullableNumber(source.context_recommendation)
    }
}

function normalizeOffer(raw: unknown): SetupOffer {
    const source = (raw ?? {}) as Record<string, unknown>
    return {
        runtime: normalizeOfferComponent(source.runtime, "runtime"),
        model: normalizeOfferComponent(source.model, "model"),
        install_root: toText(source.install_root),
        internet_needed_for_download_only: toBoolean(source.internet_needed_for_download_only),
        works_offline_after_install: toBoolean(source.works_offline_after_install),
        // The step and stage lists fall back to the local copies: a payload that
        // lost them must still render a complete wizard, never an empty one.
        steps: toTextList(source.steps, STEPS),
        stage_codes: toTextList(source.stage_codes, STAGES)
    }
}

function normalizeRecovery(raw: unknown): RecoveryState {
    const source = (raw ?? {}) as Record<string, unknown>
    return {
        interrupted: toBoolean(source.interrupted),
        interrupted_stage: toNullableText(source.interrupted_stage),
        previous_error_code: toNullableText(source.previous_error_code),
        partial_download_bytes: toNumber(source.partial_download_bytes),
        staging_ready: toBoolean(source.staging_ready),
        runtime_installed: toBoolean(source.runtime_installed),
        model_installed: toBoolean(source.model_installed)
    }
}

/**
 * Repairs a view coming from the core before the panel renders it.
 *
 * The wizard must survive a payload that is missing a field, that sends numbers
 * as strings (which `serde` does for a wide integer), or that sends `null` where
 * an array was expected. Every value that reaches the markup is therefore a
 * finite number, a string, a boolean, or an array.
 */
export function normalizeView(raw: unknown): SetupView {
    const source = (raw ?? {}) as Record<string, unknown>
    const origin = toText(source.active_origin)
    return {
        stage: toText(source.stage, "idle"),
        step: toText(source.step, "preflight"),
        step_index: toNumber(source.step_index, 1),
        component: toNullableText(source.component),
        downloaded_bytes: toNumber(source.downloaded_bytes),
        total_bytes: toNumber(source.total_bytes),
        required_bytes: toNumber(source.required_bytes),
        available_bytes: toNumber(source.available_bytes),
        missing_bytes: toNumber(source.missing_bytes),
        download_bytes: toNumber(source.download_bytes),
        temporary_bytes: toNumber(source.temporary_bytes),
        installed_bytes: toNumber(source.installed_bytes),
        rollback_bytes: toNumber(source.rollback_bytes),
        safety_reserve_bytes: toNumber(source.safety_reserve_bytes),
        runtime_state: toText(source.runtime_state, "absent"),
        runtime_version: toNullableText(source.runtime_version),
        runtime_bytes: toNumber(source.runtime_bytes),
        model_state: toText(source.model_state, "absent"),
        model_bytes: toNumber(source.model_bytes),
        active_origin: (COMPONENT_ORIGINS as readonly string[]).includes(origin)
            ? (origin as ActiveOrigin)
            : "unset",
        runtime_pre_release: toBoolean(source.runtime_pre_release),
        running: toBoolean(source.running),
        can_start: toBoolean(source.can_start),
        can_cancel: toBoolean(source.can_cancel),
        can_retry: toBoolean(source.can_retry),
        can_cleanup: toBoolean(source.can_cleanup),
        error_code: toNullableText(source.error_code),
        warning_codes: toTextList(source.warning_codes, []).filter((code) =>
            WARNING_CODES.includes(code)
        ),
        recovery: normalizeRecovery(source.recovery),
        offer: normalizeOffer(source.offer)
    }
}

// ------------------------------------------------------------------- planning

export interface SpaceMessage {
    key: string
    bytes: number
}

/**
 * The missing-space message, with the shortfall.
 *
 * The core's own `missing_bytes` is preferred; when a payload carries only the
 * plan numbers, the shortfall is derived from them, so the warning is never a
 * bare sentence without a figure.
 */
export function missingSpaceMessage(view: SetupView): SpaceMessage {
    const reported = toNumber(view.missing_bytes)
    if (reported > 0) return { key: "ai-setup-missing-space", bytes: reported }
    const shortfall = toNumber(view.required_bytes) - toNumber(view.available_bytes)
    if (shortfall > 0) return { key: "ai-setup-missing-space", bytes: shortfall }
    return { key: "ai-setup-missing-space", bytes: 0 }
}

/**
 * Whether the plan fits with less than twice the peak space free.
 *
 * The reserve the core holds back is a fraction of the peak need, so a machine
 * with under twice that is one background update away from a failed extract;
 * saying so before the download is cheaper than saying it afterwards.
 */
export function spaceIsTight(view: SetupView): boolean {
    const required = toNumber(view.required_bytes)
    const available = toNumber(view.available_bytes)
    if (required <= 0) return false
    if (available < required) return false
    return required * 2 > available
}

/**
 * The warning keys to show, without duplicates.
 *
 * The order follows `WARNING_CODES` rather than the payload, so the list does
 * not jump around when the core sends the same warnings in a different order.
 */
export function describeWarningKeys(view: SetupView): string[] {
    const present = new Set(view.warning_codes)
    const keys: string[] = []
    for (const code of WARNING_CODES) {
        if (!present.has(code)) continue
        keys.push(warningLabelKey(code))
    }
    return keys
}

export interface PlanSummary {
    download: number
    temporary: number
    installed: number
    rollback: number
    reserve: number
    peak: number
    available: number
    missing: number
}

/** The numbers the plan table renders, in one object. */
export function planSummary(view: SetupView): PlanSummary {
    return {
        download: toNumber(view.download_bytes),
        temporary: toNumber(view.temporary_bytes),
        installed: toNumber(view.installed_bytes),
        rollback: toNumber(view.rollback_bytes),
        reserve: toNumber(view.safety_reserve_bytes),
        peak: toNumber(view.required_bytes),
        available: toNumber(view.available_bytes),
        missing: missingSpaceMessage(view).bytes
    }
}

// -------------------------------------------------------------------- progress

/**
 * Applies one channel message to the progress the panel holds.
 *
 * The speed is the byte delta over the elapsed time, computed here rather than
 * read from the core, because the core reports totals and the panel is what
 * knows how long it has been showing them. A clock that did not move yields a
 * speed of zero instead of an infinite one.
 */
export function applyEvent(state: ProgressState, event: SetupEventView): ProgressReading {
    const downloaded = toNumber(event?.downloaded_bytes, toNumber(state?.downloaded))
    const total = toNumber(event?.total_bytes, toNumber(state?.total))
    const delta = downloaded - toNumber(state?.downloaded)
    const elapsedSeconds = (toNumber(state?.now) - toNumber(state?.startedAt)) / 1000
    let bytesPerSecond = 0
    if (delta > 0 && elapsedSeconds > 0) {
        const rate = delta / elapsedSeconds
        bytesPerSecond = Number.isFinite(rate) ? rate : 0
    }
    return { downloaded, total, bytesPerSecond }
}

// --------------------------------------------------------------------- controls

/** Whether the page keeps polling the core. */
export function shouldPoll(view: SetupView | null): boolean {
    return Boolean(view && view.running)
}

/** Whether a component may be removed right now, and is there to remove. */
export function canRemoveComponent(view: SetupView | null, component: ComponentCode): boolean {
    if (!view) return false
    if (!view.can_start || view.running) return false
    const state = component === "runtime" ? view.runtime_state : view.model_state
    return state !== "absent"
}

export interface ConfirmKeys {
    titleKey: string
    confirmKey: string
}

/** The two keys of the removal confirmation, one pair per component. */
export function confirmKeys(component: ComponentCode): ConfirmKeys {
    return {
        titleKey: confirmTitleKey(component),
        confirmKey: confirmActionKey(component)
    }
}

function confirmTitleKey(component: ComponentCode): string {
    return component === "model" ? "ai-setup-confirm-remove-model" : "ai-setup-confirm-remove-runtime"
}

function confirmActionKey(component: ComponentCode): string {
    return component === "model" ? "ai-setup-remove-model" : "ai-setup-remove-runtime"
}
