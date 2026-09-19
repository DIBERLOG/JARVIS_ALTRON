<script lang="ts">
    /**
     * "Запуск и трей": what the close button does, whether Windows starts the
     * application, and what a login is allowed to start.
     *
     * The page asks the core for the state; it never writes a registry key. The
     * autostart switches are disabled until autostart is on, because they only
     * mean something then — a person can still see what a login would do.
     */
    import { onMount } from "svelte"

    import { Alert, Button, Group, Space, Switch, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import { desktopApi } from "@/lib/desktop"
    import type { CloseBehavior, DesktopState } from "@/lib/desktop-model"
    import {
        AUTOSTART_SWITCHES,
        autostartNeedsRepair,
        autostartStateKey,
        autostartSwitchKey,
        autostartSwitchesEnabled,
        closeBehaviorHintKey,
        closeBehaviorKey,
        microphoneStateKey
    } from "@/lib/desktop-model"

    $: t = (key: string) => translate($translations, key)

    /** The three answers, in the order the page shows them. */
    const CLOSE_BEHAVIORS: readonly ("tray" | "exit" | "ask")[] = ["tray", "exit", "ask"]

    let state: DesktopState | null = null
    let busy = false
    let actionError = ""
    let notice = ""

    $: settings = state?.settings ?? null
    $: autostartState = state?.autostart ?? null
    $: switchesEnabled = settings ? autostartSwitchesEnabled(settings) : false
    $: microphoneState = state?.microphone ?? "idle"
    $: microphoneKey = microphoneStateKey(microphoneState)
    $: pendingTimers = state?.pending_timers ?? 0
    $: trayAvailable = state?.tray_available ?? false
    $: closeHintKey = settings ? closeBehaviorHintKey(settings.close_behavior) : "desktop-close-ask-hint"

    onMount(async () => {
        await load()
    })

    async function load() {
        try {
            state = await desktopApi.state()
            actionError = ""
        } catch (error) {
            actionError = String(error)
        }
    }

    async function setCloseBehavior(behavior: string) {
        busy = true
        try {
            await desktopApi.setCloseBehavior(behavior as CloseBehavior, true)
            await load()
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }

    async function setAutostart(enabled: boolean) {
        busy = true
        try {
            if (enabled) {
                await desktopApi.autostartEnable()
            } else {
                await desktopApi.autostartDisable()
            }
            await load()
            actionError = ""
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }

    /** The label of one of the three close answers. */
    function closeLabel(behavior: string): string {
        return t(closeBehaviorKey(behavior as CloseBehavior))
    }

    /** Flips one of the "what a login starts" switches. */
    async function toggleSwitch(name: string) {
        if (!settings) return
        await patch({ [name]: !(settings as unknown as Record<string, boolean>)[name] } as never)
    }

    /** Writes one of the "what a login starts" switches and stores it. */
    async function patch(changes: Partial<NonNullable<typeof settings>>) {
        if (!settings) return
        busy = true
        try {
            // The close behaviour and the four switches live in one document,
            // so one call stores them together.
            await desktopApi.updateSettings({ ...settings, ...changes })
            notice = t("desktop-settings-saved")
            await load()
        } catch (error) {
            actionError = String(error)
        } finally {
            busy = false
        }
    }
</script>

<Text weight={700} size="lg">{t("desktop-settings-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("desktop-settings-description")}</Text>
<Space h="sm" />

{#if actionError}
    <Alert color="red" title={t("desktop-settings-title")}>{actionError}</Alert>
    <Space h="sm" />
{/if}

{#if settings}
    <Text weight={600}>{t("desktop-close-title")}</Text>
    <Space h="xs" />
    <Group spacing="xs">
        {#each ["tray", "exit", "ask"] as behavior (behavior)}
            <Button
                size="sm"
                variant={settings.close_behavior === behavior ? "filled" : "default"}
                disabled={busy}
                on:click={() => setCloseBehavior(behavior)}
            >
                {closeLabel(behavior)}
            </Button>
        {/each}
    </Group>
    <Text size="xs" color="dimmed">{t(closeHintKey)}</Text>
    <Space h="md" />

    <Text weight={600}>{t("desktop-autostart-title")}</Text>
    <Space h="xs" />
    <Switch
        label={t("desktop-autostart-enabled")}
        checked={settings.autostart_enabled}
        disabled={busy}
        on:change={() => setAutostart(!settings.autostart_enabled)}
    />
    {#if autostartState}
        <Text size="xs" color={autostartState.error ? "orange" : "dimmed"}>
            {t(autostartStateKey(
                settings.autostart_enabled
                    ? autostartState.error
                        ? "unavailable"
                        : autostartNeedsRepair(autostartState)
                          ? "needs_attention"
                          : "enabled"
                    : "disabled"
            ))}
        </Text>
        {#if autostartState.error}
            <Text size="xs" color="orange">{autostartState.error}</Text>
        {/if}
        {#if autostartNeedsRepair(autostartState)}
            <Button size="xs" variant="subtle" disabled={busy} on:click={() => setAutostart(true)}>
                {t("desktop-autostart-repair")}
            </Button>
        {/if}
    {/if}
    <Text size="xs" color="dimmed">{t("desktop-autostart-hint")}</Text>
    <Space h="xs" />

    {#each AUTOSTART_SWITCHES as name (String(name))}
        <Switch
            label={t(autostartSwitchKey(name))}
            checked={Boolean(settings[name])}
            disabled={busy || !switchesEnabled}
            on:change={() => toggleSwitch(String(name))}
        />
    {/each}
    <Text size="xs" color="dimmed">{t("desktop-autostart-switches-hint")}</Text>
    <Space h="sm" />

    <Text weight={600}>{t("desktop-status-title")}</Text>
    <Space h="xs" />
    <Text size="sm">{t("desktop-status-microphone")}: {t(microphoneKey)}</Text>
    <Text size="sm">
        {t("desktop-status-timers")}: {pendingTimers}
    </Text>
    <Text size="sm">
        {t("desktop-status-tray")}:
        {trayAvailable ? t("desktop-status-available") : t("desktop-status-unavailable")}
    </Text>
    {#if notice}
        <Text size="sm" color="green">{notice}</Text>
    {/if}
{/if}


