/**
 * Typed wrappers around the managed local AI setup commands.
 *
 * The interface never downloads anything and never names a file: it asks the core
 * for a status and an offer, sends a consent flag, and receives progress through a
 * Tauri channel. The size of the download, the address it comes from, the checksum
 * it must match, and the directory it lands in are all decided inside Rust, so a
 * compromised page cannot redirect an installation.
 *
 * Nothing here persists a path, and nothing here is sent anywhere except to the
 * local core.
 */

import { Channel, invoke } from "@tauri-apps/api/core"

import type {
    CleanupResult,
    SetupEventView,
    SetupView,
    StartRequest,
    TestResult,
    ValidationResult
} from "./local-ai-setup-model"

/**
 * Builds the channel that carries setup progress.
 *
 * The core streams one message per stage change and per progress tick, so the
 * wizard redraws a bar instead of polling the backend for every chunk it wrote.
 */
export function setupChannel(onEvent: (event: SetupEventView) => void): Channel<SetupEventView> {
    const channel = new Channel<SetupEventView>()
    channel.onmessage = (message: SetupEventView) => {
        onEvent(message)
    }
    return channel
}

export const localAiSetupApi = {
    // state
    /** What the core knows now, including a run that is already in flight. */
    status: () => invoke<SetupView>("local_ai_setup_status"),
    /** Re-reads the disk and the offer; nothing is downloaded or written. */
    preflight: () => invoke<SetupView>("local_ai_setup_preflight"),

    // run
    /** Starts the installation; progress arrives on the channel. */
    start: (request: StartRequest, channel: Channel<SetupEventView>) =>
        invoke<SetupView>("local_ai_setup_start", { request, channel }),
    /** Retries the failed stage, reusing whatever was already verified. */
    retry: (request: StartRequest, channel: Channel<SetupEventView>) =>
        invoke<SetupView>("local_ai_setup_retry", { request, channel }),
    /** Requests cancellation; returns whether a run was in flight. */
    cancel: () => invoke<boolean>("local_ai_setup_cancel"),

    // disk
    /** Removes only the temporary files the core itself created. */
    cleanupTemp: () => invoke<CleanupResult>("local_ai_setup_cleanup_temp"),
    /** Switches the settings to the managed pair; the flag is the consent. */
    useManaged: (confirm: boolean) => invoke<SetupView>("local_ai_setup_use_managed", { confirm }),
    removeRuntime: () => invoke<SetupView>("local_ai_setup_remove_runtime"),
    removeModel: () => invoke<SetupView>("local_ai_setup_remove_model"),

    // checks
    /** Re-validates what is on disk without downloading anything. */
    validateExisting: () => invoke<ValidationResult>("local_ai_setup_validate_existing"),
    /**
     * Runs one short generation against the installed pair.
     *
     * The core keeps the prompt and the answer out of the report: only booleans
     * and timings come back, which is why nothing here is logged.
     */
    runTest: () => invoke<TestResult>("local_ai_setup_run_test")
}
