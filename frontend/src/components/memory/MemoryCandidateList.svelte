<script lang="ts">
    /**
     * Suggested memory that is waiting for a decision.
     *
     * Nothing here is memory yet: a candidate becomes an approved fact only after the
     * user picks its scope and approves it. The scope selector defaults to the profile
     * the candidate came from, and the edited text is sent with the approval so the
     * user's wording wins.
     */
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"

    import type { FactDraft, FactView, MemoryCategory, MemoryScope } from "@/lib/memory-model"
    import {
        MEMORY_CATEGORIES,
        MEMORY_SCOPES,
        categoryLabelKey,
        scopeLabelKey,
        sourceLabelKey
    } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let candidates: FactView[] = []
    /** The scope a candidate of this list belongs to by default. */
    export let defaultScope: MemoryScope = "global"
    export let busy = false
    export let loading = false

    const dispatch = createEventDispatcher<{
        approve: { id: string; draft: FactDraft }
        reject: string
    }>()

    $: t = (key: string) => translate($translations, key)

    /** One scope choice, one category, and one edited text per candidate. */
    let scopes: Record<string, MemoryScope> = {}
    let texts: Record<string, string> = {}
    let categories: Record<string, MemoryCategory> = {}

    /** Fills the editors of candidates that were not seen before. */
    function prepare(rows: FactView[]) {
        let nextScopes = scopes
        let nextTexts = texts
        let nextCategories = categories
        for (const candidate of rows) {
            if (candidate.id in nextScopes) continue
            nextScopes = { ...nextScopes, [candidate.id]: defaultScope }
            nextTexts = { ...nextTexts, [candidate.id]: candidate.content }
            nextCategories = { ...nextCategories, [candidate.id]: candidate.category }
        }
        if (nextScopes !== scopes) scopes = nextScopes
        if (nextTexts !== texts) texts = nextTexts
        if (nextCategories !== categories) categories = nextCategories
    }

    $: prepare(candidates)

    function setScope(id: string, event: Event) {
        scopes = { ...scopes, [id]: (event.currentTarget as HTMLSelectElement).value as MemoryScope }
    }

    function setCategory(id: string, event: Event) {
        categories = { ...categories, [id]: (event.currentTarget as HTMLSelectElement).value as MemoryCategory }
    }

    function setText(id: string, event: Event) {
        texts = { ...texts, [id]: (event.currentTarget as HTMLTextAreaElement).value }
    }

    function approve(candidate: FactView) {
        const content = (texts[candidate.id] ?? candidate.content).trim()
        if (content.length === 0) return
        dispatch("approve", {
            id: candidate.id,
            draft: {
                scope: scopes[candidate.id] ?? defaultScope,
                category: categories[candidate.id] ?? candidate.category,
                content,
                pinned: candidate.pinned,
                disabled: false,
                // The page sets this only after an explicit confirmation.
                accept_secret_warning: false
            }
        })
    }
</script>

<div class="candidates">
    <div class="panel-head">
        <span class="panel-title">{t('memory-candidates')}</span>
        <span class="count">{candidates.length}</span>
    </div>

    <Text size="xs" color="gray">{t('memory-candidate-scope-hint')}</Text>

    {#if loading}
        <div class="empty"><Text size="sm" color="gray">{t('memory-candidates-none')}</Text></div>
    {:else if candidates.length === 0}
        <div class="empty"><Text size="sm" color="gray">{t('memory-candidates-none')}</Text></div>
    {:else}
        <ul class="rows">
            {#each candidates as candidate (candidate.id)}
                <li class="candidate">
                    <div class="meta">
                        <span class="tag pending">{t('memory-candidate-pending')}</span>
                        <span>{t('memory-candidate-confidence')} {Math.round(candidate.confidence * 100)}%</span>
                        <span>{t(sourceLabelKey(candidate.source))}</span>
                    </div>

                    <textarea
                        class="text"
                        spellcheck="false"
                        value={texts[candidate.id] ?? candidate.content}
                        on:input={(event) => setText(candidate.id, event)}
                    ></textarea>

                    <div class="selects">
                        <label class="field">
                            <Text size="xs" color="gray">{t('memory-fact-scope')}</Text>
                            <select
                                value={scopes[candidate.id] ?? defaultScope}
                                on:change={(event) => setScope(candidate.id, event)}
                            >
                                {#each MEMORY_SCOPES as option}
                                    <option value={option}>{t(scopeLabelKey(option))}</option>
                                {/each}
                            </select>
                        </label>

                        <label class="field">
                            <Text size="xs" color="gray">{t('memory-fact-category')}</Text>
                            <select
                                value={categories[candidate.id] ?? candidate.category}
                                on:change={(event) => setCategory(candidate.id, event)}
                            >
                                {#each MEMORY_CATEGORIES as option}
                                    <option value={option}>{t(categoryLabelKey(option))}</option>
                                {/each}
                            </select>
                        </label>
                    </div>

                    {#if candidate.instruction_like}
                        <div class="instruction">{t('memory-instruction-warning')}</div>
                    {/if}

                    <div class="actions">
                        <Button size="xs" color="lime" uppercase disabled={busy} on:click={() => approve(candidate)}>
                            {t('memory-candidate-approve')}
                        </Button>
                        <Button size="xs" color="gray" uppercase disabled={busy} on:click={() => dispatch('reject', candidate.id)}>
                            {t('memory-candidate-reject')}
                        </Button>
                    </div>
                </li>
            {/each}
        </ul>
    {/if}
</div>

<style lang="scss">
    .candidates {
        background: rgba(20, 30, 35, 0.5);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 10px;
    }

    .panel-head {
        align-items: center;
        display: flex;
        gap: 6px;
    }

    .panel-title {
        color: #ffffff;
        font-size: 0.7rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .count {
        background: rgba(255, 179, 71, 0.18);
        border-radius: 8px;
        color: #ffb347;
        font-size: 0.6rem;
        padding: 1px 7px;
    }

    .rows {
        display: flex;
        flex-direction: column;
        gap: 6px;
        list-style: none;
        margin: 0;
        padding: 0;
    }

    .candidate {
        background: rgba(15, 22, 26, 0.65);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 8px;
        display: flex;
        flex-direction: column;
        gap: 5px;
        padding: 8px;
    }

    .meta {
        align-items: center;
        color: rgba(255, 255, 255, 0.5);
        display: flex;
        flex-wrap: wrap;
        font-size: 0.58rem;
        gap: 8px;
    }

    .tag {
        background: rgba(255, 179, 71, 0.15);
        border-radius: 8px;
        color: rgba(255, 179, 71, 0.9);
        padding: 0 6px;
    }

    .text {
        background: rgba(10, 18, 22, 0.8);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.7rem;
        min-height: 62px;
        padding: 7px 8px;
        resize: vertical;
        width: 100%;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .selects {
        display: grid;
        gap: 6px;
        grid-template-columns: 1fr 1fr;
    }

    .field {
        display: flex;
        flex-direction: column;
        gap: 2px;

        select {
            background: rgba(10, 18, 22, 0.75);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 6px;
            color: rgba(255, 255, 255, 0.85);
            font-size: 0.65rem;
            padding: 5px 6px;
            width: 100%;

            &:focus {
                outline: none;
                border-color: rgba(82, 254, 254, 0.5);
            }
        }
    }

    .instruction {
        color: #ffb347;
        font-size: 0.6rem;
    }

    .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }

    .empty {
        padding: 16px 0;
        text-align: center;
    }
</style>
