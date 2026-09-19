<script lang="ts">
    /**
     * The long-term memory list, with its filters.
     *
     * The list is a pure view: the page owns the query and the data, so filtering,
     * sorting, and every round trip stay in one place. Every row shows what the entry
     * is, where it belongs, how it was learned, how sure the model was, and when it
     * was last used, because a memory entry is only trustworthy if its provenance is
     * visible.
     */
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"

    import type { FactQuery, FactView, MemoryCategory, MemoryScope } from "@/lib/memory-model"
    import {
        MEMORY_CATEGORIES,
        MEMORY_SCOPES,
        categoryLabelKey,
        scopeLabelKey,
        sourceLabelKey
    } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let facts: FactView[] = []
    export let query: FactQuery
    export let selectedId: string | null = null
    export let busy = false
    export let loading = false
    /** The conversation titles a fact came from, so provenance can be named. */
    export let conversationTitles: Record<string, string> = {}

    const dispatch = createEventDispatcher<{
        query: FactQuery
        search: string
        create: void
        select: string
        pin: FactView
        disable: FactView
        remove: string
        restore: string
        purge: string
    }>()

    $: t = (key: string) => translate($translations, key)

    /** The row that is waiting for its permanent-delete confirmation. */
    let confirmingId: string | null = null

    /** `null` means "every scope" and is the first option of each filter. */
    function onScope(event: Event) {
        const value = (event.currentTarget as HTMLSelectElement).value
        dispatch("query", { ...query, scope: value === "" ? null : (value as MemoryScope) })
    }

    function onCategory(event: Event) {
        const value = (event.currentTarget as HTMLSelectElement).value
        dispatch("query", { ...query, category: value === "" ? null : (value as MemoryCategory) })
    }

    function toggleDeleted() {
        dispatch("query", { ...query, include_deleted: !query.include_deleted })
    }

    function onSearch(event: Event) {
        dispatch("search", (event.currentTarget as HTMLInputElement).value)
    }

    function lastUsed(fact: FactView): string {
        return fact.last_used_at ? `${t('memory-fact-last-used')} · ${fact.last_used_at}` : t('memory-fact-never-used')
    }

    function provenance(fact: FactView): string {
        if (fact.source_conversation_id) {
            const title = conversationTitles[fact.source_conversation_id]
            return title
                ? `${t('memory-fact-source-conversation')} · ${title}`
                : t('memory-fact-source-conversation')
        }
        return t(sourceLabelKey(fact.source))
    }
</script>

<div class="fact-panel">
    <div class="panel-head">
        <span class="panel-title">{t('memory-facts')}</span>
        <span class="count">{facts.length}</span>
        <Button size="xs" color="lime" uppercase disabled={busy} on:click={() => dispatch('create')}>
            {t('memory-fact-new')}
        </Button>
    </div>

    <input
        class="search-input"
        type="text"
        placeholder={t('memory-search')}
        value={query.search}
        on:input={onSearch}
    />

    <div class="filters">
        <label class="filter">
            <Text size="xs" color="gray">{t('memory-filter-scope')}</Text>
            <select value={query.scope ?? ""} on:change={onScope}>
                <option value="">{t('memory-filter-all')}</option>
                {#each MEMORY_SCOPES as scope}
                    <option value={scope}>{t(scopeLabelKey(scope))}</option>
                {/each}
            </select>
        </label>

        <label class="filter">
            <Text size="xs" color="gray">{t('memory-filter-category')}</Text>
            <select value={query.category ?? ""} on:change={onCategory}>
                <option value="">{t('memory-filter-all')}</option>
                {#each MEMORY_CATEGORIES as category}
                    <option value={category}>{t(categoryLabelKey(category))}</option>
                {/each}
            </select>
        </label>

        <button class="chip" class:active={query.include_deleted} on:click={toggleDeleted}>
            {t('memory-filter-trash')}
        </button>
    </div>

    {#if loading}
        <div class="empty"><Text size="sm" color="gray">{t('memory-facts')}</Text></div>
    {:else if facts.length === 0}
        <div class="empty"><Text size="sm" color="gray">{t('memory-fact-none')}</Text></div>
    {:else}
        <ul class="facts">
            {#each facts as fact (fact.id)}
                <li class="fact" class:selected={fact.id === selectedId} class:trashed={fact.deleted_at !== null}>
                    <button class="fact-main" on:click={() => dispatch('select', fact.id)}>
                        <div class="fact-text">{fact.content}</div>

                        <div class="chips-row">
                            <span class="tag scope">{t(scopeLabelKey(fact.scope))}</span>
                            <span class="tag">{t(categoryLabelKey(fact.category))}</span>
                            <span class="tag">{t(sourceLabelKey(fact.source))}</span>
                            {#if fact.pinned}<span class="tag pin">{t('memory-fact-pinned')}</span>{/if}
                            {#if fact.disabled}<span class="tag off">{t('memory-fact-disabled')}</span>{/if}
                            {#if fact.deleted_at}<span class="tag trash">{t('memory-filter-trash')}</span>{/if}
                        </div>

                        <div class="meta">
                            <span>{t('memory-candidate-confidence')} {Math.round(fact.confidence * 100)}%</span>
                            <span>{lastUsed(fact)}</span>
                        </div>

                        <div class="provenance">{provenance(fact)}</div>

                        {#if fact.instruction_like}
                            <div class="instruction">{t('memory-instruction-warning')}</div>
                        {/if}
                        {#if fact.disabled}
                            <div class="disabled-hint">{t('memory-fact-disabled-hint')}</div>
                        {/if}
                    </button>

                    <div class="fact-actions">
                        <button class="mini" on:click|stopPropagation={() => dispatch('pin', fact)}>
                            {fact.pinned ? t('memory-restore') : t('memory-fact-pinned')}
                        </button>
                        <button class="mini" on:click|stopPropagation={() => dispatch('disable', fact)}>
                            {fact.disabled ? t('memory-fact-disabled-hint') : t('memory-fact-disabled')}
                        </button>
                        {#if fact.deleted_at}
                            <button class="mini" on:click|stopPropagation={() => dispatch('restore', fact.id)}>
                                {t('memory-restore')}
                            </button>
                            {#if confirmingId === fact.id}
                                <span class="confirm">{t('memory-confirm-fact')}</span>
                                <button
                                    class="mini danger"
                                    on:click|stopPropagation={() => {
                                        confirmingId = null
                                        dispatch('purge', fact.id)
                                    }}
                                >
                                    {t('memory-purge')}
                                </button>
                                <button class="mini" on:click|stopPropagation={() => (confirmingId = null)}>
                                    {t('memory-secret-cancel')}
                                </button>
                            {:else}
                                <button class="mini danger" on:click|stopPropagation={() => (confirmingId = fact.id)}>
                                    {t('memory-purge')}
                                </button>
                            {/if}
                        {:else}
                            <button class="mini danger" on:click|stopPropagation={() => dispatch('remove', fact.id)}>
                                {t('memory-delete')}
                            </button>
                        {/if}
                    </div>
                </li>
            {/each}
        </ul>
    {/if}
</div>

<style lang="scss">
    .fact-panel {
        display: flex;
        flex-direction: column;
        gap: 6px;
        min-width: 0;
    }

    .panel-head {
        display: flex;
        align-items: center;
        gap: 6px;
    }

    .panel-title {
        color: #ffffff;
        font-size: 0.7rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .count {
        background: rgba(82, 254, 254, 0.15);
        border-radius: 8px;
        color: #52fefe;
        font-size: 0.6rem;
        margin-right: auto;
        padding: 1px 7px;
    }

    .search-input {
        width: 100%;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.72rem;
        padding: 7px 8px;

        &::placeholder {
            color: rgba(255, 255, 255, 0.35);
        }

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .filters {
        display: flex;
        align-items: flex-end;
        flex-wrap: wrap;
        gap: 6px;
    }

    .filter {
        display: flex;
        flex-direction: column;
        gap: 2px;
        flex: 1;
        min-width: 120px;

        select {
            width: 100%;
            background: rgba(10, 18, 22, 0.75);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 6px;
            color: rgba(255, 255, 255, 0.85);
            font-size: 0.65rem;
            padding: 5px 6px;

            &:focus {
                outline: none;
                border-color: rgba(82, 254, 254, 0.5);
            }
        }
    }

    .chip {
        background: rgba(35, 50, 55, 0.7);
        border: 1px solid transparent;
        border-radius: 12px;
        color: rgba(255, 255, 255, 0.7);
        cursor: pointer;
        font-size: 0.62rem;
        padding: 4px 10px;

        &.active {
            background: rgba(82, 254, 254, 0.18);
            border-color: rgba(82, 254, 254, 0.45);
            color: #52fefe;
        }
    }

    .facts {
        display: flex;
        flex-direction: column;
        gap: 5px;
        list-style: none;
        margin: 0;
        max-height: 460px;
        overflow-y: auto;
        padding: 0;
    }

    .fact {
        background: rgba(20, 30, 35, 0.6);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 8px;
        display: flex;
        flex-direction: column;

        &.selected {
            border-color: rgba(82, 254, 254, 0.45);
            background: rgba(82, 254, 254, 0.08);
        }

        &.trashed {
            opacity: 0.7;
        }
    }

    .fact-main {
        background: transparent;
        border: none;
        color: inherit;
        cursor: pointer;
        min-width: 0;
        padding: 8px 10px;
        text-align: left;
    }

    .fact-text {
        color: #ffffff;
        font-size: 0.75rem;
        line-height: 1.35;
        word-break: break-word;
    }

    .chips-row {
        display: flex;
        flex-wrap: wrap;
        gap: 3px;
        margin-top: 5px;
    }

    .tag {
        background: rgba(82, 254, 254, 0.12);
        border-radius: 8px;
        color: rgba(82, 254, 254, 0.85);
        font-size: 0.56rem;
        padding: 0 6px;

        &.scope {
            background: rgba(140, 120, 255, 0.15);
            color: rgba(190, 180, 255, 0.9);
        }

        &.pin {
            background: rgba(255, 214, 102, 0.15);
            color: rgba(255, 214, 102, 0.9);
        }

        &.off {
            background: rgba(255, 255, 255, 0.12);
            color: rgba(255, 255, 255, 0.6);
        }

        &.trash {
            background: rgba(255, 107, 107, 0.15);
            color: rgba(255, 150, 150, 0.9);
        }
    }

    .meta {
        color: rgba(255, 255, 255, 0.5);
        display: flex;
        flex-wrap: wrap;
        font-size: 0.6rem;
        gap: 8px;
        margin-top: 4px;
    }

    .provenance {
        color: rgba(255, 255, 255, 0.42);
        font-size: 0.58rem;
        margin-top: 2px;
    }

    .instruction {
        color: #ffb347;
        font-size: 0.6rem;
        margin-top: 4px;
    }

    .disabled-hint {
        color: rgba(255, 255, 255, 0.42);
        font-size: 0.58rem;
        margin-top: 2px;
    }

    .fact-actions {
        border-top: 1px dashed rgba(255, 255, 255, 0.1);
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        padding: 4px 10px 6px;
    }

    .mini {
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.55);
        cursor: pointer;
        font-size: 0.58rem;
        letter-spacing: 0.03em;
        padding: 0;
        text-transform: uppercase;

        &:hover {
            color: #52fefe;
        }

        &.danger:hover {
            color: #ff6b6b;
        }
    }

    .confirm {
        color: #ffb347;
        font-size: 0.58rem;
    }

    .empty {
        padding: 22px 0;
        text-align: center;
    }
</style>
