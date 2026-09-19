<script lang="ts">
    /**
     * Dictation settings and the dictation itself.
     *
     * What this panel does not do: it never names a path (the native dialog in
     * the core picks both files), it never starts a recording on its own (only
     * the buttons here or the tray item can), and it never keeps a transcript:
     * the text is shown until the panel is reloaded, and nothing writes it to
     * browser storage, a file, or a URL.
     */
    import { onMount } from "svelte"

    import { Alert, Button, Group, Space, Switch, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import { whisperApi } from "@/lib/whisper"
    import type { WhisperPanelView } from "@/lib/whisper"
    import type { WhisperSettings } from "@/lib/whisper-model"
    import {
        LANGUAGES,
        MAX_SILENCE_MS,
        MAX_THREADS,
        MAX_TIMEOUT_SECONDS,
        MIN_SILENCE_MS,
        MIN_THREADS,
        MIN_TIMEOUT_SECONDS,
        audioLabel,
        defaultSettings,
        errorKey,
        isBusy,
        modelIsUnverified,
        modelKindKey,
        noteKey,
        normalizedSettings,
        readinessKey,
        settingsProblem,
        stateKey,
        transcriptLength,
        transcriptPreview
    } from "@/lib/whisper-model"

    $: t = (key: string) => translate($translations, key)

    let view: WhisperPanelView | null = null
    let settings: WhisperSettings = defaultSettings()
    let expanded = false
    let busy = false
    let actionError = ""
    let saved = false

    $: status = view?.status ?? null
    $: transcript = view?.last ?? null
    $: preview = transcript ? transcriptPreview(transcript) : null
    $: running = status ? isBusy(status.state) : false

    onMount(() => {
        // The panel does not own the session: a dictation can also be started
        // from the tray, so it asks for the state again while it is open.
        void load()
        const timer = setInterval(() => void load(), 2000)
        return () => clearInterval(timer)
    })

    async function load() {
        try {
            view = await whisperApi.status()
            settings = view.settings
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        }
    }

    function describe(error: unknown): string {
        const code = typeof error === "string" ? error : String(error)
        return /^[a-z_]+$/.test(code) ? t(errorKey(code)) : code
    }

    async function pickBinary() {
        await guard(async () => {
            const chosen = await whisperApi.selectBinary()
            if (chosen) settings = chosen
        })
    }

    async function pickModel() {
        await guard(async () => {
            const chosen = await whisperApi.selectModel()
            if (chosen) settings = chosen
        })
    }

    async function save() {
        const problem = settingsProblem(settings)
        if (problem) {
            actionError = t(problem)
            return
        }
        await guard(async () => {
            settings = await whisperApi.updateSettings(normalizedSettings(settings))
            saved = true
        })
    }

    async function dictate() {
        busy = true
        try {
            await whisperApi.dictate()
            await load()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function stop() {
        busy = true
        try {
            await whisperApi.cancel()
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function transcribeFile() {
        busy = true
        try {
            await whisperApi.transcribeFile()
            await load()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function forget() {
        await guard(async () => {
            await whisperApi.clearLast()
            expanded = false
        })
    }

    async function guard(action: () => Promise<void>) {
        busy = true
        try {
            await action()
            await load()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    /** A note the core attached: a key is translated, a sentence is shown as it is. */
    function noteText(note: string): string {
        const key = noteKey(note)
        return key ? t(key) : note
    }

    function patch(changes: Partial<WhisperSettings>) {
        settings = { ...settings, ...changes }
        saved = false
    }
</script>

<Text weight={700} size="lg">{t("whisper-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("whisper-description")}</Text>
<Space h="sm" />

{#if actionError}
    <Alert color="red" title={t("whisper-title")}>{actionError}</Alert>
    <Space h="sm" />
{/if}

{#if status}
    <Alert color={status.configured && status.enabled ? "green" : "yellow"} title={t(stateKey(status.state))}>
        <Text size="sm">{t(readinessKey(status))}</Text>
        {#each status.notes as note (note)}
            <Text size="xs" color="dimmed">{noteText(note)}</Text>
        {/each}
        {#if status.model}
            <Text size="xs" color="dimmed">
                {t("whisper-model-label")}: {t(modelKindKey(status.model.kind))}
                {#if modelIsUnverified(status)}
                    — {t("whisper-model-unverified")}
                {/if}
            </Text>
        {/if}
    </Alert>
    <Space h="sm" />
{/if}

<Switch
    label={t("whisper-enabled")}
    checked={settings.enabled}
    on:change={() => patch({ enabled: !settings.enabled })}
/>
<Text size="xs" color="dimmed">{t("whisper-enabled-hint")}</Text>
<Space h="xs" />

<Group spacing="xs">
    <Button size="sm" variant="default" loading={busy} on:click={pickBinary}>
        {t("whisper-pick-binary")}
    </Button>
    <Button size="sm" variant="default" loading={busy} on:click={pickModel}>
        {t("whisper-pick-model")}
    </Button>
</Group>
<Space h="xs" />

<Group spacing="xs">
    <Button
        size="sm"
        color={running ? "red" : "green"}
        loading={busy}
        disabled={!settings.enabled || !status?.configured}
        on:click={running ? stop : dictate}
    >
        {running ? t("whisper-stop") : t("whisper-dictate")}
    </Button>
    <Button size="sm" variant="default" loading={busy} disabled={running || !status?.configured} on:click={transcribeFile}>
        {t("whisper-transcribe-file")}
    </Button>
    {#if transcript}
        <Button size="sm" variant="subtle" on:click={forget}>{t("whisper-forget")}</Button>
    {/if}
</Group>
<Space h="xs" />
<Text size="xs" color="dimmed">{t("whisper-dictate-hint")}</Text>
<Space h="sm" />

{#if transcript && preview}
    <Text weight={600}>{t("whisper-transcript")}</Text>
    <div class="transcript">
        <Text size="sm">{expanded ? transcript.text : preview.text}</Text>
    </div>
    <Group spacing="xs">
        <Text size="xs" color="dimmed">
            {transcriptLength(transcript)} {t("whisper-characters")} · {audioLabel(transcript)} · {transcript.language}
        </Text>
        {#if preview.shortened}
            <Button size="xs" variant="subtle" on:click={() => (expanded = !expanded)}>
                {expanded ? t("whisper-show-less") : t("whisper-show-all")}
            </Button>
        {/if}
    </Group>
    <Space h="sm" />
{/if}

<Text weight={600}>{t("whisper-settings-title")}</Text>
<Space h="xs" />
<label class="field">
    <span>{t("whisper-language")}</span>
    <select bind:value={settings.language} on:change={() => patch({})}>
        {#each LANGUAGES as language (language)}
            <option value={language}>{language}</option>
        {/each}
    </select>
</label>
<label class="field">
    <span>{t("whisper-threads")}</span>
    <input class="line" type="number" min={MIN_THREADS} max={MAX_THREADS} bind:value={settings.threads} />
</label>
<label class="field">
    <span>{t("whisper-max-seconds")}</span>
    <input class="line" type="number" min="1" max="300" bind:value={settings.max_seconds} />
</label>
<label class="field">
    <span>{t("whisper-silence")}</span>
    <input
        class="line"
        type="number"
        min={MIN_SILENCE_MS}
        max={MAX_SILENCE_MS}
        step="100"
        bind:value={settings.silence_ms}
    />
</label>
<label class="field">
    <span>{t("whisper-timeout")}</span>
    <input
        class="line"
        type="number"
        min={MIN_TIMEOUT_SECONDS}
        max={MAX_TIMEOUT_SECONDS}
        bind:value={settings.timeout_seconds}
    />
</label>
<Switch
    label={t("whisper-translate")}
    checked={settings.translate}
    on:change={() => patch({ translate: !settings.translate })}
/>
<Switch
    label={t("whisper-keep-audio")}
    checked={settings.keep_audio}
    on:change={() => patch({ keep_audio: !settings.keep_audio })}
/>
<Text size="xs" color="dimmed">{t("whisper-keep-audio-hint")}</Text>
<Space h="xs" />
<Group spacing="xs">
    <Button size="sm" loading={busy} on:click={save}>{t("whisper-save")}</Button>
    {#if saved}
        <Text size="sm" color="green">{t("whisper-saved")}</Text>
    {/if}
</Group>

<style>
    .field {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.8rem;
        opacity: 0.9;
        margin-bottom: 0.35rem;
    }

    .line {
        padding: 0.25rem 0.4rem;
        border-radius: 6px;
        border: 1px solid rgba(255, 255, 255, 0.15);
        background: rgba(0, 0, 0, 0.2);
        color: inherit;
        width: 8rem;
    }

    select {
        padding: 0.25rem 0.4rem;
        border-radius: 6px;
        border: 1px solid rgba(255, 255, 255, 0.15);
        background: rgba(0, 0, 0, 0.2);
        color: inherit;
        width: 8rem;
    }

    .transcript {
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 10px;
        padding: 0.5rem;
        max-height: 12rem;
        overflow: auto;
        word-break: break-word;
    }
</style>
