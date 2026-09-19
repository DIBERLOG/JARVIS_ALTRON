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

    import CommandStages from "@/components/settings/CommandStages.svelte"
    import { translate, translations } from "@/stores"
    import { phraseCheckApi, phraseReasonKey } from "@/lib/voice-input"
    import type { PhraseCheckView } from "@/lib/voice-input"

    $: t = (key: string) => translate($translations, key)

    let draft = ""
    let result: PhraseCheckView | null = null
    let busy = false
    let error = ""

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

<Text weight={700} size="lg">{t("phrase-check-title")}</Text>
<Space h="xs" />
<div class="field">
    <span>{t("phrase-check-run")}</span>
    <Input
        size="md"
        placeholder={t("phrase-check-placeholder")}
        bind:value={draft}
        on:keydown={keydown}
    />
</div>
<Group spacing="xs">
    <Button size="sm" variant="default" loading={busy} on:click={check}>
        {t("phrase-check-run")}
    </Button>
</Group>
<Text size="sm" color="dimmed">{t("phrase-check-hint")}</Text>

{#if error}
    <Text size="sm" color="red">{error}</Text>
{/if}

{#if result}
    <Space h="xs" />
    <Text size="sm" color="dimmed">
        {t("phrase-check-normalized")}: {result.normalized}
    </Text>
    {#if result.matched}
        <Text size="sm" color="green">
            {t("phrase-check-matched")}: {result.matched}
        </Text>
        <Text size="sm" color="dimmed">
            {t("phrase-check-slots")}:
            {result.slots.length > 0 ? result.slots.join(", ") : t("phrase-check-none")}
        </Text>
    {:else}
        <Text size="sm" color="orange">
            {t("phrase-check-nothing")}: {t(phraseReasonKey(result.reason))}
        </Text>
    {/if}
{/if}

<Space h="md" />
<CommandStages />

<style>
    .field {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        font-size: 0.9rem;
        opacity: 0.95;
        margin-bottom: 0.5rem;
    }
</style>
