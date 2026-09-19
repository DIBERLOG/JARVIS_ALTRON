<script lang="ts">
    /**
     * Where the context window goes.
     *
     * The rows come from `budgetBreakdown`, so the labels and the arithmetic match the
     * core: the system prompt, the response and template reserves, the memory and
     * summary sections, and the recent messages. Nothing here is a secret and nothing
     * here is a prompt: only token counts are shown.
     */
    import { Text } from "@svelteuidev/core"

    import type { BudgetView, FactView } from "@/lib/memory-model"
    import { budgetBreakdown, formatTokens } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let budget: BudgetView | null = null
    /** The approved entries, so the panel can say how many were added and how many did not fit. */
    export let facts: FactView[] = []

    $: t = (key: string) => translate($translations, key)
    $: rows = budgetBreakdown(budget)
    /** Entries the memory layer has already put into a request at least once. */
    $: usedFacts = facts.filter((fact) => fact.last_used_at !== null).length
    /** Entries that could not fit the memory budget on the last request. */
    $: droppedFacts = facts.filter((fact) => fact.last_used_at === null && !fact.disabled).length
</script>

<div class="budget">
    <div class="head">
        <span class="panel-title">{t('memory-budget-title')}</span>
        {#if budget}
            <span class="total">
                {t('memory-context-estimated')} · {formatTokens(budget.context_size)}
            </span>
        {/if}
    </div>

    {#if rows.length === 0}
        <Text size="xs" color="gray">{t('memory-context-none')}</Text>
    {:else}
        <div class="rows">
            {#each rows as row (row.key)}
                <div class="row">
                    <span class="label">{t(row.key)}</span>
                    <span class="bar">
                        <span class="fill" style="width: {Math.min(100, row.percent)}%"></span>
                    </span>
                    <span class="value">{formatTokens(row.tokens)}</span>
                    <span class="percent">{row.percent}%</span>
                </div>
            {/each}
        </div>

        <div class="notes">
            <Text size="xs" color="gray">{t('memory-context-facts-used')}: {String(usedFacts)}</Text>
            <Text size="xs" color="gray">{t('memory-context-dropped-facts')}: {String(droppedFacts)}</Text>
            <Text size="xs" color="gray">{t('memory-context-dropped-messages')}</Text>
        </div>
    {/if}

    <!-- The legend of the split: the same sections the core fills for a request. -->
    <div class="legend">
        <span class="legend-title">{t('memory-context-title')}</span>
        <div class="legend-rows">
            <span class="legend-item">{t('memory-context-memory')}</span>
            <span class="legend-item">{t('memory-context-summary')}</span>
            <span class="legend-item">{t('memory-context-history')}</span>
            <span class="legend-item">{t('memory-context-question')}</span>
        </div>
    </div>
</div>

<style lang="scss">
    .budget {
        background: rgba(20, 30, 35, 0.5);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 10px;
    }

    .head {
        align-items: center;
        display: flex;
        gap: 6px;
    }

    .panel-title {
        color: #ffffff;
        flex: 1;
        font-size: 0.7rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .total {
        color: rgba(255, 255, 255, 0.5);
        font-size: 0.58rem;
    }

    .rows {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .row {
        align-items: center;
        display: grid;
        gap: 6px;
        grid-template-columns: 130px 1fr 44px 40px;
    }

    .label {
        color: rgba(255, 255, 255, 0.75);
        font-size: 0.64rem;
    }

    .bar {
        background: rgba(255, 255, 255, 0.08);
        border-radius: 4px;
        height: 6px;
        overflow: hidden;
    }

    .fill {
        background: rgba(82, 254, 254, 0.55);
        display: block;
        height: 100%;
    }

    .value,
    .percent {
        color: rgba(255, 255, 255, 0.5);
        font-size: 0.58rem;
        text-align: right;
    }

    .notes {
        border-top: 1px dashed rgba(255, 255, 255, 0.1);
        display: flex;
        flex-direction: column;
        gap: 2px;
        padding-top: 5px;
    }

    .legend {
        border-top: 1px dashed rgba(255, 255, 255, 0.1);
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding-top: 5px;
    }

    .legend-title {
        color: rgba(255, 255, 255, 0.55);
        font-size: 0.58rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
    }

    .legend-rows {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
    }

    .legend-item {
        background: rgba(82, 254, 254, 0.1);
        border-radius: 8px;
        color: rgba(82, 254, 254, 0.85);
        font-size: 0.56rem;
        padding: 0 6px;
    }
</style>
