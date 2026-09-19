<script lang="ts">
    /**
     * The close dialog: what happens when the window's close button is pressed
     * and the answer is "ask".
     *
     * It is not a browser dialog. The core prevented the close and asked the
     * window to show this, so the choice can be remembered — which a native
     * message box cannot offer. Nothing here decides what a close means: it
     * sends the answer the user picked.
     */
    import { Button, Checkbox, Group, Space, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import { desktopApi } from "@/lib/desktop"
    import type { CloseBehavior } from "@/lib/desktop-model"
    import { CLOSE_CHOICES, closeBehaviorHintKey, closeBehaviorKey } from "@/lib/desktop-model"

    export let visible = false
    export let trayAvailable = true
    export let onDone: () => void = () => {}

    $: t = (key: string) => translate($translations, key)

    let remember = false
    let chosen: CloseBehavior = "tray"
    let busy = false
    let error = ""

    /** Answers the close: hide, exit, or cancel. */
    async function answer(behavior: CloseBehavior | null) {
        busy = true
        try {
            if (behavior === null) {
                // Cancel: the core already kept the window open, so nothing is
                // sent and the dialog simply goes away.
                visible = false
                onDone()
                return
            }
            await desktopApi.setCloseBehavior(behavior, remember)
            if (behavior === "tray") {
                await desktopApi.hideWindow()
            } else {
                await desktopApi.requestExit()
            }
            visible = false
            error = ""
            onDone()
        } catch (failure) {
            error = String(failure)
        } finally {
            busy = false
        }
    }
</script>

{#if visible}
    <div class="backdrop">
        <div class="dialog" role="alertdialog" aria-label={t("desktop-close-title")}>
            <Text weight={700} size="lg">{t("desktop-close-title")}</Text>
            <Space h="xs" />
            <Text size="sm" color="dimmed">{t("desktop-close-explanation")}</Text>
            <Space h="sm" />

            {#if !trayAvailable}
                <Text size="sm" color="orange">{t("desktop-close-no-tray")}</Text>
                <Space h="sm" />
            {/if}

            {#each CLOSE_CHOICES as choice (choice)}
                <div class="choice">
                    <Button
                        variant={chosen === choice ? "filled" : "default"}
                        size="sm"
                        disabled={busy || (choice === "tray" && !trayAvailable)}
                        on:click={() => {
                            chosen = choice
                            void answer(choice)
                        }}
                    >
                        {t(closeBehaviorKey(choice))}
                    </Button>
                    <Text size="xs" color="dimmed">{t(closeBehaviorHintKey(choice))}</Text>
                </div>
            {/each}

            <Space h="sm" />
            <Checkbox label={t("desktop-close-remember")} bind:checked={remember} />
            {#if error}
                <Space h="xs" />
                <Text size="sm" color="red">{error}</Text>
            {/if}
            <Space h="md" />
            <Group position="right">
                <Button variant="subtle" disabled={busy} on:click={() => answer(null)}>
                    {t("desktop-close-cancel")}
                </Button>
            </Group>
        </div>
    </div>
{/if}

<style>
    .backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.6);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 50;
    }

    .dialog {
        max-width: 26rem;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 12px;
        padding: 1rem;
        background: rgba(20, 20, 24, 0.98);
    }

    .choice {
        margin-bottom: 0.5rem;
    }
</style>
