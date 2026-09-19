/**
 * The conversation route: a question spoken out loud, an answer from the model.
 *
 * The question is recorded with the same engine as the global voice input and is
 * asked through the same provider the text chat uses. Nothing here can run a
 * command: the panel has no command list, and the answer is text on a screen until
 * the person decides otherwise.
 */
import { invoke } from "@tauri-apps/api/core"

/** The stages the voice host reports, in the order they happen. */
export type ConversationStage =
    | "idle"
    | "recording"
    | "transcribing"
    | "thinking"
    | "answering"
    | "speaking"
    | "finished"
    | "cancelled"
    | "failed"

export interface ConversationTurn {
    /** The length of the question; the question itself is not sent to the page. */
    question_characters: number
    answer: string
    answer_characters: number
    provider: string
    cloud: boolean
}

export interface ConversationView {
    stage: ConversationStage
    active: boolean
    turns: number
    provider: string
    provider_available: boolean
    cloud: boolean
    /** A stable code when the last turn failed: `server_not_running`, … */
    last_code: string | null
    turn: ConversationTurn | null
    /** `jarvis` or `altron`: the instructions the answer was asked under. */
    profile: string
}

export const conversationApi = {
    /** The stage, the provider and the last answer. */
    status: () => invoke<ConversationView>("conversation_status"),

    /** Asks one question: microphone, Whisper, model. */
    ask: () => invoke<ConversationView>("conversation_ask"),

    /** Abandons the question in flight. */
    cancel: () => invoke<boolean>("conversation_cancel"),

    /** Ends the conversation; what was said stays on screen. */
    stop: () => invoke<boolean>("conversation_stop"),

    /** Forgets the last answer. */
    clear: () => invoke<boolean>("conversation_clear"),

    /** Chooses the profile whose instructions the next question is asked under. */
    setProfile: (profile: string) => invoke<ConversationView>("conversation_set_profile", { profile })
}

/** The Fluent key of a stage. */
export function stageKeyOf(stage: string): string {
    return `conversation-stage-${stage}`
}

/** The Fluent key of a failure code. */
export function failureKeyOf(code: string | null | undefined): string {
    return `conversation-error-${code ?? "unknown"}`
}

/** Whether a stage means the microphone is open or the model is working. */
export function isBusy(stage: string): boolean {
    return ["recording", "transcribing", "thinking", "answering", "speaking"].includes(stage)
}
