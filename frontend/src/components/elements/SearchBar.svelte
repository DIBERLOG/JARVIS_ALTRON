<script lang="ts">
    import { translations, translate, isJarvisRunning, ipcConnected, sendTextCommand } from "@/stores"
    import { lastError, lastRecognizedText, jarvisState } from "@/lib/ipc"
    
    $: t = (key: string) => translate($translations, key)
    
    let searchQuery = ""
    let isProcessing = false
    let statusMessage = ""

    async function handleSubmit(e: Event) {
        e.preventDefault()
        
        const command = searchQuery.trim()
        if (!command || isProcessing) return
        
        if (!$isJarvisRunning || !$ipcConnected) {
            statusMessage = t('search-error-not-running')
            setTimeout(() => statusMessage = "", 3000)
            return
        }

        isProcessing = true
        statusMessage = ""

        try {
            if (!sendTextCommand(command)) throw new Error("Voice core disconnected")
            searchQuery = ""
        } catch (err) {
            console.error("Failed to send command:", err)
            statusMessage = t('search-error-failed')
        } finally {
            isProcessing = false
        }
    }

    function handleKeydown(e: KeyboardEvent) {
        if (e.key === "Escape") {
            searchQuery = ""
        }
    }
</script>

<div id="search-form" class="search" class:active={searchQuery !== ""} class:processing={isProcessing}>
    <form on:submit={handleSubmit}>
        <input
            bind:value={searchQuery}
            on:keydown={handleKeydown}
            type="text"
            name="q"
            placeholder={t('search-placeholder')}
            autocomplete="off"
            minlength="1"
            maxlength="200"
            disabled={isProcessing}
        />
        <small>{isProcessing ? '...' : 'Enter'}</small>
    </form>
    <div class="search-status" role="status" aria-live="polite">
        {#if statusMessage || $lastError}
            {statusMessage || $lastError}
        {:else if $jarvisState === 'listening'}
            Слушаю команду…
        {:else if $lastRecognizedText}
            {$jarvisState === 'processing' ? 'Обрабатываю' : 'Последняя команда'}: {$lastRecognizedText}
        {/if}
    </div>
</div>

<style lang="scss">
    .search.processing input {
        opacity: 0.6;
        cursor: wait;
    }

    .search-status {
        position: absolute;
        top: 100%;
        left: 0;
        width: 100%;
        text-align: center;
        font-size: 0.75rem;
        color: rgba(82, 254, 254, 0.8);
        white-space: normal;
        overflow-wrap: anywhere;
    }

    @keyframes fadeIn {
        from { opacity: 0; transform: translateX(-50%) translateY(-5px); }
        to { opacity: 1; transform: translateX(-50%) translateY(0); }
    }
</style>
