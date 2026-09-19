<script lang="ts">
    /**
     * The secret warning shown when a save was refused.
     *
     * The backend reports which kinds of secret it saw and never the matched text, so
     * this dialog can only name the kinds: there is nothing secret-shaped in the props
     * it receives. Saving again is an explicit, separate click.
     */
    import { createEventDispatcher } from "svelte"
    import { Alert, Button, Text } from "@svelteuidev/core"

    import type { SecretKind } from "@/lib/memory-model"
    import { secretWarningKeys } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let kinds: SecretKind[] = []
    /** A free-form message key for the operation that was refused. */
    export let contextKey = ""
    export let busy = false

    const dispatch = createEventDispatcher<{ confirm: void; cancel: void }>()

    $: t = (key: string) => translate($translations, key)
    $: labels = secretWarningKeys(kinds)
</script>

<div class="overlay">
    <div class="warning">
        <Alert title={t('memory-secret-warning-title')} color="orange" variant="outline">
            <Text size="xs" color="gray">{t('memory-secret-warning-body')}</Text>
            <ul class="kinds">
                {#each labels as label}
                    <li>{t(label)}</li>
                {/each}
            </ul>
            {#if contextKey.length > 0}
                <Text size="xs" color="gray">{t(contextKey)}</Text>
            {/if}
        </Alert>

        <Text size="xs" color="gray">{t('memory-secret-filter-note')}</Text>

        <div class="actions">
            <Button size="xs" color="red" uppercase disabled={busy} on:click={() => dispatch('confirm')}>
                {t('memory-secret-confirm')}
            </Button>
            <Button size="xs" color="gray" uppercase disabled={busy} on:click={() => dispatch('cancel')}>
                {t('memory-secret-cancel')}
            </Button>
        </div>
    </div>
</div>

<style lang="scss">
    .overlay {
        align-items: center;
        background: rgba(4, 8, 10, 0.72);
        bottom: 0;
        display: flex;
        justify-content: center;
        left: 0;
        padding: 20px;
        position: fixed;
        right: 0;
        top: 0;
        z-index: 200;
    }

    .warning {
        background: rgba(20, 30, 35, 0.98);
        border: 1px solid rgba(255, 179, 71, 0.4);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 8px;
        max-width: 440px;
        padding: 14px;
        width: 100%;
    }

    .kinds {
        color: #ffb347;
        font-size: 0.68rem;
        margin: 6px 0 0;
        padding-left: 16px;
    }

    .actions {
        display: flex;
        gap: 6px;
    }
</style>
