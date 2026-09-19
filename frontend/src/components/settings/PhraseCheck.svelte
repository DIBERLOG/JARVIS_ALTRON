<script lang="ts">
    /**
     * Check a phrase without running it.
     *
     * This is the answer to "why is my command not accepted": the phrase is sent to
     * the voice host's own matcher — the same `check_phrase` the microphone goes
     * through — and what comes back is what the matcher decided: the text it saw,
     * the command it would reach, the slots that command needs, or the reason
     * nothing matched. Nothing is executed, no program is started, no system action
     * is taken, and the phrase is not stored and not written to the log.
     */
    import { Button, Group, Input, Space, Text } from "@svelteuidev/core"

    import { commandStages, translate, translations } from "@/stores"
    import { phraseCheckApi, phraseReasonKey } from "@/lib/voice-input"
    import type { PhraseCheckView } from "@/lib/voice-input"

    $: t = (key: string) => translate($translations, key)

    let draft = ""
    let result: PhraseCheckView | null = null
    let busy = false
    let error = ""

    /** The stage name a person reads, from the core's key. */
    function stageLabel(stage: string): string {
        return `command-stage-${stage}`
    }

    /** The outcome of a stage, in one short line. */
    function stageDetail(entry: { length: number; code: string | null; success: boolean | null }): string {
        if (entry.success === true) return "ok"
        if (entry.success === false) return "error"
        if (entry.code) return entry.code
        return `${entry.length}`
    }

    async function check() {
        const phrase = draft.trim()
        if (!phrase) {
            result = null
            error = ""
            return
        }
        busy = true
        try {
            result = await phraseCheckApi.check(phrase)
            error = ""
        } catch (failure) {
            result = null
            error = typeof failure === "string" ? failure : String(failure)
        } finally {
            busy = false
        }
    }

    /** Enter runs the check; the phrase is never sent anywhere but the voice host. */
    function keydown(event: Event) {
        if ((event as KeyboardEvent).key === "Enter") {
            event.preventDefault()
            void check()
        }
    }
</script>

<Text weight={600}>{t("phrase-check-title")}</Text>
<Space h="xs" />
<div class="field">
    <span>{t("phrase-check-run")}</span>
    <Input
        size="sm"
        placeholder={t("phrase-check-placeholder")}
        bind:value={draft}
        on:keydown={keydown}
    />
</div>
<Group spacing="xs">
    <Button size="xs" variant="default" loading={busy} on:click={check}>
        {t("phrase-check-run")}
    </Button>
</Group>
<Text size="xs" color="dimmed">{t("phrase-check-hint")}</Text>

{#if error}
    <Text size="xs" color="red">{error}</Text>
{/if}

{#if result}
    <Space h="xs" />
    <Text size="xs" color="dimmed">
        {t("phrase-check-normalized")}: {result.normalized}
    </Text>
    {#if result.matched}
        <Text size="xs" color="green">
            {t("phrase-check-matched")}: {result.matched}
        </Text>
        <Text size="xs" color="dimmed">
            {t("phrase-check-slots")}:
            {result.slots.length > 0 ? result.slots.join(", ") : t("phrase-check-none")}
        </Text>
    {:else}
        <Text size="xs" color="orange">
            {t("phrase-check-nothing")}: {t(phraseReasonKey(result.reason))}
        </Text>
    {/if}
{/if}

<Space h="sm" />
<Text weight={600}>{t("command-stages-title")}</Text>
{#if $commandStages.length === 0}
    <Text size="xs" color="dimmed">{t("command-stages-empty")}</Text>
{:else}
    <ul class="stages">
        {#each $commandStages as entry, index (index)}
            <li>
                <Text size="xs" color="dimmed">
                    {t(stageLabel(entry.stage))} · {entry.length} · {stageDetail(entry)}
                    {#if entry.command_id}· {entry.command_id}{/if}
                </Text>
            </li>
        {/each}
    </ul>
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

    .stages {
        margin: 0.2rem 0 0 0;
        padding-left: 1rem;
        max-height: 8rem;
        overflow: auto;
    }
</style>
