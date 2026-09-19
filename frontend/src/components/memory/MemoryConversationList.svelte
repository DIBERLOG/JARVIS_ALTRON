<script lang="ts">
    /**
     * The stored conversations.
     *
     * Renaming and deleting are per conversation and ask for their own confirmation;
     * clearing every conversation is a bulk action and needs an explicit confirmation
     * before the page may call `clearHistory(true)`.
     */
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"

    import type { ConversationView } from "@/lib/memory-model"
    import { conversationTitle } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let conversations: ConversationView[] = []
    export let selectedId: string | null = null
    export let includeArchived = false
    export let busy = false

    const dispatch = createEventDispatcher<{
        select: string
        create: void
        archived: { archived: boolean; id: string | null }
        rename: { id: string; title: string }
        remove: string
        clear: string
        clearAll: void
    }>()

    $: t = (key: string) => translate($translations, key)
    $: placeholder = t('memory-new-conversation')

    /** The row whose title is being edited, and the row awaiting a delete confirm. */
    let renamingId: string | null = null
    let draftTitle = ""
    let confirmingId: string | null = null
    let confirmingHistory = false

    function startRename(conversation: ConversationView) {
        renamingId = conversation.id
        draftTitle = conversation.title
        confirmingId = null
    }

    function commitRename(id: string) {
        dispatch("rename", { id, title: draftTitle })
        renamingId = null
    }

    function confirmDelete(id: string) {
        dispatch("remove", id)
        confirmingId = null
    }
</script>

<div class="conversations">
    <div class="panel-head">
        <span class="panel-title">{t('memory-conversations')}</span>
        <span class="count">{conversations.length}</span>
        <Button size="xs" color="lime" uppercase disabled={busy} on:click={() => dispatch('create')}>
            {t('memory-new-conversation')}
        </Button>
    </div>

    <div class="controls">
        <button
            class="chip"
            class:active={includeArchived}
            on:click={() => dispatch('archived', { archived: !includeArchived, id: null })}
        >
            {t('memory-show-archived')}
        </button>

        {#if confirmingHistory}
            <span class="confirm">{t('memory-confirm-history')}</span>
            <Button size="xs" color="red" uppercase disabled={busy} on:click={() => { confirmingHistory = false; dispatch('clearAll') }}>
                {t('memory-clear-history')}
            </Button>
            <Button size="xs" color="gray" uppercase on:click={() => (confirmingHistory = false)}>
                {t('memory-secret-cancel')}
            </Button>
        {:else}
            <button class="chip danger" on:click={() => (confirmingHistory = true)}>
                {t('memory-clear-history')}
            </button>
        {/if}
    </div>

    {#if conversations.length === 0}
        <div class="empty"><Text size="sm" color="gray">{t('memory-conversation-none')}</Text></div>
    {:else}
        <ul class="rows">
            {#each conversations as conversation (conversation.id)}
                <li class="conversation" class:selected={conversation.id === selectedId}>
                    {#if renamingId === conversation.id}
                        <input
                            class="rename-input"
                            type="text"
                            bind:value={draftTitle}
                            on:keydown={(event) => event.key === 'Enter' && commitRename(conversation.id)}
                        />
                        <div class="row-actions">
                            <button class="mini" on:click={() => commitRename(conversation.id)}>{t('memory-rename')}</button>
                            <button class="mini" on:click={() => (renamingId = null)}>{t('memory-secret-cancel')}</button>
                        </div>
                    {:else}
                        <button class="row-main" on:click={() => dispatch('select', conversation.id)}>
                            <div class="title">
                                {conversationTitle(conversation.title, placeholder)}
                                {#if conversation.archived_at}<span class="tag archived">{t('memory-archive')}</span>{/if}
                            </div>
                            <div class="meta">
                                <span>{conversation.message_count}</span>
                                <span>{conversation.updated_at}</span>
                            </div>
                        </button>

                        <div class="row-actions">
                            <button class="mini" on:click|stopPropagation={() => startRename(conversation)}>
                                {t('memory-rename')}
                            </button>
                            <button
                                class="mini"
                                on:click|stopPropagation={() => dispatch('clear', conversation.id)}
                            >
                                {t('memory-clear-conversation')}
                            </button>
                            {#if conversation.archived_at}
                                <button class="mini" on:click|stopPropagation={() => dispatch('archived', { archived: false, id: conversation.id })}>
                                    {t('memory-restore')}
                                </button>
                            {:else}
                                <button class="mini" on:click|stopPropagation={() => dispatch('archived', { archived: true, id: conversation.id })}>
                                    {t('memory-archive')}
                                </button>
                            {/if}
                            {#if confirmingId === conversation.id}
                                <span class="confirm">{t('memory-confirm-conversation')}</span>
                                <button class="mini danger" on:click|stopPropagation={() => confirmDelete(conversation.id)}>
                                    {t('memory-delete')}
                                </button>
                                <button class="mini" on:click|stopPropagation={() => (confirmingId = null)}>
                                    {t('memory-secret-cancel')}
                                </button>
                            {:else}
                                <button class="mini danger" on:click|stopPropagation={() => (confirmingId = conversation.id)}>
                                    {t('memory-delete')}
                                </button>
                            {/if}
                        </div>
                    {/if}
                </li>
            {/each}
        </ul>
    {/if}
</div>

<style lang="scss">
    .conversations {
        background: rgba(20, 30, 35, 0.5);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 10px;
        min-width: 0;
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
        background: rgba(82, 254, 254, 0.15);
        border-radius: 8px;
        color: #52fefe;
        font-size: 0.6rem;
        margin-right: auto;
        padding: 1px 7px;
    }

    .controls {
        align-items: center;
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
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

        &.danger {
            color: rgba(255, 150, 150, 0.85);

            &:hover {
                background: rgba(255, 107, 107, 0.15);
            }
        }
    }

    .confirm {
        color: #ffb347;
        font-size: 0.6rem;
    }

    .rows {
        display: flex;
        flex-direction: column;
        gap: 5px;
        list-style: none;
        margin: 0;
        max-height: 320px;
        overflow-y: auto;
        padding: 0;
    }

    .conversation {
        background: rgba(15, 22, 26, 0.65);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 8px;
        display: flex;
        flex-direction: column;
        min-width: 0;

        &.selected {
            background: rgba(82, 254, 254, 0.08);
            border-color: rgba(82, 254, 254, 0.45);
        }
    }

    .row-main {
        background: transparent;
        border: none;
        color: inherit;
        cursor: pointer;
        min-width: 0;
        padding: 7px 9px 3px;
        text-align: left;
    }

    .title {
        align-items: center;
        color: #ffffff;
        display: flex;
        font-size: 0.74rem;
        gap: 5px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .tag {
        background: rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        color: rgba(255, 255, 255, 0.6);
        font-size: 0.55rem;
        padding: 0 6px;
    }

    .meta {
        color: rgba(255, 255, 255, 0.45);
        display: flex;
        font-size: 0.58rem;
        gap: 8px;
        margin-top: 2px;
    }

    .row-actions {
        align-items: center;
        display: flex;
        flex-wrap: wrap;
        gap: 7px;
        padding: 3px 9px 7px;
    }

    .rename-input {
        background: rgba(10, 18, 22, 0.85);
        border: 1px solid rgba(82, 254, 254, 0.4);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.7rem;
        margin: 7px 9px 0;
        padding: 5px 7px;

        &:focus {
            outline: none;
        }
    }

    .mini {
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.55);
        cursor: pointer;
        font-size: 0.57rem;
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

    .empty {
        padding: 18px 0;
        text-align: center;
    }
</style>
