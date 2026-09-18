<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Text } from "@svelteuidev/core"
    import { Star, StarFilled, Trash, Update } from "radix-icons-svelte"

    import type { NoteSort, NoteSummary } from "@/lib/notes-model"
    import {
        NOTE_SORTS,
        displayExcerpt,
        displayTitle,
        relativeTime,
        sortOptionKey,
        timeUnitKey
    } from "@/lib/notes-model"
    import { translations, translate } from "@/stores"

    export let items: NoteSummary[] = []
    export let selectedId: string | null = null
    export let sort: NoteSort = "updated_desc"
    export let loading = false

    const dispatch = createEventDispatcher<{
        select: string
        pin: { id: string; pinned: boolean }
        trash: string
        restore: string
        sort: NoteSort
    }>()

    $: t = (key: string) => translate($translations, key)
    $: options = NOTE_SORTS.map((value) => ({ label: t(sortOptionKey(value)), value }))
    $: untitled = t("notes-untitled")
    $: noText = t("notes-no-text")

    let sortValue: string = sort
    $: sortValue = sort

    function timeLabel(iso: string): string {
        const relative = relativeTime(iso)
        if (relative.unit === "now") return t("notes-time-now")
        if (relative.unit === "date") return iso.slice(0, 10)
        return `${relative.value} ${t(timeUnitKey(relative.unit))}`
    }

    function onSortChange(event: Event) {
        const value = (event.currentTarget as HTMLSelectElement).value
        dispatch("sort", value as NoteSort)
    }
</script>

<div class="list-header">
    <select class="sort-select" value={sortValue} on:change={onSortChange}>
        {#each options as option}
            <option value={option.value}>{option.label}</option>
        {/each}
    </select>
</div>

{#if loading}
    <div class="empty"><Text size="sm" color="gray">{t('notes-loading')}</Text></div>
{:else if items.length === 0}
    <div class="empty">
        <Text size="sm" color="gray">{t('notes-empty')}</Text>
    </div>
{:else}
    <ul class="rows">
        {#each items as item (item.id)}
            <li>
                <button
                    class="row"
                    class:active={item.id === selectedId}
                    on:click={() => dispatch('select', item.id)}
                >
                    <div class="row-main">
                        <div class="row-title">
                            {#if item.pinned}
                                <span class="pin"><StarFilled size={11} /></span>
                            {/if}
                            <span class="title-text">{displayTitle(item.title, untitled)}</span>
                        </div>
                        <div class="row-excerpt">{displayExcerpt(item.excerpt, noText)}</div>
                        <div class="row-meta">
                            <span class="when"><Update size={10} /> {timeLabel(item.updated_at)}</span>
                            {#if item.tags.length > 0}
                                <span class="tags">
                                    {#each item.tags.slice(0, 3) as tag}
                                        <span class="tag">{tag}</span>
                                    {/each}
                                </span>
                            {/if}
                        </div>
                    </div>
                </button>

                <div class="row-actions">
                    <button
                        class="mini"
                        title={item.pinned ? t('notes-unpin') : t('notes-pin')}
                        on:click|stopPropagation={() => dispatch('pin', { id: item.id, pinned: !item.pinned })}
                    >
                        {#if item.pinned}
                            <StarFilled size={12} />
                        {:else}
                            <Star size={12} />
                        {/if}
                    </button>
                    {#if item.deleted_at}
                        <button
                            class="mini"
                            title={t('notes-restore')}
                            on:click|stopPropagation={() => dispatch('restore', item.id)}
                        >
                            <Update size={12} />
                        </button>
                    {:else}
                        <button
                            class="mini danger"
                            title={t('notes-trash')}
                            on:click|stopPropagation={() => dispatch('trash', item.id)}
                        >
                            <Trash size={12} />
                        </button>
                    {/if}
                </div>
            </li>
        {/each}
    </ul>
{/if}

<style lang="scss">
    .list-header {
        margin-bottom: 6px;
    }

    .sort-select {
        width: 100%;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: rgba(255, 255, 255, 0.8);
        font-size: 0.62rem;
        letter-spacing: 0.03em;
        padding: 5px 6px;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .rows {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    li {
        position: relative;
        display: flex;
        align-items: stretch;
    }

    .row {
        flex: 1;
        text-align: left;
        background: rgba(20, 30, 35, 0.6);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 8px;
        padding: 8px 34px 8px 10px;
        cursor: pointer;
        color: inherit;
        min-width: 0;

        &:hover {
            background: rgba(35, 50, 55, 0.75);
        }

        &.active {
            border-color: rgba(82, 254, 254, 0.45);
            background: rgba(82, 254, 254, 0.08);
        }
    }

    .row-title {
        display: flex;
        align-items: center;
        gap: 4px;
    }

    .pin {
        color: #52fefe;
        display: flex;
    }

    .title-text {
        color: #ffffff;
        font-size: 0.76rem;
        font-weight: 500;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .row-excerpt {
        color: rgba(255, 255, 255, 0.55);
        font-size: 0.66rem;
        margin-top: 2px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .row-meta {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-top: 4px;
        color: rgba(255, 255, 255, 0.4);
        font-size: 0.58rem;
    }

    .when {
        display: inline-flex;
        align-items: center;
        gap: 3px;
    }

    .tags {
        display: inline-flex;
        gap: 3px;
        overflow: hidden;
    }

    .tag {
        background: rgba(82, 254, 254, 0.12);
        border-radius: 8px;
        color: rgba(82, 254, 254, 0.85);
        padding: 0 6px;
        max-width: 70px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .row-actions {
        position: absolute;
        right: 4px;
        top: 6px;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .mini {
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.45);
        cursor: pointer;
        display: flex;
        padding: 3px;

        &:hover {
            color: #52fefe;
        }

        &.danger:hover {
            color: #ff6b6b;
        }
    }

    .empty {
        padding: 24px 0;
        text-align: center;
    }
</style>
