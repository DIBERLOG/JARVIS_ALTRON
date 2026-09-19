/**
 * Typed wrappers around the local AI commands.
 *
 * The interface never starts a process and never speaks HTTP: it asks the gateway
 * (inside Rust) for a status, a validation report, and a generation, and it
 * receives streamed tokens through a Tauri channel.
 *
 * Nothing here persists the conversation, and nothing here is sent anywhere
 * except to the local gateway.
 */

import { Channel, invoke } from "@tauri-apps/api/core"

import type {
    GenerationEvent,
    GenerationRequest,
    LocalAiReport,
    LocalAiSettings,
    LocalAiStatus
} from "./local-ai-model"

/**
 * Builds the channel that carries generation events.
 *
 * The core streams one event per token through this channel, so the chat panel
 * redraws as text arrives instead of polling the backend for every token.
 */
export function generationChannel(
    onEvent: (event: GenerationEvent) => void
): Channel<GenerationEvent> {
    const channel = new Channel<GenerationEvent>()
    channel.onmessage = (message: GenerationEvent) => {
        onEvent(message)
    }
    return channel
}

export const localAiApi = {
    // settings
    getConfig: () => invoke<LocalAiSettings>("local_ai_get_config"),
    /** Runs the core validation without saving or starting anything. */
    validate: (config: LocalAiSettings) => invoke<LocalAiReport>("local_ai_validate", { config }),
    /** Stores the settings and applies them to the running gateway. */
    saveConfig: (config: LocalAiSettings) => invoke<LocalAiReport>("local_ai_save_config", { config }),
    /** Returns the written path, or an empty string when the user cancels. */
    exportConfig: (config: LocalAiSettings) =>
        invoke<string>("local_ai_export_config", { config }),
    importConfig: () => invoke<LocalAiSettings | null>("local_ai_import_config"),

    // process lifecycle
    start: () => invoke<LocalAiStatus>("local_ai_start"),
    stop: () => invoke<LocalAiStatus>("local_ai_stop"),
    restart: () => invoke<LocalAiStatus>("local_ai_restart"),
    status: () => invoke<LocalAiStatus>("local_ai_status"),

    // pickers
    selectServer: () => invoke<string | null>("local_ai_select_server"),
    selectModel: () => invoke<string | null>("local_ai_select_model"),

    // generation
    /** Returns the generation id; tokens arrive on the channel. */
    generate: (channel: Channel<GenerationEvent>, request: GenerationRequest) =>
        invoke<string>("local_ai_generate", { channel, request }),
    /** Requests cancellation; returns whether a generation was running. */
    cancel: () => invoke<boolean>("local_ai_cancel")
}
