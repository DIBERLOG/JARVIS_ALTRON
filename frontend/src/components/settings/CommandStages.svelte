<script lang="ts">
    /**
     * The last stages a spoken phrase passed.
     *
     * One line per safe diagnostic the voice host sent: the stage name, the length
     * of the text the matcher saw, a reason code and a command id. There is no
     * transcript in a line and nothing here is stored — the ring lives in memory for
     * the session, so "why was my command not accepted" has an answer that does not
     * involve reading a log.
     */
    import { Space, Text } from "@svelteuidev/core"

    import { commandStages, translate, translations } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    /** The Fluent key of a stage, from the core's own name. */
    function stageLabel(stage: string): string {
        return `command-stage-${stage}`
    }

    /** The outcome of a stage, in one short line. */
    function stageDetail(entry: {
        length: number
        code: string | null
        success: boolean | null
    }): string {
        if (entry.success === true) return "ok"
        if (entry.success === false) return "error"
        if (entry.code) return entry.code
        return `${entry.length}`
    }
</script>

<Text weight={600}>{t("command-stages-title")}</Text>
{#if $commandStages.length === 0}
    <Space h="xs" />
    <Text size="sm" color="dimmed">{t("command-stages-empty")}</Text>
{:else}
    <ul class="stages">
        {#each $commandStages as entry, index (index)}
            <li>
                <Text size="sm" color="dimmed">
                    {t(stageLabel(entry.stage))} · {entry.length} · {stageDetail(entry)}
                    {#if entry.command_id}· {entry.command_id}{/if}
                </Text>
            </li>
        {/each}
    </ul>
{/if}

<style>
    .stages {
        margin: 0.25rem 0 0 0;
        padding-left: 1.1rem;
        max-height: 14rem;
        overflow: auto;
    }
</style>
