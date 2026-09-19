/**
 * Typed wrappers around the global voice input commands.
 *
 * The interface never sees the transcript of a running request through this
 * module: `status` carries a stage, a length and a content-free code, and the
 * preview call is the one place the text is read — into a field on the page, and
 * nowhere else. Nothing here writes to browser storage.
 */

import { invoke } from "@tauri-apps/api/core"

/** The stages the core reports, as the panel shows them. */
export type VoiceInputStage =
    | "idle"
    | "preparing"
    | "confirming"
    | "handover"
    | "recording"
    | "transcribing"
    | "correcting"
    | "inserting"
    | "delivered"
    | "failed"
    | "cancelled"

/** The settings of the feature, as the core stores them. */
export interface VoiceInputSettings {
    enabled: boolean
    phrase: string
    speak_confirmation: boolean
    language: string
    autocorrect: boolean
    punctuation: boolean
    preference: "ui_automation" | "clipboard"
    clipboard_seconds: number
    preview_before_insert: boolean
}

/** What the panel shows. No window, no field, no text. */
export interface VoiceInputView {
    settings: VoiceInputSettings
    status: {
        stage: VoiceInputStage
        characters: number
        has_text: boolean
    }
    whisper_configured: boolean
    /** The voice host, as its own typed state. */
    host: {
        state:
            | "stopped"
            | "starting"
            | "listening"
            | "stopping"
            | "incompatible_version"
            | "executable_missing"
            | "ipc_unavailable"
            | "crashed"
        state_key: string
        protocol_version: number | null
        expected_version: number
    }
    vosk_available: boolean
    clipboard: {
        armed: boolean
        seconds_left: number | null
    }
    error_code: string | null
    error_key: string | null
    has_result: boolean
    characters: number
}

/** What a finished request produced. The text is not in it. */
export interface VoiceInputOutcome {
    stage: VoiceInputStage
    characters: number
    method: "ui_automation" | "clipboard" | null
    rule: string | null
    error_code: string | null
    audio_ms: number
}

/** A notice the core sends the window: a key and a length, never the text. */
export interface VoiceInputNotice {
    key: string
    characters: number
}

/** What a phrase would reach, without reaching it. */
export interface PhraseCheckView {
    /** The phrase as the matcher sees it: lower case, `ё` folded, wake word removed. */
    normalized: string
    /** The command a phrase would reach, by identifier; null when nothing matched. */
    matched: string | null
    /** How close the best candidate was, as a whole percentage. */
    score: number
    /** The slots the matched command declares, by name. */
    slots: string[]
    /** A stable reason when nothing matched: empty, no_commands, no_match. */
    reason: string | null
}

export const phraseCheckApi = {
    /**
     * Asks what a phrase would do. Nothing is executed, no program is started, and
     * the phrase is not stored: the voice host answers with the command identifier,
     * the slots it needs and the reason nothing matched.
     */
    check: (phrase: string) =>
        invoke<PhraseCheckView>("check_phrase_without_running", { phrase })
}

/** The Fluent key of a reason a phrase was not accepted. */
export function phraseReasonKey(reason: string | null | undefined): string {
    return `phrase-check-reason-${reason ?? "no_match"}`
}

export const voiceInputApi = {
    /** The state, the settings and the last failure. */
    status: () => invoke<VoiceInputView>("voice_input_status"),

    /** Starts one global dictation. Resolves when it has finished. */
    start: () => invoke<VoiceInputOutcome>("voice_input_start"),

    /** Cancels what is running; false when nothing was. */
    cancel: () => invoke<boolean>("voice_input_cancel"),

    /** Stores the settings atomically and returns what was stored. */
    updateSettings: (settings: VoiceInputSettings) =>
        invoke<VoiceInputSettings>("voice_input_update_settings", { settings }),

    /** Forgets the last result and wipes the clipboard if it is still ours. */
    clearResult: () => invoke<boolean>("voice_input_clear_result"),

    /** The text of the last result, for the panel's preview. In memory only. */
    preview: () => invoke<string | null>("voice_input_preview"),

    /** Copies the last result again, with a fresh cleanup timer. */
    copyAgain: () => invoke<boolean>("voice_input_copy_again")
}

/** The Fluent key of a stage. */
export function stageKey(stage: VoiceInputStage): string {
    return `voice-input-stage-${stage}`
}

/** Whether the stage means a request is in flight. */
export function isRunning(stage: VoiceInputStage): boolean {
    return !["idle", "delivered", "failed", "cancelled"].includes(stage)
}

/** The Fluent key of a reason the feature cannot start, or null when it can. */
export function startProblem(view: VoiceInputView): string | null {
    if (!view.settings.enabled) return "voice-input-problem-disabled"
    if (!view.whisper_configured) return "voice-input-problem-whisper"
    if (!view.vosk_available) return "voice-input-problem-vosk"
    if (isRunning(view.status.stage)) return "voice-input-problem-busy"
    return null
}
