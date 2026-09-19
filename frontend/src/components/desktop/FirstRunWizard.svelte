<script lang="ts">
    /**
     * The first-run wizard: ten steps, every one of which can be skipped.
     *
     * The steps that configure a feature reuse the same commands the settings
     * pages use, so there is one implementation of "choose a model" and one of
     * "allow a program". This component owns the order and the progress, and
     * nothing else.
     *
     * Two rules:
     *
     * * a skipped step is stored as skipped, never as done, so the summary can
     *   say "not configured" instead of pretending;
     * * no password is typed here. The storage step opens the existing storage
     *   page: a master password belongs to the component that owns the
     *   derivation, and it must not pass through this state.
     */
    import { onMount } from "svelte"

    import { Alert, Button, Group, Space, Text } from "@svelteuidev/core"

    import { translate, translations, setLanguage } from "@/stores"
    import { desktopApi } from "@/lib/desktop"
    import type { DesktopState, SetupState } from "@/lib/desktop-model"
    import {
        SETUP_LANGUAGES,
        microphoneStateKey,
        nextSetupStep,
        setupProgress,
        setupStepHintKey,
        setupStepKey,
        setupSummary,
        stepAfter,
        stepSection
    } from "@/lib/desktop-model"

    export let onClose: () => void = () => {}
    export let onOpenSection: (section: string) => void = () => {}

    $: t = (key: string) => translate($translations, key)

    let setup: SetupState | null = null
    let desktop: DesktopState | null = null
    let step: string | null = null
    let busy = false
    let actionError = ""

    $: progress = setup ? setupProgress(setup) : 0
    $: summary = setup ? setupSummary(setup) : { completed: [], skipped: [], remaining: [] }

    onMount(async () => {
        await load()
    })

    async function load() {
        try {
            setup = await desktopApi.setup()
            desktop = await desktopApi.state()
            step = nextSetupStep(setup) ?? "diagnostics"
            actionError = ""
        } catch (error) {
            actionError = String(error)
        }
    }

    async function chooseLanguage(language: string) {
        busy = true
        try {
            // The language applies at once: the wizard is the first thing a new
            // profile sees, and it should be readable while it runs.
            await setLanguage(language)
            setup = await desktopApi.completeStep("language", language)
            actionError = ""
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }

    async function complete() {
        if (!step) return
        busy = true
        try {
            setup = await desktopApi.completeStep(step)
            await advance()
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }

    async function skip() {
        if (!step) return
        busy = true
        try {
            setup = await desktopApi.skipStep(step)
            await advance()
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }

    async function advance() {
        const after = step ? stepAfter(step) : null
        if (after) {
            step = after
            return
        }
        await finish()
    }

    /** Finishing marks every step the user never saw as skipped. */
    async function finish() {
        const timestamp = new Date().toISOString()
        setup = await desktopApi.finishSetup(timestamp)
        step = null
    }

    function open(section: string) {
        onOpenSection(section)
    }
</script>

<Text weight={700} size="lg">{t("setup-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("setup-description")}</Text>
<Space h="xs" />
<Text size="xs" color="dimmed">
    {Math.round(progress * 100)}% · {summary.completed.length} {t("setup-done")} ·
    {summary.skipped.length} {t("setup-skipped")}
</Text>
<Space h="sm" />

{#if actionError}
    <Alert color="red" title={t("setup-title")}>{actionError}</Alert>
    <Space h="sm" />
{/if}

{#if step}
    <Text weight={600}>{t(setupStepKey(step))}</Text>
    <Text size="xs" color="dimmed">{t(setupStepHintKey(step))}</Text>
    <Space h="xs" />

    {#if step === "language"}
        <Group spacing="xs">
            {#each SETUP_LANGUAGES as language (language)}
                <Button size="sm" disabled={busy} on:click={() => chooseLanguage(language)}>
                    {language}
                </Button>
            {/each}
        </Group>
    {:else if step === "storage"}
        <Text size="sm">{t("setup-storage-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("notes-vault")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "local_ai"}
        <Text size="sm">{t("setup-local-ai-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("local-ai")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "whisper"}
        <Text size="sm">{t("setup-whisper-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("voice")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "microphone"}
        <Text size="sm">{t("setup-microphone-body")}</Text>
        {#if desktop}
            <Text size="xs" color="dimmed">
                {t("desktop-status-microphone")}: {t(microphoneStateKey(desktop.microphone))}
            </Text>
        {/if}
        <Button size="xs" variant="subtle" on:click={() => open("voice")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "vosk"}
        <Text size="sm">{t("setup-vosk-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("voice")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "dictionaries"}
        <Text size="sm">{t("setup-dictionaries-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("autocorrect")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "windows_actions"}
        <Text size="sm">{t("setup-windows-actions-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("windows-actions")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "autostart"}
        <Text size="sm">{t("setup-autostart-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("startup-tray")}>
            {t("setup-open-settings")}
        </Button>
    {:else if step === "diagnostics"}
        <Text size="sm">{t("setup-diagnostics-body")}</Text>
        <Button size="xs" variant="subtle" on:click={() => open("diagnostics")}>
            {t("setup-open-settings")}
        </Button>
    {/if}

    <Space h="md" />
    <Group spacing="xs">
        <Button size="sm" loading={busy} on:click={complete}>{t("setup-next")}</Button>
        <Button size="sm" variant="default" loading={busy} on:click={skip}>{t("setup-skip")}</Button>
        <Button size="sm" variant="subtle" disabled={busy} on:click={finish}>{t("setup-finish")}</Button>
    </Group>
{:else}
    <Text weight={600}>{t("setup-summary-title")}</Text>
    <Space h="xs" />
    {#if summary.completed.length > 0}
        <Text size="sm">{t("setup-summary-done")}</Text>
        {#each summary.completed as done (done)}
            <Text size="xs" color="dimmed">· {t(setupStepKey(done))}</Text>
        {/each}
    {/if}
    {#if summary.skipped.length > 0}
        <Text size="sm" color="orange">{t("setup-summary-skipped")}</Text>
        {#each summary.skipped as skipped (skipped)}
            <Text size="xs" color="dimmed">· {t(setupStepKey(skipped))}</Text>
        {/each}
    {/if}
    {#if summary.remaining.length > 0}
        <Text size="sm">{t("setup-summary-remaining")}</Text>
    {/if}
    <Space h="md" />
    <Group spacing="xs">
        <Button size="sm" on:click={onClose}>{t("setup-close")}</Button>
        <Button size="sm" variant="subtle" on:click={() => open("diagnostics")}>
            {t("setup-open-diagnostics")}
        </Button>
    </Group>
{/if}


