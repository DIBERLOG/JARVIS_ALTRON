<script lang="ts">
    /**
     * The memory dashboard.
     *
     * The counts exist only while the storage is unlocked: the backend reports zeroes
     * for a locked store, and the page does not render this strip in that state, so a
     * locked memory can never look like an empty one.
     */
    import type { MemoryStats } from "@/lib/memory-model"
    import { statsRowKeys, statsRowValue } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let stats: MemoryStats

    $: t = (key: string) => translate($translations, key)
    $: keys = statsRowKeys(stats)
</script>

<div class="stats">
    <span class="panel-title">{t('memory-stats-title')}</span>
    <div class="rows">
        {#each keys as key}
            <div class="stat">
                <span class="value">{statsRowValue(stats, key)}</span>
                <span class="label">{t(key)}</span>
            </div>
        {/each}
    </div>
</div>

<style lang="scss">
    .stats {
        align-items: center;
        background: rgba(20, 30, 35, 0.5);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 10px;
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
        padding: 8px 10px;
    }

    .panel-title {
        color: #ffffff;
        font-size: 0.66rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .rows {
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
    }

    .stat {
        align-items: baseline;
        display: flex;
        gap: 5px;
    }

    .value {
        color: #52fefe;
        font-size: 0.85rem;
    }

    .label {
        color: rgba(255, 255, 255, 0.55);
        font-size: 0.6rem;
        letter-spacing: 0.03em;
    }
</style>
