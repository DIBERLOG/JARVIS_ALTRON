<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"
    import { ExclamationTriangle } from "radix-icons-svelte"

    import type { VaultConflictResolution, VaultConflictView } from "@/lib/vault-model"
    import { CONFLICT_RESOLUTIONS, conflictOptionKey } from "@/lib/vault-model"
    import { translations, translate } from "@/stores"

    export let conflicts: VaultConflictView[] = []
    export let busy = false

    const dispatch = createEventDispatcher<{
        resolve: { conflict: string; resolution: VaultConflictResolution }
    }>()

    $: t = (key: string) => translate($translations, key)

    /** Both sides are always retained; every resolution is offered for a note
     *  conflict, while a non-item conflict cannot be copied. */
    function resolutionsFor(conflict: VaultConflictView): VaultConflictResolution[] {
        return conflict.is_vault_item
            ? [...CONFLICT_RESOLUTIONS]
            : CONFLICT_RESOLUTIONS.filter((resolution) => resolution !== "keep_both")
    }
</script>

{#if conflicts.length > 0}
    <div class="conflicts">
        <div class="conflicts-title">
            <ExclamationTriangle size={14} />
            <Text size="sm">{t('vault-conflicts')} ({conflicts.length})</Text>
        </div>

        {#each conflicts as conflict (conflict.conflict_id)}
            <div class="conflict">
                <div class="side">
                    <span class="label">{t('vault-conflict-current')} · r{conflict.current_revision}</span>
                    <span class="name">{conflict.current_name ?? t('vault-untitled')}</span>
                </div>
                <div class="side incoming">
                    <span class="label">{t('vault-conflict-incoming')} · r{conflict.incoming_revision}</span>
                    <span class="name">{conflict.incoming_name ?? t('vault-conflict-unreadable')}</span>
                </div>
                <div class="choices">
                    {#each resolutionsFor(conflict) as resolution}
                        <Button
                            size="xs"
                            color={resolution === "accept_incoming" ? "cyan" : "gray"}
                            uppercase
                            disabled={busy}
                            on:click={() => dispatch('resolve', { conflict: conflict.conflict_id, resolution })}
                        >
                            {t(conflictOptionKey(resolution))}
                        </Button>
                    {/each}
                </div>
            </div>
        {/each}
    </div>
{/if}

<style lang="scss">
    .conflicts {
        border: 1px solid rgba(255, 179, 71, 0.35);
        background: rgba(60, 40, 10, 0.35);
        border-radius: 8px;
        padding: 8px;
        margin-bottom: 8px;
        display: flex;
        flex-direction: column;
        gap: 8px;
    }

    .conflicts-title {
        display: flex;
        align-items: center;
        gap: 6px;
        color: #ffb347;
    }

    .conflict {
        background: rgba(15, 22, 26, 0.7);
        border-radius: 6px;
        padding: 6px 8px;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .side {
        display: flex;
        flex-direction: column;
        gap: 1px;
        min-width: 0;
    }

    .side.incoming {
        border-top: 1px dashed rgba(255, 255, 255, 0.12);
        padding-top: 4px;
    }

    .label {
        font-size: 0.55rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: rgba(255, 255, 255, 0.45);
    }

    .name {
        font-size: 0.7rem;
        color: #ffffff;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .choices {
        display: flex;
        gap: 5px;
        flex-wrap: wrap;
        margin-top: 2px;
    }
</style>
