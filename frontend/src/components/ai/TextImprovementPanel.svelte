<script lang="ts">
    /**
     * The AI text improvement: choose a mode, read the difference, then decide.
     *
     * This component exists to make three things impossible:
     *
     * * **an unseen change.** The proposal is never applied on the way back: the panel
     *   shows the whole difference and only the user's confirmation dispatches `apply`;
     * * **a silent rewrite.** A change that touches a large part of the text, or a mode
     *   that rewrites rather than corrects, is labelled as a rewrite, and the counts are
     *   always visible;
     * * **a leak.** The text is sent by the backend only, through the local gateway, after
     *   the secret filter has seen it; this component never talks to a model and never
     *   stores anything.
     */
    import { createEventDispatcher } from "svelte"
    import { Alert, Button, Text } from "@svelteuidev/core"
    import { Check, Cross2, MagicWand } from "radix-icons-svelte"

    import { translate, translations } from "@/stores"
    import type { ImprovementMode, TextImprovementPreview } from "@/lib/autocorrect-model"
    import {
        IMPROVEMENT_MODES,
        improvementModeLabelKey,
        improvementWarningLabelKey,
        instructionIsUsable,
        isPreviewApplicable,
        previewIsRewrite,
        previewStatsLine,
        textTooLongForImprovement
    } from "@/lib/autocorrect-model"

    export let preview: TextImprovementPreview | null = null
    export let text = ""
    export let busy = false
    export let actionError = ""

    const dispatch = createEventDispatcher<{
        request: { mode: ImprovementMode; instruction: string }
        apply: void
        cancel: void
        close: void
    }>()

    $: t = (key: string) => translate($translations, key)
    $: modes = IMPROVEMENT_MODES
    $: usable = instructionIsUsable(mode, instruction)
    $: tooLong = textTooLongForImprovement(text)
    $: applicable = isPreviewApplicable(preview)
    $: rewrite = previewIsRewrite(preview)

    let mode: ImprovementMode = "correct_spelling"
    let instruction = ""

    function request() {
        if (!usable || tooLong || busy) return
        dispatch("request", { mode, instruction })
    }
</script>

<div class="improve">
    <div class="improve-bar">
        <span class="improve-title">
            <MagicWand size={12} />
            {t('autocorrect-improve-title')}
        </span>
        <span class="improve-spacer" />
        <button class="improve-close" on:click={() => dispatch('close')} title={t('autocorrect-improve-close')}>
            <Cross2 size={12} />
        </button>
    </div>

    <Text size="xs" color="gray">{t('autocorrect-improve-hint')}</Text>

    <div class="improve-modes">
        {#each modes as entry}
            <button
                class="improve-mode"
                class:active={mode === entry}
                on:click={() => (mode = entry)}
                disabled={busy}
            >
                {t(improvementModeLabelKey(entry))}
            </button>
        {/each}
    </div>

    {#if mode === "custom_instruction"}
        <input
            class="improve-input"
            type="text"
            placeholder={t('autocorrect-improve-instruction')}
            bind:value={instruction}
            disabled={busy}
        />
    {/if}

    {#if tooLong}
        <Text size="xs" color="red">{t('autocorrect-improve-too-long')}</Text>
    {/if}

    {#if actionError}
        <Text size="xs" color="red">{actionError}</Text>
    {/if}

    <div class="improve-row">
        {#if busy}
            <Button size="xs" color="red" uppercase on:click={() => dispatch('cancel')}>
                {t('autocorrect-improve-stop')}
            </Button>
        {:else}
            <Button size="xs" color="cyan" uppercase on:click={request} disabled={!usable || tooLong}>
                {t('autocorrect-improve-run')}
            </Button>
        {/if}
    </div>

    {#if preview}
        <div class="improve-preview">
            <div class="improve-preview-bar">
                <span class="improve-stats">{previewStatsLine(preview)}</span>
                {#if rewrite}
                    <span class="improve-flag">{t('autocorrect-improve-rewrite')}</span>
                {/if}
                {#if preview.model}
                    <span class="improve-model">{preview.model}</span>
                {/if}
            </div>

            {#each preview.warnings as warning}
                <Text size="xs" color="gray">{t(improvementWarningLabelKey(warning))}</Text>
            {/each}

            <Text size="xs" color="gray">{t('autocorrect-improve-preview-title')}</Text>
            <div class="improve-diff">
                {#each preview.diff.segments as segment}
                    <span class="segment {segment.kind}">{segment.text}</span>
                {/each}
            </div>

            <div class="improve-row">
                <Button size="xs" color="lime" uppercase on:click={() => dispatch('apply')} disabled={!applicable || busy}>
                    <Check size={12} />
                    {t('autocorrect-improve-apply')}
                </Button>
                <Text size="xs" color="gray">{t('autocorrect-improve-apply-hint')}</Text>
            </div>
        </div>
    {/if}
</div>

<style lang="scss">
    .improve {
        display: flex;
        flex-direction: column;
        gap: 5px;
        padding: 7px 8px;
        background: rgba(15, 24, 30, 0.75);
        border: 1px solid rgba(82, 254, 254, 0.18);
        border-radius: 8px;
    }

    .improve-bar {
        display: flex;
        align-items: center;
        gap: 6px;
    }

    .improve-title {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        font-size: 0.62rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: rgba(82, 254, 254, 0.85);
    }

    .improve-spacer {
        flex: 1;
    }

    .improve-close {
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.5);
        cursor: pointer;

        &:hover {
            color: #ffffff;
        }
    }

    .improve-modes {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
    }

    .improve-mode {
        background: transparent;
        border: 1px solid rgba(255, 255, 255, 0.15);
        border-radius: 5px;
        color: rgba(255, 255, 255, 0.75);
        cursor: pointer;
        font-size: 0.64rem;
        padding: 2px 6px;

        &.active {
            background: rgba(82, 254, 254, 0.16);
            border-color: rgba(82, 254, 254, 0.45);
            color: #d8ffff;
        }

        &:disabled {
            opacity: 0.5;
        }
    }

    .improve-input {
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.68rem;
        padding: 5px 7px;
    }

    .improve-row {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-wrap: wrap;
    }

    .improve-preview {
        display: flex;
        flex-direction: column;
        gap: 4px;
        border-top: 1px solid rgba(255, 255, 255, 0.08);
        padding-top: 5px;
    }

    .improve-preview-bar {
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .improve-stats {
        font-family: monospace;
        font-size: 0.64rem;
        color: rgba(138, 200, 50, 0.95);
    }

    .improve-flag {
        font-size: 0.58rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: rgba(255, 214, 102, 0.9);
    }

    .improve-model {
        font-size: 0.58rem;
        color: rgba(255, 255, 255, 0.4);
    }

    .improve-diff {
        max-height: 220px;
        overflow: auto;
        background: rgba(8, 14, 17, 0.8);
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 6px;
        color: rgba(255, 255, 255, 0.85);
        font-size: 0.68rem;
        line-height: 1.5;
        padding: 6px 7px;
        white-space: pre-wrap;
        word-break: break-word;
    }

    .segment {
        &.added {
            background: rgba(138, 200, 50, 0.22);
            color: #e8ffd0;
        }

        &.removed {
            background: rgba(255, 107, 107, 0.18);
            color: #ffd8d8;
            text-decoration: line-through;
        }
    }
</style>
