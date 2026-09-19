/**
 * Interface-side logic for the full backup and restore.
 *
 * The rules encoded here:
 *
 * * the window never shows a path. A component is named by a logical name the
 *   core produced, and this module turns it into a translated label;
 * * the window never holds a secret. The password is typed into a field, sent
 *   once, and never stored, never logged, and never put in a URL or in browser
 *   storage;
 * * a restore is never one click: the container is opened and verified first,
 *   the person sees what it holds, and only then does the confirm button exist.
 */

/** One entry of the preview, as the core reports it. */
export interface BackupPreviewEntry {
    name: string
    kind: "sqlite" | "document" | "portable_key"
    bytes: number
}

/** What the core tells the window about a container before a restore. */
export interface BackupPreview {
    format_version: number
    created_at: string
    app_version: string
    total_bytes: number
    entries: BackupPreviewEntry[]
    warnings: string[]
}

/** The status of the feature, and what a full backup would contain. */
export interface BackupPanelView {
    status: {
        available: boolean
        format_version: number
        key_envelope_present: boolean
        interrupted_restore: string | null
        previous_state_present: boolean
        newest_safety_backup: string | null
        last_operation: string | null
        last_error_code: string | null
    }
    plan: {
        included: string[]
        absent: string[]
        available: boolean
        key_envelope_present: boolean
    }
}

/** The shortest password the core accepts. */
export const MIN_PASSWORD_BYTES = 8

/**
 * The Fluent key of a logical component name.
 *
 * A name is `component/file`; the label is the component, so the list a person
 * reads is a list of features and not of file names.
 */
export function componentLabelKey(name: string): string {
    const component = name.split("/")[0] ?? ""
    const known = ["notes", "vault", "memory", "autocorrect", "key", "settings"]
    return known.includes(component) ? `backup-component-${component}` : "backup-component-other"
}

/** The Fluent key of a content-free warning code from the core. */
export function warningKey(code: string): string {
    const known = ["no_vault", "no_notes", "no_key_envelope"]
    return known.includes(code) ? `backup-warning-${code}` : "backup-warning-unknown"
}

/** Why a password cannot be used, or null when it can. */
export function passwordProblem(password: string): string | null {
    // Bytes, not characters: the core counts bytes, and a password of a few
    // Cyrillic characters is longer in bytes than in characters.
    const bytes = new TextEncoder().encode(password).length
    if (bytes === 0) return "backup-password-empty"
    if (bytes < MIN_PASSWORD_BYTES) return "backup-password-short"
    return null
}

/** A size a person can read, without a path or a name. */
export function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return "0"
    if (bytes < 1024) return `${bytes}`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
}

/** The Fluent key of the last operation, whether it succeeded, or why it did not. */
export function operationKey(status: BackupPanelView["status"]): string {
    if (!status.last_operation) return "backup-operation-none"
    const failed = Boolean(status.last_error_code && status.last_error_code !== "none")
    const key = `backup-operation-${status.last_operation}`
    return failed ? `${key}-failed` : key
}
/** Whether a container may be restored: the two components without which it is not one. */
export function containerIsRestorable(preview: BackupPreview): boolean {
    const names = preview.entries.map((entry) => entry.name)
    return (
        names.includes("notes/sync.sqlite3") && names.includes("key/portable-envelope.json")
    )
}

/** The warning a person must see before restoring, in order of importance. */
export function previewWarnings(preview: BackupPreview): string[] {
    return preview.warnings.map(warningKey)
}
