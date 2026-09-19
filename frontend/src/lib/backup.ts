/**
 * Typed wrappers around the backup commands.
 *
 * Nothing here chooses a path: the core opens the native dialog, and the window
 * only ever learns the *name* of the container it made or read. The password is
 * passed through and is not kept anywhere.
 */

import { invoke } from "@tauri-apps/api/core"

import type { BackupPanelView, BackupPreview } from "./backup-model"

/** What an export produced, as the core reports it. */
export interface ExportReport {
    format_version: number
    created_at: string
    app_version: string
    components: string[]
    total_bytes: number
    /** File name only. */
    file: string
}

/** What a restore did. */
export interface RestoreOutcome {
    report: {
        restored: string[]
        absent: string[]
        safety_backup: string | null
        previous_state: string | null
        local_key: string
        rolled_back: boolean | null
        total_bytes: number
    }
    storage_locked: boolean
    safety_backup: string | null
    previous_state: boolean
}

export const backupApi = {
    /** The state of the feature, and what a full backup would contain. */
    status: () => invoke<BackupPanelView>("backup_status"),

    /**
     * Creates a full backup. The destination is chosen in the native dialog.
     *
     * `overwrite` is false unless the person confirmed that an existing file may
     * be replaced: the core refuses to replace a file without that.
     */
    export: (password: string, overwrite: boolean) =>
        invoke<ExportReport>("backup_export", { password, overwrite }),

    /**
     * Opens a container and reports what it holds, changing nothing.
     *
     * Returns null when the person closed the dialog.
     */
    inspect: (password: string) =>
        invoke<BackupPreview | null>("backup_inspect", { password }),

    /**
     * Restores a container over the current state.
     *
     * The container is chosen in the native dialog again, so the restore is of
     * the file the person picked and not of a path from an earlier call.
     */
    restore: (password: string, confirmed: boolean) =>
        invoke<RestoreOutcome | null>("backup_restore", { password, confirmed }),

    /** Deletes the state a previous restore replaced, when the person asks. */
    discardPrevious: () => invoke<boolean>("backup_discard_previous"),

    /** Deletes one safety backup, by file name. */
    deleteSafety: (name: string) => invoke<boolean>("backup_delete_safety", { name })
}
