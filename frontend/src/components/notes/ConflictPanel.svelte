<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"
    import { ExclamationTriangle } from "radix-icons-svelte"

    import type { NoteConflictResolution, NoteConflictView } from "@/lib/notes-model"
    import { CONFLICT_RESOLUTIONS, conflictOptionKey, displayExcerpt, displayTitle } from "@/lib/notes-model"
    import { translations, translate } from "@/stores"

    export let conflicts: NoteConflictView[] = []
    export let busy = false

    const dispatch = createEventDispatcher<{
        resolve: { conflict: string; resolution: NoteConflictResolution }
    }>()

    $: t = (key: string) => translate($translations, key)
    $: untitled = t("notes-untitled")
    $: noText = t("notes-no-text")

    /** Only resolutions that make sense for a given conflict are offered. */
    function resolutionsFor(conflict: NoteConflictView): NoteConflictResolution[] {
        return conflict.incoming
            ? [...CONFLICT_RESOLUTIONS]
            : CONFLICT_RESOLUTIONS.filter((resolution) => resolution !== "keep_both")
    }
</script>

{#if conflicts.length > 0}
    <div class="conflicts">
        <div class="conflicts-title">
            <ExclamationTriangle size={14} />
            <Text size="sm">{t('notes-conflicts')} ({conflicts.length})</Text>
        </div>

        {#each conflicts as conflict (conflict.conflict_id)}
            <div class="conflict">
                <div class="side">
                    <span class="label">{t('notes-conflict-current')} · r{conflict.current_revision}</span>
                    <span class="title">{displayTitle(conflict.current?.title ?? "", untitled)}</span>
                    <span class="excerpt">{displayExcerpt(conflict.current?.body ?? "", noText)}</span>
                </div>
                <div class="side incoming">
                    <span class="label">{t('notes-conflict-incoming')} · r{conflict.incoming_revision}</span>
                    <span class="title">
                        {conflict.incoming
                            ? displayTitle(conflict.incoming.title, untitled)
                            : t('notes-conflict-unreadable')}
                    </span>
                    <span class="excerpt">{displayExcerpt(conflict.incoming?.body ?? "", noText)}</span>
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
        margin-bottom: 10px;
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

    .title {
        font-size: 0.7rem;
        color: #ffffff;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .excerpt {
        font-size: 0.62rem;
        color: rgba(255, 255, 255, 0.5);
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
