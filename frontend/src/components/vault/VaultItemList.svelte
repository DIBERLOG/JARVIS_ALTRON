<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Text } from "@svelteuidev/core"
    import { Star, StarFilled, Trash, Update } from "radix-icons-svelte"

    import type { VaultItemSummary, VaultSort } from "@/lib/vault-model"
    import { VAULT_SORTS, itemSubtitle, itemTitle, sortOptionKey } from "@/lib/vault-model"
    import { translations, translate } from "@/stores"

    export let items: VaultItemSummary[] = []
    export let selectedId: string | null = null
    export let sort: VaultSort = "name_asc"
    export let loading = false

    const dispatch = createEventDispatcher<{
        select: string
        favorite: { id: string; favorite: boolean }
        trash: string
        restore: string
        sort: VaultSort
    }>()

    $: t = (key: string) => translate($translations, key)
    $: untitled = t("vault-untitled")
    $: nothing = t("vault-none")

    let sortValue: string = sort
    $: sortValue = sort

    function onSortChange(event: Event) {
        dispatch("sort", (event.currentTarget as HTMLSelectElement).value as VaultSort)
    }
</script>

<div class="list-header">
    <select class="sort-select" value={sortValue} on:change={onSortChange}>
        {#each VAULT_SORTS as option}
            <option value={option}>{t(sortOptionKey(option))}</option>
        {/each}
    </select>
</div>

{#if loading}
    <div class="empty"><Text size="sm" color="gray">{t('vault-loading')}</Text></div>
{:else if items.length === 0}
    <div class="empty"><Text size="sm" color="gray">{t('vault-empty')}</Text></div>
{:else}
    <ul class="rows">
        {#each items as item (item.id)}
            <li>
                <button class="row" class:active={item.id === selectedId} on:click={() => dispatch('select', item.id)}>
                    <div class="row-main">
                        <div class="row-title">
                            {#if item.favorite}
                                <span class="favorite"><StarFilled size={11} /></span>
                            {/if}
                            <span class="title-text">{itemTitle(item.name, untitled)}</span>
                        </div>
                        <div class="row-subtitle">{itemSubtitle(item, nothing)}</div>
                        {#if item.tags.length > 0}
                            <div class="row-tags">
                                {#each item.tags.slice(0, 3) as tag}
                                    <span class="tag">{tag}</span>
                                {/each}
                            </div>
                        {/if}
                    </div>
                </button>

                <div class="row-actions">
                    <button
                        class="mini"
                        title={item.favorite ? t('vault-unfavorite') : t('vault-favorite')}
                        on:click|stopPropagation={() => dispatch('favorite', { id: item.id, favorite: !item.favorite })}
                    >
                        {#if item.favorite}
                            <StarFilled size={12} />
                        {:else}
                            <Star size={12} />
                        {/if}
                    </button>
                    {#if item.deleted_at}
                        <button class="mini" title={t('vault-restore')} on:click|stopPropagation={() => dispatch('restore', item.id)}>
                            <Update size={12} />
                        </button>
                    {:else}
                        <button class="mini danger" title={t('vault-trash')} on:click|stopPropagation={() => dispatch('trash', item.id)}>
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

    .favorite {
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

    .row-subtitle {
        color: rgba(255, 255, 255, 0.55);
        font-size: 0.66rem;
        margin-top: 2px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .row-tags {
        display: flex;
        gap: 3px;
        margin-top: 4px;
        overflow: hidden;
    }

    .tag {
        background: rgba(82, 254, 254, 0.12);
        border-radius: 8px;
        color: rgba(82, 254, 254, 0.85);
        font-size: 0.58rem;
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
