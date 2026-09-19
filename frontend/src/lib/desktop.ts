/**
 * Typed wrappers around the desktop shell commands.
 *
 * The window asks; the core decides. Nothing here writes a registry key, touches
 * the tray, builds a diagnostics document, or keeps a password: a close answer
 * is a word, an autostart change is a call, and a diagnostics report comes back
 * already built and already checked.
 */

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

import type {
    AutostartStatus,
    CloseBehavior,
    DesktopSettings,
    DesktopState,
    DiagnosticsView,
    SetupState
} from "./desktop-model"

/** The event the core emits when the close button was pressed and asks. */
export const CLOSE_REQUESTED_EVENT = "desktop-close-requested"
/** The event the core emits when the tray changed something. */
export const STATE_CHANGED_EVENT = "desktop-state-changed"
/** The event the core emits when the tray asked for the settings page. */
export const OPEN_SETTINGS_EVENT = "desktop-open-settings"

export const desktopApi = {
    /** The tray, the microphone, autostart, and the first-run state in one call. */
    state: () => invoke<DesktopState>("desktop_get_state"),
    showWindow: () => invoke<void>("desktop_show_window"),
    hideWindow: () => invoke<void>("desktop_hide_window"),
    /** The one exit route: tray, dialog, and shutdown all use it. */
    requestExit: () => invoke<void>("desktop_request_exit"),
    getCloseBehavior: () => invoke<CloseBehavior>("desktop_get_close_behavior"),
    /** `remember` false leaves the setting alone and only answers this close. */
    setCloseBehavior: (behavior: CloseBehavior, remember: boolean) =>
        invoke<DesktopSettings>("desktop_set_close_behavior", { behavior, remember }),
    /** Stores the close behaviour and the autostart switches in one document. */
    updateSettings: (settings: DesktopSettings) =>
        invoke<DesktopSettings>("desktop_update_settings", { settings }),
    /** Locks the encrypted stores and drops the spelling journals. */
    lockStorage: () => invoke<boolean>("desktop_lock_storage"),

    autostart: () => invoke<AutostartStatus>("autostart_get_state"),
    autostartEnable: () => invoke<AutostartStatus>("autostart_enable"),
    autostartDisable: () => invoke<AutostartStatus>("autostart_disable"),

    setup: () => invoke<SetupState>("setup_get_state"),
    /** `language` is stored only by the language step. */
    completeStep: (step: string, language: string | null = null) =>
        invoke<SetupState>("setup_complete_step", { step, language }),
    skipStep: (step: string) => invoke<SetupState>("setup_skip_step", { step }),
    finishSetup: (timestamp: string) => invoke<SetupState>("setup_finish", { timestamp }),
    /** Runs the wizard again without touching any setting. */
    resetSetup: () => invoke<SetupState>("setup_reset"),

    diagnosticsRun: () => invoke<DiagnosticsView>("diagnostics_run"),
    diagnosticsPreview: () => invoke<string[]>("diagnostics_preview"),
    /** Returns the written path, or null when the user cancels the dialog. */
    diagnosticsExport: () => invoke<string | null>("diagnostics_export"),
    diagnosticsSummary: () => invoke<string>("diagnostics_summary")
}

/** Subscribes to the close request; the window then shows its own dialog. */
export async function onCloseRequested(handler: () => void): Promise<() => void> {
    return listen(CLOSE_REQUESTED_EVENT, () => handler())
}

/** Subscribes to the tray's state changes. */
export async function onStateChanged(handler: () => void): Promise<() => void> {
    return listen(STATE_CHANGED_EVENT, () => handler())
}

/** Subscribes to the tray's "open the settings" request. */
export async function onOpenSettings(handler: () => void): Promise<() => void> {
    return listen(OPEN_SETTINGS_EVENT, () => handler())
}
