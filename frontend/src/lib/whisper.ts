/**
 * Typed wrappers around the dictation commands.
 *
 * The window never runs a process and never names a path: picking the
 * executable and the model happens in the native dialog, inside the core. The
 * transcript that comes back is shown and never stored — no browser storage, no
 * file, no URL.
 */

import { invoke } from "@tauri-apps/api/core"

import type { DictationStatus, Transcript, WhisperSettings } from "./whisper-model"

export interface WhisperPanelView {
    status: DictationStatus
    settings: WhisperSettings
    last: Transcript | null
}

export const whisperApi = {
    /** State, settings, and the transcript of the last dictation, in one call. */
    status: () => invoke<WhisperPanelView>("whisper_status"),
    updateSettings: (settings: WhisperSettings) =>
        invoke<WhisperSettings>("whisper_update_settings", { settings }),

    /** Opens the native dialog for the executable; null when the user cancels. */
    selectBinary: () => invoke<WhisperSettings | null>("whisper_select_binary"),
    /** Opens the native dialog for the model; null when the user cancels. */
    selectModel: () => invoke<WhisperSettings | null>("whisper_select_model"),

    /** Records from the microphone and returns the text. */
    dictate: () => invoke<Transcript>("whisper_dictate"),
    /** Transcribes a 16 kHz mono WAV the user picks; null when cancelled. */
    transcribeFile: () => invoke<Transcript | null>("whisper_transcribe_file"),
    /** Stops a recording or a transcription; false when nothing was running. */
    cancel: () => invoke<boolean>("whisper_cancel"),
    /** Forgets the transcript the panel is showing. */
    clearLast: () => invoke<void>("whisper_clear_last")
}
