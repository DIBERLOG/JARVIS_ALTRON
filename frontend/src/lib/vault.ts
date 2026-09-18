/**
 * Typed wrappers around the password vault commands.
 *
 * Commands run on worker threads in the backend, so Argon2id, SQLite, and file
 * dialogs never freeze the window. A password only crosses this boundary through
 * `reveal`; the clipboard commands return status only.
 */

import { invoke } from "@tauri-apps/api/core"

import type {
    ClipboardStatus,
    GeneratedPassword,
    GeneratedSecretStatus,
    IdleStatus,
    PasswordPolicy,
    SecretRevealResult,
    VaultConflictOutcome,
    VaultConflictResolution,
    VaultConflictView,
    VaultItemDetails,
    VaultItemDraft,
    VaultItemList,
    VaultMetadataDraft,
    VaultQuery,
    VaultStatus
} from "./vault-model"

/** Short, non-secret label for the storage directory. */
export function vaultStorageLabel(status: VaultStatus): string {
    const path = status.storage.data_dir ?? ""
    if (path.length <= 46) return path
    const separator = path.includes("\\") ? "\\" : "/"
    const parts = path.split(separator)
    let result = parts[parts.length - 1]
    for (let index = parts.length - 2; index >= 0; index -= 1) {
        const candidate = `${parts[index]}${separator}…${separator}${result}`
        if (candidate.length > 46) break
        result = candidate
    }
    return `…${separator}${result}`
}

export const vaultApi = {
    // storage and key lifecycle (shared with the notes storage)
    status: () => invoke<VaultStatus>("vault_status"),
    initialize: (password: string) => invoke<VaultStatus>("vault_initialize", { password }),
    unlockWithPassword: (password: string) =>
        invoke<VaultStatus>("vault_unlock_password", { password }),
    unlockWithDpapi: () => invoke<VaultStatus>("vault_unlock_dpapi"),
    lock: () => invoke<VaultStatus>("vault_lock"),
    importBackup: (envelope: string, password: string) =>
        invoke<VaultStatus>("vault_import_backup", { envelope, password }),
    importBackupFile: (password: string) =>
        invoke<VaultStatus>("vault_import_backup_file", { password }),
    /** Returns the written path, or an empty string when the user cancels. */
    exportBackupFile: (password: string) =>
        invoke<string>("vault_export_backup_file", { password }),
    changeMasterPassword: (current: string, newPassword: string) =>
        invoke<{ backup_replaced: boolean; dpapi_updated: boolean; backup_path: string }>(
            "vault_change_master_password",
            { current, newPassword }
        ),

    // idle locking
    setIdleTimeout: (seconds: number) => invoke<IdleStatus>("vault_set_idle_timeout", { seconds }),
    idleStatus: () => invoke<IdleStatus>("vault_idle_status"),
    /** Records interface activity so mouse movement keeps the vault unlocked. */
    touch: () => invoke<IdleStatus>("vault_touch"),

    // items
    list: (query: VaultQuery) => invoke<VaultItemList>("vault_list", { query }),
    get: (id: string) => invoke<VaultItemDetails | null>("vault_get", { id }),
    create: (draft: VaultItemDraft) => invoke<VaultItemDetails>("vault_create", { draft }),
    update: (id: string, draft: VaultItemDraft) =>
        invoke<VaultItemDetails>("vault_update", { id, draft }),
    updateMetadata: (id: string, metadata: VaultMetadataDraft) =>
        invoke<VaultItemDetails>("vault_update_metadata", { id, metadata }),
    updateSecrets: (id: string, password: string, notes: string) =>
        invoke<VaultItemDetails>("vault_update_secrets", { id, password, notes }),
    setFavorite: (id: string, favorite: boolean) =>
        invoke<VaultItemDetails>("vault_set_favorite", { id, favorite }),
    trash: (id: string) => invoke<VaultItemDetails>("vault_trash", { id }),
    restore: (id: string) => invoke<VaultItemDetails>("vault_restore", { id }),
    purge: (id: string) => invoke<void>("vault_purge", { id }),
    tags: () => invoke<string[]>("vault_tags"),

    // secrets
    reveal: (id: string) => invoke<SecretRevealResult>("vault_reveal", { id }),
    copyUsername: (id: string, clearAfterSeconds: number) =>
        invoke<ClipboardStatus>("vault_copy_username", { id, clearAfterSeconds }),
    copyPassword: (id: string, clearAfterSeconds: number) =>
        invoke<ClipboardStatus>("vault_copy_password", { id, clearAfterSeconds }),
    clipboardStatus: () => invoke<ClipboardStatus>("vault_clipboard_status"),
    clipboardClear: () => invoke<ClipboardStatus>("vault_clipboard_clear"),

    // generator
    generatePassword: (policy: PasswordPolicy) =>
        invoke<GeneratedPassword>("vault_generate_password", { policy }),
    generateAndCopy: (policy: PasswordPolicy, clearAfterSeconds: number) =>
        invoke<GeneratedSecretStatus>("vault_generate_and_copy", { policy, clearAfterSeconds }),

    // conflicts
    conflicts: () => invoke<VaultConflictView[]>("vault_conflicts"),
    resolveConflict: (conflict: string, resolution: VaultConflictResolution) =>
        invoke<VaultConflictOutcome>("vault_resolve_conflict", { conflict, resolution })
}
