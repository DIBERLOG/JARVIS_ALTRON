/**
 * Typed wrappers around the autocorrect commands.
 *
 * The interface never reads SQLite, never receives a key, and never decrypts a word: it
 * sends text, receives issues, suggestions, and previews, and asks for a change to be
 * applied. Everything heavy (dictionary parsing, checking, encryption, the model) happens
 * in Rust.
 *
 * Two properties of this module are deliberate:
 *
 * * the AI path is split in two calls — `improveText` only *produces* a preview, and
 *   `applyImprovement` is a separate, explicit step the user has to confirm;
 * * nothing is persisted in the browser.
 */

import { invoke } from "@tauri-apps/api/core"

import type {
    AutocorrectSettings,
    AutocorrectStatusView,
    CheckOrigin,
    CheckView,
    Correction,
    CorrectionBatch,
    DictionaryExportResult,
    DictionaryState,
    ImportOutcome,
    ImprovementMode,
    Language,
    LanguageMode,
    Suggestion,
    TextImprovementPreview,
    UndoOutcome,
    UndoStatus,
    UserDictionaryEntry,
    UserDictionaryStats,
    WordExportResult
} from "./autocorrect-model"

/** Hard cap used when a caller asks for "everything" from the word list. */
export const WORD_LIST_PAGE_LIMIT = 200

export interface WordQuery {
    search: string
    language: Language | null
    imported_only: boolean
    offset: number
    limit: number
}

export interface ImproveTextRequest {
    text: string
    mode: ImprovementMode
    instruction?: string | null
    persona?: "jarvis" | "altron"
    language?: LanguageMode
}

export const autocorrectApi = {
    // status and settings
    status: () => invoke<AutocorrectStatusView>("autocorrect_status"),
    getSettings: () => invoke<AutocorrectSettings>("autocorrect_get_settings"),
    updateSettings: (settings: AutocorrectSettings) =>
        invoke<AutocorrectStatusView>("autocorrect_update_settings", { settings }),
    reloadDictionaries: () => invoke<AutocorrectStatusView>("autocorrect_reload_dictionaries"),

    // checking
    /**
     * Checks a text.
     *
     * `auto_corrections` is what safe auto-correction *would* do; it is empty unless the
     * user switched safe auto-correction on, and applying it is still a separate call.
     */
    check: (origin: CheckOrigin, text: string) =>
        invoke<CheckView>("autocorrect_check", { origin, text }),
    suggest: (word: string) => invoke<Suggestion[]>("autocorrect_suggest", { word }),

    // corrections
    /**
     * Applies the corrections the user accepted.
     *
     * `expectedVersion` is the version the check ran on: the backend refuses a stale
     * request instead of replacing text that moved.
     */
    apply: (scope: string, text: string, expectedVersion: string | null, corrections: Correction[]) =>
        invoke<CorrectionBatch>("autocorrect_apply", { scope, text, expectedVersion, corrections }),
    undo: (scope: string, text: string) =>
        invoke<UndoOutcome>("autocorrect_undo", { scope, text }),
    undoStatus: (scope: string) => invoke<UndoStatus>("autocorrect_undo_status", { scope }),

    // the user's own words
    listWords: (query: WordQuery) =>
        invoke<UserDictionaryEntry[]>("autocorrect_dictionary_list", { query }),
    addWord: (word: string, language: Language) =>
        invoke<UserDictionaryEntry>("autocorrect_dictionary_add", { word, language }),
    removeWord: (id: string) => invoke<void>("autocorrect_dictionary_remove", { id }),
    /** Silences a word for this session only; nothing is stored. */
    ignoreWord: (word: string) => invoke<void>("autocorrect_dictionary_ignore", { word }),
    unignoreWord: (word: string) => invoke<boolean>("autocorrect_dictionary_unignore", { word }),
    ignoredWords: () => invoke<string[]>("autocorrect_dictionary_ignored"),
    wordStats: () => invoke<UserDictionaryStats>("autocorrect_dictionary_stats"),
    importWordsFile: (language: Language) =>
        invoke<ImportOutcome>("autocorrect_dictionary_import_file", { language }),
    /** Writes an unprotected text file; the interface must confirm first. */
    exportWordsFile: (confirmed: boolean) =>
        invoke<WordExportResult>("autocorrect_dictionary_export_file", { confirmed }),
    exportWordsBackup: () =>
        invoke<DictionaryExportResult>("autocorrect_dictionary_export_backup"),
    importWordsBackup: () => invoke<ImportOutcome>("autocorrect_dictionary_import_backup"),

    // the explicit AI improvement
    /**
     * Produces a preview. Nothing is applied by this call.
     *
     * The text is checked by the secret filter before it is sent, and the answer comes
     * back as a difference the user has to confirm.
     */
    improveText: (request: ImproveTextRequest) =>
        invoke<TextImprovementPreview>("autocorrect_improve_text", { request }),
    cancelImprovement: () => invoke<boolean>("autocorrect_cancel_improvement"),
    /** Applies a preview the user confirmed, and journals it for undo. */
    applyImprovement: (
        scope: string,
        text: string,
        preview: TextImprovementPreview,
        expectedVersion: string | null
    ) =>
        invoke<CorrectionBatch>("autocorrect_apply_improvement", {
            scope,
            text,
            preview,
            expectedVersion
        }),

    // user replacement rules
    addRule: (pattern: string, replacement: string, autoApply: boolean) =>
        invoke<AutocorrectSettings>("autocorrect_rule_add", { pattern, replacement, autoApply }),
    removeRule: (index: number) =>
        invoke<AutocorrectSettings>("autocorrect_rule_remove", { index })
}

/** The words of one language, for the dictionary manager. */
export function wordQuery(search: string, language: Language | null): WordQuery {
    return {
        search,
        language,
        imported_only: false,
        offset: 0,
        limit: WORD_LIST_PAGE_LIMIT
    }
}

/** The dictionary state of one language, or `null` when nothing is known. */
export function dictionaryFor(
    dictionaries: DictionaryState[],
    language: Language
): DictionaryState | null {
    return dictionaries.find((entry) => entry.language === language) ?? null
}
