/**
 * Interface-side logic for the password vault.
 *
 * Like `notes-model.ts`, everything here is free of Tauri and Svelte
 * dependencies so it can be unit tested with the Node test runner
 * (`npm run test:ui`). It holds masking and reveal rules, the generator policy
 * validation, the idle and clipboard option sets, and small state helpers.
 *
 * Two rules are encoded here on purpose:
 *
 * * a secret is never rendered unless the user explicitly revealed it, and
 *   hiding clears the value from component state;
 * * an item whose secret was never revealed is saved through the metadata-only
 *   path, so renaming cannot erase a stored password.
 *
 * `localStorage`, `sessionStorage`, the URL, and IndexedDB are never touched:
 * there is no function in this module that persists anything.
 */

import type { SaveState } from "./notes-model"
import type { StorageStatus } from "./notes-model"

export type { SaveState, StorageStatus }

/**
 * Save-indicator label, kept local so this module has no runtime imports and can
 * be loaded directly by the Node test runner.
 *
 * It mirrors `saveIndicatorKey` in `notes-model.ts` and returns the same message
 * keys, so the vault reuses the existing translations.
 */
export function saveIndicatorKey(state: SaveState, dirty: boolean): string {
    if (state === "saving") return "notes-saving"
    if (state === "error") return "notes-save-error"
    if (dirty || state === "dirty") return "notes-dirty"
    return "notes-saved"
}

export interface VaultItemSummary {
    id: string
    revision: number
    name: string
    username: string
    url_host: string | null
    tags: string[]
    favorite: boolean
    has_password: boolean
    created_at: string
    updated_at: string
    deleted_at: string | null
}

export interface VaultItemDetails {
    id: string
    revision: number
    name: string
    username: string
    urls: string[]
    tags: string[]
    favorite: boolean
    created_at: string
    updated_at: string
    deleted_at: string | null
}

export interface SecretRevealResult {
    id: string
    revision: number
    password: string
    notes: string
    reveal_timeout_seconds: number
}

export interface VaultItemDraft {
    name: string
    username: string
    password: string
    urls: string[]
    notes: string
    tags: string[]
    favorite: boolean
}

export interface VaultMetadataDraft {
    name: string
    username: string
    urls: string[]
    tags: string[]
    favorite: boolean
}

export interface VaultItemList {
    items: VaultItemSummary[]
    unreadable: number
    scanned: number
}

export type VaultTrashFilter = "active" | "trashed" | "all"
export type VaultSort = "name_asc" | "updated_desc" | "created_desc"
export type VaultConflictResolution = "keep_current" | "accept_incoming" | "keep_both"

export interface VaultQuery {
    search: string
    tag: string | null
    trash: VaultTrashFilter
    favorites_only: boolean
    sort: VaultSort
}

export interface VaultConflictView {
    conflict_id: string
    entity_id: string
    is_vault_item: boolean
    current_name: string | null
    incoming_name: string | null
    current_revision: number
    incoming_revision: number
}

export interface VaultConflictOutcome {
    resolution: VaultConflictResolution
    created_entity_id: string | null
    updated_entity_id: string | null
}

export interface VaultStats {
    items_total: number
    items_trashed: number
    favorites: number
    conflicts_pending: number
    unreadable: number
}

export interface VaultStatus {
    storage: StorageStatus
    stats: VaultStats
}

export interface ClipboardStatus {
    armed: boolean
    remaining_seconds: number
    timeout_seconds: number
}

export interface IdleStatus {
    timeout_seconds: number
    automatic: boolean
    remaining_seconds: number | null
}

export interface PasswordPolicy {
    length: number
    lowercase: boolean
    uppercase: boolean
    digits: boolean
    symbols: boolean
    exclude_similar: boolean
    require_each_category: boolean
}

export interface GeneratedPassword {
    password: string
    entropy_bits: number
}

export interface GeneratedSecretStatus {
    entropy_bits: number
    clipboard: ClipboardStatus
}

export const VAULT_SORTS: readonly VaultSort[] = ["name_asc", "updated_desc", "created_desc"]
export const VAULT_TRASH_FILTERS: readonly VaultTrashFilter[] = ["active", "trashed", "all"]
export const CONFLICT_RESOLUTIONS: readonly VaultConflictResolution[] = [
    "keep_current",
    "accept_incoming",
    "keep_both"
]

/** Idle timeouts offered by the interface, in seconds. Zero means never. */
export const IDLE_TIMEOUT_OPTIONS: readonly number[] = [60, 300, 900, 1800, 0]
export const DEFAULT_IDLE_TIMEOUT_SECONDS = 300

/** Automatic clipboard-clear delays, in seconds. */
export const CLIPBOARD_TIMEOUT_OPTIONS: readonly number[] = [15, 30, 45, 60]
export const DEFAULT_CLIPBOARD_TIMEOUT_SECONDS = 30

/** Matches `MIN_PASSWORD_BYTES` in the core crypto layer. */
export const MIN_MASTER_PASSWORD_LENGTH = 8

export const GENERATOR_LENGTH_MIN = 8
export const GENERATOR_LENGTH_MAX = 128
export const GENERATOR_LENGTH_DEFAULT = 20

/** Character used to mask a hidden secret. */
export const MASK_CHARACTER = "•"
const MASK_LENGTH = 12

const LOWERCASE = "abcdefghijklmnopqrstuvwxyz"
const UPPERCASE = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
const DIGITS = "0123456789"
const SYMBOLS = "!@#$%^&*()-_=+[]{};:,.?/"
const SIMILAR = "il1IL|Lo0O"

export function defaultQuery(): VaultQuery {
    return {
        search: "",
        tag: null,
        trash: "active",
        favorites_only: false,
        sort: "name_asc"
    }
}

export function defaultPolicy(): PasswordPolicy {
    return {
        length: GENERATOR_LENGTH_DEFAULT,
        lowercase: true,
        uppercase: true,
        digits: true,
        symbols: true,
        exclude_similar: false,
        require_each_category: true
    }
}

// --------------------------------------------------------------- masking

/**
 * The value a password field may display: a fixed-length mask while hidden, the
 * real value only after an explicit reveal.
 */
export function maskedSecret(value: string, revealed: boolean): string {
    if (!revealed) {
        return MASK_CHARACTER.repeat(MASK_LENGTH)
    }
    return value
}

/** The value to store in component state for a hidden secret: nothing. */
export function hiddenSecretValue(): string {
    return ""
}

/** State to assign when hiding a secret again. */
export function hiddenSecrets(): { password: string; notes: string } {
    return { password: "", notes: "" }
}

export function isSecretEmpty(value: string): boolean {
    return value.length === 0
}

// ------------------------------------------------------------- dirty state

export function metadataFromDetails(item: VaultItemDetails): VaultMetadataDraft {
    return {
        name: item.name,
        username: item.username,
        urls: [...item.urls],
        tags: [...item.tags],
        favorite: item.favorite
    }
}

export function isMetadataDirty(
    item: VaultItemDetails | null,
    metadata: VaultMetadataDraft | null
): boolean {
    if (!item || !metadata) return false
    if (item.name !== metadata.name) return true
    if (item.username !== metadata.username) return true
    if (item.favorite !== metadata.favorite) return true
    if (item.urls.length !== metadata.urls.length) return true
    if (item.urls.some((url, index) => url !== metadata.urls[index])) return true
    if (item.tags.length !== metadata.tags.length) return true
    return item.tags.some((tag, index) => tag !== metadata.tags[index])
}

/**
 * Whether the revealed secret differs from what was revealed.
 *
 * Only meaningful after a reveal: without one, the interface does not hold the
 * secret at all and must not report it as changed.
 */
export function isSecretDirty(
    revealed: boolean,
    original: { password: string; notes: string } | null,
    current: { password: string; notes: string }
): boolean {
    if (!revealed || !original) return false
    return original.password !== current.password || original.notes !== current.notes
}

/** Save-state machine reuse: the vault uses the same indicator as the notes. */
export function nextSaveState(current: SaveState, event: "edit" | "save_started" | "save_ok" | "save_failed" | "reset"): SaveState {
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

// ------------------------------------------------------------------ urls

/** One URL per line, trimmed and de-duplicated. */
export function parseUrlInput(value: string, limit = 16): string[] {
    const urls: string[] = []
    for (const part of value.split(/[\n,]/)) {
        const url = part.trim()
        if (url.length === 0) continue
        if (urls.includes(url)) continue
        urls.push(url)
        if (urls.length >= limit) break
    }
    return urls
}

export function formatUrlInput(urls: readonly string[]): string {
    return urls.join("\n")
}

// ------------------------------------------------------------- generator

export function clampGeneratorLength(value: number): number {
    if (!Number.isFinite(value)) return GENERATOR_LENGTH_DEFAULT
    const rounded = Math.round(value)
    if (rounded < GENERATOR_LENGTH_MIN) return GENERATOR_LENGTH_MIN
    if (rounded > GENERATOR_LENGTH_MAX) return GENERATOR_LENGTH_MAX
    return rounded
}

function alphabetFor(policy: PasswordPolicy): string {
    let alphabet = ""
    if (policy.lowercase) alphabet += LOWERCASE
    if (policy.uppercase) alphabet += UPPERCASE
    if (policy.digits) alphabet += DIGITS
    if (policy.symbols) alphabet += SYMBOLS
    if (policy.exclude_similar) {
        alphabet = [...alphabet].filter((character) => !SIMILAR.includes(character)).join("")
    }
    return alphabet
}

export function selectedCategoryCount(policy: PasswordPolicy): number {
    let count = 0
    if (policy.lowercase) count += 1
    if (policy.uppercase) count += 1
    if (policy.digits) count += 1
    if (policy.symbols) count += 1
    return count
}

/**
 * Local mirror of the core estimate, used for the live preview while the user
 * adjusts the policy. The generated password itself always comes from the core
 * CSPRNG generator.
 */
export function estimateEntropyBits(policy: PasswordPolicy): number {
    const size = alphabetFor(policy).length
    if (size <= 1) return 0
    return policy.length * Math.log2(size)
}

export interface PolicyCheck {
    ok: boolean
    errorKey?: string
}

export function validatePolicy(policy: PasswordPolicy): PolicyCheck {
    if (selectedCategoryCount(policy) === 0) {
        return { ok: false, errorKey: "vault-generator-needs-category" }
    }
    if (policy.length < GENERATOR_LENGTH_MIN || policy.length > GENERATOR_LENGTH_MAX) {
        return { ok: false, errorKey: "vault-generator-length-invalid" }
    }
    if (policy.exclude_similar && alphabetFor(policy).length === 0) {
        return { ok: false, errorKey: "vault-generator-needs-category" }
    }
    if (policy.require_each_category && policy.length < selectedCategoryCount(policy)) {
        return { ok: false, errorKey: "vault-generator-too-short" }
    }
    return { ok: true }
}

// ------------------------------------------------------------- clipboard

export function clipboardIndicatorKey(status: ClipboardStatus | null): string {
    if (!status || !status.armed) return "vault-clipboard-idle"
    return "vault-clipboard-armed"
}

/** Countdown text, e.g. "30" plus the unit key handled by the component. */
export function clipboardRemaining(status: ClipboardStatus | null): number {
    if (!status || !status.armed) return 0
    return Math.max(0, Math.floor(status.remaining_seconds))
}

export function isClipboardArmed(status: ClipboardStatus | null): boolean {
    return Boolean(status && status.armed)
}

// ------------------------------------------------------------------ idle

export function idleOptionKey(seconds: number): string {
    switch (seconds) {
        case 0:
            return "vault-idle-never"
        case 60:
            return "vault-idle-1"
        case 300:
            return "vault-idle-5"
        case 900:
            return "vault-idle-15"
        case 1800:
            return "vault-idle-30"
        default:
            return "vault-idle-5"
    }
}

export function clipboardOptionKey(seconds: number): string {
    switch (seconds) {
        case 15:
            return "vault-clipboard-15"
        case 30:
            return "vault-clipboard-30"
        case 45:
            return "vault-clipboard-45"
        case 60:
            return "vault-clipboard-60"
        default:
            return "vault-clipboard-30"
    }
}

export function normalizeIdleTimeout(seconds: number): number {
    return IDLE_TIMEOUT_OPTIONS.includes(seconds) ? seconds : DEFAULT_IDLE_TIMEOUT_SECONDS
}

export function normalizeClipboardTimeout(seconds: number): number {
    return CLIPBOARD_TIMEOUT_OPTIONS.includes(seconds)
        ? seconds
        : DEFAULT_CLIPBOARD_TIMEOUT_SECONDS
}

/** Whether the idle deadline has passed for the given status and clock. */
export function isIdleExpired(status: IdleStatus | null, elapsedSeconds: number): boolean {
    if (!status || !status.automatic) return false
    if (status.remaining_seconds === null) return false
    return elapsedSeconds >= status.remaining_seconds
}

// ------------------------------------------------------- master password

export interface PasswordChangeCheck {
    ok: boolean
    errorKey?: string
}

export function checkPasswordChange(
    current: string,
    next: string,
    confirmation: string,
    minLength = MIN_MASTER_PASSWORD_LENGTH
): PasswordChangeCheck {
    if (current.length === 0) return { ok: false, errorKey: "vault-change-needs-current" }
    if (next.length < minLength) return { ok: false, errorKey: "vault-password-short" }
    if (next !== confirmation) return { ok: false, errorKey: "vault-password-mismatch" }
    if (next === current) return { ok: false, errorKey: "vault-change-same-password" }
    return { ok: true }
}

// ------------------------------------------------------------ presentation

export function itemTitle(name: string, placeholder: string): string {
    const trimmed = name.trim()
    return trimmed.length > 0 ? trimmed : placeholder
}

export function itemSubtitle(summary: VaultItemSummary, noneLabel: string): string {
    if (summary.username.trim().length > 0) return summary.username
    if (summary.url_host) return summary.url_host
    return noneLabel
}

export function sortOptionKey(sort: VaultSort): string {
    return `vault-sort-${sort.replace(/_/g, "-")}`
}

export function trashOptionKey(filter: VaultTrashFilter): string {
    return `vault-filter-${filter}`
}

export function conflictOptionKey(resolution: VaultConflictResolution): string {
    return `vault-conflict-${resolution.replace(/_/g, "-")}`
}

/** Minimum-length rule shared by the interface and the core. */
export function needsMasterPasswordSetup(status: VaultStatus | null): boolean {
    if (!status) return false
    return status.storage.state === "uninitialized"
}

export function isKeyMissing(status: VaultStatus | null): boolean {
    if (!status) return false
    return status.storage.state === "key_missing"
}

export function isUnlocked(status: VaultStatus | null): boolean {
    if (!status) return false
    return status.storage.state === "unlocked"
}

/** Everything the interface must clear when the vault locks. */
export interface ClearedVaultView {
    items: VaultItemSummary[]
    selected: null
    metadata: null
    revealed: false
    secrets: { password: string; notes: string }
    conflicts: []
    tags: string[]
}

export function clearedView(): ClearedVaultView {
    return {
        items: [],
        selected: null,
        metadata: null,
        revealed: false,
        secrets: hiddenSecrets(),
        conflicts: [],
        tags: []
    }
}
