<script lang="ts">
    /**
     * Streaming chat with the local model.
     *
     * The component only talks to the local gateway: it never starts a process
     * itself and never calls the model server directly. Tokens arrive on a Tauri
     * channel, so the answer is rendered as it is produced, and the conversation
     * lives in this component's memory only — closing the window ends it.
     */
    import { afterUpdate, onMount, onDestroy } from "svelte"

    import { translate, translations } from "@/stores"
    import { generationChannel, localAiApi } from "@/lib/local-ai"
    import {
        applyGenerationEvent,
        beginExchange,
        buildRequest,
        canGenerate,
        canSend,
        canStart,
        canStop,
        defaultSettings,
        emptyChatView,
        failExchange,
        formatDuration,
        formatUsage,
        normalizeSettings,
        profileLabelKey,
        serverSummary,
        stateLabelKey,
        thinkingAvailable,
        thinkingLabelKey,
        validateDraft,
        PROFILES,
        THINKING_MODES
    } from "@/lib/local-ai-model"
    import type {
        AiProfile,
        ChatView,
        GenerationEvent,
        LocalAiSettings,
        LocalAiStatus,
        ThinkingMode
    } from "@/lib/local-ai-model"

    import { Button, Text } from "@svelteuidev/core"

    $: t = (key: string) => translate($translations, key)

    let settings: LocalAiSettings = defaultSettings()
    let status: LocalAiStatus | null = null
    let chat: ChatView = emptyChatView()
    let draft = ""
    let busy = false
    let actionError = ""
    let messageBox: HTMLDivElement | null = null
    let statusTimer: ReturnType<typeof setInterval> | null = null

    /** The settings are incomplete or refused, so the server cannot start. */
    $: draftIssues = validateDraft(settings)
    $: blocked = draftIssues.some((issue) => issue.level === "blocked")
    $: configured = settings.server.server_path.trim().length > 0 && settings.server.model_path.trim().length > 0
    $: sendable = canSend(chat, status, draft, settings)
    // Only warn about reasoning once a server is running and cannot honour the
    // preference: a stopped server says nothing about support.
    $: thinkingUnsupported = canGenerate(status) && !thinkingAvailable(status, settings.thinking)

    onMount(async () => {
        await refreshSettings()
        await refreshStatus()
        // A slow status refresh keeps uptime, capabilities, and a crashed server
        // visible; it is never used to follow a generation, which streams.
        statusTimer = setInterval(() => {
            if (status && status.state !== "stopped") {
                void refreshStatus()
            }
        }, 5000)
    })

    onDestroy(() => {
        if (statusTimer) clearInterval(statusTimer)
        statusTimer = null
    })

    async function refreshSettings() {
        try {
            settings = normalizeSettings(await localAiApi.getConfig())
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function refreshStatus() {
        try {
            status = await localAiApi.status()
            if (status.last_error && !chat.error) actionError = status.last_error
        } catch (error) {
            actionError = describeError(error)
        }
    }

    function describeError(error: unknown): string {
        if (typeof error === "string") return error
        if (error instanceof Error) return error.message
        return String(error)
    }

    /** Persists the two switches that the chat itself owns. */
    async function persistSwitches() {
        try {
            await localAiApi.saveConfig(settings)
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function onProfileChange(event: Event) {
        const value = (event.target as HTMLSelectElement).value as AiProfile
        settings = { ...settings, profile: value }
        await persistSwitches()
    }

    async function onThinkingChange(event: Event) {
        const value = (event.target as HTMLSelectElement).value as ThinkingMode
        settings = { ...settings, thinking: value }
        await persistSwitches()
    }

    async function startServer() {
        busy = true
        actionError = ""
        try {
            status = await localAiApi.start()
        } catch (error) {
            actionError = describeError(error)
            await refreshStatus()
        }
        busy = false
    }

    async function stopServer() {
        busy = true
        actionError = ""
        try {
            status = await localAiApi.stop()
            chat = { ...chat, generating: false }
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function restartServer() {
        busy = true
        actionError = ""
        try {
            status = await localAiApi.restart()
        } catch (error) {
            actionError = describeError(error)
            await refreshStatus()
        }
        busy = false
    }

    function scrollToLatest() {
        if (messageBox) messageBox.scrollTop = messageBox.scrollHeight
    }

    // Scrolling after the DOM update keeps the newest token visible while the
    // answer streams, without a timer and without polling.
    afterUpdate(scrollToLatest)

    async function send() {
        if (!sendable) return
        const prompt = draft.trim()
        const request = buildRequest(chat, settings, prompt)
        draft = ""
        chat = beginExchange(chat, prompt, Date.now())

        const channel = generationChannel((event: GenerationEvent) => {
            chat = applyGenerationEvent(chat, event, Date.now())
            if (event.type === "completed" || event.type === "cancelled" || event.type === "failed") {
                void refreshStatus()
            }
        })

        try {
            await localAiApi.generate(channel, request)
        } catch (error) {
            chat = failExchange(chat, describeError(error))
        }
    }

    async function stopGeneration() {
        try {
            await localAiApi.cancel()
        } catch (error) {
            actionError = describeError(error)
        }
    }

    function clearConversation() {
        chat = emptyChatView()
        actionError = ""
    }
</script>

<div class="local-ai">
    <div class="ai-header">
        <div class="ai-title">
            <span class="ai-dot" class:live={canGenerate(status)} />
            <Text weight={600}>{t('ai-chat-title')}</Text>
            <span class="ai-state">{status ? t(stateLabelKey(status.state)) : t('ai-chat-state-stopped')}</span>
        </div>
        <div class="ai-controls">
            {#if canStart(status)}
                <Button
                    size="xs"
                    color="lime"
                    uppercase
                    on:click={startServer}
                    disabled={busy || blocked}
                >
                    {t('ai-chat-start')}
                </Button>
            {/if}
            {#if canStop(status)}
                <Button size="xs" color="gray" uppercase on:click={stopServer} disabled={busy}>
                    {t('ai-chat-stop-server')}
                </Button>
                <Button size="xs" color="gray" uppercase on:click={restartServer} disabled={busy}>
                    {t('ai-chat-restart')}
                </Button>
            {/if}
            {#if chat.entries.length > 0}
                <Button size="xs" color="gray" variant="subtle" uppercase on:click={clearConversation}>
                    {t('ai-chat-clear')}
                </Button>
            {/if}
        </div>
    </div>

    <p class="ai-note">{t('ai-chat-local-note')}</p>

    <div class="ai-switches">
        <label class="ai-field">
            <span>{t('ai-chat-profile')}</span>
            <select value={settings.profile} on:change={onProfileChange} disabled={busy}>
                {#each PROFILES as profile}
                    <option value={profile}>{t(profileLabelKey(profile))}</option>
                {/each}
            </select>
            <small>{t('ai-chat-profile-desc')}</small>
        </label>

        <label class="ai-field">
            <span>{t('ai-chat-thinking')}</span>
            <select value={settings.thinking} on:change={onThinkingChange} disabled={busy}>
                {#each THINKING_MODES as mode}
                    <option value={mode}>{t(thinkingLabelKey(mode))}</option>
                {/each}
            </select>
            <small>{t('ai-chat-thinking-desc')}</small>
        </label>
    </div>

    {#if thinkingUnsupported}
        <p class="ai-warn">{t('ai-chat-thinking-unavailable')}</p>
    {/if}

    {#if !configured}
        <p class="ai-warn">{t('ai-chat-needs-config')}</p>
    {:else if !canGenerate(status) && status && status.state !== 'starting'}
        <p class="ai-warn">{t('ai-chat-not-running')}</p>
    {/if}

    {#if status}
        <p class="ai-server">
            {serverSummary(status)}
            {#if status.uptime}· {t('ai-chat-uptime')} {status.uptime}{/if}
        </p>
        <div class="ai-capabilities">
            <span class:on={status.capabilities.streaming}>{t('ai-chat-cap-streaming')}</span>
            <span class:on={status.capabilities.thinking_switch}>{t('ai-chat-cap-thinking')}</span>
            <span class:on={status.capabilities.reasoning_field_observed}>{t('ai-chat-cap-reasoning')}</span>
        </div>
        {#if status.out_of_memory_hint}
            <p class="ai-warn">{t('ai-chat-out-of-memory')}</p>
        {/if}
        {#if status.stderr_tail.length > 0}
            <details class="ai-stderr">
                <summary>{t('ai-chat-stderr')}{status.stderr_truncated ? ' …' : ''}</summary>
                <pre>{status.stderr_tail.join("\n")}</pre>
            </details>
        {/if}
    {/if}

    {#if actionError}
        <p class="ai-warn">{actionError}</p>
    {/if}

    <div class="ai-messages" bind:this={messageBox}>
        {#each chat.entries as entry, index (index)}
            <div class="ai-message" class:user={entry.role === "user"}>
                {#if entry.role === "assistant" && entry.thinking.length > 0}
                    <details class="ai-thinking-box">
                        <summary>{t('ai-chat-reasoning')}</summary>
                        <pre>{entry.thinking}</pre>
                    </details>
                {/if}
                <div class="ai-bubble">{entry.text}</div>
            </div>
        {/each}
        {#if chat.generating}
            <p class="ai-generating">{t('ai-chat-generating')}</p>
        {/if}
        {#if chat.cancelled}
            <p class="ai-warn">{t('ai-chat-cancelled')}</p>
        {/if}
        {#if !chat.generating && !chat.cancelled && chat.finishReason === null && chat.error === null && chat.entries.length > 0 && chat.entries[chat.entries.length - 1].text.trim().length === 0}
            <p class="ai-warn">{t('ai-chat-no-answer')}</p>
        {/if}
    </div>

    <div class="ai-input">
        <textarea
            bind:value={draft}
            placeholder={t('ai-chat-input')}
            rows="3"
            disabled={chat.generating}
            on:keydown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                    event.preventDefault()
                    void send()
                }
            }}
        />
        <div class="ai-input-actions">
            {#if chat.generating}
                <Button size="xs" color="red" uppercase on:click={stopGeneration}>
                    {t('ai-chat-stop')}
                </Button>
            {:else}
                <Button size="xs" color="lime" uppercase on:click={send} disabled={!sendable}>
                    {t('ai-chat-send')}
                </Button>
            {/if}
            <span class="ai-meta">
                {#if chat.elapsedMs > 0}
                    {t('ai-chat-elapsed')} {formatDuration(chat.elapsedMs)}
                {/if}
                {#if formatUsage(chat.usage)}
                    · {t('ai-chat-tokens')} {formatUsage(chat.usage)}
                {/if}
            </span>
        </div>
    </div>
</div>

<style lang="scss">
.local-ai {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 1rem;
    background: rgba(20, 28, 32, 0.75);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 10px;
}

.ai-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
}

.ai-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
}

.ai-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.25);

    &.live {
        background: #52fefe;
        box-shadow: 0 0 8px rgba(82, 254, 254, 0.7);
    }
}

.ai-state {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: rgba(255, 255, 255, 0.45);
}

.ai-controls {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
}

.ai-note {
    margin: 0;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.45);
}

.ai-switches {
    display: flex;
    gap: 0.75rem;
    flex-wrap: wrap;
}

.ai-field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    flex: 1 1 190px;

    span {
        font-size: 0.72rem;
        color: rgba(255, 255, 255, 0.7);
    }

    select {
        background: rgba(30, 40, 45, 0.9);
        color: #fff;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 6px;
        padding: 0.35rem 0.5rem;
        font-size: 0.8rem;
        font-family: inherit;
    }

    small {
        font-size: 0.65rem;
        color: rgba(255, 255, 255, 0.35);
    }
}

.ai-warn {
    margin: 0;
    font-size: 0.75rem;
    color: #ffb84d;
}

.ai-server {
    margin: 0;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.55);
    font-family: monospace;
}

.ai-capabilities {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;

    span {
        font-size: 0.62rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        padding: 0.1rem 0.4rem;
        border-radius: 4px;
        border: 1px solid rgba(255, 255, 255, 0.12);
        color: rgba(255, 255, 255, 0.35);

        &.on {
            color: #52fefe;
            border-color: rgba(82, 254, 254, 0.4);
        }
    }
}

.ai-stderr {
    font-size: 0.7rem;
    color: rgba(255, 255, 255, 0.5);

    summary {
        cursor: pointer;
    }

    pre {
        max-height: 140px;
        overflow: auto;
        white-space: pre-wrap;
        word-break: break-all;
        margin: 0.35rem 0 0;
        padding: 0.4rem;
        background: rgba(0, 0, 0, 0.35);
        border-radius: 6px;
    }
}

.ai-messages {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    max-height: 320px;
    min-height: 60px;
    overflow-y: auto;
    padding: 0.4rem;
    background: rgba(0, 0, 0, 0.25);
    border-radius: 8px;
}

.ai-message {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.25rem;

    &.user {
        align-items: flex-end;

        .ai-bubble {
            background: rgba(82, 254, 254, 0.12);
            border-color: rgba(82, 254, 254, 0.25);
        }
    }
}

.ai-bubble {
    max-width: 85%;
    padding: 0.45rem 0.6rem;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(30, 40, 45, 0.85);
    font-size: 0.82rem;
    line-height: 1.35;
    white-space: pre-wrap;
    word-break: break-word;
}

.ai-thinking-box {
    max-width: 85%;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.5);

    summary {
        cursor: pointer;
    }

    pre {
        white-space: pre-wrap;
        word-break: break-word;
        margin: 0.3rem 0 0;
        max-height: 180px;
        overflow: auto;
        padding: 0.4rem;
        background: rgba(0, 0, 0, 0.35);
        border-radius: 6px;
    }
}

.ai-generating {
    margin: 0;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.5);
    animation: pulse 1.4s ease-in-out infinite;
}

@keyframes pulse {
    50% {
        opacity: 0.4;
    }
}

.ai-input {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;

    textarea {
        width: 100%;
        resize: vertical;
        background: rgba(30, 40, 45, 0.9);
        color: #fff;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        padding: 0.5rem;
        font-family: inherit;
        font-size: 0.82rem;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.4);
        }
    }
}

.ai-input-actions {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
}

.ai-meta {
    font-size: 0.68rem;
    color: rgba(255, 255, 255, 0.4);
}
</style>
