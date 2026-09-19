/**
 * Interface-side logic for local dictation.
 *
 * Everything here is free of Tauri and Svelte dependencies, so it can be unit
 * tested with the Node test runner. It mirrors the shapes the core sends, holds
 * the cheap checks that give instant feedback, and formats what the panel shows.
 *
 * Three rules are encoded here on purpose:
 *
 * * the interface never names a model or an executable path. It asks the core to
 *   open the native file dialog, which is the only way either file is chosen;
 * * a transcript is shown and never stored. Nothing here writes it to browser
 *   storage, to a URL, or to a file, and the dictation button is the only thing
 *   that can start a recording;
 * * the state words the panel renders come from the core (`idle`, `recording`,
 *   `transcribing`) — the interface does not invent a "listening" state.
 */

export type DictationState = "idle" | "recording" | "transcribing"
export type ModelKind =
    | "tiny"
    | "tiny_en"
    | "base"
    | "base_en"
    | "small"
    | "small_en"
    | "medium"
    | "medium_en"
    | "large_v1"
    | "large_v2"
    | "large_v3"
    | "unknown"

export interface BinaryProbe {
    size_bytes: number
    architecture: "x86_64" | "x86" | "arm64" | { other: number }
}

export interface ModelProbe {
    size_bytes: number
    kind: ModelKind
    container: string
    notes: string[]
}

export interface DictationStatus {
    state: DictationState
    enabled: boolean
    configured: boolean
    binary: BinaryProbe | null
    model: ModelProbe | null
    binary_path: string
    model_path: string
    /** The executable's own name, safe to show anywhere. */
    binary_name: string
    /** The model's own name, safe to show anywhere. */
    model_name: string
    notes: string[]
}

export interface WhisperSettings {
    enabled: boolean
    binary_path: string
    model_path: string
    language: string
    translate: boolean
    threads: number
    max_seconds: number
    silence_ms: number
    timeout_seconds: number
    keep_audio: boolean
    allow_from_window: boolean
    schema_version: number
}

export interface TranscriptSegment {
    start_ms: number
    end_ms: number
    text: string
}

export interface Transcript {
    text: string
    segments: TranscriptSegment[]
    language: string
    audio_ms: number
    duration_ms: number
}

/** The languages the core offers, in the order the select shows them. */
export const LANGUAGES: readonly string[] = ["auto", "ru", "en", "ua", "de", "fr", "es"]

/** The ranges the core enforces, mirrored for instant feedback only. */
export const MIN_THREADS = 1
export const MAX_THREADS = 32
export const DEFAULT_THREADS = 4
export const MIN_SECONDS = 1
export const MAX_SECONDS = 300
export const DEFAULT_SECONDS = 30
export const MIN_SILENCE_MS = 500
export const MAX_SILENCE_MS = 10_000
export const MIN_TIMEOUT_SECONDS = 5
export const MAX_TIMEOUT_SECONDS = 600

/** How many characters of a transcript the panel previews before "show all". */
export const PREVIEW_CHARS = 400

/** The Fluent key of a state. */
export function stateKey(state: DictationState): string {
    return `whisper-state-${state}`
}

/** The Fluent key of a model size the core reported. */
export function modelKindKey(kind: ModelKind): string {
    return `whisper-model-${kind}`
}

/** The Fluent key of a content-free error code. */
export function errorKey(code: string): string {
    return KNOWN_ERROR_CODES.includes(code) ? `whisper-error-${code}` : "whisper-error-unknown"
}

/**
 * The recorder's own codes, mirroring `RecorderError::code()` in the core.
 *
 * They are separate from the whisper codes because they answer a different
 * question — whether this machine can record at all — and the panel shows them
 * beside the microphone check, not beside a transcription.
 */
export const RECORDER_CODES: readonly string[] = [
    "not_initialized",
    "no_input_device",
    "unsupported_configuration",
    "device_failed",
    "already_running",
    "not_running",
    "backend_unavailable",
    "permission_denied"
]

/** Every code the core can put in an error, so none reaches the user raw. */
export const KNOWN_ERROR_CODES: readonly string[] = [
    "disabled",
    "not_configured",
    "invalid_configuration",
    "binary_unavailable",
    "model_unavailable",
    "model_unknown",
    "wrong_architecture",
    "audio_unavailable",
    "audio_empty",
    "busy",
    "process_unavailable",
    "process_failed",
    "timed_out",
    "invalid_response",
    "cancelled",
    "unsupported_language",
    "storage",
    ...RECORDER_CODES
]

/** The result of the microphone check, as the core sends it. */
export interface MicrophoneCheck {
    status: {
        backend: string
        native_ready: boolean
        device_count: number
        selected_index: number
        frame_length: number
    }
    frames_read: number
    level: number
    released: boolean
    error_code: string | null
}

/**
 * A one-line answer for the microphone check.
 *
 * The level is a number, not a verdict: a quiet room really is quiet, so the
 * sentence says what was measured and leaves the conclusion to the person.
 */
export function microphoneCheckKey(check: MicrophoneCheck): string {
    if (check.error_code) return errorKey(check.error_code)
    if (!check.released) return errorKey("device_failed")
    if (check.frames_read === 0) return "whisper-mic-check-silent"
    return "whisper-mic-check-heard"
}

/**
 * The note keys the core can build, without its own prefix.
 *
 * They mirror `WhisperError::code()` in the core: every error the status can
 * report has a key, so the user never sees a raw identifier. The translation
 * test reads the core's own list and requires each key in all three locales.
 */
export const NOTE_CODES: readonly string[] = [
    "disabled",
    "not_configured",
    "invalid_configuration",
    "binary_unavailable",
    "model_unavailable",
    "model_unknown",
    "wrong_architecture",
    "audio_unavailable",
    "audio_empty",
    "busy",
    "process_unavailable",
    "process_failed",
    "timed_out",
    "invalid_response",
    "cancelled",
    "unsupported_language",
    "storage"
]

/** The Fluent key of a note built from an error code. */
export function noteKeyForCode(code: string): string {
    return `whisper-note-${code.replace(/_/g, "-")}`
}

/**
 * The Fluent key of a note the core attached to the status.
 *
 * The core sends either a Fluent key (its own vocabulary) or a content-free
 * sentence; a sentence is shown as it is, because inventing a translation for it
 * would be worse than showing the honest one.
 */
export function noteKey(note: string): string | null {
    return note.startsWith("windows-whisper-note-") ? note.replace("windows-whisper-note-", "whisper-note-") : null
}

/** Whether a state means the microphone is open right now. */
export function isRecording(state: DictationState): boolean {
    return state === "recording"
}

/** Whether the panel's dictation button should offer to stop instead. */
export function isBusy(state: DictationState): boolean {
    return state !== "idle"
}

/** The local check of the numbers before they are sent. */
export function settingsProblem(settings: WhisperSettings): string | null {
    if (!Number.isFinite(settings.threads) || settings.threads < MIN_THREADS || settings.threads > MAX_THREADS) {
        return "whisper-error-threads"
    }
    if (
        !Number.isFinite(settings.max_seconds) ||
        settings.max_seconds < MIN_SECONDS ||
        settings.max_seconds > MAX_SECONDS
    ) {
        return "whisper-error-seconds"
    }
    if (
        !Number.isFinite(settings.silence_ms) ||
        settings.silence_ms < MIN_SILENCE_MS ||
        settings.silence_ms > MAX_SILENCE_MS
    ) {
        return "whisper-error-silence"
    }
    if (
        !Number.isFinite(settings.timeout_seconds) ||
        settings.timeout_seconds < MIN_TIMEOUT_SECONDS ||
        settings.timeout_seconds > MAX_TIMEOUT_SECONDS
    ) {
        return "whisper-error-timeout"
    }
    if (!LANGUAGES.includes(settings.language)) {
        return "whisper-error-language"
    }
    return null
}

/** The settings sent back, with every number inside the range the core accepts. */
export function normalizedSettings(settings: WhisperSettings): WhisperSettings {
    return {
        ...settings,
        threads: clamp(settings.threads, MIN_THREADS, MAX_THREADS, DEFAULT_THREADS),
        max_seconds: clamp(settings.max_seconds, MIN_SECONDS, MAX_SECONDS, DEFAULT_SECONDS),
        silence_ms: clamp(settings.silence_ms, MIN_SILENCE_MS, MAX_SILENCE_MS, 1_500),
        timeout_seconds: clamp(settings.timeout_seconds, MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS, 120),
        language: LANGUAGES.includes(settings.language) ? settings.language : "auto",
        binary_path: settings.binary_path.trim(),
        model_path: settings.model_path.trim()
    }
}

function clamp(value: number, minimum: number, maximum: number, fallback: number): number {
    if (!Number.isFinite(value)) return fallback
    return Math.min(maximum, Math.max(minimum, Math.round(value)))
}

/** A transcript with runs of whitespace collapsed, for a text field. */
export function cleanedTranscript(transcript: Transcript): string {
    return transcript.text.split(/\s+/).filter(Boolean).join(" ")
}

/** The transcript shortened for the panel, and whether it was shortened. */
export function transcriptPreview(transcript: Transcript): { text: string; shortened: boolean } {
    const cleaned = cleanedTranscript(transcript)
    if (cleaned.length <= PREVIEW_CHARS) return { text: cleaned, shortened: false }
    return { text: `${cleaned.slice(0, PREVIEW_CHARS - 1)}…`, shortened: true }
}

/** Characters in a transcript, for the counter next to the box. */
export function transcriptLength(transcript: Transcript): number {
    return cleanedTranscript(transcript).length
}

/** Seconds of audio, as a short label. */
export function audioLabel(transcript: Transcript): string {
    const seconds = transcript.audio_ms / 1000
    if (seconds < 60) return `${seconds.toFixed(1)} s`
    return `${Math.floor(seconds / 60)} min ${Math.round(seconds % 60)} s`
}

/** Whether the model's own notes say its contents were not verified. */
export function modelIsUnverified(status: DictationStatus): boolean {
    return (status.model?.notes ?? []).some((note) => note.includes("hash"))
}

/** A one-line summary of what is ready and what is missing. */
export function readinessKey(status: DictationStatus): string {
    if (!status.enabled) return "whisper-readiness-disabled"
    if (status.configured) return "whisper-readiness-ready"
    if (!status.binary_path && !status.model_path) return "whisper-readiness-empty"
    if (!status.binary_path || !status.binary) return "whisper-readiness-binary"
    return "whisper-readiness-model"
}

/** The default settings the panel starts from before the core answers. */
export function defaultSettings(): WhisperSettings {
    return {
        enabled: false,
        binary_path: "",
        model_path: "",
        language: "auto",
        translate: false,
        threads: DEFAULT_THREADS,
        max_seconds: DEFAULT_SECONDS,
        silence_ms: 1_500,
        timeout_seconds: 120,
        keep_audio: false,
        allow_from_window: true,
        schema_version: 1
    }
}
// ------------------------------------------------------------------ discovery

/** Where a discovered candidate was found. */
export type CandidateSource = "bundled_runtime" | "known_directory" | "path"

export interface ExecutableCandidate {
    path: string
    name: string
    source: CandidateSource
}

export interface ModelCandidate {
    path: string
    name: string
    source: CandidateSource
    kind: ModelKind
    size_bytes: number
}

export interface CandidatePair {
    executable: ExecutableCandidate
    model: ModelCandidate
}

export interface RejectedCandidate {
    name: string
    source: CandidateSource
    code: string
    detail: string
}

export interface DiscoveryReport {
    pairs: CandidatePair[]
    executables: ExecutableCandidate[]
    models: ModelCandidate[]
    rejected: RejectedCandidate[]
    searched: string[]
}

/** The Fluent key of where a candidate was found. */
export function candidateSourceKey(source: CandidateSource): string {
    // The core sends the full name; the label is the short one.
    const short =
        source === "bundled_runtime" ? "bundled" : source === "known_directory" ? "known" : "path"
    return `whisper-discovery-source-${short}`
}

/** Whether a search found something usable. */
export function discoveryFound(report: DiscoveryReport | null): boolean {
    return Boolean(report && report.executables.length > 0 && report.models.length > 0)
}

/** Whether the user has to choose between pairs. */
export function discoveryNeedsChoice(report: DiscoveryReport | null): boolean {
    return Boolean(report && report.pairs.length > 1)
}

/** A one-line result of a search, with no path in it. */
export function discoverySummaryKey(report: DiscoveryReport | null): string {
    if (!report) return "whisper-discovery-idle"
    if (!discoveryFound(report)) {
        return report.rejected.length > 0
            ? "whisper-discovery-nothing-usable"
            : "whisper-discovery-nothing"
    }
    return discoveryNeedsChoice(report)
        ? "whisper-discovery-choose"
        : "whisper-discovery-one"
}