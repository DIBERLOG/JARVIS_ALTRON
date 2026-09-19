<script lang="ts">
    /**
     * A translated list of messages, used for the outcome of an export or an import,
     * for the large-memory warning, and for the warnings the backend reports.
     *
     * The component only receives keys, so a caller cannot accidentally hand it a raw
     * backend string that might carry text from the database.
     */
    import { Alert, Text } from "@svelteuidev/core"

    import { translations, translate } from "@/stores"

    export let titleKey = ""
    export let messageKeys: string[] = []
    /** Rendered before the list, for example a record count. */
    export let detail = ""

    $: t = (key: string) => translate($translations, key)
</script>

{#if messageKeys.length > 0 || detail.length > 0}
    <Alert title={titleKey.length > 0 ? t(titleKey) : t('memory-warning-generic')} color="orange" variant="outline">
        {#if detail.length > 0}
            <Text size="xs" color="gray">{detail}</Text>
        {/if}
        {#each messageKeys as key}
            <Text size="xs" color="gray">{t(key)}</Text>
        {/each}
    </Alert>
{/if}
