/**
 * Interface-side logic for the local AI runtime.
 *
 * Like `notes-model.ts` and `vault-model.ts`, everything here is free of Tauri
 * and Svelte dependencies so it can be unit tested with the Node test runner
 * (`npm run test:ui`). It holds the settings shape and its cheap local checks,
 * the streaming reducer that turns generation events into a chat view, and the
 * formatting helpers the chat panel renders.
 *
 * Two rules are encoded here on purpose:
 *
 * * the chat view is never persisted — no browser storage, no URL, no file — so
 *   closing the window ends the conversation (conversation memory is a later
 *   stage, and it must not arrive by accident);
 * * the local checks below mirror the Rust validation for instant feedback, but
 *   they are never the authority: the interface always shows the report that
 *   `local_ai_validate` returns, and starting the server is refused by the core.
 */

export type AiProfile = "jarvis" | "altron"
export type ThinkingMode = "auto" | "enabled" | "disabled"
export type CheckLevel = "ok" | "warning" | "blocked"
export type LocalAiState = "stopped" | "starting" | "ready" | "generating" | "stopping" | "failed"
export type ChatRole = "system" | "user" | "assistant"

/** Configuration schema version, matching `CONFIG_SCHEMA_VERSION` in the core. */
export const CONFIG_SCHEMA_VERSION = 1

/** Defaults, mirroring `LocalAiConfig::default()` in the core. */
export const DEFAULT_HOST = "127.0.0.1"
export const DEFAULT_PORT = 8080
export const DEFAULT_CONTEXT_SIZE = 8192
export const DEFAULT_MAX_TOKENS = 1024
export const DEFAULT_TEMPERATURE = 0.7
export const DEFAULT_TOP_P = 0.95
export const DEFAULT_STARTUP_TIMEOUT_SECONDS = 120

export const MIN_CONTEXT_SIZE = 512
export const MAX_CONTEXT_SIZE = 262144
export const MAX_THREADS = 256
export const MAX_GPU_LAYERS = 1000
export const MIN_PORT = 1024
export const MIN_MAX_TOKENS = 1
export const MAX_MAX_TOKENS = 32768
export const MIN_STARTUP_TIMEOUT_SECONDS = 5
export const MAX_STARTUP_TIMEOUT_SECONDS = 900

/** Hosts the local server is allowed to bind in this version. */
export const LOOPBACK_HOSTS: readonly string[] = ["127.0.0.1", "::1", "localhost"]

export const PROFILES: readonly AiProfile[] = ["jarvis", "altron"]
export const THINKING_MODES: readonly ThinkingMode[] = ["auto", "enabled", "disabled"]

export interface LocalModelSettings {
    server_path: string
    model_path: string
    host: string
    port: number
    context_size: number
    cpu_threads: number
    gpu_layers: number
    startup_timeout_seconds: number
}

export interface LocalAiSettings {
    server: LocalModelSettings
    temperature: number
    top_p: number
    max_tokens: number
    profile: AiProfile
    thinking: ThinkingMode
    allow_lan: boolean
    schema_version: number
}

export interface ChatMessage {
    role: ChatRole
    content: string
}

export interface GenerationRequest {
    messages: ChatMessage[]
    profile?: AiProfile
    thinking?: ThinkingMode
    stream: boolean
    max_tokens?: number
    temperature?: number
    top_p?: number
}

export interface ValidationIssue {
    level: CheckLevel
    field: string
    /** Content-free message, produced by the core in English. */
    message: string
}

export interface GgufInfo {
    version: number
    tensor_count: number
    metadata_entries: number
    architecture: string | null
    name: string | null
    quantisation: string | null
}

/** A model file that passed the file-level checks in the core. */
export interface ModelFileInfo {
    path: string
    file_name: string
    size_bytes: number
    gguf: GgufInfo
}

export interface ResourceEstimate {
    model_size_bytes: number
    context_size: number
    kv_cache_bytes: number
    overhead_bytes: number
    estimated_need_bytes: number
    available_ram_bytes: number
    total_ram_bytes: number
    headroom_bytes: number
    level: CheckLevel
    gpu_layers_requested: number
    notes: string[]
    issues: ValidationIssue[]
}

/** What the running server reported about itself. */
export interface LocalAiCapabilities {
    endpoint_available: boolean
    model_id: string | null
    models: string[]
    streaming: boolean
    thinking_switch: boolean
    reasoning_field_observed: boolean
    build_info: string | null
    notes: string[]
}

export interface LocalAiStatus {
    state: LocalAiState
    host: string
    port: number
    pid: number | null
    model_file: string | null
    server_file: string | null
    profile: AiProfile
    thinking: ThinkingMode
    capabilities: LocalAiCapabilities
    last_error: string | null
    stderr_tail: string[]
    stderr_truncated: boolean
    out_of_memory_hint: boolean
    uptime: string | null
    generating: boolean
}

export interface LocalAiReport {
    level: CheckLevel
    issues: ValidationIssue[]
    model: ModelFileInfo | null
    server_file: string | null
    resources: ResourceEstimate
}

export interface GenerationUsage {
    prompt_tokens?: number
    completion_tokens?: number
    total_tokens?: number
}

/**
 * Events streamed by the core while a generation runs.
 *
 * `token` carries visible answer text, `thinking` carries a separate reasoning
 * field when the model and the server provide one.
 */
export type GenerationEvent =
    | {
          type: "started"
          id: string
          model: string
          profile: AiProfile
          thinking: ThinkingMode
          thinking_applied: boolean
          stream: boolean
      }
    | { type: "token"; text: string }
    | { type: "thinking"; text: string }
    | {
          type: "completed"
          usage: GenerationUsage | null
          finish_reason: string | null
          duration_ms: number
          cancelled: boolean
      }
    | { type: "cancelled"; partial: boolean; duration_ms: number }
    | { type: "failed"; error: string }

// ------------------------------------------------------------------ settings

export function defaultSettings(): LocalAiSettings {
    return {
        server: {
            server_path: "",
            model_path: "",
            host: DEFAULT_HOST,
            port: DEFAULT_PORT,
            context_size: DEFAULT_CONTEXT_SIZE,
            cpu_threads: 0,
            gpu_layers: 0,
            startup_timeout_seconds: DEFAULT_STARTUP_TIMEOUT_SECONDS
        },
        temperature: DEFAULT_TEMPERATURE,
        top_p: DEFAULT_TOP_P,
        max_tokens: DEFAULT_MAX_TOKENS,
        profile: "jarvis",
        thinking: "auto",
        allow_lan: false,
        schema_version: CONFIG_SCHEMA_VERSION
    }
}

function isFiniteNumber(value: unknown): value is number {
    return typeof value === "number" && Number.isFinite(value)
}

function clampInteger(value: unknown, fallback: number, min: number, max: number): number {
    if (!isFiniteNumber(value)) return fallback
    const rounded = Math.round(value)
    if (rounded < min) return min
    if (rounded > max) return max
    return rounded
}

function clampNumber(value: unknown, fallback: number, min: number, max: number): number {
    if (!isFiniteNumber(value)) return fallback
    if (value < min) return min
    if (value > max) return max
    return value
}

/**
 * Repairs a configuration coming from the backend or from a form field.
 *
 * A value the user has not finished typing must never turn into `NaN` or into a
 * number the core would refuse for a reason the interface could have explained.
 */
export function normalizeSettings(input: Partial<LocalAiSettings> | null | undefined): LocalAiSettings {
    const fallback = defaultSettings()
    if (!input || typeof input !== "object") return fallback
    const server = (input.server ?? {}) as Partial<LocalModelSettings>
    const profile = PROFILES.includes(input.profile as AiProfile)
        ? (input.profile as AiProfile)
        : fallback.profile
    const thinking = THINKING_MODES.includes(input.thinking as ThinkingMode)
        ? (input.thinking as ThinkingMode)
        : fallback.thinking
    return {
        server: {
            // Paths are trimmed, because a pasted path often carries spaces and
            // the core trims before it uses one.
            server_path: typeof server.server_path === "string" ? server.server_path.trim() : "",
            model_path: typeof server.model_path === "string" ? server.model_path.trim() : "",
            host:
                typeof server.host === "string" && server.host.trim().length > 0
                    ? server.host.trim()
                    : DEFAULT_HOST,
            port: clampInteger(server.port, DEFAULT_PORT, 1, 65535),
            context_size: clampInteger(
                server.context_size,
                DEFAULT_CONTEXT_SIZE,
                MIN_CONTEXT_SIZE,
                MAX_CONTEXT_SIZE
            ),
            cpu_threads: clampInteger(server.cpu_threads, 0, 0, MAX_THREADS),
            gpu_layers: clampInteger(server.gpu_layers, 0, 0, MAX_GPU_LAYERS),
            startup_timeout_seconds: clampInteger(
                server.startup_timeout_seconds,
                DEFAULT_STARTUP_TIMEOUT_SECONDS,
                MIN_STARTUP_TIMEOUT_SECONDS,
                MAX_STARTUP_TIMEOUT_SECONDS
            )
        },
        temperature: clampNumber(input.temperature, DEFAULT_TEMPERATURE, 0, 2),
        top_p: clampNumber(input.top_p, DEFAULT_TOP_P, 0.05, 1),
        max_tokens: clampInteger(input.max_tokens, DEFAULT_MAX_TOKENS, MIN_MAX_TOKENS, MAX_MAX_TOKENS),
        profile,
        thinking,
        allow_lan: input.allow_lan === true,
        schema_version: CONFIG_SCHEMA_VERSION
    }
}

/** A local check result: a level, the field, and a message key for the label. */
export interface LocalIssue {
    level: CheckLevel
    field: string
    /** Translation key of a localized explanation. */
    key: string
}

function localIssue(level: CheckLevel, field: string, key: string): LocalIssue {
    return { level, field, key }
}

/**
 * Cheap client-side checks, so the form can explain a problem while the user
 * types instead of only when the server is started.
 *
 * The authoritative report still comes from `local_ai_validate`: this function
 * cannot see the filesystem, the GGUF header, or the memory of the machine.
 */
export function validateDraft(settings: LocalAiSettings): LocalIssue[] {
    const issues: LocalIssue[] = []
    const server = settings.server

    if (server.server_path.trim().length === 0) {
        issues.push(localIssue("blocked", "server_path", "ai-issue-server-missing"))
    }
    if (server.model_path.trim().length === 0) {
        issues.push(localIssue("blocked", "model_path", "ai-issue-model-missing"))
    } else if (!server.model_path.toLowerCase().endsWith(".gguf")) {
        issues.push(localIssue("warning", "model_path", "ai-issue-model-extension"))
    }
    if (settings.allow_lan) {
        issues.push(localIssue("blocked", "allow_lan", "ai-issue-allow-lan"))
    }
    if (!LOOPBACK_HOSTS.includes(server.host.trim())) {
        issues.push(localIssue("blocked", "host", "ai-issue-host-loopback"))
    }
    if (server.port < MIN_PORT) {
        issues.push(localIssue("blocked", "port", "ai-issue-port"))
    }
    if (server.context_size < MIN_CONTEXT_SIZE || server.context_size > MAX_CONTEXT_SIZE) {
        issues.push(localIssue("blocked", "context_size", "ai-issue-context"))
    }
    if (server.cpu_threads > MAX_THREADS) {
        issues.push(localIssue("blocked", "cpu_threads", "ai-issue-threads"))
    }
    if (server.gpu_layers > MAX_GPU_LAYERS) {
        issues.push(localIssue("blocked", "gpu_layers", "ai-issue-gpu-layers"))
    }
    if (
        server.startup_timeout_seconds < MIN_STARTUP_TIMEOUT_SECONDS ||
        server.startup_timeout_seconds > MAX_STARTUP_TIMEOUT_SECONDS
    ) {
        issues.push(localIssue("blocked", "startup_timeout_seconds", "ai-issue-timeout"))
    }
    if (settings.temperature < 0 || settings.temperature > 2) {
        issues.push(localIssue("blocked", "temperature", "ai-issue-temperature"))
    }
    if (settings.top_p < 0.05 || settings.top_p > 1) {
        issues.push(localIssue("blocked", "top_p", "ai-issue-top-p"))
    }
    if (settings.max_tokens < MIN_MAX_TOKENS || settings.max_tokens > MAX_MAX_TOKENS) {
        issues.push(localIssue("blocked", "max_tokens", "ai-issue-max-tokens"))
    }
    return issues
}

export function isBlocked(issues: readonly { level: CheckLevel }[]): boolean {
    return issues.some((issue) => issue.level === "blocked")
}

export function hasWarning(issues: readonly { level: CheckLevel }[]): boolean {
    return issues.some((issue) => issue.level === "warning")
}

export function worstLevel(issues: readonly { level: CheckLevel }[]): CheckLevel {
    if (isBlocked(issues)) return "blocked"
    if (hasWarning(issues)) return "warning"
    return "ok"
}

/** Localized label key for a configuration field name. */
export function fieldLabelKey(field: string): string {
    switch (field) {
        case "server_path":
            return "ai-settings-field-server-path"
        case "model_path":
            return "ai-settings-field-model-path"
        case "host":
            return "ai-settings-field-host"
        case "port":
            return "ai-settings-field-port"
        case "context_size":
            return "ai-settings-field-context"
        case "cpu_threads":
            return "ai-settings-field-threads"
        case "gpu_layers":
            return "ai-settings-field-gpu-layers"
        case "startup_timeout_seconds":
            return "ai-settings-field-timeout"
        case "temperature":
            return "ai-settings-field-temperature"
        case "top_p":
            return "ai-settings-field-top-p"
        case "max_tokens":
            return "ai-settings-field-max-tokens"
        case "allow_lan":
            return "ai-settings-field-allow-lan"
        case "schema_version":
            return "ai-settings-field-schema"
        default:
            return "ai-settings-field-generic"
    }
}

// -------------------------------------------------------------------- status

export function stateLabelKey(state: LocalAiState): string {
    return `ai-chat-state-${state}`
}

export function profileLabelKey(profile: AiProfile): string {
    return `ai-chat-profile-${profile}`
}

export function thinkingLabelKey(mode: ThinkingMode): string {
    return `ai-chat-thinking-${mode}`
}

export function reportLevelKey(level: CheckLevel): string {
    return `ai-settings-report-${level}`
}

/** Whether the server can accept a generation right now. */
export function canGenerate(status: LocalAiStatus | null): boolean {
    if (!status) return false
    return status.state === "ready" || status.state === "generating"
}

/** Whether the server is in a state where starting it does something useful. */
export function canStart(status: LocalAiStatus | null): boolean {
    if (!status) return true
    return status.state === "stopped" || status.state === "failed"
}

/** Whether a stop or restart request makes sense. */
export function canStop(status: LocalAiStatus | null): boolean {
    if (!status) return false
    return status.state !== "stopped"
}

/**
 * The thinking preference can only be honoured when the running server says its
 * chat template has a switch; `auto` always passes through unchanged.
 */
export function thinkingAvailable(status: LocalAiStatus | null, mode: ThinkingMode): boolean {
    if (mode === "auto") return true
    return Boolean(status?.capabilities.thinking_switch)
}

/** A short, non-secret description of what is running. */
export function serverSummary(status: LocalAiStatus | null): string {
    if (!status) return ""
    const model = status.capabilities.model_id ?? status.model_file ?? ""
    const endpoint = `${status.host}:${status.port}`
    return model.length > 0 ? `${model} · ${endpoint}` : endpoint
}

// ------------------------------------------------------------------ chat view

export interface ChatEntry {
    role: "user" | "assistant"
    text: string
    /** Reasoning output, when the server sent it as a separate field. */
    thinking: string
}

export interface ChatView {
    entries: ChatEntry[]
    generating: boolean
    generationId: string | null
    startedAtMs: number | null
    elapsedMs: number
    error: string | null
    cancelled: boolean
    finishReason: string | null
    usage: GenerationUsage | null
    /** Whether the answer arrived as a stream rather than in one piece. */
    streamed: boolean
    thinkingApplied: boolean
    model: string | null
    profile: AiProfile | null
    thinking: ThinkingMode | null
}

export function emptyChatView(): ChatView {
    return {
        entries: [],
        generating: false,
        generationId: null,
        startedAtMs: null,
        elapsedMs: 0,
        error: null,
        cancelled: false,
        finishReason: null,
        usage: null,
        streamed: false,
        thinkingApplied: false,
        model: null,
        profile: null,
        thinking: null
    }
}

/**
 * Adds the user's message and the empty assistant entry it will fill.
 *
 * The assistant entry is created before the request is sent, so the answer
 * streams into a visible bubble instead of appearing at the end.
 */
export function beginExchange(view: ChatView, prompt: string, nowMs: number): ChatView {
    return {
        ...view,
        entries: [
            ...view.entries,
            { role: "user", text: prompt, thinking: "" },
            { role: "assistant", text: "", thinking: "" }
        ],
        generating: true,
        generationId: null,
        startedAtMs: nowMs,
        elapsedMs: 0,
        error: null,
        cancelled: false,
        finishReason: null,
        usage: null,
        streamed: false,
        thinkingApplied: false
    }
}

/** Marks the request as failed before any event arrived. */
export function failExchange(view: ChatView, message: string): ChatView {
    return {
        ...view,
        generating: false,
        startedAtMs: null,
        elapsedMs: 0,
        error: message
    }
}

function withLastAssistant(
    view: ChatView,
    update: (entry: ChatEntry) => ChatEntry
): ChatEntry[] {
    const entries = view.entries.slice()
    for (let index = entries.length - 1; index >= 0; index -= 1) {
        if (entries[index].role === "assistant") {
            // A new object and a new array: the template must see the change on
            // every token, and Svelte compares the array by identity.
            entries[index] = update(entries[index])
            return entries
        }
    }
    return [...entries, update({ role: "assistant", text: "", thinking: "" })]
}

/**
 * Applies one streamed event to the chat view.
 *
 * `nowMs` is passed in rather than read from the clock, so the reducer is
 * deterministic and can be tested without waiting.
 */
export function applyGenerationEvent(view: ChatView, event: GenerationEvent, nowMs: number): ChatView {
    switch (event.type) {
        case "started":
            return {
                ...view,
                generating: true,
                generationId: event.id,
                model: event.model,
                profile: event.profile,
                thinking: event.thinking,
                thinkingApplied: event.thinking_applied,
                streamed: event.stream,
                startedAtMs: view.startedAtMs ?? nowMs,
                error: null,
                cancelled: false
            }
        case "token":
            return {
                ...view,
                entries: withLastAssistant(view, (entry) => ({
                    ...entry,
                    text: entry.text + event.text
                }))
            }
        case "thinking":
            return {
                ...view,
                entries: withLastAssistant(view, (entry) => ({
                    ...entry,
                    thinking: entry.thinking + event.text
                }))
            }
        case "completed":
            return {
                ...view,
                generating: false,
                finishReason: event.finish_reason,
                usage: event.usage,
                cancelled: event.cancelled,
                elapsedMs:
                    view.startedAtMs === null ? event.duration_ms : Math.max(0, nowMs - view.startedAtMs)
            }
        case "cancelled":
            return {
                ...view,
                generating: false,
                cancelled: true,
                elapsedMs:
                    view.startedAtMs === null ? event.duration_ms : Math.max(0, nowMs - view.startedAtMs)
            }
        case "failed":
            return {
                ...view,
                generating: false,
                error: event.error,
                elapsedMs:
                    view.startedAtMs === null ? view.elapsedMs : Math.max(0, nowMs - view.startedAtMs)
            }
        default:
            return view
    }
}

/** Whether the user may send the current draft. */
export function canSend(
    view: ChatView,
    status: LocalAiStatus | null,
    draft: string,
    settings: LocalAiSettings
): boolean {
    if (view.generating) return false
    if (draft.trim().length === 0) return false
    if (!canGenerate(status)) return false
    return !isBlocked(validateDraft(settings))
}

/** The request built from the chat view and the settings. */
export function buildRequest(
    view: ChatView,
    settings: LocalAiSettings,
    draft: string
): GenerationRequest {
    const messages: ChatMessage[] = []
    for (const entry of view.entries) {
        if (entry.text.trim().length === 0) continue
        messages.push({ role: entry.role, content: entry.text })
    }
    messages.push({ role: "user", content: draft.trim() })
    return {
        messages,
        profile: settings.profile,
        thinking: settings.thinking,
        stream: true,
        max_tokens: settings.max_tokens,
        temperature: settings.temperature,
        top_p: settings.top_p
    }
}

// ---------------------------------------------------------------- formatting

/** Byte count for display, using the same units as the core report. */
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

/** Duration for display: milliseconds below a second, one decimal above. */
export function formatDuration(ms: number): string {
    if (!isFiniteNumber(ms) || ms <= 0) return "0 ms"
    if (ms < 1000) return `${Math.round(ms)} ms`
    return `${(ms / 1000).toFixed(1)} s`
}

/** Token counts, only when the server reported them. */
export function formatUsage(usage: GenerationUsage | null): string {
    if (!usage) return ""
    const parts: string[] = []
    if (isFiniteNumber(usage.prompt_tokens)) parts.push(`${usage.prompt_tokens} in`)
    if (isFiniteNumber(usage.completion_tokens)) parts.push(`${usage.completion_tokens} out`)
    if (parts.length === 0 && isFiniteNumber(usage.total_tokens)) {
        parts.push(`${usage.total_tokens} total`)
    }
    return parts.join(" · ")
}

/** One line about the model file, never claiming it was verified. */
export function describeModel(model: ModelFileInfo | null): string {
    if (!model) return ""
    const details: string[] = []
    if (model.gguf.quantisation) details.push(model.gguf.quantisation)
    details.push(formatBytes(model.size_bytes))
    if (model.gguf.architecture) details.push(model.gguf.architecture)
    return `${model.file_name} (${details.join(", ")})`
}

/** The number of issues the report will show, for a compact summary. */
export function reportIssueCount(report: LocalAiReport | null): number {
    return report ? report.issues.length : 0
}
