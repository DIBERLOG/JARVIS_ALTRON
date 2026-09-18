<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Alert, Button, Input, Space, Text } from "@svelteuidev/core"
    import { ExclamationTriangle, LockClosed, LockOpen1, Upload } from "radix-icons-svelte"

    import { vaultApi } from "@/lib/vault"
    import type { VaultStatus } from "@/lib/vault-model"
    import { MIN_MASTER_PASSWORD_LENGTH, isKeyMissing, needsMasterPasswordSetup } from "@/lib/vault-model"
    import { vaultStorageLabel } from "@/lib/vault"
    import { translations, translate } from "@/stores"

    export let status: VaultStatus

    const dispatch = createEventDispatcher<{ changed: VaultStatus; failed: string }>()

    $: t = (key: string) => translate($translations, key)
    $: freshInstall = needsMasterPasswordSetup(status)
    $: keyMissing = isKeyMissing(status)

    let password = ""
    let confirmation = ""
    let envelope = ""
    let busy = false

    function report(error: unknown) {
        dispatch("failed", typeof error === "string" ? error : String(error))
    }

    async function run(action: () => Promise<VaultStatus>) {
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
            report(t("vault-password-short"))
            return
        }
        if (password !== confirmation) {
            report(t("vault-password-mismatch"))
            return
        }
        await run(() => vaultApi.initialize(password))
    }

    async function unlockWithPassword() {
        if (envelope.trim().length > 0) {
            await run(() => vaultApi.importBackup(envelope.trim(), password))
            return
        }
        await run(() => vaultApi.unlockWithPassword(password))
    }

    async function importFromFile() {
        await run(() => vaultApi.importBackupFile(password))
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
            {#if freshInstall}
                {t('vault-created-title')}
            {:else if keyMissing}
                {t('vault-key-missing-title')}
            {:else}
                {t('vault-locked-title')}
            {/if}
        </h2>

        <Text size="sm" color="gray">
            {#if freshInstall}
                {t('vault-created-desc')}
            {:else if keyMissing}
                {t('vault-key-missing-desc')}
            {:else}
                {t('vault-locked-desc')}
            {/if}
        </Text>

        <Space h="md" />

        {#if freshInstall}
            <Input
                type="password"
                placeholder={t('vault-master-password')}
                variant="filled"
                autocomplete="new-password"
                bind:value={password}
            />
            <Space h="sm" />
            <Input
                type="password"
                placeholder={t('vault-password-confirm')}
                variant="filled"
                autocomplete="new-password"
                bind:value={confirmation}
            />
            <Space h="sm" />
            <Text size="xs" color="gray">{t('vault-password-hint')}</Text>
            <Space h="md" />
            <Button color="lime" radius="md" size="sm" uppercase fullSize on:click={createStorage} disabled={busy}>
                {t('vault-create')}
            </Button>
        {:else}
            {#if status.storage.dpapi_available}
                <Button
                    color="cyan"
                    radius="md"
                    size="sm"
                    uppercase
                    fullSize
                    on:click={() => run(() => vaultApi.unlockWithDpapi())}
                    disabled={busy}
                >
                    {t('vault-unlock-dpapi')}
                </Button>
                <Space h="sm" />
            {/if}

            <Input
                type="password"
                placeholder={t('vault-master-password')}
                variant="filled"
                autocomplete="current-password"
                bind:value={password}
            />

            {#if !status.storage.backup_available}
                <Space h="sm" />
                <Alert title={t('vault-import-required')} icon={ExclamationTriangle} color="orange" variant="outline">
                    <Text size="xs" color="gray">{t('vault-import-required-desc')}</Text>
                </Alert>
                <Space h="sm" />
                <textarea
                    class="envelope-input"
                    placeholder={t('vault-envelope-placeholder')}
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
                disabled={busy}
            >
                <LockOpen1 size={14} />
                {t('vault-unlock')}
            </Button>
            <Space h="sm" />
            <Button color="gray" radius="md" size="sm" uppercase fullSize on:click={importFromFile} disabled={busy}>
                <Upload size={14} />
                {t('vault-import-file')}
            </Button>
        {/if}
    </div>

    <div class="storage-hint">
        <Text size="xs" color="gray">
            {t('vault-storage-dir')}: <span class="path">{vaultStorageLabel(status)}</span>
        </Text>
        <Text size="xs" color="gray">{t('vault-shared-storage-hint')}</Text>
    </div>

    <div class="warning">
        <ExclamationTriangle size={14} />
        <Text size="xs" color="gray">{t('vault-experimental-warning')}</Text>
    </div>
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
    .warning {
        margin-top: 12px;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 4px;
        text-align: center;
    }

    .warning {
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
