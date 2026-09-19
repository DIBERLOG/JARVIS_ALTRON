<script lang="ts">
    /**
     * The spelling panel: what a check found, and the one-click fixes for it.
     *
     * The panel never edits text itself. Every action it offers is a request: "apply this
     * suggestion", "remember this word", "silence it for now", "undo the last change".
     * The text changes in Rust under a version guard, and the answer comes back as a
     * batch the route stores.
     *
     * The two rules the panel encodes:
     *
     * * a suggestion is always applied through the range the backend reported, so a
     *   highlight cannot drift;
     * * an unknown word is never presented as a mistake to fix blindly: it is a question
     *   with "add to dictionary" and "ignore" next to it.
     */
    import { createEventDispatcher } from "svelte"
    import { Alert, Button, Text } from "@svelteuidev/core"
    import { Check, ExclamationTriangle, MagnifyingGlass, Update } from "radix-icons-svelte"

    import { translate, translations } from "@/stores"
    import type {
        AutocorrectStatus,
        CheckReport,
        Correction,
        Language,
        SpellingIssue,
        UndoStatus
    } from "@/lib/autocorrect-model"
    import {
        VISIBLE_ISSUE_LIMIT,
        issueIsOneClickFix,
        languageOfWord,
        reasonLabelKey,
        reportSummaryKey,
        sourceLabelKey
    } from "@/lib/autocorrect-model"

    export let report: CheckReport | null = null
    export let status: AutocorrectStatus | null = null
    export let undo: UndoStatus | null = null
    export let currentText = ""
    export let checkedText = ""
    export let busy = false
    export let actionError = ""
    /** Whether this editor may offer the AI improvement at all. */
    export let allowImprove = true

    const dispatch = createEventDispatcher<{
        apply: Correction[]
        ignore: string
        add: { word: string; language: Language }
        undo: void
        improve: void
        reload: void
    }>()

    $: t = (key: string) => translate($translations, key)
    $: issues = report?.issues ?? []
    $: visible = issues.slice(0, VISIBLE_ISSUE_LIMIT)
    $: hidden = Math.max(0, issues.length - visible.length)
    $: stale = report !== null && checkedText !== currentText
    $: safeFixes = issues.filter(issueIsOneClickFix)
    $: summaryKey = reportSummaryKey(report)

    /** One correction for one issue, exactly as the backend reported the range. */
    function applySuggestion(issue: SpellingIssue, suggestion: string) {
        dispatch("apply", [
            {
                range: issue.range,
                original: issue.word,
                replacement: suggestion,
                reason: issue.reason
            }
        ])
    }

    function applyAllSafe() {
        const corrections: Correction[] = []
        for (const issue of safeFixes) {
            const suggestion = issue.suggestions.find((entry) => entry.text !== issue.word)
            if (!suggestion) continue
            corrections.push({
                range: issue.range,
                original: issue.word,
                replacement: suggestion.text,
                reason: issue.reason
            })
        }
        if (corrections.length > 0) dispatch("apply", corrections)
    }

    function addWord(issue: SpellingIssue) {
        const language = issue.language ?? languageOfWord(issue.word)
        if (!language) return
        dispatch("add", { word: issue.word, language })
    }
</script>

<div class="spelling">
    <div class="spelling-bar">
        <span class="spelling-title">
            <MagnifyingGlass size={12} />
            {t('autocorrect-title')}
        </span>
        <span class="spelling-summary" class:dirty={issues.length > 0}>
            {t(summaryKey)}
            {#if issues.length > 0}
                · {issues.length}
            {/if}
        </span>
        <span class="spelling-spacer" />
        {#if undo && undo.can_undo}
            <Button size="xs" color="gray" variant="subtle" uppercase on:click={() => dispatch('undo')} disabled={busy}>
                <Update size={12} />
                {t('autocorrect-undo')}
            </Button>
        {/if}
        {#if allowImprove}
            <Button size="xs" color="cyan" variant="subtle" uppercase on:click={() => dispatch('improve')} disabled={busy}>
                {t('autocorrect-improve-open')}
            </Button>
        {/if}
    </div>

    {#if report && !report.enabled}
        <Text size="xs" color="gray">{t('autocorrect-disabled')}</Text>
    {/if}

    {#if report && report.unavailable.length > 0}
        <Alert title={t('autocorrect-dictionary-missing-title')} color="orange" variant="outline">
            <Text size="xs" color="gray">
                {t('autocorrect-dictionary-missing-body')}
            </Text>
            {#if status}
                <p class="spelling-path">{status.dictionary_dir}</p>
            {/if}
            <div class="spelling-row">
                <Button size="xs" color="gray" uppercase on:click={() => dispatch('reload')}>
                    {t('autocorrect-reload')}
                </Button>
            </div>
        </Alert>
    {/if}

    {#if stale}
        <Text size="xs" color="gray">{t('autocorrect-stale')}</Text>
    {/if}

    {#if report && report.timed_out}
        <Text size="xs" color="gray">{t('autocorrect-timeout')}</Text>
    {/if}

    {#if actionError}
        <Text size="xs" color="red">{actionError}</Text>
    {/if}

    {#if issues.length === 0 && report}
        <Text size="xs" color="gray">
            <Check size={11} />
            {t('autocorrect-clean')}
        </Text>
    {/if}

    {#each visible as issue, index (index)}
        <div class="spelling-issue">
            <div class="spelling-word">
                <span class="spelling-text">{issue.word}</span>
                <span class="spelling-reason">{t(reasonLabelKey(issue.reason))}</span>
            </div>
            <div class="spelling-suggestions">
                {#each issue.suggestions.filter((entry) => entry.text !== issue.word) as suggestion}
                    <button
                        class="spelling-chip"
                        class:primary={issue.auto_fixable}
                        title={t(sourceLabelKey(suggestion.source))}
                        on:click={() => applySuggestion(issue, suggestion.text)}
                        disabled={busy}
                    >
                        {suggestion.text.length > 0 ? suggestion.text : t('autocorrect-delete')}
                    </button>
                {/each}
                {#if issue.reason === "unknown_word"}
                    <button class="spelling-chip ghost" on:click={() => addWord(issue)} disabled={busy}>
                        {t('autocorrect-add-word')}
                    </button>
                    <button class="spelling-chip ghost" on:click={() => dispatch('ignore', issue.word)} disabled={busy}>
                        {t('autocorrect-ignore')}
                    </button>
                {/if}
            </div>
        </div>
    {/each}

    {#if hidden > 0}
        <Text size="xs" color="gray">
            <ExclamationTriangle size={11} />
            {t('autocorrect-more-issues')} {hidden}
        </Text>
    {/if}

    {#if safeFixes.length > 0}
        <div class="spelling-row">
            <Button size="xs" color="lime" uppercase on:click={applyAllSafe} disabled={busy}>
                {t('autocorrect-apply-safe')} ({safeFixes.length})
            </Button>
        </div>
    {/if}
</div>

<style lang="scss">
    .spelling {
        display: flex;
        flex-direction: column;
        gap: 5px;
        padding: 7px 8px;
        background: rgba(15, 24, 28, 0.7);
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 8px;
    }

    .spelling-bar {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-wrap: wrap;
    }

    .spelling-title {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        font-size: 0.62rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: rgba(82, 254, 254, 0.85);
    }

    .spelling-summary {
        font-size: 0.62rem;
        color: rgba(255, 255, 255, 0.55);

        &.dirty {
            color: rgba(255, 214, 102, 0.9);
        }
    }

    .spelling-spacer {
        flex: 1;
    }

    .spelling-issue {
        display: flex;
        flex-direction: column;
        gap: 3px;
        padding: 4px 0;
        border-top: 1px solid rgba(255, 255, 255, 0.05);
    }

    .spelling-word {
        display: flex;
        align-items: baseline;
        gap: 6px;
    }

    .spelling-text {
        font-family: monospace;
        font-size: 0.72rem;
        color: #ffffff;
        word-break: break-all;
    }

    .spelling-reason {
        font-size: 0.58rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: rgba(255, 255, 255, 0.45);
    }

    .spelling-suggestions {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
    }

    .spelling-chip {
        background: rgba(82, 254, 254, 0.1);
        border: 1px solid rgba(82, 254, 254, 0.25);
        border-radius: 5px;
        color: #d8ffff;
        cursor: pointer;
        font-size: 0.66rem;
        padding: 2px 6px;

        &:hover:not(:disabled) {
            background: rgba(82, 254, 254, 0.22);
        }

        &:disabled {
            cursor: default;
            opacity: 0.5;
        }

        &.primary {
            border-color: rgba(138, 200, 50, 0.4);
            color: #e8ffd0;
        }

        &.ghost {
            background: transparent;
            border-color: rgba(255, 255, 255, 0.15);
            color: rgba(255, 255, 255, 0.7);
        }
    }

    .spelling-row {
        display: flex;
        gap: 6px;
        flex-wrap: wrap;
    }

    .spelling-path {
        font-family: monospace;
        font-size: 0.62rem;
        color: rgba(255, 255, 255, 0.6);
        word-break: break-all;
        margin: 4px 0 2px;
    }
</style>
