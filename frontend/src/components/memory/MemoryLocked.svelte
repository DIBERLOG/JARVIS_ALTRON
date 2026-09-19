<script lang="ts">
    /**
     * The storage gate of the memory page.
     *
     * Memory shares the encrypted storage of the notes and the password vault, so
     * unlocking here is the same operation as unlocking there: the same master
     * password, the same DPAPI key, the same key backup. This component therefore
     * talks to the vault commands directly through `invoke` and never imports the
     * vault or the notes API: the memory interface must not gain a handle on note or
     * credential data.
     *
     * Nothing is counted or listed before the storage is unlocked.
     */
    import { invoke } from "@tauri-apps/api/core"
    import { createEventDispatcher } from "svelte"
    import { Alert, Button, Input, Space, Text } from "@svelteuidev/core"
    import { goto } from "@roxi/routify"

    import { isKeyMissing, needsSetup, storageStateLabelKey } from "@/lib/memory-model"
    import type { MemoryStatusView } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let status: MemoryStatusView
    export let busy = false

    const dispatch = createEventDispatcher<{ unlocked: void; failed: string }>()

    $: t = (key: string) => translate($translations, key)
    $: setup = needsSetup(status)
    $: keyMissing = isKeyMissing(status)

    let password = ""

    function report(error: unknown) {
        dispatch("failed", typeof error === "string" ? error : String(error))
    }

    async function unlockWithPassword() {
        if (password.length === 0) {
            report(t("memory-unlock-password"))
            return
        }
        try {
            // The command answers with the shared storage status; the page only needs
            // to know that it succeeded before it asks for a fresh view.
            await invoke("vault_unlock_password", { password })
            password = ""
            dispatch("unlocked")
        } catch (error) {
            report(error)
        }
    }

    async function unlockWithDpapi() {
        try {
            await invoke("vault_unlock_dpapi")
            dispatch("unlocked")
        } catch (error) {
            report(error)
        }
    }
</script>

<div class="gate">
    <div class="gate-card">
        <span class="state-chip">{t(storageStateLabelKey(status.storage.state))}</span>

        <h2 class="gate-title">{t('memory-storage-locked')}</h2>

        <Text size="sm" color="gray">{t('memory-local-note')}</Text>
        <Space h="sm" />
        <Text size="xs" color="gray">{t('memory-unlock-hint')}</Text>

        {#if setup}
            <Space h="md" />
            <Alert title={t('memory-storage-uninitialized')} color="orange" variant="outline">
                <Text size="xs" color="gray">{t('memory-needs-setup-hint')}</Text>
                <Space h="sm" />
                <button class="link" on:click={() => $goto('/notes')}>{t('memory-needs-setup-hint')}</button>
            </Alert>
        {:else if keyMissing}
            <Space h="md" />
            <Alert title={t('memory-storage-key-missing')} color="red" variant="outline">
                <Text size="xs" color="gray">{t('memory-key-missing-hint')}</Text>
                <Space h="sm" />
                <button class="link" on:click={() => $goto('/vault')}>{t('memory-import')}</button>
            </Alert>
        {:else}
            <Space h="md" />
            {#if status.storage.dpapi_available}
                <Button
                    color="cyan"
                    radius="md"
                    size="sm"
                    uppercase
                    fullSize
                    disabled={busy}
                    on:click={unlockWithDpapi}
                >
                    {t('memory-unlock')}
                </Button>
                <Space h="sm" />
            {/if}

            <Input
                type="password"
                placeholder={t('memory-unlock-password')}
                variant="filled"
                autocomplete="current-password"
                bind:value={password}
            />
            <Space h="md" />
            <Button
                color="lime"
                radius="md"
                size="sm"
                uppercase
                fullSize
                disabled={busy}
                on:click={unlockWithPassword}
            >
                {t('memory-unlock')}
            </Button>
        {/if}
    </div>

    <Text size="xs" color="gray">{t('memory-settings-off-note')}</Text>
</div>

<style lang="scss">
    .gate {
        display: flex;
        flex-direction: column;
        gap: 10px;
        align-items: center;
        padding: 8px 4px 20px;
    }

    .gate-card {
        width: 100%;
        max-width: 460px;
        background: rgba(20, 30, 35, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 10px;
        padding: 18px 16px;
        text-align: center;
    }

    .gate-title {
        font-size: 0.95rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        margin: 8px 0;
        color: #ffffff;
    }

    .state-chip {
        display: inline-block;
        background: rgba(82, 254, 254, 0.12);
        border-radius: 10px;
        color: #52fefe;
        font-size: 0.58rem;
        letter-spacing: 0.06em;
        padding: 2px 8px;
        text-transform: uppercase;
    }

    .link {
        background: transparent;
        border: none;
        color: #52fefe;
        cursor: pointer;
        font-size: 0.62rem;
        letter-spacing: 0.04em;
        padding: 0;
        text-transform: uppercase;
    }
</style>
