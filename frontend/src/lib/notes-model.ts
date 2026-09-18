/**
 * Interface-side logic for the notes feature.
 *
 * Everything here is intentionally free of Tauri and Svelte dependencies so it
 * can be unit tested with the Node test runner (`npm run test:ui`). It holds
 * display formatting, the autosave debouncer, and small state reducers.
 *
 * Note content never leaves this module through logs: no function here prints a
 * title or a body.
 */

export interface NoteSummary {
    id: string
    revision: number
    title: string
    excerpt: string
    folder_id: string | null
    tags: string[]
    pinned: boolean
    created_at: string
    updated_at: string
    deleted_at: string | null
}

export interface Note {
    id: string
    revision: number
    title: string
    body: string
    folder_id: string | null
    tags: string[]
    pinned: boolean
    created_at: string
    updated_at: string
    deleted_at: string | null
}

export interface NoteFolder {
    id: string
    revision: number
    name: string
    created_at: string
    updated_at: string
    deleted_at: string | null
}

export interface NoteDraft {
    title: string
    body: string
    folder_id: string | null
    tags: string[]
}

export interface NoteList {
    items: NoteSummary[]
    unreadable: number
    scanned: number
}

export type TrashFilter = "active" | "trashed" | "all"
export type NoteSort = "updated_desc" | "updated_asc" | "created_desc" | "created_asc" | "title_asc"
export type NoteConflictResolution = "keep_current" | "accept_incoming" | "keep_both"

export interface NoteQuery {
    search: string
    folder_id: string | null
    tag: string | null
    trash: TrashFilter
    pinned_first: boolean
    sort: NoteSort
}

export interface NoteStats {
    notes_total: number
    notes_trashed: number
    folders_total: number
    conflicts_pending: number
    unreadable: number
}

export type StorageState = "uninitialized" | "locked" | "unlocked" | "key_missing"

export interface StorageStatus {
    state: StorageState
    dpapi_available: boolean
    backup_available: boolean
    has_stored_data: boolean
    data_dir: string
    stats: NoteStats
}

export interface NoteConflictView {
    conflict_id: string
    entity_id: string
    entity_type: string
    is_note: boolean
    current: Note | null
    incoming: Note | null
    current_revision: number
    incoming_revision: number
}

export interface ConflictResolutionOutcome {
    resolution: NoteConflictResolution
    created_entity_id: string | null
    updated_entity_id: string | null
}

/** Matches `MIN_PASSWORD_BYTES` in the crypto layer. */
export const MIN_MASTER_PASSWORD_LENGTH = 8

/** Autosave delay after the last keystroke, in milliseconds. */
export const AUTOSAVE_DELAY_MS = 700
/** Search delay, so every keystroke does not re-run the in-memory search. */
export const SEARCH_DELAY_MS = 200

export const NOTE_SORTS: readonly NoteSort[] = [
    "updated_desc",
    "updated_asc",
    "created_desc",
    "created_asc",
    "title_asc"
]

export const TRASH_FILTERS: readonly TrashFilter[] = ["active", "trashed", "all"]

export const CONFLICT_RESOLUTIONS: readonly NoteConflictResolution[] = [
    "keep_current",
    "accept_incoming",
    "keep_both"
]

export function defaultQuery(): NoteQuery {
    return {
        search: "",
        folder_id: null,
        tag: null,
        trash: "active",
        pinned_first: true,
        sort: "updated_desc"
    }
}

export function sortOptionKey(sort: NoteSort): string {
    return `notes-sort-${sort.replace(/_/g, "-")}`
}

export function trashOptionKey(filter: TrashFilter): string {
    return `notes-filter-${filter}`
}

export function conflictOptionKey(resolution: NoteConflictResolution): string {
    return `notes-conflict-${resolution.replace(/_/g, "-")}`
}

/** Indicator label for the current save state. */
export function saveIndicatorKey(state: SaveState, dirty: boolean): string {
    if (state === "saving") return "notes-saving"
    if (state === "error") return "notes-save-error"
    if (dirty || state === "dirty") return "notes-dirty"
    return "notes-saved"
}

export type SaveState = "idle" | "dirty" | "saving" | "saved" | "error"

export type SaveEvent = "edit" | "save_started" | "save_ok" | "save_failed" | "reset"

/**
 * Pure state machine behind the save indicator.
 *
 * Autosave must never claim success for an unsaved change, so only the save
 * pipeline itself can move the state to `saved`.
 */
export function nextSaveState(current: SaveState, event: SaveEvent): SaveState {
    switch (event) {
        case "edit":
            return current === "saving" ? "saving" : "dirty"
        case "save_started":
            return "saving"
        case "save_ok":
            return "saved"
        case "save_failed":
            return "error"
        case "reset":
            return "idle"
        default:
            return current
    }
}

export interface RelativeTime {
    unit: "now" | "minute" | "hour" | "day" | "date"
    value: number
}

/** Coarse relative time, used for the "changed" label on list rows. */
export function relativeTime(iso: string, now: Date = new Date()): RelativeTime {
    const timestamp = Date.parse(iso)
    if (Number.isNaN(timestamp)) {
        return { unit: "date", value: 0 }
    }
    const seconds = Math.max(0, Math.floor((now.getTime() - timestamp) / 1000))
    if (seconds < 45) return { unit: "now", value: 0 }
    const minutes = Math.floor(seconds / 60)
    if (minutes < 45) return { unit: "minute", value: Math.max(1, minutes) }
    const hours = Math.floor(minutes / 60)
    if (hours < 22) return { unit: "hour", value: Math.max(1, hours) }
    const days = Math.floor(hours / 24)
    if (days < 7) return { unit: "day", value: Math.max(1, days) }
    return { unit: "date", value: days }
}

export function timeUnitKey(unit: RelativeTime["unit"]): string {
    return `notes-time-${unit}`
}

export function formatAbsoluteDate(iso: string, locale?: string): string {
    const timestamp = Date.parse(iso)
    if (Number.isNaN(timestamp)) return ""
    const options: Intl.DateTimeFormatOptions = {
        year: "numeric",
        month: "short",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit"
    }
    return new Date(timestamp).toLocaleString(locale, options)
}

export function displayTitle(title: string, placeholder: string): string {
    const trimmed = title.trim()
    return trimmed.length > 0 ? trimmed : placeholder
}

export function displayExcerpt(excerpt: string, placeholder: string): string {
    const trimmed = excerpt.trim()
    return trimmed.length > 0 ? trimmed : placeholder
}

/** Splits a tag input on commas or newlines, trims, and drops duplicates. */
export function parseTagInput(value: string, limit = 32): string[] {
    const tags: string[] = []
    for (const part of value.split(/[,\n]/)) {
        const tag = part.trim().replace(/^#+/, "").trim()
        if (tag.length === 0) continue
        if (tags.some((existing) => existing.toLowerCase() === tag.toLowerCase())) continue
        tags.push(tag)
        if (tags.length >= limit) break
    }
    return tags
}

export function formatTagInput(tags: readonly string[]): string {
    return tags.join(", ")
}

export function draftFromNote(note: Note): NoteDraft {
    return {
        title: note.title,
        body: note.body,
        folder_id: note.folder_id,
        tags: [...note.tags]
    }
}

/** Whether the editor holds changes that are not stored yet. */
export function isDirty(note: Note | null, draft: NoteDraft | null): boolean {
    if (!draft) return false
    if (!note) return draft.title.trim().length > 0 || draft.body.trim().length > 0
    if (note.title !== draft.title) return true
    if (note.body !== draft.body) return true
    if (note.folder_id !== draft.folder_id) return true
    if (note.tags.length !== draft.tags.length) return true
    return note.tags.some((tag, index) => tag !== draft.tags[index])
}

export interface TimerApi {
    setTimeout(handler: () => void, delayMs: number): unknown
    clearTimeout(handle: unknown): void
}

export interface Debouncer<Args extends unknown[]> {
    schedule(...args: Args): void
    flush(): void
    cancel(): void
    isPending(): boolean
}

const defaultTimers: TimerApi = {
    setTimeout: (handler, delayMs) => setTimeout(handler, delayMs),
    clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>)
}

/**
 * Trailing-edge debouncer used for autosave and search.
 *
 * `flush` runs a pending call immediately, which is what happens when the user
 * switches notes or closes the editor: nothing may be lost.
 */
export function createDebouncer<Args extends unknown[]>(
    delayMs: number,
    action: (...args: Args) => void,
    timers: TimerApi = defaultTimers
): Debouncer<Args> {
    let handle: unknown = null
    let pendingArgs: Args | null = null

    const clear = () => {
        if (handle !== null) {
            timers.clearTimeout(handle)
            handle = null
        }
    }

    return {
        schedule(...args: Args) {
            pendingArgs = args
            clear()
            handle = timers.setTimeout(() => {
                handle = null
                const queued = pendingArgs
                pendingArgs = null
                if (queued) action(...queued)
            }, delayMs)
        },
        flush() {
            if (pendingArgs === null) return
            clear()
            const queued = pendingArgs
            pendingArgs = null
            action(...queued)
        },
        cancel() {
            clear()
            pendingArgs = null
        },
        isPending() {
            return pendingArgs !== null
        }
    }
}

/** Folder picker options, with an explicit "no folder" entry. */
export function folderOptions(
    folders: readonly NoteFolder[],
    noneLabel: string
): { value: string; label: string }[] {
    const options = [{ value: "", label: noneLabel }]
    for (const folder of folders) {
        if (folder.deleted_at) continue
        options.push({ value: folder.id, label: folder.name })
    }
    return options
}

/** Root path shown in the storage section of the interface. */
export function shortenPath(path: string, maxLength = 46): string {
    if (path.length <= maxLength) return path
    const separator = path.includes("\\") ? "\\" : "/"
    const parts = path.split(separator)
    let result = parts[parts.length - 1]
    for (let index = parts.length - 2; index >= 0; index -= 1) {
        const candidate = `${parts[index]}${separator}…${separator}${result}`
        if (candidate.length > maxLength) break
        result = candidate
    }
    return `…${separator}${result}`
}
