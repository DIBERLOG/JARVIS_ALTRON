<script lang="ts">
    /**
     * Unresolved conflicts, with the two resolutions.
     *
     * A conflict means two revisions of the same entry arrived; the store keeps both,
     * so this panel only has to ask which side should win.
     */
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"

    import type { MemoryConflictView } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let conflicts: MemoryConflictView[] = []
    export let busy = false

    const dispatch = createEventDispatcher<{
        resolve: { conflict: string; resolution: "keep_current" | "accept_incoming" }
    }>()

    $: t = (key: string) => translate($translations, key)
</script>

{#if conflicts.length > 0}
    <div class="conflicts">
        <div class="head">
            <span class="panel-title">{t('memory-conflicts')}</span>
            <span class="count">{conflicts.length}</span>
        </div>

        {#each conflicts as conflict (conflict.conflict_id)}
            <div class="conflict">
                <div class="meta">
                    <span>{t('memory-context-title')}</span>
                    <span>{conflict.entity_type}</span>
                    <span>r{conflict.current_revision} → r{conflict.incoming_revision}</span>
                    {#if !conflict.incoming_available}
                        <span class="unreadable">{t('memory-stats-unreadable')}</span>
                    {/if}
                </div>

                <div class="choices">
                    <Button
                        size="xs"
                        color="gray"
                        uppercase
                        disabled={busy}
                        on:click={() => dispatch('resolve', { conflict: conflict.conflict_id, resolution: 'keep_current' })}
                    >
                        {t('memory-conflict-keep-current')}
                    </Button>
                    <Button
                        size="xs"
                        color="cyan"
                        uppercase
                        disabled={busy || !conflict.incoming_available}
                        on:click={() => dispatch('resolve', { conflict: conflict.conflict_id, resolution: 'accept_incoming' })}
                    >
                        {t('memory-conflict-accept-incoming')}
                    </Button>
                </div>
            </div>
        {/each}
    </div>
{/if}

<style lang="scss">
    .conflicts {
        background: rgba(60, 40, 10, 0.35);
        border: 1px solid rgba(255, 179, 71, 0.35);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 9px;
    }

    .head {
        align-items: center;
        display: flex;
        gap: 6px;
    }

    .panel-title {
        color: #ffb347;
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

    .conflict {
        background: rgba(15, 22, 26, 0.7);
        border-radius: 6px;
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 6px 8px;
    }

    .meta {
        align-items: center;
        color: rgba(255, 255, 255, 0.55);
        display: flex;
        flex-wrap: wrap;
        font-size: 0.58rem;
        gap: 8px;
    }

    .unreadable {
        color: #ff9b9b;
    }

    .choices {
        display: flex;
        flex-wrap: wrap;
        gap: 5px;
    }
</style>
