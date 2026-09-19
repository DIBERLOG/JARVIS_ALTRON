<script lang="ts">
    /**
     * Global voice input.
     *
     * What this panel does not do: it never stores the transcript in browser
     * storage, never puts it in a URL, and never sends it to the model. The text
     * of the last request lives in the core's memory, is shown here, and is put
     * on the protected clipboard for the person to paste. There is no automatic
     * Ctrl+V, and no keystroke is synthesized anywhere.
     */
    import { onMount } from "svelte"

    import { Alert, Button, Group, Input, Space, Switch, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import { voiceInputApi, isRunning, stageKey, startProblem } from "@/lib/voice-input"
    import type { VoiceInputSettings, VoiceInputView } from "@/lib/voice-input"

    $: t = (key: string) => translate($translations, key)

    let view: VoiceInputView | null = null
    /** The drafts: a poll must not overwrite what is being typed. */
    let phraseDraft = ""
    let languageDraft = "auto"
    let secondsDraft = 30
    let editingPhrase = false
    let busy = false
    let actionError = ""
    let preview: string | null = null
    let notice = ""

    $: running = view ? isRunning(view.status.stage) : false
    $: problem = view ? startProblem(view) : null

    onMount(() => {
        void load()
        const timer = setInterval(() => void load(), 2000)
        let stop: (() => void) | null = null
        void import("@tauri-apps/api/event").then(({ listen }) =>
            listen<{ key: string; characters: number }>("voice-input-notice", (event) => {
                notice = t(event.payload.key)
                void load()
            }).then((unlisten) => {
                stop = unlisten
            })
        )
        return () => {
            clearInterval(timer)
            if (stop) stop()
        }
    })

    async function load() {
        try {
            view = await voiceInputApi.status()
            // A field being edited keeps what was typed.
            if (!editingPhrase) phraseDraft = view.settings.phrase
            languageDraft = view.settings.language
            secondsDraft = view.settings.clipboard_seconds
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        }
    }

    function describe(error: unknown): string {
        const code = typeof error === "string" ? error : String(error)
        return /^[a-z_]+$/.test(code) ? t(`voice-input-error-${code.replace(/_/g, "-")}`) : code
    }

    /** Stores one change, and shows what the core kept. */
    async function store(changes: Partial<VoiceInputSettings>) {
        if (!view) return
        const candidate = { ...view.settings, ...changes }
        busy = true
        try {
            await voiceInputApi.updateSettings(candidate)
            actionError = ""
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    function enabled(): boolean {
        return view?.settings.enabled ?? false
    }
    function punctuation(): boolean {
        return view?.settings.punctuation ?? true
    }
    function autocorrect(): boolean {
        return view?.settings.autocorrect ?? true
    }
    function confirmation(): boolean {
        return view?.settings.speak_confirmation ?? true
    }
    function hasResult(): boolean {
        return view?.has_result ?? false
    }

    /** Commits the phrase field, on blur or on Enter, never on every key. */
    async function commitPhrase() {
        editingPhrase = false
        if (!view) return
        const trimmed = phraseDraft.trim()
        if (trimmed === view.settings.phrase) {
            phraseDraft = view.settings.phrase
            return
        }
        await store({ phrase: trimmed })
        phraseDraft = view ? view.settings.phrase : trimmed
    }

    /** Enter commits the field without leaving it. */
    function phraseKeydown(event: Event) {
        const key = (event as KeyboardEvent).key
        if (key === "Enter") {
            event.preventDefault()
            void commitPhrase()
        }
    }

    async function commitLanguage() {
        await store({ language: languageDraft })
    }

    async function commitSeconds() {
        if (!Number.isFinite(secondsDraft)) return
        await store({ clipboard_seconds: Math.round(secondsDraft) })
    }

    async function start() {
        busy = true
        notice = ""
        try {
            await voiceInputApi.start()
            actionError = ""
            preview = await voiceInputApi.preview()
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function cancel() {
        busy = true
        try {
            await voiceInputApi.cancel()
            notice = t("voice-input-cancelled")
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function showPreview() {
        try {
            preview = await voiceInputApi.preview()
        } catch (error) {
            actionError = describe(error)
        }
    }

    async function copyAgain() {
        busy = true
        try {
            await voiceInputApi.copyAgain()
            notice = t("voice-input-copied")
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function forget() {
        busy = true
        try {
            await voiceInputApi.clearResult()
            preview = null
            notice = ""
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }
</script>

<Text weight={700} size="lg">{t("voice-input-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("voice-input-description")}</Text>
<Space h="sm" />

{#if actionError}
    <Alert color="red" title={t("voice-input-title")}>{actionError}</Alert>
    <Space h="sm" />
{/if}

{#if view}
    <Alert
        color={view.settings.enabled ? (running ? "blue" : "green") : "yellow"}
        title={t(stageKey(view.status.stage))}
    >
        <Text size="xs" color="dimmed">
            {t("voice-input-vosk")}: {t(view.host.state_key)} ·
            {t("voice-input-whisper")}: {view.whisper_configured ? t("voice-input-ready") : t("voice-input-missing")}
        </Text>
        {#if view.host.state !== "listening"}
            <Text size="xs" color="orange">{t("voice-host-hint")}</Text>
        {/if}
        {#if view.has_result}
            <Text size="xs" color="dimmed">
                {t("voice-input-last-result")}: {view.characters} {t("voice-input-characters")}
            </Text>
        {/if}
        {#if view.settings.enabled && !view.whisper_configured}
            <Text size="xs" color="orange">{t("voice-input-whisper-hint")}</Text>
        {/if}
    </Alert>
    <Space h="sm" />
{/if}

<Switch label={t("voice-input-enabled")} checked={enabled()} on:change={() => store({ enabled: !enabled() })} />
<Text size="xs" color="dimmed">{t("voice-input-enabled-hint")}</Text>
<Space h="xs" />

<label class="field">
    <span>{t("voice-input-phrase")}</span>
    <Input
        size="sm"
        bind:value={phraseDraft}
        on:focus={() => (editingPhrase = true)}
        on:blur={() => void commitPhrase()}
        on:keydown={phraseKeydown}
    />
</label>
<Text size="xs" color="dimmed">{t("voice-input-phrase-hint")}</Text>
<Space h="xs" />

<label class="field">
    <span>{t("voice-input-language")}</span>
    <select bind:value={languageDraft} on:change={() => void commitLanguage()}>
        {#each ["auto", "ru", "en", "ua", "de", "fr", "es"] as language (language)}
            <option value={language}>{language}</option>
        {/each}
    </select>
</label>

<Switch
    label={t("voice-input-punctuation")}
    checked={punctuation()}
    on:change={() => store({ punctuation: !punctuation() })}
/>
<Switch
    label={t("voice-input-autocorrect")}
    checked={autocorrect()}
    on:change={() => store({ autocorrect: !autocorrect() })}
/>
<Switch
    label={t("voice-input-confirmation")}
    checked={confirmation()}
    on:change={() => store({ speak_confirmation: !confirmation() })}
/>
<Text size="xs" color="dimmed">{t("voice-input-confirmation-hint")}</Text>
<Space h="xs" />

<label class="field">
    <span>{t("voice-input-clipboard-timeout")}</span>
    <input
        class="line"
        type="number"
        min="15"
        max="60"
        step="5"
        bind:value={secondsDraft}
        on:change={() => void commitSeconds()}
    />
</label>
<Text size="xs" color="dimmed">{t("voice-input-clipboard-hint")}</Text>
<Space h="sm" />

<Group spacing="xs">
    <Button size="sm" color="green" loading={busy} disabled={problem !== null} on:click={start}>
        {t("voice-input-start")}
    </Button>
    <Button size="sm" color="red" variant="default" loading={busy} disabled={!running} on:click={cancel}>
        {t("voice-input-cancel")}
    </Button>
    <Button size="sm" variant="subtle" loading={busy} disabled={!hasResult()} on:click={showPreview}>
        {t("voice-input-preview")}
    </Button>
</Group>
{#if problem}
    <Text size="xs" color="orange">{t(problem)}</Text>
{/if}
{#if notice}
    <Text size="xs" color="green">{notice}</Text>
{/if}
<Space h="sm" />

{#if preview}
    <Text weight={600}>{t("voice-input-result")}</Text>
    <div class="result">
        <Text size="sm">{preview}</Text>
    </div>
    <Group spacing="xs">
        <Button size="xs" variant="default" loading={busy} on:click={copyAgain}>
            {t("voice-input-copy-again")}
        </Button>
        <Button size="xs" variant="subtle" loading={busy} on:click={forget}>
            {t("voice-input-clear")}
        </Button>
    </Group>
    <Text size="xs" color="dimmed">{t("voice-input-ctrl-v")}</Text>
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

    .result {
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        padding: 0.5rem;
        max-height: 10rem;
        overflow: auto;
        white-space: pre-wrap;
        opacity: 0.95;
    }
</style>
