<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Alert, Button, Input, Text } from "@svelteuidev/core"
    import { Download, ExclamationTriangle, LockClosed } from "radix-icons-svelte"

    import { vaultApi, vaultStorageLabel } from "@/lib/vault"
    import type { IdleStatus, VaultStatus } from "@/lib/vault-model"
    import {
        CLIPBOARD_TIMEOUT_OPTIONS,
        IDLE_TIMEOUT_OPTIONS,
        clipboardOptionKey,
        checkPasswordChange,
        idleOptionKey
    } from "@/lib/vault-model"
    import { translations, translate } from "@/stores"

    export let status: VaultStatus
    export let idleSeconds = 300
    export let clipboardSeconds = 30

    const dispatch = createEventDispatcher<{
        idle: number
        clipboard: number
        changed: VaultStatus
        failed: string
    }>()

    $: t = (key: string) => translate($translations, key)
    $: automatic = idleSeconds !== 0

    let open = false
    let current = ""
    let next = ""
    let confirmation = ""
    let exportPassword = ""
    let busy = false
    let notice = ""

    function report(error: unknown) {
        dispatch("failed", typeof error === "string" ? error : String(error))
    }

    function onIdleChange(event: Event) {
        dispatch("idle", Number((event.currentTarget as HTMLSelectElement).value))
    }

    function onClipboardChange(event: Event) {
        dispatch("clipboard", Number((event.currentTarget as HTMLSelectElement).value))
    }

    async function changeMasterPassword() {
        const check = checkPasswordChange(current, next, confirmation)
        if (!check.ok) {
            report(t(check.errorKey ?? "vault-error"))
            return
        }
        busy = true
        try {
            const outcome = await vaultApi.changeMasterPassword(current, next)
            current = ""
            next = ""
            confirmation = ""
            notice = outcome.dpapi_updated
                ? t("vault-change-done-dpapi")
                : t("vault-change-done")
        } catch (error) {
            report(error)
        } finally {
            busy = false
        }
    }

    async function exportBackup() {
        if (exportPassword.length === 0) {
            report(t("vault-export-needs-password"))
            return
        }
        busy = true
        try {
            const path = await vaultApi.exportBackupFile(exportPassword)
            exportPassword = ""
            if (path.length > 0) {
                notice = t("vault-export-done")
            }
        } catch (error) {
            report(error)
        } finally {
            busy = false
        }
    }

    async function lock() {
        try {
            dispatch("changed", await vaultApi.lock())
        } catch (error) {
            report(error)
        }
    }
</script>

<div class="security">
    <button class="security-toggle" on:click={() => (open = !open)}>
        <LockClosed size={13} />
        {t('vault-security')}
        <span class="arrow">{open ? '▾' : '▸'}</span>
    </button>

    {#if open}
        <div class="security-body">
            <Alert title={t('vault-experimental-title')} icon={ExclamationTriangle} color="orange" variant="outline">
                <Text size="xs" color="gray">{t('vault-experimental-warning')}</Text>
            </Alert>

            <label class="setting">
                <Text size="xs" color="gray">{t('vault-idle-timeout')}</Text>
                <select value={String(idleSeconds)} on:change={onIdleChange}>
                    {#each IDLE_TIMEOUT_OPTIONS as option}
                        <option value={String(option)}>{t(idleOptionKey(option))}</option>
                    {/each}
                </select>
                <Text size="xs" color="gray">{automatic ? t('vault-idle-automatic') : t('vault-idle-disabled')}</Text>
            </label>

            <label class="setting">
                <Text size="xs" color="gray">{t('vault-clipboard-timeout')}</Text>
                <select value={String(clipboardSeconds)} on:change={onClipboardChange}>
                    {#each CLIPBOARD_TIMEOUT_OPTIONS as option}
                        <option value={String(option)}>{t(clipboardOptionKey(option))}</option>
                    {/each}
                </select>
            </label>

            <div class="change-password">
                <Text size="xs" color="gray">{t('vault-change-password')}</Text>
                <Input type="password" size="xs" variant="filled" autocomplete="current-password" placeholder={t('vault-current-password')} bind:value={current} />
                <Input type="password" size="xs" variant="filled" autocomplete="new-password" placeholder={t('vault-new-password')} bind:value={next} />
                <Input type="password" size="xs" variant="filled" autocomplete="new-password" placeholder={t('vault-password-confirm')} bind:value={confirmation} />
                <div class="security-actions">
                    <Button size="xs" color="cyan" uppercase on:click={changeMasterPassword} disabled={busy}>
                        {t('vault-change-submit')}
                    </Button>
                    <Button size="xs" color="gray" uppercase on:click={lock}>
                        {t('vault-lock')}
                    </Button>
                </div>
            </div>

            <div class="export">
                <Text size="xs" color="gray">{t('vault-export-backup')}</Text>
                <div class="security-actions">
                    <Input type="password" size="xs" variant="filled" autocomplete="new-password" placeholder={t('vault-backup-password')} bind:value={exportPassword} />
                    <Button size="xs" color="gray" uppercase on:click={exportBackup} disabled={busy}>
                        <Download size={12} />
                        {t('vault-export')}
                    </Button>
                </div>
            </div>

            {#if notice}
                <Text size="xs" color="green">{notice}</Text>
            {/if}

            <Text size="xs" color="gray">
                {t('vault-storage-dir')}: {vaultStorageLabel(status)}
            </Text>
            <Text size="xs" color="gray">{t('vault-no-rotation')}</Text>
        </div>
    {/if}
</div>

<style lang="scss">
    .security {
        border-top: 1px dashed rgba(255, 255, 255, 0.12);
        padding-top: 6px;
    }

    .security-toggle {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.6);
        cursor: pointer;
        font-size: 0.62rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        padding: 0;

        &:hover {
            color: #52fefe;
        }
    }

    .arrow {
        font-size: 0.6rem;
    }

    .security-body {
        display: flex;
        flex-direction: column;
        gap: 8px;
        margin-top: 8px;
        background: rgba(15, 22, 26, 0.6);
        border-radius: 6px;
        padding: 8px;
    }

    .setting {
        display: flex;
        flex-direction: column;
        gap: 3px;

        select {
            width: 100%;
            background: rgba(10, 18, 22, 0.75);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 6px;
            color: rgba(255, 255, 255, 0.85);
            font-size: 0.66rem;
            padding: 5px 6px;
        }
    }

    .change-password,
    .export {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .security-actions {
        display: flex;
        gap: 5px;
        flex-wrap: wrap;
        align-items: center;
    }
</style>
