<script lang="ts">
    /**
     * The safe Windows commands: quick actions, the window list, timers and reminders, the
     * allowed programs, the audit log, and a place to try a spoken phrase.
     *
     * What this panel deliberately does not do:
     *
     * * it never sends an executable path. Adding a program opens the native file dialog in the
     *   core, so the path is chosen by the user in a window this component cannot influence;
     * * it never decides whether an action may run. It asks, and it shows whichever answer comes
     *   back: done, refused, or "confirm this first" — which is shown by
     *   `ConfirmationDialog` from the preview alone;
     * * it keeps nothing in browser storage, so a reload forgets a pending confirmation;
     * * it never renders an error message from the core as if it were trusted text: only the
     *   stable code is translated.
     */
    import { onMount, onDestroy } from "svelte"

    import { Alert, Button, Group, Space, Switch, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import ConfirmationDialog from "./ConfirmationDialog.svelte"
    import { windowsActionsApi, onFired } from "@/lib/windows-actions"
    import type {
        ActionPreview,
        ActionResult,
        AllowedApplicationView,
        AuditEntry,
        ScheduledKind,
        ScheduledView,
        VoiceRoute,
        WindowOperation,
        WindowSummary,
        WindowsActionsOverview,
        WindowsActionSettings
    } from "@/lib/windows-actions-model"
    import {
        MAX_CONFIRM_TTL_SECONDS,
        MIN_CONFIRM_TTL_SECONDS,
        POLICY_VOLUME_STEP,
        WINDOW_OPERATIONS,
        actionTypeKey,
        cancelScheduled,
        captureWindow,
        createReminder,
        createTimer,
        errorKey,
        firedNotification,
        formatSeconds,
        lockWorkstation,
        minutesToSeconds,
        mute,
        normalizedSettings,
        reminderInputProblem,
        resultSummary,
        riskKey,
        scheduledKindKey,
        scheduledStatusKey,
        screenshot,
        screenshotTargetKey,
        shortenTitle,
        sourceKey,
        statusKey,
        timerInputProblem,
        toolAvailabilityKey,
        voiceReasonKey,
        volumeDown,
        volumeUp,
        windowOperation,
        windowOperationKey,
        windowStateKey
    } from "@/lib/windows-actions-model"

    $: t = (key: string) => translate($translations, key)

    let overview: WindowsActionsOverview | null = null
    let settings: WindowsActionSettings | null = null
    let windows: WindowSummary[] = []
    let scheduled: ScheduledView[] = []
    let audit: AuditEntry[] = []
    let fired: ScheduledView[] = []

    let preview: ActionPreview | null = null
    let lastResult: ActionResult | null = null

    let timerMinutes = 5
    let reminderMinutes = 10
    let reminderMessage = ""
    let applicationName = ""
    let voicePhrase = ""

    let busy = false
    let notice = ""
    let actionError = ""
    let saved = false
    let unsubscribe: (() => void) | null = null

    $: capabilities = overview?.capabilities ?? null
    $: allowedApplications = overview?.allowed_applications ?? []
    $: tools = overview?.tools ?? null
    $: policy = overview?.policy ?? []
    $: platformSupported = capabilities?.platform_supported ?? false

    onMount(async () => {
        await load()
        unsubscribe = await onFired(() => {
            void collectFired()
        })
        // A reminder that fired while this panel was not open is still waiting in the core.
        await collectFired()
    })

    onDestroy(() => {
        unsubscribe?.()
    })

    async function load() {
        busy = true
        try {
            overview = await windowsActionsApi.overview()
            settings = { ...overview.settings }
            audit = await windowsActionsApi.auditLog(200)
            scheduled = await windowsActionsApi.scheduled()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    function describe(error: unknown): string {
        const code = typeof error === "string" ? error : String(error)
        // The core answers with a stable code; anything else is shown as it is, because it can
        // only be a transport failure and hiding it would hide a real problem.
        return /^[a-z_]+$/.test(code) ? t(errorKey(code)) : code
    }

    async function collectFired() {
        try {
            const items = await windowsActionsApi.takeFired()
            if (items.length > 0) fired = [...fired, ...items]
            scheduled = await windowsActionsApi.scheduled()
        } catch (error) {
            actionError = describe(error)
        }
    }

    /** Sends one action and reacts to the three possible answers. */
    async function send(action: Parameters<typeof windowsActionsApi.request>[0]) {
        busy = true
        notice = ""
        try {
            const outcome = await windowsActionsApi.request(action)
            if (outcome.outcome === "awaiting_confirmation") {
                preview = outcome.preview
            } else {
                lastResult = outcome.result
                preview = null
                await refreshAfterAction()
            }
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function refreshAfterAction() {
        audit = await windowsActionsApi.auditLog(200)
        scheduled = await windowsActionsApi.scheduled()
        if (overview) overview = { ...overview, audit_entries: audit.length }
    }

    async function confirmPending(token: string) {
        busy = true
        try {
            lastResult = await windowsActionsApi.confirm(token)
            preview = null
            await refreshAfterAction()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
            preview = null
        } finally {
            busy = false
        }
    }

    async function cancelPending() {
        busy = true
        try {
            await windowsActionsApi.cancel()
            preview = null
            await refreshAfterAction()
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function refreshWindows() {
        busy = true
        try {
            windows = await windowsActionsApi.listWindows()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function actOnWindow(id: string, operation: WindowOperation) {
        await send(windowOperation(id, operation))
    }

    async function startTimer() {
        const problem = timerInputProblem(timerMinutes)
        if (problem) {
            actionError = t(problem)
            return
        }
        await send(createTimer(minutesToSeconds(timerMinutes)))
    }

    async function startReminder() {
        const problem = reminderInputProblem(reminderMinutes, reminderMessage)
        if (problem) {
            actionError = t(problem)
            return
        }
        await send(createReminder(minutesToSeconds(reminderMinutes), reminderMessage.trim()))
        reminderMessage = ""
    }

    async function stopScheduled(kind: ScheduledKind, id: string) {
        await send(cancelScheduled(kind, id))
    }

    async function addApplication() {
        if (applicationName.trim().length === 0) {
            actionError = t("windows-actions-error-name-empty")
            return
        }
        busy = true
        try {
            // The core opens the file dialog; this call cannot name a path.
            const added = await windowsActionsApi.addAllowedApplication(applicationName.trim())
            if (added) {
                applicationName = ""
                await load()
            }
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function removeApplication(id: string) {
        await guard(async () => {
            await windowsActionsApi.removeAllowedApplication(id)
        })
    }

    async function setApplicationEnabled(id: string, enabled: boolean) {
        await guard(async () => {
            await windowsActionsApi.setAllowedApplicationEnabled(id, enabled)
        })
    }

    async function reacceptApplication(id: string) {
        await guard(async () => {
            await windowsActionsApi.reacceptAllowedApplication(id)
        })
    }

    async function startApplication(id: string) {
        await send({ action: "launch_allowed_application", application_id: id })
    }

    async function guard(action: () => Promise<unknown>) {
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

    async function saveSettings() {
        if (!settings) return
        busy = true
        try {
            settings = await windowsActionsApi.updateSettings(normalizedSettings(settings))
            saved = true
            await load()
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function clearLog() {
        await guard(async () => {
            await windowsActionsApi.clearAuditLog(true)
        })
    }

    async function exportLog() {
        busy = true
        try {
            const written = await windowsActionsApi.exportAuditLog(true)
            notice = written === null ? "" : `${written}`
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    async function tryVoice() {
        const phrase = voicePhrase.trim()
        if (phrase.length === 0) return
        busy = true
        try {
            const route: VoiceRoute = await windowsActionsApi.routeVoice(phrase)
            if (route.result === "requested") {
                if (route.outcome.outcome === "awaiting_confirmation") {
                    preview = route.outcome.preview
                } else {
                    lastResult = route.outcome.result
                    await refreshAfterAction()
                }
            } else if (route.result === "ambiguous") {
                // The router says what it could not decide, in its own words.
                notice = t(voiceReasonKey(route.reason))
            } else if (route.result === "disabled") {
                notice = t("windows-actions-voice-disabled")
            } else {
                notice = t("windows-actions-voice-not-an-action")
            }
            actionError = ""
        } catch (error) {
            actionError = describe(error)
        } finally {
            busy = false
        }
    }

    /** Applies a settings change, or does nothing while the settings are still loading. */
    function patchSettings(patch: Partial<WindowsActionSettings>) {
        if (!settings) return
        settings = { ...settings, ...patch }
    }

    function patchScreenshots(patch: Partial<WindowsActionSettings["screenshots"]>) {
        if (!settings) return
        settings = { ...settings, screenshots: { ...settings.screenshots, ...patch } }
    }

    function dismissFired(index: number) {
        fired = fired.filter((_, position) => position !== index)
    }
</script>

{#if capabilities && !platformSupported}
    <Alert color="yellow" title={t("windows-actions-title")}>
        <Text size="sm">{t("windows-actions-unavailable")}</Text>
    </Alert>
    <Space h="md" />
{/if}

{#if actionError}
    <Alert color="red" title={t("windows-actions-title")}>{actionError}</Alert>
    <Space h="sm" />
{/if}

{#if fired.length > 0}
    {#each fired as item, index (item.id + item.status)}
        <Alert color="green" title={t(scheduledKindKey(item.kind))}>
            <Text size="sm">
                {t(firedNotification(item).body_key)}
                {item.message ?? ""}
            </Text>
            <Space h="xs" />
            <Button size="xs" variant="subtle" on:click={() => dismissFired(index)}>
                {t("windows-actions-fired-dismiss")}
            </Button>
        </Alert>
        <Space h="xs" />
    {/each}
    <Space h="sm" />
{/if}

<Text weight={700} size="lg">{t("windows-actions-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("windows-actions-description")}</Text>
<Space h="md" />

{#if capabilities}
    <Group spacing="xs">
        <Text size="sm">{t("windows-actions-capability-volume")}: {capabilities.volume ? "✓" : "—"}</Text>
        <Text size="sm">{t("windows-actions-capability-screenshots")}: {capabilities.screenshots ? "✓" : "—"}</Text>
        <Text size="sm">{t("windows-actions-capability-windows")}: {capabilities.windows ? "✓" : "—"}</Text>
        <Text size="sm">{t("windows-actions-capability-lock")}: {capabilities.lock_workstation ? "✓" : "—"}</Text>
        <Text size="sm">
            {t("windows-actions-capability-notifications")}: {capabilities.notifications ? "✓" : "—"}
        </Text>
    </Group>
    <Space h="xs" />
    <Text size="sm" color="dimmed">{t("windows-actions-capability-ai-tools")}: {tools ? t(toolAvailabilityKey(tools)) : ""}</Text>
    {#each capabilities.notes as note (note)}
        <Text size="xs" color="dimmed">{note}</Text>
    {/each}
    <Space h="md" />
{/if}

{#if preview}
    <ConfirmationDialog {preview} {busy} onConfirm={confirmPending} onCancel={cancelPending} />
    <Space h="md" />
{/if}

{#if lastResult}
    {@const summary = resultSummary(lastResult)}
    <Alert color="green" title={t("windows-actions-last-result")}>
        <Text size="sm">{t(summary.key)}{summary.value ? `: ${summary.value}` : ""}</Text>
        {#if summary.path}
            <Text size="xs" color="dimmed">{summary.path}</Text>
        {/if}
    </Alert>
    <Space h="md" />
{/if}

<!-- Quick actions -->
<Text weight={600}>{t("windows-actions-quick")}</Text>
<Space h="xs" />
<Group spacing="xs">
    <Button size="sm" disabled={busy || !capabilities?.volume} on:click={() => send(volumeUp(POLICY_VOLUME_STEP))}>
        {t("windows-actions-quick-volume-up")}
    </Button>
    <Button size="sm" disabled={busy || !capabilities?.volume} on:click={() => send(volumeDown(POLICY_VOLUME_STEP))}>
        {t("windows-actions-quick-volume-down")}
    </Button>
    <Button size="sm" disabled={busy || !capabilities?.volume} on:click={() => send(mute(true))}>
        {t("windows-actions-quick-mute")}
    </Button>
    <Button size="sm" disabled={busy || !capabilities?.volume} on:click={() => send(mute(false))}>
        {t("windows-actions-quick-unmute")}
    </Button>
</Group>
<Space h="xs" />
<Group spacing="xs">
    <Button size="sm" disabled={busy || !capabilities?.lock_workstation} on:click={() => send(lockWorkstation())}>
        {t("windows-actions-quick-lock")}
    </Button>
    <Button size="sm" disabled={busy || !capabilities?.screenshots} on:click={() => send(screenshot())}>
        {t("windows-actions-quick-screenshot")}
    </Button>
    <Button size="sm" variant="default" disabled={busy || !capabilities?.windows} on:click={refreshWindows}>
        {t("windows-actions-quick-list-windows")}
    </Button>
</Group>
<Space h="md" />

<!-- Windows -->
<Text weight={600}>{t("windows-actions-windows-title")}</Text>
<Space h="xs" />
<Text size="xs" color="dimmed">{t("windows-actions-windows-hint")}</Text>
<Space h="xs" />
{#if windows.length === 0}
    <Text size="sm" color="dimmed">{t("windows-actions-windows-empty")}</Text>
{:else}
    <table class="rows">
        <thead>
            <tr>
                <th>{t("windows-actions-windows-column-title")}</th>
                <th>{t("windows-actions-windows-column-process")}</th>
                <th>{t("windows-actions-windows-column-monitor")}</th>
                <th>{t("windows-actions-windows-column-state")}</th>
                <th />
            </tr>
        </thead>
        <tbody>
            {#each windows as item (item.id)}
                <tr>
                    <td>
                        {shortenTitle(item.title)}
                        {#if item.sensitive}
                            <Text size="xs" color="orange">{t("windows-actions-windows-sensitive")}</Text>
                        {/if}
                        {#if item.foreground}
                            <Text size="xs" color="dimmed">{t("windows-actions-windows-foreground")}</Text>
                        {/if}
                    </td>
                    <td>{item.process}</td>
                    <td>{item.monitor}</td>
                    <td>{t(windowStateKey(item.state))}</td>
                    <td class="actions">
                        {#each WINDOW_OPERATIONS as operation (operation)}
                            <Button
                                size="xs"
                                variant="subtle"
                                disabled={busy}
                                on:click={() => actOnWindow(item.id, operation)}
                            >
                                {t(windowOperationKey(operation))}
                            </Button>
                        {/each}
                        <Button size="xs" variant="subtle" disabled={busy} on:click={() => send(captureWindow(item.id))}>
                            {t("windows-actions-windows-capture")}
                        </Button>
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
{/if}
<Space h="md" />

<!-- Timers and reminders -->
<Text weight={600}>{t("windows-actions-timers-title")}</Text>
<Space h="xs" />
<Group spacing="xs">
    <label class="field">
        <span>{t("windows-actions-timer-minutes")}</span>
        <input class="line" type="number" min="1" bind:value={timerMinutes} />
    </label>
    <Button size="sm" disabled={busy} on:click={startTimer}>{t("windows-actions-timers-create")}</Button>
</Group>
<Space h="xs" />
<Group spacing="xs">
    <label class="field">
        <span>{t("windows-actions-reminder-minutes")}</span>
        <input class="line" type="number" min="1" bind:value={reminderMinutes} />
    </label>
    <label class="field">
        <span>{t("windows-actions-reminder-message")}</span>
        <input class="line wide" placeholder={t("windows-actions-reminder-placeholder")} bind:value={reminderMessage} />
    </label>
    <Button size="sm" disabled={busy} on:click={startReminder}>{t("windows-actions-timers-create")}</Button>
</Group>
<Space h="xs" />
{#if scheduled.length === 0}
    <Text size="sm" color="dimmed">{t("windows-actions-timers-empty")}</Text>
{:else}
    <table class="rows">
        <thead>
            <tr>
                <th>{t("windows-actions-timer-kind")}</th>
                <th>{t("windows-actions-windows-column-title")}</th>
                <th>{t("windows-actions-timer-countdown")}</th>
                <th />
            </tr>
        </thead>
        <tbody>
            {#each scheduled as item (item.id)}
                <tr>
                    <td>{t(scheduledKindKey(item.kind))}</td>
                    <td>
                        {item.message ?? ""}
                        {#if item.message_unreadable}
                            <Text size="xs" color="orange">{t("windows-actions-fired-unreadable")}</Text>
                        {/if}
                    </td>
                    <td>{item.status === "pending" ? formatSeconds(item.remaining_seconds) : t(scheduledStatusKey(item.status))}</td>
                    <td class="actions">
                        <Button size="xs" variant="subtle" disabled={busy} on:click={() => stopScheduled(item.kind, item.id)}>
                            {t("windows-actions-timers-cancel")}
                        </Button>
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
    <Space h="xs" />
    <Button size="xs" variant="subtle" disabled={busy} on:click={() => guard(() => windowsActionsApi.pruneScheduled())}>
        {t("windows-actions-timers-prune")}
    </Button>
{/if}
<Space h="md" />

<!-- Allowed programs -->
<Text weight={600}>{t("windows-actions-apps-title")}</Text>
<Space h="xs" />
<Text size="xs" color="dimmed">{t("windows-actions-apps-hint")}</Text>
<Space h="xs" />
<Group spacing="xs">
    <label class="field">
        <span>{t("windows-actions-apps-display-name")}</span>
        <input class="line" placeholder={t("windows-actions-apps-display-name-placeholder")} bind:value={applicationName} />
    </label>
    <Button size="sm" disabled={busy} on:click={addApplication}>{t("windows-actions-apps-add")}</Button>
</Group>
<Space h="xs" />
{#if allowedApplications.length === 0}
    <Text size="sm" color="dimmed">{t("windows-actions-apps-empty")}</Text>
{:else}
    {#each allowedApplications as application (application.id)}
        <div class="application">
            <Text size="sm" weight={600}>
                {application.display_name}
                <Text size="xs" color="dimmed">{application.executable_file_name}</Text>
            </Text>
            <Text size="xs" color="dimmed">{application.path}</Text>
            {#if application.fixed_arguments.length > 0}
                <Text size="xs" color="dimmed">
                    {t("windows-actions-apps-arguments")}: {application.fixed_arguments.join(" ")}
                </Text>
            {/if}
            {#if application.identity_changed}
                <Text size="xs" color="orange">
                    {t("windows-actions-apps-changed")}
                    {application.identity_reason ? ` (${application.identity_reason})` : ""}
                </Text>
                <Button size="xs" variant="subtle" disabled={busy} on:click={() => reacceptApplication(application.id)}>
                    {t("windows-actions-apps-reaccept")}
                </Button>
            {/if}
            <Group spacing="xs">
                <Switch
                    label={t("windows-actions-apps-enabled")}
                    checked={application.enabled}
                    disabled={busy}
                    on:change={() => setApplicationEnabled(application.id, !application.enabled)}
                />
                <Button
                    size="xs"
                    disabled={busy || !application.enabled || application.identity_changed}
                    on:click={() => startApplication(application.id)}
                >
                    {t("windows-actions-apps-start")}
                </Button>
                <Button size="xs" variant="subtle" color="red" disabled={busy} on:click={() => removeApplication(application.id)}>
                    {t("windows-actions-apps-remove")}
                </Button>
            </Group>
        </div>
        <Space h="xs" />
    {/each}
{/if}
<Space h="md" />

<!-- Spoken phrases -->
<Text weight={600}>{t("windows-actions-voice-title")}</Text>
<Space h="xs" />
<Text size="xs" color="dimmed">{t("windows-actions-voice-hint")}</Text>
<Space h="xs" />
<Group spacing="xs">
    <label class="field">
        <span>{t("windows-actions-voice-title")}</span>
        <input class="line wide" placeholder={t("windows-actions-voice-placeholder")} bind:value={voicePhrase} />
    </label>
    <Button size="sm" disabled={busy} on:click={tryVoice}>{t("windows-actions-voice-run")}</Button>
</Group>
{#if notice}
    <Text size="sm">{notice}</Text>
{/if}
<Space h="md" />

<!-- Settings -->
{#if settings}
    <Text weight={600}>{t("windows-actions-settings-title")}</Text>
    <Space h="xs" />
    <Switch
        label={t("windows-actions-settings-ai-tools")}
        checked={settings.ai_tools_enabled}
        on:change={() => patchSettings({ ai_tools_enabled: !settings?.ai_tools_enabled })}
    />
    <Switch
        label={t("windows-actions-settings-voice")}
        checked={settings.voice_actions_enabled}
        on:change={() => patchSettings({ voice_actions_enabled: !settings?.voice_actions_enabled })}
    />
    <Space h="xs" />
    <label class="field">
        <span>{t("windows-actions-settings-ttl")}</span>
        <input
            class="line"
            type="number"
            min={MIN_CONFIRM_TTL_SECONDS}
            max={MAX_CONFIRM_TTL_SECONDS}
            bind:value={settings.confirm_ttl_seconds}
        />
    </label>
    <label class="field">
        <span>{t("windows-actions-settings-screenshots-dir")}</span>
        <input class="line wide" bind:value={settings.screenshots.directory} />
    </label>
    <Switch
        label={t("windows-actions-settings-block-sensitive")}
        checked={settings.screenshots.block_sensitive_windows}
        on:change={() =>
            patchScreenshots({ block_sensitive_windows: !settings?.screenshots.block_sensitive_windows })}
    />
    <Space h="xs" />
    <Group spacing="xs">
        <Button size="sm" loading={busy} on:click={saveSettings}>{t("windows-actions-settings-save")}</Button>
        {#if saved}
            <Text size="sm" color="green">{t("windows-actions-settings-saved")}</Text>
        {/if}
    </Group>
    <Space h="md" />
{/if}

<!-- Policy table -->
<Text weight={600}>{t("windows-actions-audit-column-risk")}</Text>
<Space h="xs" />
<table class="rows">
    <tbody>
        {#each policy as row (row.action_type)}
            <tr>
                <td>{t(actionTypeKey(row.action_type))}</td>
                <td>{t(riskKey(row.risk))}</td>
                <td>{row.note}</td>
            </tr>
        {/each}
    </tbody>
</table>
<Space h="md" />

<!-- Audit log -->
<Text weight={600}>{t("windows-actions-audit-title")}</Text>
<Space h="xs" />
<Text size="xs" color="dimmed">{t("windows-actions-audit-hint")}</Text>
<Text size="xs" color="orange">{t("windows-actions-audit-not-evidence")}</Text>
<Space h="xs" />
{#if audit.length === 0}
    <Text size="sm" color="dimmed">{t("windows-actions-audit-empty")}</Text>
{:else}
    <table class="rows">
        <thead>
            <tr>
                <th>{t("windows-actions-audit-column-time")}</th>
                <th>{t("windows-actions-audit-column-action")}</th>
                <th>{t("windows-actions-audit-column-source")}</th>
                <th>{t("windows-actions-audit-column-risk")}</th>
                <th>{t("windows-actions-audit-column-result")}</th>
                <th>{t("windows-actions-audit-column-duration")}</th>
                <th>{t("windows-actions-audit-column-target")}</th>
            </tr>
        </thead>
        <tbody>
            {#each audit as entry, index (`${entry.timestamp}-${index}`)}
                <tr>
                    <td>{entry.timestamp}</td>
                    <td>{t(actionTypeKey(entry.action_type))}</td>
                    <td>{t(sourceKey(entry.source))}</td>
                    <td>{t(riskKey(entry.risk))}</td>
                    <td>{t(statusKey(entry.decision))}</td>
                    <td>{entry.duration_ms} ms</td>
                    <td>{entry.target ?? ""}</td>
                </tr>
            {/each}
        </tbody>
    </table>
{/if}
<Space h="xs" />
<Group spacing="xs">
    <Button size="xs" variant="subtle" color="red" disabled={busy} on:click={clearLog}>
        {t("windows-actions-audit-clear")}
    </Button>
    <Button size="xs" variant="subtle" disabled={busy} on:click={exportLog}>
        {t("windows-actions-audit-export")}
    </Button>
</Group>
<style>
    .rows {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.82rem;
    }

    .rows th,
    .rows td {
        padding: 0.2rem 0.35rem;
        text-align: left;
        vertical-align: top;
        border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    }

    .rows th {
        opacity: 0.7;
        font-weight: 600;
    }

    .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.15rem;
    }

    .application {
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 10px;
        padding: 0.5rem;
    }

    .field {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.8rem;
        opacity: 0.9;
    }

    .line {
        padding: 0.25rem 0.4rem;
        border-radius: 6px;
        border: 1px solid rgba(255, 255, 255, 0.15);
        background: rgba(0, 0, 0, 0.2);
        color: inherit;
        width: 8rem;
    }

    .line.wide {
        width: 18rem;
    }
</style>