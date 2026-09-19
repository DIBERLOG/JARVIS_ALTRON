/**
 * The catalogue the «Команды» page is built from.
 *
 * What crosses this boundary is the core's own answer, and it carries no path, no
 * executable, no argument and no secret: a pack is named by its logical name. The
 * page reads it, filters it in memory, and writes nothing anywhere — no storage,
 * no URL, no console. Checking a phrase goes through the same voice host matcher
 * the microphone uses and is the only thing the page can send.
 */
import { invoke } from "@tauri-apps/api/core"

/** A slot a command declares, by name and by the entity it expects. */
export interface CatalogSlot {
    name: string
    entity: string
}

/** One card. */
export interface CatalogEntry {
    id: string
    /** The logical name of the pack, never a path. */
    pack: string
    /** One of the catalogue's categories. */
    category: string
    /** `pack` for a command the loader read, `settings` for the global voice input. */
    source: string
    description: string
    phrases: string[]
    slots: CatalogSlot[]
    /** `safe`, `confirm` or `forbidden`, from the core's `RiskLevel`. */
    risk_level: string
    requires_confirmation: boolean
    enabled: boolean
    /** `no_phrases`, `executable_missing`, `script_missing`, `unsupported_type`. */
    unavailable_reason: string | null
}

/** A pack the loader could not read, by logical name. */
export interface UnreadablePack {
    pack: string
    reason: string
}

export interface CommandCatalog {
    language: string
    categories: string[]
    entries: CatalogEntry[]
    unreadable: UnreadablePack[]
}

export const commandCatalogApi = {
    /** The installed catalogue, in the language the window is shown in. */
    load: () => invoke<CommandCatalog>("command_catalog")
}

/** The Fluent key of a category. */
export function categoryKey(category: string): string {
    return `command-category-${category}`
}

/** The Fluent key of a risk level. */
export function riskKey(level: string): string {
    return `command-risk-${level}`
}

/** The Fluent key of a reason a command cannot run. */
export function unavailableKey(reason: string | null | undefined): string {
    return `command-unavailable-${reason ?? "unknown"}`
}

/** The Fluent key of a reason a pack was not read. */
export function packReasonKey(reason: string): string {
    return `command-pack-reason-${reason}`
}

/**
 * The text a search is compared against, folded the way the matcher folds a
 * phrase: lower case and `ё` written as `е`, so searching for "королеве" finds
 * "Королёве".
 */
export function foldForSearch(text: string): string {
    return text.toLowerCase().replace(/ё/g, "е").trim()
}

/**
 * Whether one card answers a search. The person may type a phrase they say, a
 * command identifier, the name of a pack, a slot name, or a word of the
 * description: all of them are what is written on the card.
 */
export function matchesQuery(entry: CatalogEntry, query: string): boolean {
    const needle = foldForSearch(query)
    if (!needle) return true
    const haystack = [
        entry.id,
        entry.pack,
        entry.description,
        ...entry.phrases,
        ...entry.slots.map((slot) => slot.name)
    ]
        .map(foldForSearch)
        .join(" \u0000 ")
    return haystack.includes(needle)
}

/** The cards of one category, or all of them for an empty category. */
export function filterEntries(
    entries: CatalogEntry[],
    category: string,
    query: string
): CatalogEntry[] {
    return entries.filter(
        (entry) =>
            (category === "" || entry.category === category) && matchesQuery(entry, query)
    )
}
