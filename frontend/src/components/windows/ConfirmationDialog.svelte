<script lang="ts">
    /**
     * The confirmation dialog for a risky action.
     *
     * Two rules shape it:
     *
     * * it renders exactly what the core put into `ActionPreview` — the fields, the risk, and
     *   the source. Nothing here builds an action, adds a path, or changes a number: the
     *   confirmation that comes back carries only the token, and the core runs the request it
     *   stored, not anything this component could send;
     * * it is not a browser dialog. No `confirm()`, no `window.prompt`, and nothing about the
     *   pending action is written to browser storage, so a reload forgets it — which is the
     *   honest behaviour for an approval that expires.
     */
    import { Alert, Button, Group, Space, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import type { ActionPreview } from "@/lib/windows-actions-model"
    import {
        actionTypeKey,
        confirmationTitleKey,
        isDangerous,
        previewFields,
        previewValueKey,
        riskKey,
        sourceKey
    } from "@/lib/windows-actions-model"

    export let preview: ActionPreview
    export let busy = false
    export let onConfirm: (token: string) => void
    export let onCancel: () => void

    $: t = (key: string) => translate($translations, key)
    $: fields = previewFields(preview)
    $: dangerous = isDangerous(preview)

    /** The value of a field: a known word is translated, free text is shown as it is. */
    function value(field: { label_key: string; value: string }): string {
        const key = previewValueKey(field.label_key, field.value)
        return key ? t(key) : field.value
    }
</script>

<div class="confirmation" role="alertdialog" aria-label={t("windows-actions-confirm-title")}>
    <Text weight={700} size="lg">{t("windows-actions-confirm-title")}</Text>
    <Space h="xs" />
    <Text size="sm">{t(confirmationTitleKey(preview))}</Text>
    <Space h="sm" />
    <Alert color={dangerous ? "red" : "yellow"} title={t(riskKey(preview.risk))}>
        <Text size="sm">{t("windows-actions-confirm-warning")}</Text>
    </Alert>
    <Space h="sm" />
    <table class="fields">
        {#each fields as field (field.label_key + field.value)}
            <tr>
                <td class="label">{t(field.label_key)}</td>
                <td class="value">{value(field)}</td>
            </tr>
        {/each}
        <tr>
            <td class="label">{t("windows-actions-audit-column-source")}</td>
            <td class="value">{t(sourceKey(preview.source))}</td>
        </tr>
    </table>
    {#if preview.consequences.length > 0}
        <Space h="sm" />
        <ul class="consequences">
            {#each preview.consequences as consequence (consequence)}
                <li>{t(consequence)}</li>
            {/each}
        </ul>
    {/if}
    <Space h="sm" />
    <Text size="sm" color="dimmed">
        {t("windows-actions-confirm-expires")}
        {preview.expires_in_seconds} s
    </Text>
    <Space h="md" />
    <Group position="right">
        <Button variant="default" disabled={busy} on:click={() => onCancel()}>
            {t("windows-actions-confirm-cancel")}
        </Button>
        <Button color={dangerous ? "red" : "green"} loading={busy} on:click={() => onConfirm(preview.token)}>
            {t("windows-actions-confirm-accept")}
        </Button>
    </Group>
</div>

<style>
    .confirmation {
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 12px;
        padding: 1rem;
        background: rgba(20, 20, 24, 0.95);
    }

    .fields {
        width: 100%;
        border-collapse: collapse;
    }

    .fields td {
        padding: 0.15rem 0.35rem;
        vertical-align: top;
        font-size: 0.85rem;
    }

    .label {
        opacity: 0.7;
        white-space: nowrap;
    }

    .value {
        word-break: break-word;
    }

    .consequences {
        margin: 0;
        padding-left: 1.1rem;
        font-size: 0.82rem;
        opacity: 0.85;
    }
</style>
