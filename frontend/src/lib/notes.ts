/**
 * Typed wrappers around the notes commands.
 *
 * Every command runs on a worker thread in the backend, so a save never freezes
 * the window. Errors arrive as content-free messages; nothing here logs note
 * text, and no title or body is ever placed in a URL.
 */

import { invoke } from "@tauri-apps/api/core"

import type {
    ConflictResolutionOutcome,
    Note,
    NoteConflictResolution,
    NoteConflictView,
    NoteDraft,
    NoteFolder,
    NoteList,
    NoteQuery,
    NoteSummary,
    StorageStatus
} from "./notes-model"

export const notesApi = {
    // storage / key lifecycle
    status: () => invoke<StorageStatus>("notes_status"),
    initialize: (password: string) => invoke<StorageStatus>("notes_initialize", { password }),
    unlockWithDpapi: () => invoke<StorageStatus>("notes_unlock_dpapi"),
    unlockWithPassword: (password: string) =>
        invoke<StorageStatus>("notes_unlock_password", { password }),
    lock: () => invoke<StorageStatus>("notes_lock"),
    importBackup: (envelope: string, password: string) =>
        invoke<StorageStatus>("notes_import_backup", { envelope, password }),
    importBackupFile: (password: string) =>
        invoke<StorageStatus>("notes_import_backup_file", { password }),
    exportBackup: (password: string) => invoke<string>("notes_export_backup", { password }),
    /** Returns the written path, or an empty string when the user cancels. */
    exportBackupFile: (password: string) =>
        invoke<string>("notes_export_backup_file", { password }),

    // notes
    list: (query: NoteQuery) => invoke<NoteList>("notes_list", { query }),
    get: (id: string) => invoke<Note | null>("notes_get", { id }),
    create: (draft: NoteDraft) => invoke<Note>("notes_create", { draft }),
    update: (id: string, draft: NoteDraft) => invoke<Note>("notes_update", { id, draft }),
    autosave: (id: string, draft: NoteDraft) => invoke<Note>("notes_autosave", { id, draft }),
    setPinned: (id: string, pinned: boolean) =>
        invoke<Note>("notes_set_pinned", { id, pinned }),
    trash: (id: string) => invoke<Note>("notes_trash", { id }),
    restore: (id: string) => invoke<Note>("notes_restore", { id }),
    purge: (id: string) => invoke<void>("notes_purge", { id }),

    // folders
    folders: () => invoke<NoteFolder[]>("notes_folders"),
    createFolder: (name: string) => invoke<NoteFolder>("notes_create_folder", { name }),
    renameFolder: (id: string, name: string) =>
        invoke<NoteFolder>("notes_rename_folder", { id, name }),
    trashFolder: (id: string) => invoke<NoteFolder>("notes_trash_folder", { id }),
    restoreFolder: (id: string) => invoke<NoteFolder>("notes_restore_folder", { id }),
    purgeFolder: (id: string) => invoke<void>("notes_purge_folder", { id }),

    // tags and conflicts
    tags: () => invoke<string[]>("notes_tags"),
    conflicts: () => invoke<NoteConflictView[]>("notes_conflicts"),
    resolveConflict: (conflict: string, resolution: NoteConflictResolution) =>
        invoke<ConflictResolutionOutcome>("notes_resolve_conflict", { conflict, resolution })
}

export type { NoteSummary }
