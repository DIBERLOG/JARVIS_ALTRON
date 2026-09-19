<script lang="ts">
    /**
     * Full backup and restore.
     *
     * What this panel does not do: it never shows a path, never keeps the
     * password, and never restores without the person having seen what the
     * container holds. The core does the work — verify, stage, safety backup,
     * move aside, install, verify, commit, or roll back — and this panel reports
     * what happened and offers the parts that cannot be automatic (the
     * confirmation, and deleting what the restore left behind).
     */
    import { onMount } from "svelte"

    import { Alert, Button, Group, Input, Space, Switch, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import { backupApi } from "@/lib/backup"
    import type { ExportReport, RestoreOutcome } from "@/lib/backup"
    import type { BackupPanelView, BackupPreview } from "@/lib/backup-model"
    import {
        componentLabelKey,
        containerIsRestorable,
        formatBytes,
        operationKey,
        passwordProblem,
        previewWarnings
    } from "@/lib/backup-model"

    $: t = (key: string) => translate($translations, key)

    let view: BackupPanelView | null = null
    let password = ""
    let overwrite = false
    let busy = false
    let actionError = ""
    let exported: ExportReport | null = null
    let preview: BackupPreview | null = null
    let restored: RestoreOutcome | null = null
    /** The step the panel is showing, so a long operation is not a frozen button. */
    let step = ""

    $: problem = passwordProblem(password)
    $: canExport = !busy && problem === null
    $: canRestore = !busy && problem === null && preview !== null && containerIsRestorable(preview)

    onMount(() => {
        void load()
    })

    async function load() {
        try {
            view = await backupApi.status()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        }
    }

    function describe(error: unknown): string {
        const code = typeof error === "string" ? error : String(error)
        if (code === "cancelled") return t("backup-cancelled")
        return /^[a-z_]+$/.test(code) ? t(`backup-error-${code}`) : code
    }

    /** Creates a full backup, after the native dialog chose where it goes. */
    async function createBackup() {
        if (!canExport) return
        busy = true
        exported = null
        step = t("backup-step-export")
        try {
            exported = await backupApi.export(password, overwrite)
            actionError = ""
            step = ""
            await load()
        } catch (error) {
            actionError = describe(error)
            step = ""
        } finally {
            busy = false
        }
    }

    /** Opens a container and shows what it holds. Nothing is changed yet. */
    async function openContainer() {
        if (problem !== null) return
        busy = true
        preview = null
        restored = null
        step = t("backup-step-inspect")
        try {
            preview = await backupApi.inspect(password)
            actionError = ""
            step = ""
        } catch (error) {
            actionError = describe(error)
            step = ""
        } finally {
            busy = false
        }
    }

    /** Restores the container the person just opened and confirmed. */
    async function restoreContainer() {
        if (!canRestore) return
        busy = true
        restored = null
        step = t("backup-step-restore")
        try {
            restored = await backupApi.restore(password, true)
            preview = null
            actionError = ""
            step = ""
            await load()
        } catch (error) {
            actionError = describe(error)
            step = ""
        } finally {
            busy = false
        }
    }

    async function discardPrevious() {
        await guard(async () => {
            await backupApi.discardPrevious()
        })
    }

    async function deleteSafety(name: string) {
        await guard(async () => {
            await backupApi.deleteSafety(name)
        })
    }

    /** Deletes the newest safety backup, from the name the core reported. */
    function deleteNewestSafetyBackup() {
        const name = view?.status.newest_safety_backup
        if (name) void deleteSafety(name)
    }

    async function guard(action: () => Promise<void>) {
        busy = true
        try {
            await action()
            actionError = ""
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }
</script>

<Text weight={700} size="lg">{t("backup-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("backup-description")}</Text>
<Space h="sm" />

{#if actionError}
    <Alert color="red" title={t("backup-title")}>{actionError}</Alert>
    <Space h="sm" />
{/if}

{#if view}
    <Alert
        color={view.status.interrupted_restore ? "orange" : view.status.available ? "green" : "yellow"}
        title={t(operationKey(view.status))}
    >
        {#if view.status.interrupted_restore}
            <Text size="sm">
                {t("backup-interrupted")} — {view.status.interrupted_restore}
            </Text>
        {/if}
        <Text size="xs" color="dimmed">
            {t("backup-format-version")}: {view.status.format_version}
        </Text>
        {#if !view.status.key_envelope_present}
            <Text size="xs" color="dimmed">{t("backup-no-key-envelope")}</Text>
        {/if}
    </Alert>
    <Space h="sm" />

    <!-- What a full backup would contain. Logical component names only. -->
    <Text weight={600}>{t("backup-included")}</Text>
    {#each view.plan.included as name (name)}
        <Text size="xs" color="dimmed">• {t(componentLabelKey(name))}</Text>
    {/each}
    {#if view.plan.absent.length > 0}
        <Text size="xs" color="dimmed">
            {t("backup-absent")}: {view.plan.absent.map(componentLabelKey).map((key) => t(key)).join(", ")}
        </Text>
    {/if}
    <Space h="sm" />
{/if}

<label class="field">
    <span>{t("backup-password")}</span>
    <Input
        type="password"
        size="sm"
        autocomplete="current-password"
        bind:value={password}
        placeholder={t("backup-password-placeholder")}
    />
</label>
<Text size="xs" color="dimmed">{t("backup-password-hint")}</Text>
{#if problem}
    <Text size="xs" color="orange">{t(problem)}</Text>
{/if}
<Space h="xs" />

<Switch label={t("backup-overwrite")} bind:checked={overwrite} />
<Text size="xs" color="dimmed">{t("backup-overwrite-hint")}</Text>
<Space h="xs" />

<Group spacing="xs">
    <Button size="sm" color="green" loading={busy && !preview} disabled={!canExport} on:click={createBackup}>
        {t("backup-create")}
    </Button>
    <Button size="sm" variant="default" loading={busy} disabled={problem !== null} on:click={openContainer}>
        {t("backup-open")}
    </Button>
</Group>
{#if step}
    <Text size="xs" color="blue">{step}</Text>
{/if}
<Space h="sm" />

{#if exported}
    <Alert color="green" title={t("backup-export-done")}>
        <Text size="sm">{exported.file}</Text>
        <Text size="xs" color="dimmed">
            {exported.components.length} {t("backup-components")} · {formatBytes(exported.total_bytes)} ·
            {t("backup-format-version")} {exported.format_version}
        </Text>
        <Text size="xs" color="dimmed">{t("backup-restore-needs-password")}</Text>
    </Alert>
    <Space h="sm" />
{/if}

{#if preview}
    <Alert color="yellow" title={t("backup-preview-title")}>
        <Text size="sm">
            {preview.app_version} · {preview.created_at} · {formatBytes(preview.total_bytes)}
        </Text>
        {#each preview.entries as entry (entry.name)}
            <Text size="xs" color="dimmed">
                • {t(componentLabelKey(entry.name))} ({formatBytes(entry.bytes)})
            </Text>
        {/each}
        {#each previewWarnings(preview) as key (key)}
            <Text size="xs" color="orange">{t(key)}</Text>
        {/each}
    </Alert>
    <Space h="xs" />
    <Text size="xs" color="dimmed">{t("backup-restore-warning")}</Text>
    <Group spacing="xs">
        <Button size="sm" color="red" loading={busy} disabled={!canRestore} on:click={restoreContainer}>
            {t("backup-restore")}
        </Button>
    </Group>
    <Space h="sm" />
{/if}

{#if restored}
    <Alert color={restored.report.rolled_back ? "orange" : "green"} title={t("backup-restore-done")}>
        {#if restored.report.rolled_back}
            <Text size="sm">{t("backup-rolled-back")}</Text>
        {:else}
            <Text size="xs" color="dimmed">
                {restored.report.restored.length} {t("backup-components")} · {t("backup-local-key")}:
                {restored.report.local_key}
            </Text>
            <Text size="xs" color="dimmed">{t("backup-storage-locked")}</Text>
            <Text size="xs" color="dimmed">{t("backup-whisper-off")}</Text>
        {/if}
    </Alert>
    <Space h="sm" />
{/if}

{#if view?.status.newest_safety_backup || view?.status.previous_state_present}
    <Text weight={600}>{t("backup-left-behind")}</Text>
    <Text size="xs" color="dimmed">{t("backup-left-behind-hint")}</Text>
    {#if view?.status.newest_safety_backup}
        <Group spacing="xs">
            <Text size="xs" color="dimmed">{view.status.newest_safety_backup}</Text>
            <Button size="xs" variant="subtle" loading={busy} on:click={deleteNewestSafetyBackup}>
                {t("backup-delete")}
            </Button>
        </Group>
    {/if}
    {#if view?.status.previous_state_present}
        <Group spacing="xs">
            <Button size="xs" variant="subtle" loading={busy} on:click={discardPrevious}>
                {t("backup-discard-previous")}
            </Button>
        </Group>
    {/if}
    <Space h="sm" />
{/if}

<style>
    .field {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.8rem;
        opacity: 0.9;
        margin-bottom: 0.35rem;
    }
</style>
