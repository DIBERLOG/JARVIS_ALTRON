/**
 * Typed wrappers around the Windows-action commands.
 *
 * The interface asks for an action by name and receives either a result or a preview. It never
 * sends an executable path, never builds a command line, and never decides a risk level: every
 * one of those lives in the core, behind one policy and one confirmation gate.
 *
 * The pending action is held in the core, not here, so a reload forgets it — which is exactly
 * what should happen to an approval that expires.
 */

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

import type {
    AiActionOutcome,
    ActionPreview,
    ActionRequestOutcome,
    ActionResult,
    ActionSource,
    AuditEntry,
    AllowedApplicationView,
    ScheduledView,
    VoiceRoute,
    WindowSummary,
    WindowsAction,
    WindowsActionSettings,
    WindowsActionsOverview
} from "./windows-actions-model"

/** The event the core emits when a timer or reminder fires. */
export const FIRED_EVENT = "windows-actions-fired"

export const windowsActionsApi = {
    /** Capabilities, settings, policy table, and the allowed programs, in one call. */
    overview: () => invoke<WindowsActionsOverview>("windows_actions_overview"),
    updateSettings: (settings: WindowsActionSettings) =>
        invoke<WindowsActionSettings>("windows_actions_update_settings", { settings }),

    /** Asks for one action; the answer is a result or the preview that must be confirmed. */
    request: (action: WindowsAction, source: ActionSource = "direct_gui") =>
        invoke<ActionRequestOutcome>("windows_actions_request", { action, source }),
    pending: () => invoke<ActionPreview | null>("windows_actions_pending"),
    /** Confirms with the token from the preview; the core runs the request it stored. */
    confirm: (token: string) => invoke<ActionResult>("windows_actions_confirm", { token }),
    cancel: () => invoke<boolean>("windows_actions_cancel"),

    listWindows: () => invoke<WindowSummary[]>("windows_actions_list_windows"),
    scheduled: () => invoke<ScheduledView[]>("windows_actions_scheduled"),
    takeFired: () => invoke<ScheduledView[]>("windows_actions_take_fired"),
    pruneScheduled: () => invoke<number>("windows_actions_prune_scheduled"),

    auditLog: (limit: number = 200) => invoke<AuditEntry[]>("windows_actions_audit_log", { limit }),
    clearAuditLog: (confirmed: boolean) =>
        invoke<void>("windows_actions_clear_audit_log", { confirmed }),
    /** Returns the number of written lines, or null when the user cancels. */
    exportAuditLog: (confirmed: boolean) =>
        invoke<number | null>("windows_actions_export_audit_log", { confirmed }),

    /** Opens the native file dialog in the core; the program is never named by the interface. */
    addAllowedApplication: (
        displayName: string,
        fixedArguments: string[] = [],
        workingDirectory: string | null = null
    ) =>
        invoke<AllowedApplicationView | null>("windows_actions_add_allowed_application", {
            displayName,
            fixedArguments,
            workingDirectory
        }),
    removeAllowedApplication: (id: string) =>
        invoke<AllowedApplicationView>("windows_actions_remove_allowed_application", { id }),
    setAllowedApplicationEnabled: (id: string, enabled: boolean) =>
        invoke<AllowedApplicationView>("windows_actions_set_allowed_application_enabled", {
            id,
            enabled
        }),
    reacceptAllowedApplication: (id: string) =>
        invoke<AllowedApplicationView>("windows_actions_reaccept_allowed_application", { id }),

    routeVoice: (text: string) => invoke<VoiceRoute>("windows_actions_route_voice", { text }),
    tools: () => invoke<unknown[]>("windows_actions_tools"),

    /**
     * Sends one phrase to the local model with the action catalogue attached.
     *
     * The model's answer is either prose — which is shown and never acted on — or a structured
     * tool call, which the core decodes against the tool's schema and puts through the usual
     * policy and confirmation. A model that cannot carry tool calls is reported as such.
     */
    aiRequest: (phrase: string) => invoke<AiActionOutcome>("windows_actions_ai_request", { phrase })
}

/**
 * Subscribes to the "a timer or reminder fired" signal.
 *
 * The signal carries nothing: the items themselves are collected with `takeFired`, so a missed
 * event cannot lose a reminder — the next poll still finds it.
 */
export async function onFired(handler: () => void): Promise<() => void> {
    return listen(FIRED_EVENT, () => handler())
}
