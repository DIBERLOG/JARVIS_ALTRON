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

    import { translate, translations, commandDraft } from "@/stores"
    import { whisperApi } from "@/lib/whisper"
    import type { WhisperPanelView } from "@/lib/whisper"
    import type { DiscoveryReport, MicrophoneCheck, NumericField, WhisperSettings } from "@/lib/whisper-model"
    import {
        candidateSourceKey,
        cleanedTranscript,
        discoverySummaryKey,
        draftsAfterPoll,
        LANGUAGES,
        MAX_SILENCE_MS,
        MAX_THREADS,
        MAX_TIMEOUT_SECONDS,
        MAX_SECONDS,
        MIN_SECONDS,
        MIN_SILENCE_MS,
        MIN_THREADS,
        MIN_TIMEOUT_SECONDS,
        audioLabel,
        defaultSettings,
        errorKey,
        isBusy,
        isDirty,
        microphoneCheckKey,
        modelIsUnverified,
        modelKindKey,
        noteKey,
        normalizedSettings,
        NUMERIC_FIELDS,
        numericDrafts,
        parseNumericDraft,
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
    let discovery: DiscoveryReport | null = null
    let microphone: MicrophoneCheck | null = null
    let inserted = false
    /** The transcript the "inserted" line belongs to. */
    let shownText = ""
    /** The text each numeric field shows while it is being edited. */
    let drafts = numericDrafts(defaultSettings())
    /** The field the person is in, if any: a poll must not touch it. */
    let editing: NumericField | null = null

    $: status = view?.status ?? null
    $: transcript = view?.last ?? null
    $: preview = transcript ? transcriptPreview(transcript) : null
    $: running = status ? isBusy(status.state) : false

    onMount(() => {
        // The panel does not own the session: a dictation can also be started
        // from the tray, so it asks for the state again while it is open, and it
        // listens for the announcement that a transcript arrived.
        void load()
        const timer = setInterval(() => void load(), 2000)
        let stop: (() => void) | null = null
        void import("@tauri-apps/api/event").then(({ listen }) =>
            listen("whisper-transcript-ready", () => {
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
            view = await whisperApi.status()
            settings = view.settings
            // A field being edited keeps what was typed: a poll that overwrote it
            // would throw the value away before it was ever saved.
            drafts = draftsAfterPoll(drafts, editing, settings)
            // A new transcript is not the one that was inserted, so the line that
            // says "inserted" belongs to the previous one and goes away. The text
            // itself stays: the core holds it, not this component.
            const incoming = view.last?.text ?? ""
            if (incoming !== shownText) {
                shownText = incoming
                inserted = false
            }
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        }
    }

    function describe(error: unknown): string {
        const code = typeof error === "string" ? error : String(error)
        return /^[a-z_]+$/.test(code) ? t(errorKey(code)) : code
    }

    /** Looks for a build that is already installed. Nothing is saved here. */
    async function discover() {
        busy = true
        try {
            discovery = await whisperApi.discover()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    /** Stores a pair the user confirmed; the core validates it again. */
    async function applyPair(index: number) {
        const pair = discovery?.pairs[index]
        if (!pair) return
        busy = true
        try {
            settings = await whisperApi.applyDiscovered(pair.executable.path, pair.model.path)
            discovery = null
            saved = true
            actionError = ""
            await load()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
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

    /**
     * Opens the microphone for a moment.
     *
     * Nothing is recorded and nothing is transcribed: the core reads a few
     * frames, measures a level, and releases the device on every path. The code
     * it answers with is the recorder's own, so the panel can name the cause.
     */
    async function checkMicrophone() {
        busy = true
        try {
            microphone = await whisperApi.checkMicrophone()
            actionError = ""
        } catch (error) {
            microphone = null
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    /** The measured level as a percentage: a fraction is unreadable. */
    function levelLabel(level: number): string {
        return `${Math.round(level * 100)} %`
    }

    /**
     * Commits one numeric field.
     *
     * Called from `blur` and from Enter, never from `input`: a value is sent
     * once, when the person is done with it. What comes back is the document the
     * core stored, so the field shows the saved value and not the typed one.
     */
    async function commitNumeric(field: NumericField) {
        const parsed = parseNumericDraft(field, drafts[field])
        if ("problem" in parsed) {
            if (isDirty(field, drafts, settings)) {
                actionError = t(parsed.problem)
            }
            // Back to what the document holds: an out-of-range value is not
            // something the core would keep anyway.
            drafts = { ...drafts, [field]: numericDrafts(settings)[field] }
            editing = null
            return
        }
        editing = null
        if (!isDirty(field, { ...drafts, [field]: String(parsed.value) }, settings)) {
            // Nothing changed: no write, and the field shows the stored value.
            drafts = { ...drafts, [field]: numericDrafts(settings)[field] }
            return
        }
        await store({ [field]: parsed.value })
        // `store` reloads, so the field now shows exactly what was stored.
        drafts = { ...drafts, [field]: numericDrafts(settings)[field] }
    }

    /** Enter commits the field without leaving it. */
    function numericKeydown(event: KeyboardEvent, field: NumericField) {
        if (event.key === "Enter") {
            event.preventDefault()
            void commitNumeric(field)
        }
        if (event.key === "Escape") {
            drafts = { ...drafts, [field]: numericDrafts(settings)[field] }
            editing = null
            actionError = ""
        }
    }

    /**
     * Puts the transcript into the command field on the home page.
     *
     * The text is not executed: a transcript is what was said, and sending it as
     * a command is a decision the person makes on the home page, in the field
     * where every other command is typed.
     */
    function insertIntoCommandField() {
        if (!transcript) return
        commandDraft.set(cleanedTranscript(transcript))
        inserted = true
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

    /**
     * Stores a switch or a select immediately.
     *
     * A switch that only changes what this page holds would be overwritten by
     * the next state poll, which is exactly the defect this replaces: the value
     * has to reach the core, and the page then shows what the core kept.
     */
    async function store(changes: Partial<WhisperSettings>) {
        if (!settings) return
        const candidate = { ...settings, ...changes }
        const problem = settingsProblem(candidate)
        if (problem) {
            actionError = t(problem)
            return
        }
        settings = candidate
        busy = true
        try {
            settings = await whisperApi.updateSettings(normalizedSettings(candidate))
            saved = true
            actionError = ""
            await load()
        } catch (error) {
            actionError = describe(error)
            await load()
        } finally {
            busy = false
        }
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
        {#if status.binary_name || status.model_name}
            <Text size="xs" color="dimmed">
                {t("whisper-file-executable")}: {status.binary_name || "—"} ·
                {t("whisper-file-model")}: {status.model_name || "—"}
            </Text>
        {/if}
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
    on:change={() => store({ enabled: !settings.enabled })}
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

<!-- Looking for a build that is already installed. Nothing is saved until the
     user confirms a pair, and nothing is downloaded. -->
<Group spacing="xs">
    <Button size="sm" variant="default" loading={busy} on:click={discover}>
        {t("whisper-discovery-button")}
    </Button>
</Group>
<Text size="xs" color="dimmed">{t("whisper-discovery-hint")}</Text>
{#if discovery}
    <Text size="sm">{t(discoverySummaryKey(discovery))}</Text>
    {#each discovery.pairs as pair, index (`${pair.executable.path}-${pair.model.path}`)}
        <Group spacing="xs">
            <Text size="xs" color="dimmed">
                {pair.executable.name} · {pair.model.name}
                ({t(modelKindKey(pair.model.kind))}, {t(candidateSourceKey(pair.model.source))})
            </Text>
            <Button size="xs" variant="subtle" loading={busy} on:click={() => applyPair(index)}>
                {t("whisper-discovery-apply")}
            </Button>
        </Group>
    {/each}
    {#if discovery.rejected.length > 0}
        <Text size="xs" color="orange">{t("whisper-discovery-rejected")}</Text>
        {#each discovery.rejected as candidate (candidate.name + candidate.code)}
            <Text size="xs" color="dimmed">{candidate.name}: {candidate.detail}</Text>
        {/each}
    {/if}
{/if}
<Space h="xs" />

<!-- The microphone check records nothing: it opens the device, measures a level,
     and gives the device straight back. It is the answer to "is dictation even
     possible on this machine" without a transcription in between. -->
<Group spacing="xs">
    <Button
        size="sm"
        variant="default"
        loading={busy}
        disabled={running}
        on:click={checkMicrophone}
    >
        {t("whisper-mic-check-button")}
    </Button>
</Group>
<Text size="xs" color="dimmed">{t("whisper-mic-check-hint")}</Text>
{#if microphone}
    <Text size="xs" color={microphone.error_code ? "orange" : "dimmed"}>
        {t(microphoneCheckKey(microphone))}
        {#if !microphone.error_code}
            · {t("whisper-mic-check-level")}: {levelLabel(microphone.level)}
            · {t("whisper-mic-check-devices")}: {microphone.status.device_count}
            · {t("whisper-mic-check-backend")}: {microphone.status.backend}
        {/if}
    </Text>
{/if}
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
    <!-- The result of the dictation: the text itself, in the panel, where the
         button that produced it is. It stays here until the person forgets it or
         replaces it with a new dictation, and a state poll cannot clear it. -->
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
    <Group spacing="xs">
        <Button size="xs" variant="default" on:click={insertIntoCommandField}>
            {t("whisper-insert-command")}
        </Button>
        {#if inserted}
            <Text size="xs" color="green">{t("whisper-inserted")}</Text>
        {/if}
    </Group>
    <Text size="xs" color="dimmed">{t("whisper-insert-hint")}</Text>
    <Space h="sm" />
{/if}

<Text weight={600}>{t("whisper-settings-title")}</Text>
<Space h="xs" />
<label class="field">
    <span>{t("whisper-language")}</span>
    <select bind:value={settings.language} on:change={() => store({ language: settings.language })}>
        {#each LANGUAGES as language (language)}
            <option value={language}>{language}</option>
        {/each}
    </select>
</label>
<!-- The numbers are edited in a draft and committed on blur or Enter. A poll
     refreshes every field except the one being edited, so a value is never
     taken away before it is saved. -->
<label class="field">
    <span>{t("whisper-threads")}</span>
    <input
        class="line"
        type="number"
        min={MIN_THREADS}
        max={MAX_THREADS}
        bind:value={drafts.threads}
        on:focus={() => (editing = "threads")}
        on:blur={() => void commitNumeric("threads")}
        on:keydown={(event) => numericKeydown(event, "threads")}
    />
</label>
<label class="field">
    <span>{t("whisper-max-seconds")}</span>
    <input
        class="line"
        type="number"
        min={MIN_SECONDS}
        max={MAX_SECONDS}
        bind:value={drafts.max_seconds}
        on:focus={() => (editing = "max_seconds")}
        on:blur={() => void commitNumeric("max_seconds")}
        on:keydown={(event) => numericKeydown(event, "max_seconds")}
    />
</label>
<label class="field">
    <span>{t("whisper-silence")}</span>
    <input
        class="line"
        type="number"
        min={MIN_SILENCE_MS}
        max={MAX_SILENCE_MS}
        step="100"
        bind:value={drafts.silence_ms}
        on:focus={() => (editing = "silence_ms")}
        on:blur={() => void commitNumeric("silence_ms")}
        on:keydown={(event) => numericKeydown(event, "silence_ms")}
    />
</label>
<label class="field">
    <span>{t("whisper-timeout")}</span>
    <input
        class="line"
        type="number"
        min={MIN_TIMEOUT_SECONDS}
        max={MAX_TIMEOUT_SECONDS}
        bind:value={drafts.timeout_seconds}
        on:focus={() => (editing = "timeout_seconds")}
        on:blur={() => void commitNumeric("timeout_seconds")}
        on:keydown={(event) => numericKeydown(event, "timeout_seconds")}
    />
</label>
<Switch
    label={t("whisper-translate")}
    checked={settings.translate}
    on:change={() => store({ translate: !settings.translate })}
/>
<Switch
    label={t("whisper-keep-audio")}
    checked={settings.keep_audio}
    on:change={() => store({ keep_audio: !settings.keep_audio })}
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
