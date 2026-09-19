<script lang="ts">
    /**
     * Talking to Jarvis, out loud.
     *
     * The microphone button asks one question: the voice host hands the microphone
     * over, Whisper writes the question down, the local model answers, and the answer
     * appears here. The stage line says which of those is happening — «слушаю»,
     * «распознаю», «думаю», «отвечаю» — and Stop abandons it at any of them.
     *
     * Nothing here executes anything: an answer is text on this screen. The only
     * thing the panel sends is a question, and the only thing it stores in memory is
     * the answer it is showing.
     */
    import { onMount } from "svelte"
    import { Button, Group, Space, Text } from "@svelteuidev/core"

    import { conversationApi, failureKeyOf, isBusy, stageKeyOf } from "@/lib/conversation"
    import type { ConversationView } from "@/lib/conversation"
    import { translate, translations } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    let view: ConversationView | null = null
    let busy = false
    let error = ""

    $: running = view ? isBusy(view.stage) : false

    onMount(() => {
        void load()
        const timer = setInterval(() => {
            // A poll must not fight the button: while a question is in flight the
            // stage comes from the events, and the button is disabled anyway.
            if (!busy) void load()
        }, 1500)
        let stop: (() => void) | null = null
        void import("@tauri-apps/api/event").then(({ listen }) =>
            listen<ConversationView>("conversation-stage", (event) => {
                view = event.payload
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
            view = await conversationApi.status()
        } catch (failure) {
            error = describe(failure)
        }
    }

    function describe(failure: unknown): string {
        const code = typeof failure === "string" ? failure : String(failure)
        return t(failureKeyOf(code))
    }

    /**
     * The key of a profile, from the small closed set the route reports. A key is
     * never built from a value: the two profiles are named here, and anything else
     * is JARVIS.
     */
    function profileKey(profile: string): string {
        if (profile === "altron") return "conversation-profile-altron"
        return "conversation-profile-jarvis"
    }

    async function ask() {
        busy = true
        error = ""
        try {
            view = await conversationApi.ask()
            if (view?.last_code) error = t(failureKeyOf(view.last_code))
        } catch (failure) {
            error = describe(failure)
        } finally {
            busy = false
        }
    }

    async function cancel() {
        try {
            await conversationApi.cancel()
            await load()
        } catch (failure) {
            error = describe(failure)
        }
    }

    async function forget() {
        try {
            await conversationApi.clear()
            error = ""
            await load()
        } catch (failure) {
            error = describe(failure)
        }
    }
</script>

<Text weight={700} size="lg">{t("conversation-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("conversation-description")}</Text>
<Space h="sm" />

{#if view}
    <Group spacing="xs">
        <Button size="sm" color="green" loading={busy} disabled={running} on:click={ask}>
            {t("conversation-ask")}
        </Button>
        <Button size="sm" color="red" variant="default" disabled={!running} on:click={cancel}>
            {t("conversation-stop")}
        </Button>
        <Button size="sm" variant="subtle" disabled={!view.turn} on:click={forget}>
            {t("conversation-clear")}
        </Button>
    </Group>
    <Space h="xs" />
    <Text size="sm" color="dimmed">
        {t("conversation-stage")}: {t(stageKeyOf(view.stage))} · {t("conversation-provider")}:
        {view.provider} · {t("conversation-profile")}: {t(profileKey(view.profile))}
    </Text>
    {#if !view.provider_available}
        <Text size="sm" color="orange">{t("conversation-provider-unavailable")}</Text>
    {/if}
    {#if view.cloud}
        <Text size="sm" color="orange">{t("conversation-cloud-warning")}</Text>
    {/if}
{/if}

{#if error}
    <Text size="sm" color="red">{error}</Text>
{/if}

{#if view?.turn}
    <Space h="sm" />
    <Text size="sm" color="dimmed">
        {t("conversation-question")}: {view.turn.question_characters} {t("conversation-characters")}
    </Text>
    <div class="answer">
        <Text size="sm">{view.turn.answer}</Text>
    </div>
    <Text size="xs" color="dimmed">
        {t("conversation-answer")}: {view.turn.answer_characters} {t("conversation-characters")}
    </Text>
{/if}

<style>
    .answer {
        border: 1px solid rgba(45, 212, 191, 0.35);
        border-radius: 10px;
        padding: 0.6rem;
        max-height: 18rem;
        overflow: auto;
        white-space: pre-wrap;
        font-size: 0.95rem;
    }
</style>
