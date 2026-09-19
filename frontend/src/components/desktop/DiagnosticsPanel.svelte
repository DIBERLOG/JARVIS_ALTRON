<script lang="ts">
    /**
     * Runtime diagnostics: what is present, what is missing, and what could not
     * be checked.
     *
     * The report is built, redacted, and screened by the core. This panel only
     * shows the preview and offers to save it — it never assembles a document of
     * its own, and it never puts a path on screen that the core did not already
     * replace.
     */
    import { onMount } from "svelte"

    import { Alert, Button, Group, Space, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import { desktopApi } from "@/lib/desktop"
    import type { DiagnosticsView } from "@/lib/desktop-model"
    import {
        attentionComponents,
        canExport,
        componentNameKey,
        componentStateKey,
        reportSummaryKey,
        sizeLabel
    } from "@/lib/desktop-model"

    $: t = (key: string) => translate($translations, key)

    let view: DiagnosticsView | null = null
    let busy = false
    let actionError = ""
    let exported = ""

    onMount(async () => {
        await run()
    })

    async function run() {
        busy = true
        try {
            view = await desktopApi.diagnosticsRun()
            actionError = ""
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }

    async function exportReport() {
        busy = true
        try {
            const path = await desktopApi.diagnosticsExport()
            exported = path ?? ""
            actionError = ""
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }

    async function copySummary() {
        busy = true
        try {
            const summary = await desktopApi.diagnosticsSummary()
            // The clipboard is the only place this text goes, and the summary
            // was screened by the core before it was returned.
            await navigator.clipboard.writeText(summary)
            exported = t("desktop-diagnostics-copied")
            actionError = ""
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }
</script>

<Text weight={700} size="lg">{t("desktop-diagnostics-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("desktop-diagnostics-description")}</Text>
<Space h="sm" />

{#if actionError}
    <Alert color="red" title={t("desktop-diagnostics-title")}>{actionError}</Alert>
    <Space h="sm" />
{/if}

<Group spacing="xs">
    <Button size="sm" loading={busy} on:click={run}>{t("desktop-diagnostics-run")}</Button>
    <Button size="sm" variant="default" loading={busy} disabled={!view || !canExport(view)} on:click={exportReport}>
        {t("desktop-diagnostics-export")}
    </Button>
    <Button size="sm" variant="subtle" loading={busy} disabled={!view} on:click={copySummary}>
        {t("desktop-diagnostics-copy")}
    </Button>
</Group>
{#if exported}
    <Text size="xs" color="green">{exported}</Text>
{/if}
<Space h="sm" />

{#if view}
    <Alert color={attentionComponents(view.report).length > 0 ? "yellow" : "green"} title={t("desktop-diagnostics-title")}>
        <Text size="sm">{t(reportSummaryKey(view.report))}</Text>
        {#if !view.screen_passed}
            <Text size="xs" color="red">
                {t("desktop-diagnostics-blocked")} {view.screen_error ?? ""}
            </Text>
        {/if}
    </Alert>
    <Space h="sm" />

    <table class="rows">
        <thead>
            <tr>
                <th>{t("desktop-diagnostics-component")}</th>
                <th>{t("desktop-diagnostics-state")}</th>
                <th>{t("desktop-diagnostics-detail")}</th>
            </tr>
        </thead>
        <tbody>
            {#each view.report.components as component (component.name)}
                <tr>
                    <td>{t(componentNameKey(component.name))}</td>
                    <td class={component.state}>{t(componentStateKey(component.state))}</td>
                    <td>{component.detail ?? ""}</td>
                </tr>
            {/each}
        </tbody>
    </table>
    <Space h="sm" />

    {#if view.report.database_sizes.length > 0 || view.report.model_sizes.length > 0}
        <Text weight={600}>{t("desktop-diagnostics-sizes")}</Text>
        {#each [...view.report.database_sizes, ...view.report.model_sizes] as file (file.name)}
            <Text size="xs" color="dimmed">{file.name}: {sizeLabel(file.bytes)}</Text>
        {/each}
        <Space h="sm" />
    {/if}

    {#if view.report.recent_errors.length > 0}
        <Text weight={600}>{t("desktop-diagnostics-errors")}</Text>
        {#each view.report.recent_errors as category (category.code)}
            <Text size="xs" color="dimmed">{category.code}: {category.count}</Text>
        {/each}
        <Space h="sm" />
    {/if}

    <Text weight={600}>{t("desktop-diagnostics-preview")}</Text>
    <div class="preview">
        {#each view.preview as line, index (index)}
            <Text size="xs">{line}</Text>
        {/each}
    </div>
{/if}

<style>
    .rows {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.8rem;
    }

    .rows th,
    .rows td {
        padding: 0.15rem 0.3rem;
        text-align: left;
        vertical-align: top;
        border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    }

    .rows th {
        opacity: 0.7;
    }

    .preview {
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        padding: 0.5rem;
        max-height: 18rem;
        overflow: auto;
        font-family: monospace;
    }
</style>
