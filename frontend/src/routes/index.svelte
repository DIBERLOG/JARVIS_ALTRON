<script lang="ts">
    import { onMount, onDestroy } from "svelte"
    import { invoke } from "@tauri-apps/api/core"

    import SearchBar from "@/components/elements/SearchBar.svelte"
    import ArcReactor from "@/components/elements/ArcReactor.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import Stats from "@/components/elements/Stats.svelte"
    import Footer from "@/components/Footer.svelte"
    import LocalChat from "@/components/ai/LocalChat.svelte"
    import ConversationPanel from "@/components/ai/ConversationPanel.svelte"
    import CloseDialog from "@/components/desktop/CloseDialog.svelte"
    import FirstRunWizard from "@/components/desktop/FirstRunWizard.svelte"
    import { onCloseRequested, onStateChanged, onOpenSettings, desktopApi } from "@/lib/desktop"
    import { needsWizard } from "@/lib/desktop-model"
    import type { DesktopState } from "@/lib/desktop-model"
    import { goto } from "@roxi/routify"
    
    import {
        isJarvisRunning,
        updateJarvisStats,
        enableIpc,
        disableIpc,
        translate,
        translations
    } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    let processRunning = false
    // The shell: the close answer, the first-run wizard, and the tray state.
    let closeDialogVisible = false
    let desktopState: DesktopState | null = null
    let unlisten: (() => void)[] = []

    $: wizardVisible = desktopState ? needsWizard(desktopState.setup) : false
    let launching = false
    let wasRunning = false  // track previous state

    isJarvisRunning.subscribe((value) => {
        processRunning = value
        if (value) {
            enableIpc()
            wasRunning = true
        } else if (wasRunning) {
            // only disable if it was running before
            disableIpc()
            wasRunning = false
        }
    })

    onMount(() => {
        updateJarvisStats()
    })

    onDestroy(() => {
        for (const stop of unlisten) {
            stop()
        }
        unlisten = []
        disableIpc()
    })

    async function runAssistant() {
        launching = true
        try {
            await invoke("run_jarvis_app")
            setTimeout(async () => {
                await updateJarvisStats()
                launching = false
            }, 2500)
        } catch (err) {
            console.error("Failed to run jarvis-app:", err)
            launching = false
        }
    }
</script>

<div class="app-container assist-page">

    <div class="search search-section">
        <HDivider />
        <SearchBar />
    </div>

    <div class="reactor-section">
        <div class="reactor-wrapper" class:dimmed={!processRunning}>
            <ArcReactor />
        </div>
        
        {#if !processRunning}
            <div class="offline-badge">
                <span class="offline-icon">⚠</span>
                <span class="offline-text">{t('assistant-not-running')}</span>
                <small>{t('assistant-offline-hint')}</small>
            </div>
            <button 
                class="start-button" 
                on:click={runAssistant}
                disabled={launching}
            >
                {launching ? t('btn-starting') : t('btn-start')}
            </button>
        {/if}
    </div>

    <HDivider noMargin />
    <div class="local-ai-section">
        <ConversationPanel />
        <HDivider noMargin />
        <LocalChat />
    </div>
    <HDivider noMargin />
    <Stats />
    <Footer />
</div>

<style lang="scss">
.local-ai-section {
    margin: 1rem 0;
}
    .wizard-backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.75);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 40;
        overflow: auto;
        padding: 1.5rem;
    }

    .wizard-panel {
        max-width: 34rem;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 12px;
        padding: 1rem;
        background: rgba(20, 20, 24, 0.98);
    }</style>
<!-- The close dialog is shown by the core's request, and it is the only place a
     close answer is chosen. The wizard appears on a profile that has not
     finished it, and closing it leaves every feature as it was. -->
<CloseDialog
    visible={closeDialogVisible}
    trayAvailable={desktopState?.tray_available ?? true}
    onDone={() => (closeDialogVisible = false)}
/>

{#if wizardVisible}
    <div class="wizard-backdrop">
        <div class="wizard-panel">
            <FirstRunWizard
                onClose={async () => {
                    desktopState = await desktopApi.state()
                }}
                onOpenSection={(section) => {
                    closeDialogVisible = false
                    $goto("/settings")
                    void section
                }}
            />
        </div>
    </div>
{/if}