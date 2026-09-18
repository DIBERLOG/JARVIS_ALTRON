<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Alert, Button, Input, Space, Text } from "@svelteuidev/core"
    import {
        Download,
        ExclamationTriangle,
        LockClosed,
        LockOpen1,
        Upload
    } from "radix-icons-svelte"

    import { notesApi } from "@/lib/notes"
    import { MIN_MASTER_PASSWORD_LENGTH, shortenPath } from "@/lib/notes-model"
    import type { StorageStatus } from "@/lib/notes-model"
    import { translations, translate } from "@/stores"

    export let status: StorageStatus

    const dispatch = createEventDispatcher<{ changed: StorageStatus; failed: string }>()

    $: t = (key: string) => translate($translations, key)

    let password = ""
    let confirmation = ""
    let envelope = ""
    let busy = false

    $: isFreshInstall = status.state === "uninitialized"
    $: keyMissing = status.state === "key_missing"
    $: canUnlockWithPassword = status.backup_available || keyMissing || envelope.trim().length > 0

    function report(error: unknown) {
        const message = typeof error === "string" ? error : String(error)
        dispatch("failed", message)
    }

    async function run(action: () => Promise<StorageStatus>) {
        busy = true
        try {
            const next = await action()
            password = ""
            confirmation = ""
            dispatch("changed", next)
        } catch (error) {
            report(error)
        } finally {
            busy = false
        }
    }

    async function createStorage() {
        if (password.length < MIN_MASTER_PASSWORD_LENGTH) {
            dispatch("failed", t("notes-password-short"))
            return
        }
        if (password !== confirmation) {
            dispatch("failed", t("notes-password-mismatch"))
            return
        }
        await run(() => notesApi.initialize(password))
    }

    async function unlockWithPassword() {
        if (envelope.trim().length > 0) {
            await run(() => notesApi.importBackup(envelope.trim(), password))
            return
        }
        await run(() => notesApi.unlockWithPassword(password))
    }

    async function unlockWithDpapi() {
        await run(() => notesApi.unlockWithDpapi())
    }

    async function importFromFile() {
        await run(() => notesApi.importBackupFile(password))
    }
</script>

<div class="locked">
    <Space h="lg" />

    <div class="locked-card">
        <div class="locked-icon">
            {#if keyMissing}
                <ExclamationTriangle size={26} />
            {:else}
                <LockClosed size={26} />
            {/if}
        </div>

        <h2 class="locked-title">
            {#if isFreshInstall}
                {t('notes-uninitialized-title')}
            {:else if keyMissing}
                {t('notes-key-missing-title')}
            {:else}
                {t('notes-locked-title')}
            {/if}
        </h2>

        <Text size="sm" color="gray">
            {#if isFreshInstall}
                {t('notes-uninitialized-desc')}
            {:else if keyMissing}
                {t('notes-key-missing-desc')}
            {:else}
                {t('notes-locked-desc')}
            {/if}
        </Text>

        <Space h="md" />

        {#if isFreshInstall}
            <Input
                type="password"
                placeholder={t('notes-password')}
                variant="filled"
                autocomplete="new-password"
                bind:value={password}
            />
            <Space h="sm" />
            <Input
                type="password"
                placeholder={t('notes-password-confirm')}
                variant="filled"
                autocomplete="new-password"
                bind:value={confirmation}
            />
            <Space h="sm" />
            <Text size="xs" color="gray">{t('notes-password-hint')}</Text>
            <Space h="md" />
            <Button
                color="lime"
                radius="md"
                size="sm"
                uppercase
                fullSize
                on:click={createStorage}
                disabled={busy}
            >
                {t('notes-create')}
            </Button>
        {:else}
            {#if status.dpapi_available}
                <Button
                    color="cyan"
                    radius="md"
                    size="sm"
                    uppercase
                    fullSize
                    on:click={unlockWithDpapi}
                    disabled={busy}
                >
                    {t('notes-unlock-dpapi')}
                </Button>
                <Space h="sm" />
            {/if}

            <Input
                type="password"
                placeholder={t('notes-password')}
                variant="filled"
                autocomplete="current-password"
                bind:value={password}
            />

            {#if !status.backup_available}
                <Space h="sm" />
                <Alert
                    title={t('notes-import-required')}
                    icon={ExclamationTriangle}
                    color="orange"
                    variant="outline"
                >
                    <Text size="xs" color="gray">{t('notes-import-required-desc')}</Text>
                </Alert>
                <Space h="sm" />
                <textarea
                    class="envelope-input"
                    placeholder={t('notes-envelope-placeholder')}
                    spellcheck="false"
                    bind:value={envelope}
                ></textarea>
            {/if}

            <Space h="md" />
            <Button
                color="lime"
                radius="md"
                size="sm"
                uppercase
                fullSize
                on:click={unlockWithPassword}
                disabled={busy || !canUnlockWithPassword}
            >
                <LockOpen1 size={14} />
                {t('notes-unlock')}
            </Button>
            <Space h="sm" />
            <Button
                color="gray"
                radius="md"
                size="sm"
                uppercase
                fullSize
                on:click={importFromFile}
                disabled={busy}
            >
                <Upload size={14} />
                {t('notes-import-file')}
            </Button>
        {/if}
    </div>

    <div class="storage-hint">
        <Text size="xs" color="gray">
            {t('notes-storage-dir')}: <span class="path">{shortenPath(status.data_dir)}</span>
        </Text>
        {#if status.has_stored_data && !keyMissing}
            <Text size="xs" color="gray">{t('notes-has-data-hint')}</Text>
        {/if}
    </div>

    {#if isFreshInstall}
        <div class="portable-hint">
            <Download size={14} />
            <Text size="xs" color="gray">{t('notes-export-hint')}</Text>
        </div>
    {/if}
</div>

<style lang="scss">
    .locked {
        padding: 0 4px 20px;
    }

    .locked-card {
        background: rgba(20, 30, 35, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 10px;
        padding: 18px 16px;
        text-align: center;
    }

    .locked-icon {
        color: #52fefe;
        margin-bottom: 8px;
    }

    .locked-title {
        font-size: 0.95rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        margin: 0 0 8px;
        color: #ffffff;
    }

    .envelope-input {
        width: 100%;
        min-height: 76px;
        resize: vertical;
        background: rgba(10, 18, 22, 0.85);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: rgba(255, 255, 255, 0.8);
        font-family: monospace;
        font-size: 0.65rem;
        padding: 8px;
    }

    .storage-hint,
    .portable-hint {
        margin-top: 14px;
        text-align: center;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 4px;
    }

    .portable-hint {
        flex-direction: row;
        justify-content: center;
        gap: 6px;
        opacity: 0.85;
    }

    .path {
        color: #52fefe;
        word-break: break-all;
    }
</style>
