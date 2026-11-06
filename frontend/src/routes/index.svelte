<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"

    import SearchBar from "@/components/elements/SearchBar.svelte"
    import ArcReactor from "@/components/elements/ArcReactor.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import Stats from "@/components/elements/Stats.svelte"
    import Footer from "@/components/Footer.svelte"
    
    import {
        isJarvisRunning,
        updateJarvisStats,
        translate,
        translations
    } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    let processRunning = false
    let launching = false
    // IPC belongs to App.svelte, not this route. Navigating must keep it alive.
    $: processRunning = $isJarvisRunning

    onMount(() => {
        updateJarvisStats()
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
        <div class="reactor-wrapper">
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
    <Stats />
    <Footer />
</div>
