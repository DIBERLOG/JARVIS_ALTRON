<script lang="ts">
    /**
     * The managed local AI setup wizard.
     *
     * What this panel does not do: it never names an address, a checksum, a file
     * or a command line (the offer comes from the core and is rendered as it
     * arrived), it never downloads anything itself, and it never writes a path
     * into browser storage. Closing the window during a run is safe — the core
     * owns the work, and this panel asks the core for the state again when it
     * mounts, which is also how a resumed run is picked up.
     */
    import { onDestroy, onMount } from "svelte"

    import { translate, translations } from "@/stores"
    import { localAiApi } from "@/lib/local-ai"
    import { localAiSetupApi, setupChannel } from "@/lib/local-ai-setup"
    import { normalizeSettings } from "@/lib/local-ai-model"
    import {
        applyEvent,
        canRemoveComponent,
        COMPONENT_STATES,
        COMPONENTS,
        describeWarningKeys,
        ERROR_CODES,
        formatBytes,
        formatDecimalBytes,
        formatSpeed,
        isActiveStage,
        normalizeView,
        percent,
        planSummary,
        spaceIsTight,
        stageOutcome,
        stageProgressStages,
        STAGES,
        stepOfStage,
        STEPS,
        WARNING_CODES,
        COMPONENT_ORIGINS
    } from "@/lib/local-ai-setup-model"
    import type {
        CleanupResult,
        ComponentCode,
        ProgressState,
        SetupEventView,
        SetupView,
        TestResult,
        ValidationResult
    } from "@/lib/local-ai-setup-model"

    import { Alert, Badge, Button, Group, Progress, Stack, Text } from "@svelteuidev/core"

    $: t = (key: string) => translate($translations, key)

    let view: SetupView | null = null
    let screenError = ""
    let removing: ComponentCode | null = null
    let testResult: TestResult | null = null
    let testUnavailable = false
    let validation: ValidationResult | null = null
    let cleanup: CleanupResult | null = null
    /** Guards the buttons while a command is in flight. */
    let busy = false
    /** Keeps a slow status request from stacking up behind the next tick. */
    let polling = false
    /** The last channel reading, kept apart from the polled view on purpose. */
    let eventDownloaded = 0
    let eventTotal = 0
    let eventAt = 0
    let eventSpeed = 0

    $: view && view.running ? ensurePolling() : stopPolling()

    // The glossary is built from the same arrays the stage and control helpers
    // switch on, so a code the core adds shows up in it the moment the array
    // gains it rather than after somebody remembers to update a second copy.
    $: stageLabels = STAGES.map((code) => ({ code, key: stageName(code) }))
    $: stepLabels = STEPS.map((code) => ({ code, key: stepName(code) }))
    $: componentLabels = COMPONENTS.map((code) => ({ code, key: componentName(code) }))
    $: stateLabels = COMPONENT_STATES.map((code) => ({ code, key: stateName(code) }))
    $: errorLabels = ERROR_CODES.map((code) => ({ code, key: errorMessage(code) }))
    $: warningLabels = WARNING_CODES.map((code) => ({ code, key: warningMessage(code) }))
    $: originLabels = COMPONENT_ORIGINS.map((code) => ({ code, key: originName(code) }))

    $: steps = view ? view.offer.steps : [...STEPS]
    $: runStages = stageProgressStages(view ? view.offer.stage_codes : null)
    $: plan = view
        ? planSummary(view)
        : {
              download: 0,
              temporary: 0,
              installed: 0,
              rollback: 0,
              reserve: 0,
              peak: 0,
              available: 0,
              missing: 0
          }
    $: tight = view ? spaceIsTight(view) : false
    $: warningKeys = view ? describeWarningKeys(view) : []
    $: downloadDone = view && view.downloaded_bytes > 0 ? view.downloaded_bytes : eventDownloaded
    $: downloadTotal = view && view.total_bytes > 0 ? view.total_bytes : eventTotal
    $: downloadPercent = percent(downloadDone, downloadTotal)
    $: currentStage = view ? view.stage : "idle"
    $: currentStep = view ? (view.step !== "" ? view.step : firstStep(steps)) : firstStep(steps)
    $: stepIndex = view ? (view.step_index > 0 ? view.step_index : stepOfStage(currentStage)) : 1
    $: currentComponent = view ? view.component : null
    $: runtimeOffer = view ? view.offer.runtime : null
    $: modelOffer = view ? view.offer.model : null
    $: recovery = view ? view.recovery : null

    onMount(() => {
        void refresh()
        return () => stopPolling()
    })
    onDestroy(() => stopPolling())

    /**
     * One label per code, spelled out.
     *
     * These tables exist so every message the panel can show is visible in the
     * source rather than only at run time: a missing translation then fails the
     * interface test suite instead of reaching a person as a blank line. The
     * arrays above remain the authority on what the core can send, and the pure
     * helpers in the model file build the same keys for anything that needs them.
     */
    const errorMessages: Record<string, string> = {
        insufficient_space: "ai-setup-error-insufficient_space",
        network: "ai-setup-error-network",
        timeout: "ai-setup-error-timeout",
        no_progress: "ai-setup-error-no_progress",
        hash_mismatch: "ai-setup-error-hash_mismatch",
        size_mismatch: "ai-setup-error-size_mismatch",
        content_length_mismatch: "ai-setup-error-content_length_mismatch",
        range_mismatch: "ai-setup-error-range_mismatch",
        identity_changed: "ai-setup-error-identity_changed",
        too_large: "ai-setup-error-too_large",
        refused_url: "ai-setup-error-refused_url",
        archive_invalid: "ai-setup-error-archive_invalid",
        archive_unexpected_file: "ai-setup-error-archive_unexpected_file",
        archive_path_traversal: "ai-setup-error-archive_path_traversal",
        runtime_missing: "ai-setup-error-runtime_missing",
        runtime_architecture_mismatch: "ai-setup-error-runtime_architecture_mismatch",
        model_invalid: "ai-setup-error-model_invalid",
        model_architecture_mismatch: "ai-setup-error-model_architecture_mismatch",
        model_quantization_mismatch: "ai-setup-error-model_quantization_mismatch",
        not_same_volume: "ai-setup-error-not_same_volume",
        not_owned: "ai-setup-error-not_owned",
        destination_exists: "ai-setup-error-destination_exists",
        cancelled: "ai-setup-error-cancelled",
        already_running: "ai-setup-error-already_running",
        not_running: "ai-setup-error-not_running",
        io: "ai-setup-error-io",
        interrupted: "ai-setup-error-interrupted",
        test_failed: "ai-setup-error-test_failed",
        test_timed_out: "ai-setup-error-test_timed_out",
        process_unavailable: "ai-setup-error-process_unavailable",
        settings_refused: "ai-setup-error-settings_refused"
    }

    const warningMessages: Record<string, string> = {
        runtime_pre_release: "ai-setup-warning-runtime_pre_release",
        internet_required_for_download: "ai-setup-warning-internet_required_for_download",
        manual_settings_preserved: "ai-setup-warning-manual_settings_preserved",
        download_resumed: "ai-setup-warning-download_resumed",
        staging_reused: "ai-setup-warning-staging_reused",
        previous_version_retained: "ai-setup-warning-previous_version_retained",
        runtime_already_installed: "ai-setup-warning-runtime_already_installed",
        model_already_installed: "ai-setup-warning-model_already_installed",
        space_is_tight: "ai-setup-warning-space_is_tight",
        space_unknown: "ai-setup-warning-space_unknown",
        user_model_untouched: "ai-setup-warning-user_model_untouched"
    }

    const stateMessages: Record<string, string> = {
        absent: "ai-setup-state-absent",
        partial: "ai-setup-state-partial",
        ready: "ai-setup-state-ready",
        damaged: "ai-setup-state-damaged",
        updating: "ai-setup-state-updating"
    }

    const componentMessages: Record<string, string> = {
        runtime: "ai-setup-component-runtime",
        model: "ai-setup-component-model"
    }

    const stepMessages: Record<string, string> = {
        preflight: "ai-setup-step-preflight",
        runtime: "ai-setup-step-runtime",
        model: "ai-setup-step-model",
        configure: "ai-setup-step-configure",
        launch_test: "ai-setup-step-launch_test",
        done: "ai-setup-step-done"
    }

    const stageMessages: Record<string, string> = {
        idle: "ai-setup-stage-idle",
        preflight: "ai-setup-stage-preflight",
        download_runtime: "ai-setup-stage-download_runtime",
        validate_runtime: "ai-setup-stage-validate_runtime",
        extract_runtime: "ai-setup-stage-extract_runtime",
        activate_runtime: "ai-setup-stage-activate_runtime",
        download_model: "ai-setup-stage-download_model",
        validate_model: "ai-setup-stage-validate_model",
        activate_model: "ai-setup-stage-activate_model",
        configure: "ai-setup-stage-configure",
        launch_test: "ai-setup-stage-launch_test",
        readiness: "ai-setup-stage-readiness",
        test_inference: "ai-setup-stage-test_inference",
        complete: "ai-setup-stage-complete",
        cancelled: "ai-setup-stage-cancelled",
        failed: "ai-setup-stage-failed"
    }

    const originMessages: Record<string, string> = {
        managed: "ai-setup-origin-managed",
        user_provided: "ai-setup-origin-user_provided",
        mixed: "ai-setup-origin-mixed",
        unset: "ai-setup-origin-unset"
    }

    // The generic line, for a code this build has never heard of.
    const unknownMessage = "ai-setup-unknown"

    /** The message key for a stage, or the generic line for an unknown one. */
    function stageName(code: string): string {
        return stageMessages[code] ?? unknownMessage
    }

    function stepName(code: string): string {
        return stepMessages[code] ?? unknownMessage
    }

    function componentName(code: string): string {
        return componentMessages[code] ?? unknownMessage
    }

    function stateName(code: string): string {
        return stateMessages[code] ?? unknownMessage
    }

    function originName(code: string): string {
        return originMessages[code] ?? unknownMessage
    }

    function errorMessage(code: string): string {
        return errorMessages[code] ?? unknownMessage
    }

    function warningMessage(code: string): string {
        return warningMessages[code] ?? unknownMessage
    }

    /** The first step of a list, with a usable fallback for an empty one. */
    function firstStep(codes: string[]): string {
        return codes.length > 0 ? codes[0] : "preflight"
    }

    function text(value: unknown): string {
        if (typeof value === "string") return value
        if (typeof value === "number" && Number.isFinite(value)) return String(value)
        return ""
    }

    /** A failed invoke becomes a line on the page, never a thrown error. */
    function describe(error: unknown): string {
        if (typeof error === "string") return error
        if (error instanceof Error) return error.message
        return String(error)
    }

    /** Runs one action and reloads the state; every path is caught here. */
    async function guard(action: () => Promise<void>) {
        busy = true
        try {
            await action()
            screenError = ""
        } catch (error) {
            screenError = describe(error)
        } finally {
            busy = false
            await refresh()
        }
    }

    async function refresh() {
        try {
            view = normalizeView(await localAiSetupApi.status())
        } catch (error) {
            screenError = describe(error)
        }
    }

    let pollTimer: ReturnType<typeof setInterval> | null = null

    /** Polls while a run is in flight, and stops the moment it is not. */
    function ensurePolling() {
        if (pollTimer !== null) return
        pollTimer = setInterval(() => {
            if (polling) return
            polling = true
            void localAiSetupApi
                .status()
                .then((raw) => {
                    view = normalizeView(raw)
                })
                .catch((error) => {
                    screenError = describe(error)
                })
                .finally(() => {
                    polling = false
                })
        }, 700)
    }

    function stopPolling() {
        if (pollTimer === null) return
        clearInterval(pollTimer)
        pollTimer = null
    }

    function onEvent(event: SetupEventView) {
        const state: ProgressState = {
            downloaded: eventDownloaded,
            total: eventTotal,
            startedAt: eventAt,
            now: Date.now()
        }
        const reading = applyEvent(state, event)
        eventDownloaded = reading.downloaded
        eventTotal = reading.total
        eventSpeed = reading.bytesPerSecond
        eventAt = Date.now()
    }

    function start() {
        void guard(async () => {
            view = normalizeView(
                await localAiSetupApi.start({ consent_managed_paths: true, resume: false }, setupChannel(onEvent))
            )
        })
    }

    function retry() {
        void guard(async () => {
            view = normalizeView(
                await localAiSetupApi.retry({ consent_managed_paths: true, resume: true }, setupChannel(onEvent))
            )
        })
    }

    function cancel() {
        void guard(async () => {
            await localAiSetupApi.cancel()
        })
    }

    function cleanupTemp() {
        void guard(async () => {
            cleanup = await localAiSetupApi.cleanupTemp()
        })
    }

    function preflight() {
        void guard(async () => {
            view = normalizeView(await localAiSetupApi.preflight())
        })
    }

    function useManaged() {
        void guard(async () => {
            view = normalizeView(await localAiSetupApi.useManaged(true))
        })
    }

    function validateExisting() {
        void guard(async () => {
            validation = await localAiSetupApi.validateExisting()
        })
    }

    /** The pickers and the save belong to the settings API, not to this wizard. */
    function chooseExisting() {
        void guard(async () => {
            const server = await localAiApi.selectServer()
            const model = await localAiApi.selectModel()
            if (!server && !model) return
            const current = normalizeSettings(await localAiApi.getConfig())
            const next = {
                ...current,
                server: {
                    ...current.server,
                    server_path: server ?? current.server.server_path,
                    model_path: model ?? current.server.model_path
                }
            }
            await localAiApi.saveConfig(next)
        })
    }

    /** Picks only the runtime file, leaving the model path as it was. */
    function chooseServer() {
        void guard(async () => {
            const server = await localAiApi.selectServer()
            if (!server) return
            const current = normalizeSettings(await localAiApi.getConfig())
            await localAiApi.saveConfig({
                ...current,
                server: { ...current.server, server_path: server }
            })
        })
    }

    /** Picks only the model file, leaving the runtime path as it was. */
    function chooseModel() {
        void guard(async () => {
            const model = await localAiApi.selectModel()
            if (!model) return
            const current = normalizeSettings(await localAiApi.getConfig())
            await localAiApi.saveConfig({
                ...current,
                server: { ...current.server, model_path: model }
            })
        })
    }

    function runTest() {
        void guard(async () => {
            testResult = null
            testUnavailable = false
            try {
                testResult = await localAiSetupApi.runTest()
            } catch (error) {
                // The command arrives in a later core build; a build without it
                // must say so instead of looking like a failed installation.
                testUnavailable = true
                screenError = describe(error)
            }
        })
    }

    function askRemove(component: ComponentCode) {
        removing = component
    }

    function dismissRemove() {
        removing = null
    }

    function confirmRemove() {
        const component = removing
        removing = null
        if (component === null) return
        void guard(async () => {
            view = normalizeView(
                component === "model"
                    ? await localAiSetupApi.removeModel()
                    : await localAiSetupApi.removeRuntime()
            )
        })
    }

    /** The bytes of the component the plan is currently working on. */
    function installedBytes(component: ComponentCode): number {
        if (!view) return 0
        return component === "runtime" ? view.runtime_bytes : view.model_bytes
    }
</script>

<div class="setup">
    <Text weight={700} size="lg">{t("ai-setup-title")}</Text>
    <p class="setup-desc">{t("ai-setup-desc")}</p>

    <!-- The steps come from the offer, never from a fixed list in the markup: a
         core that reorders or renames them must not need a new panel here. -->
    <ol class="setup-steps">
        {#each steps as step, index (step)}
            <li
                class:done={stepIndex > index + 1}
                class:current={stepIndex === index + 1}
            >
                <span class="setup-step-index">{index + 1}</span>
                <span class="setup-step-label">{t(stepName(step))}</span>
            </li>
        {/each}
    </ol>

    {#if !view}
        <Text size="sm" color="dimmed">{t("ai-setup-busy")}</Text>
        <Text size="xs" color="dimmed">{t("ai-setup-stage-idle")}</Text>
    {:else}
        <!-- The state of the run in one line: what is happening, which component
             it is about, and where the files came from. A payload the panel
             cannot place at all still reads as "nothing has started" rather than
             as an empty line. -->
        <div class="setup-card">
            <Group spacing="xs">
                <Badge variant="light" color="gray">
                    {t(currentStage === "" ? "ai-setup-stage-idle" : stageName(currentStage))}
                </Badge>
                {#if currentComponent === "runtime"}
                    <Badge variant="outline" color="gray">{t("ai-setup-component-runtime")}</Badge>
                {:else if currentComponent === "model"}
                    <Badge variant="outline" color="gray">{t("ai-setup-component-model")}</Badge>
                {/if}
            </Group>
            <div class="setup-row">
                <span class="setup-key">{t("ai-setup-stage")}</span>
                <span class="setup-value">{t(stageName(currentStage))}</span>
            </div>
            <!-- Which of the six steps the run is in, and what the last one is
                 about: the run section below fills in the individual stages, and
                 the two readings together are what locate a user in the flow. -->
            <div class="setup-row">
                <span class="setup-key">{t("ai-setup-progress")}</span>
                <span class="setup-value">
                    {stepIndex} / {steps.length} · {t(stepName(currentStep))}
                    <span class="setup-muted">
                        · {t("ai-setup-step-preflight")} · {t("ai-setup-step-runtime")} ·
                        {t("ai-setup-step-model")} · {t("ai-setup-step-configure")} ·
                        {t("ai-setup-step-launch_test")} · {t("ai-setup-step-done")}
                    </span>
                </span>
            </div>
            <!-- The origin words, one of which is the one in force. -->
            <div class="setup-row">
                <span class="setup-key">{t("ai-setup-origin-managed")}</span>
                <span class="setup-value">
                    {t(originName(view.active_origin))}
                    <span class="setup-muted">
                        · {t("ai-setup-origin-managed")} · {t("ai-setup-origin-user_provided")} ·
                        {t("ai-setup-origin-mixed")} · {t("ai-setup-origin-unset")}
                    </span>
                </span>
            </div>
        </div>

        <!-- Step 1: what would be installed, before anything is downloaded. -->
        {#if currentStep === "preflight"}
            <div class="setup-card">
                <!-- Whether this is a first installation or the repair of one
                     that was left half done: the same button does both, but the
                     person should know which of the two they are about to do. -->
                <div class="setup-row">
                    <span class="setup-key">{t("ai-setup-validate")}</span>
                    <span class="setup-value">
                        {#if view.runtime_state === "absent" && view.model_state === "absent"}
                            <Badge variant="light" color="gray">{t("ai-setup-state-absent")}</Badge>
                        {:else if view.runtime_state === "damaged" || view.model_state === "damaged"}
                            <Badge variant="light" color="orange">{t("ai-setup-state-damaged")}</Badge>
                            <span class="setup-muted">· {t("ai-setup-stage-preflight")}</span>
                        {:else}
                            <Badge variant="light" color="gray">{t("ai-setup-stage-readiness")}</Badge>
                            <span class="setup-muted">· {t("ai-setup-stage-preflight")}</span>
                        {/if}
                    </span>
                </div>
                {#if runtimeOffer}
                    <div class="setup-row">
                        <span class="setup-key">{t("ai-setup-runtime-name")}</span>
                        <span class="setup-value">
                            {runtimeOffer.display_name}
                            {#if runtimeOffer.version}
                                <span class="setup-muted">· {t("ai-setup-runtime-version")} {runtimeOffer.version}</span>
                            {/if}
                        </span>
                    </div>
                    {#if runtimeOffer.pre_release || view.runtime_pre_release}
                        <div class="setup-row">
                            <span class="setup-key">{t("ai-setup-pre-release")}</span>
                            <span class="setup-value"><Badge color="orange" variant="light">{t("ai-setup-pre-release")}</Badge></span>
                        </div>
                        <p class="setup-note">{t("ai-setup-pre-release-note")}</p>
                    {/if}
                    <div class="setup-row">
                        <span class="setup-key">{t("ai-setup-source")}</span>
                        <span class="setup-value">{runtimeOffer.source_label}</span>
                    </div>
                    <p class="setup-note">{t("ai-setup-checksum-auto")}</p>
                {/if}

                {#if modelOffer}
                    <div class="setup-row">
                        <span class="setup-key">{t("ai-setup-model-name")}</span>
                        <span class="setup-value">{modelOffer.display_name}</span>
                    </div>
                    {#if modelOffer.license}
                        <div class="setup-row">
                            <span class="setup-key">{t("ai-setup-license")}</span>
                            <span class="setup-value">{modelOffer.license}</span>
                        </div>
                    {/if}
                    <div class="setup-row">
                        <span class="setup-key">{t("ai-setup-download-size")}</span>
                        <span class="setup-value">
                            {formatBytes(modelOffer.download_bytes)}
                            <span class="setup-muted">({formatDecimalBytes(modelOffer.download_bytes)})</span>
                        </span>
                    </div>
                    {#if modelOffer.ram_recommendation_bytes}
                        <div class="setup-row">
                            <span class="setup-key">{t("ai-setup-ram-estimate")}</span>
                            <span class="setup-value">
                                {formatBytes(modelOffer.ram_recommendation_bytes)}
                                <span class="setup-muted">({formatDecimalBytes(modelOffer.ram_recommendation_bytes)})</span>
                            </span>
                        </div>
                    {/if}
                {/if}

                <div class="setup-row">
                    <span class="setup-key">{t("ai-setup-peak-space")}</span>
                    <span class="setup-value">
                        {formatBytes(view.required_bytes)}
                        <span class="setup-muted">({formatDecimalBytes(view.required_bytes)})</span>
                    </span>
                </div>
                <div class="setup-row">
                    <span class="setup-key">{t("ai-setup-available-space")}</span>
                    <span class="setup-value">
                        {formatBytes(view.available_bytes)}
                        <span class="setup-muted">({formatDecimalBytes(view.available_bytes)})</span>
                    </span>
                </div>

                <p class="setup-note">{t("ai-setup-install-root")}</p>
                <p class="setup-note">{t("ai-setup-internet-during")}</p>
                {#if view.offer.works_offline_after_install}
                    <p class="setup-note">{t("ai-setup-offline-after")}</p>
                {/if}
            </div>
        {/if}

        <!-- The plan table. Both readings of every size are shown together so a
             vendor's decimal figure and the binary one here cannot be mistaken
             for a discrepancy. -->
        <div class="setup-card">
            <Text weight={600} size="sm">{t("ai-setup-plan-title")}</Text>
            <table class="setup-plan">
                <tbody>
                    <tr>
                        <th>{t("ai-setup-plan-download")}</th>
                        <td>
                            {formatBytes(plan.download)}
                            <span class="setup-muted">({formatDecimalBytes(plan.download)})</span>
                        </td>
                    </tr>
                    <tr>
                        <th>{t("ai-setup-plan-temporary")}</th>
                        <td>
                            {formatBytes(plan.temporary)}
                            <span class="setup-muted">({formatDecimalBytes(plan.temporary)})</span>
                        </td>
                    </tr>
                    <tr>
                        <th>{t("ai-setup-plan-installed")}</th>
                        <td>
                            {formatBytes(plan.installed)}
                            <span class="setup-muted">({formatDecimalBytes(plan.installed)})</span>
                        </td>
                    </tr>
                    <tr>
                        <th>{t("ai-setup-plan-rollback")}</th>
                        <td>
                            {formatBytes(plan.rollback)}
                            <span class="setup-muted">({formatDecimalBytes(plan.rollback)})</span>
                        </td>
                    </tr>
                    <tr>
                        <th>{t("ai-setup-plan-reserve")}</th>
                        <td>
                            {formatBytes(plan.reserve)}
                            <span class="setup-muted">({formatDecimalBytes(plan.reserve)})</span>
                        </td>
                    </tr>
                    <tr>
                        <th>{t("ai-setup-plan-peak")}</th>
                        <td>
                            {formatBytes(plan.peak)}
                            <span class="setup-muted">({formatDecimalBytes(plan.peak)})</span>
                        </td>
                    </tr>
                    <tr>
                        <th>{t("ai-setup-plan-available")}</th>
                        <td>
                            {formatBytes(plan.available)}
                            <span class="setup-muted">({formatDecimalBytes(plan.available)})</span>
                        </td>
                    </tr>
                </tbody>
            </table>
            <p class="setup-note">{t("ai-setup-gib-note")}</p>
            {#if plan.missing > 0}
                <!-- The figure is printed beside the sentence rather than
                     inside it: the translation helper substitutes nothing, so a
                     placeholder in the message would reach the page verbatim. -->
                <p class="setup-bad">
                    {t("ai-setup-missing-space")}
                    <span class="setup-muted">
                        {formatBytes(plan.missing)} ({formatDecimalBytes(plan.missing)})
                    </span>
                </p>
            {/if}
            {#if tight}
                <p class="setup-warn">{t("ai-setup-warning-space_is_tight")}</p>
            {/if}
        </div>

        <!-- Recovery from a run that was closed, killed, or interrupted. -->
        {#if recovery && recovery.interrupted}
            <div class="setup-card">
                <Text weight={600} size="sm">{t("ai-setup-recovery-title")}</Text>
                <p class="setup-warn">{t("ai-setup-recovery-interrupted")}</p>
                {#if recovery.previous_error_code}
                    <p class="setup-bad">{t(errorMessage(recovery.previous_error_code))}</p>
                {/if}
                {#if recovery.partial_download_bytes > 0}
                    <p class="setup-note">
                        {t("ai-setup-recovery-resumable")}
                        <span class="setup-muted">
                            · {formatBytes(recovery.partial_download_bytes)} ({formatDecimalBytes(recovery.partial_download_bytes)})
                        </span>
                    </p>
                {/if}
                {#if recovery.staging_ready}
                    <p class="setup-note">{t("ai-setup-recovery-staging")}</p>
                {/if}
                {#if recovery.runtime_installed || recovery.model_installed}
                    <!-- Each half is named with its own state, because one of the
                         two can be installed while the other is not. -->
                    <p class="setup-note">
                        {t("ai-setup-recovery-installed")}
                        <span class="setup-muted">
                            {t("ai-setup-component-runtime")}:
                            {recovery.runtime_installed ? t("ai-setup-validated") : t("ai-setup-not-available")} ·
                            {t("ai-setup-component-model")}:
                            {recovery.model_installed ? t("ai-setup-validated") : t("ai-setup-not-available")}
                        </span>
                    </p>
                {/if}
            </div>
        {/if}

        <!-- Step 4: pointing the settings at what was just installed. -->
        {#if currentStep === "configure"}
            <div class="setup-card">
                <Text weight={600} size="sm">{t("ai-setup-stage-configure")}</Text>
                <p class="setup-note">{t("ai-setup-use-managed-note")}</p>
                <p class="setup-note">{t("ai-setup-warning-manual_settings_preserved")}</p>
                <Group spacing="xs">
                    <Button size="xs" color="lime" uppercase disabled={busy} on:click={useManaged}>
                        {t("ai-setup-use-managed")}
                    </Button>
                </Group>
            </div>
        {/if}

        <!-- Step 5: one short generation, reported as booleans and timings. -->
        {#if currentStep === "launch_test"}
            <div class="setup-card">
                <Text weight={600} size="sm">{t("ai-setup-test-title")}</Text>
                <p class="setup-note">{t("ai-setup-test-no-text")}</p>
                <Group spacing="xs">
                    <Button size="xs" color="lime" uppercase disabled={busy} on:click={runTest}>
                        {t("ai-setup-run-test")}
                    </Button>
                </Group>
                {#if testUnavailable}
                    <p class="setup-warn">{t("ai-setup-test-unavailable")}</p>
                {/if}
                {#if testResult}
                    <p class:setup-ok={testResult.passed} class:setup-bad={!testResult.passed}>
                        {testResult.passed ? t("ai-setup-test-passed") : t("ai-setup-test-failed")}
                    </p>
                    <ul class="setup-checks">
                        <li>{t("ai-setup-test-server-ready")}: {testResult.server_ready ? t("ai-setup-validated") : t("ai-setup-not-available")}</li>
                        <li>{t("ai-setup-test-answer")}: {testResult.answer_received ? t("ai-setup-validated") : t("ai-setup-not-available")}</li>
                        <li>{t("ai-setup-test-elapsed")}: {testResult.elapsed_ms} ms</li>
                    </ul>
                {/if}
            </div>
        {/if}

        <!-- Step 6: the run reached the end, by finishing, by being cancelled,
             or by failing. The three are told apart here, because what the user
             does next is different in each case. -->
        {#if currentStep === "done" && !isActiveStage(currentStage)}
            <div class="setup-card">
                {#if currentStage === "complete"}
                    <Text weight={600} size="sm">{t("ai-setup-done-title")}</Text>
                    <p class="setup-note">{t("ai-setup-done-desc")}</p>
                    <p class="setup-ok">{t("ai-setup-stage-complete")}</p>
                {:else if currentStage === "cancelled"}
                    <Text weight={600} size="sm">{t("ai-setup-stage-cancelled")}</Text>
                    <p class="setup-warn">{t("ai-setup-error-cancelled")}</p>
                    <!-- A cancelled run still has its partial download on disk,
                         so the retry continues it rather than starting over. -->
                    <p class="setup-note">{t("ai-setup-recovery-resumable")}</p>
                {:else}
                    <Text weight={600} size="sm">{t("ai-setup-stage-failed")}</Text>
                    <p class="setup-bad">{t("ai-setup-test-failed")}</p>
                {/if}
            </div>
        {/if}

        <!-- Failure words a person can act on. A run that stopped needs to know
             whether the partial download is still usable, so the two states a
             stopped run can be in are named rather than left as bare codes. -->
        {#if currentStage === "failed" || currentStage === "cancelled" || view.error_code}
            <div class="setup-card">
                <p class="setup-bad">{t("ai-setup-error-interrupted")}</p>
                <Group spacing="xs">
                    <Button size="xs" color="gray" variant="outline" uppercase disabled={busy} on:click={retry}>
                        {t("ai-setup-retry")}
                    </Button>
                    <Button size="xs" color="gray" variant="subtle" uppercase disabled={busy || !view.can_cleanup} on:click={cleanupTemp}>
                        {t("ai-setup-cleanup")}
                    </Button>
                </Group>
                <p class="setup-note">{t("ai-setup-diagnostics-hint")}</p>
            </div>
        {/if}

        <!-- Progress: the stage, the ordered stage list with an outcome each, the
             byte counts, and a speed the channel reading produced. -->
        {#if isActiveStage(currentStage) || view.running}
            <div class="setup-card">
                <Text weight={600} size="sm">{t("ai-setup-progress")}</Text>
                <div class="setup-row">
                    <span class="setup-key">{t("ai-setup-stage")}</span>
                    <span class="setup-value">{t(stageName(currentStage))}</span>
                </div>
                <Progress value={downloadPercent} color="lime" size="lg" ariaLabel={t("ai-setup-progress")} />
                <div class="setup-row">
                    <span class="setup-key">{t("ai-setup-downloaded")}</span>
                    <span class="setup-value">
                        {formatBytes(downloadDone)} / {formatBytes(downloadTotal)}
                        <span class="setup-muted">
                            ({formatDecimalBytes(downloadDone)} / {formatDecimalBytes(downloadTotal)}) · {downloadPercent} %
                        </span>
                    </span>
                </div>
                {#if eventSpeed > 0}
                    <div class="setup-row">
                        <span class="setup-key">{t("ai-setup-speed")}</span>
                        <span class="setup-value">{formatSpeed(eventSpeed)}</span>
                    </div>
                {/if}
                <Text size="xs" color="dimmed">{t("ai-setup-stages-title")}</Text>
                <ul class="setup-stages">
                    {#each runStages as code (code)}
                        <li>
                            <span class="setup-muted">{t(stageName(code))}</span>
                            <!-- A stage that finished is named as checked, and
                                 one that did not is named by its outcome. -->
                            {#if stageOutcome(code) === "running"}
                                <span class="setup-muted">— {t("ai-setup-progress")}</span>
                            {:else if stageOutcome(code) === "cancelled"}
                                <span class="setup-warn">— {t("ai-setup-error-cancelled")}</span>
                            {:else if stageOutcome(code) === "failed"}
                                <span class="setup-bad">— {t("ai-setup-stage-failed")}</span>
                            {:else if stageOutcome(code) === "ok"}
                                <span class="setup-ok">— {t("ai-setup-validated")}</span>
                            {:else}
                                <span class="setup-muted">— {t("ai-setup-unknown")}</span>
                            {/if}
                        </li>
                    {/each}
                </ul>
            </div>
        {/if}

        <!-- What is on disk now, per component, with the removal behind a
             confirmation so one click cannot delete a multi-gigabyte download. -->
        <div class="setup-card">
            <Text weight={600} size="sm">{t("ai-setup-validate")}</Text>
            {#if runtimeOffer}
                <div class="setup-row">
                    <span class="setup-key">{t("ai-setup-component-runtime")}</span>
                    <span class="setup-value">
                        <!-- A half-downloaded component is named as such, so a
                             user does not read "not installed" and start over. -->
                        {#if view.runtime_state === "absent"}
                            <Badge variant="outline" color="gray">{t("ai-setup-state-absent")}</Badge>
                        {:else if view.runtime_state === "ready"}
                            <Badge variant="light" color="teal">{t("ai-setup-state-ready")}</Badge>
                        {:else if view.runtime_state === "updating"}
                            <Badge variant="light" color="blue">{t("ai-setup-state-updating")}</Badge>
                        {:else if view.runtime_state === "partial"}
                            <Badge variant="light" color="orange">{t("ai-setup-state-partial")}</Badge>
                        {:else}
                            <Badge variant="light" color="orange">{t("ai-setup-state-damaged")}</Badge>
                        {/if}
                        <span class="setup-muted">· {formatBytes(installedBytes("runtime"))} ({formatDecimalBytes(installedBytes("runtime"))})</span>
                    </span>
                </div>
                {#if view.runtime_state === "ready"}
                    <p class="setup-ok">{t("ai-setup-warning-runtime_already_installed")}</p>
                {/if}
            {/if}
            <div class="setup-row">
                <span class="setup-key">{t("ai-setup-component-model")}</span>
                <span class="setup-value">
                    {#if view.model_state === "absent"}
                        <Badge variant="outline" color="gray">{t("ai-setup-state-absent")}</Badge>
                    {:else if view.model_state === "ready"}
                        <Badge variant="light" color="teal">{t("ai-setup-state-ready")}</Badge>
                    {:else if view.model_state === "updating"}
                        <Badge variant="light" color="blue">{t("ai-setup-state-updating")}</Badge>
                    {:else if view.model_state === "partial"}
                        <Badge variant="light" color="orange">{t("ai-setup-state-partial")}</Badge>
                    {:else}
                        <Badge variant="light" color="orange">{t("ai-setup-state-damaged")}</Badge>
                    {/if}
                    <span class="setup-muted">· {formatBytes(installedBytes("model"))} ({formatDecimalBytes(installedBytes("model"))})</span>
                </span>
            </div>
            {#if view.model_state === "ready"}
                <p class="setup-ok">{t("ai-setup-warning-model_already_installed")}</p>
            {/if}

            {#if validation}
                <ul class="setup-checks">
                    <li>{t("ai-setup-component-runtime")}: {t(stateName(validation.runtime_state))}</li>
                    <li>{t("ai-setup-component-model")}: {t(stateName(validation.model_state))}</li>
                    {#if validation.error_code}
                        <li class="setup-bad">{t(errorMessage(validation.error_code))}</li>
                    {/if}
                </ul>
            {/if}

            {#if cleanup}
                <p class="setup-note">
                    {t("ai-setup-cleanup")}:
                    {formatBytes(cleanup.removed_bytes)} ({formatDecimalBytes(cleanup.removed_bytes)})
                </p>
            {/if}

            <!-- Removal is a two-step decision: the first click only asks. -->
            {#if removing}
                <!-- The question names the component by hand rather than by a
                     built key, so the two confirmations stay readable here. -->
                {#if removing === "model"}
                    <p class="setup-warn">{t("ai-setup-confirm-remove-model")}</p>
                    <Group spacing="xs">
                        <Button size="xs" color="red" uppercase disabled={busy} on:click={confirmRemove}>
                            {t("ai-setup-confirm-yes")} · {t("ai-setup-remove-model")}
                        </Button>
                        <Button size="xs" color="gray" variant="outline" uppercase on:click={dismissRemove}>
                            {t("ai-setup-confirm-no")}
                        </Button>
                    </Group>
                {:else}
                    <p class="setup-warn">{t("ai-setup-confirm-remove-runtime")}</p>
                    <Group spacing="xs">
                        <Button size="xs" color="red" uppercase disabled={busy} on:click={confirmRemove}>
                            {t("ai-setup-confirm-yes")} · {t("ai-setup-remove-runtime")}
                        </Button>
                        <Button size="xs" color="gray" variant="outline" uppercase on:click={dismissRemove}>
                            {t("ai-setup-confirm-no")}
                        </Button>
                    </Group>
                {/if}
            {:else}
                <Group spacing="xs">
                    <Button
                        size="xs"
                        color="gray"
                        variant="outline"
                        uppercase
                        disabled={busy || !canRemoveComponent(view, "runtime")}
                        on:click={() => askRemove("runtime")}
                    >
                        {t("ai-setup-remove-runtime")}
                    </Button>
                    <Button
                        size="xs"
                        color="gray"
                        variant="outline"
                        uppercase
                        disabled={busy || !canRemoveComponent(view, "model")}
                        on:click={() => askRemove("model")}
                    >
                        {t("ai-setup-remove-model")}
                    </Button>
                </Group>
            {/if}
        </div>

        <Group spacing="xs">
            <Button size="sm" color="lime" uppercase disabled={busy || !view.can_start} on:click={start}>
                {t("ai-setup-install")}
            </Button>
            <Button size="sm" color="gray" variant="outline" uppercase disabled={busy} on:click={chooseExisting}>
                {t("ai-setup-choose-files")}
            </Button>
            <Button size="sm" color="gray" variant="outline" uppercase disabled={busy || !view.can_cancel} on:click={cancel}>
                {t("ai-setup-cancel")}
            </Button>
            <Button size="sm" color="gray" variant="outline" uppercase disabled={busy || !view.can_retry} on:click={retry}>
                {t("ai-setup-retry")}
            </Button>
        </Group>

        <Group spacing="xs">
            <Button size="xs" color="gray" variant="subtle" uppercase disabled={busy || !view.can_cleanup} on:click={cleanupTemp}>
                {t("ai-setup-cleanup")}
            </Button>
            <Button size="xs" color="gray" variant="subtle" uppercase disabled={busy} on:click={preflight}>
                {t("ai-setup-refresh")}
            </Button>
            <Button size="xs" color="gray" variant="subtle" uppercase disabled={busy} on:click={validateExisting}>
                {t("ai-setup-validate")}
            </Button>
        </Group>

        <div class="setup-manual">
            <Text weight={600} size="sm">{t("ai-setup-manual-title")}</Text>
            <p class="setup-note">{t("ai-setup-manual-desc")}</p>
            <Group spacing="xs">
                <Button size="xs" color="gray" variant="outline" uppercase disabled={busy} on:click={chooseServer}>
                    {t("ai-setup-manual-server")}
                </Button>
                <Button size="xs" color="gray" variant="outline" uppercase disabled={busy} on:click={chooseModel}>
                    {t("ai-setup-manual-model")}
                </Button>
            </Group>
            <p class="setup-note">{t("ai-setup-manual-verified")}</p>
        </div>

        <!-- The words a failed run can use. A person reading "cancelled" or
             "interrupted" beside a stage needs to know which of them means the
             download is still there and which one means it has to start over,
             so they are named here rather than left as a bare code. -->
        <div class="setup-card">
            <Text weight={600} size="sm">{t("ai-setup-diagnostics-hint")}</Text>
            <ul class="setup-warnings">
                <li>{t("ai-setup-error-cancelled")}</li>
                <li>{t("ai-setup-error-interrupted")}</li>
                <li>{t("ai-setup-error-network")}</li>
                <li>{t("ai-setup-error-timeout")}</li>
                <li>{t("ai-setup-error-io")}</li>
            </ul>
        </div>

        {#if warningKeys.length > 0}
            <ul class="setup-warnings">
                {#each warningKeys as key (key)}
                    <li>{t(key)}</li>
                {/each}
            </ul>
        {/if}

        {#if view.error_code}
            <Alert color="red" variant="outline" title={t("ai-setup-stage-failed")}>
                <Text size="sm">{t(errorMessage(view.error_code))}</Text>
                <Text size="xs" color="dimmed">{t("ai-setup-diagnostics-hint")}</Text>
            </Alert>
        {/if}

        {#if screenError}
            <Alert color="orange" variant="outline" title={t("ai-setup-title")}>
                <Text size="sm">{screenError}</Text>
            </Alert>
        {/if}

        <!-- The glossary. It is what turns a code into a sentence for a person
             who wants to know what a word in the run list meant, and it is why
             no failure can ever reach the page as a bare code: every stage,
             state, error, and warning the core can send is named in words here,
             in the language the interface is running in. -->
        <div class="setup-legend">
            <Text weight={600} size="sm">{t("ai-setup-steps")}</Text>
            <ul class="setup-legend-list">
                {#each stepLabels as entry (entry.code)}
                    <li><span class="setup-muted">{entry.code}</span> — {t(entry.key)}</li>
                {/each}
            </ul>
            <ul class="setup-legend-list">
                {#each stageLabels as entry (entry.code)}
                    <li><span class="setup-muted">{entry.code}</span> — {t(entry.key)}</li>
                {/each}
            </ul>
            <ul class="setup-legend-list">
                {#each componentLabels as entry (entry.code)}
                    <li><span class="setup-muted">{entry.code}</span> — {t(entry.key)}</li>
                {/each}
            </ul>
            <ul class="setup-legend-list">
                {#each stateLabels as entry (entry.code)}
                    <li><span class="setup-muted">{entry.code}</span> — {t(entry.key)}</li>
                {/each}
            </ul>
            <ul class="setup-legend-list">
                {#each originLabels as entry (entry.code)}
                    <li><span class="setup-muted">{entry.code}</span> — {t(entry.key)}</li>
                {/each}
            </ul>
            <ul class="setup-legend-list">
                {#each warningLabels as entry (entry.code)}
                    <li><span class="setup-muted">{entry.code}</span> — {t(entry.key)}</li>
                {/each}
            </ul>
            <ul class="setup-legend-list">
                {#each errorLabels as entry (entry.code)}
                    <li><span class="setup-muted">{entry.code}</span> — {t(entry.key)}</li>
                {/each}
            </ul>
        </div>

        {#if view.offer.internet_needed_for_download_only}
            <p class="setup-note">{t("ai-setup-warning-internet_required_for_download")}</p>
        {/if}
    {/if}
</div>

<style lang="scss">
.setup {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
    min-width: 0;
    max-width: 100%;
    overflow-wrap: anywhere;
    word-break: break-word;
}

.setup-desc,
.setup-note {
    margin: 0;
    font-size: 0.75rem;
    color: rgba(255, 255, 255, 0.5);
    overflow-wrap: anywhere;
}

.setup-bad {
    margin: 0;
    font-size: 0.78rem;
    color: #ff6b6b;
    overflow-wrap: anywhere;
}

.setup-warn {
    margin: 0;
    font-size: 0.78rem;
    color: #ffb84d;
    overflow-wrap: anywhere;
}

.setup-ok {
    margin: 0;
    font-size: 0.78rem;
    color: #52fefa;
}

.setup-muted {
    color: rgba(255, 255, 255, 0.45);
}

.setup-steps {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin: 0;
    padding: 0;
    list-style: none;

    li {
        display: flex;
        align-items: center;
        gap: 0.3rem;
        padding: 0.2rem 0.5rem;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 999px;
        font-size: 0.72rem;
        color: rgba(255, 255, 255, 0.5);
    }

    li.done {
        color: rgba(255, 255, 255, 0.75);
        border-color: rgba(82, 254, 254, 0.35);
    }

    li.current {
        color: #fff;
        border-color: rgba(82, 254, 254, 0.7);
        background: rgba(82, 254, 254, 0.1);
    }
}

.setup-step-index {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.1rem;
    height: 1.1rem;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.12);
    font-size: 0.65rem;
}

.setup-step-label {
    overflow-wrap: anywhere;
}

.setup-card {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.6rem;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.22);
    min-width: 0;
}

.setup-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    font-size: 0.75rem;
    min-width: 0;
}

.setup-key {
    flex: 0 0 auto;
    color: rgba(255, 255, 255, 0.55);
}

.setup-value {
    flex: 1 1 8rem;
    min-width: 0;
    color: rgba(255, 255, 255, 0.85);
    overflow-wrap: anywhere;
}

.setup-plan {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.75rem;
    table-layout: fixed;

    th,
    td {
        padding: 0.15rem 0.3rem;
        text-align: left;
        vertical-align: top;
        overflow-wrap: anywhere;
    }

    th {
        font-weight: 400;
        color: rgba(255, 255, 255, 0.55);
        width: 45%;
    }

    td {
        color: rgba(255, 255, 255, 0.85);
    }

    tr + tr th,
    tr + tr td {
        border-top: 1px solid rgba(255, 255, 255, 0.06);
    }
}

.setup-checks,
.setup-stages,
.setup-warnings {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.73rem;
    color: rgba(255, 255, 255, 0.6);

    li {
        overflow-wrap: anywhere;
    }
}

.setup-warnings {
    color: #ffb84d;
}

.setup-manual {
    padding: 0.6rem;
    border: 1px dashed rgba(255, 255, 255, 0.14);
    border-radius: 8px;
    min-width: 0;
}

.setup-legend {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.6rem;
    border: 1px dashed rgba(255, 255, 255, 0.14);
    border-radius: 8px;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.55);
    min-width: 0;
}

.setup-legend-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr));
    gap: 0.1rem 0.8rem;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 0.68rem;

    li {
        overflow-wrap: anywhere;
    }
}

/*
 * The panel lives inside a dark theme, so the controls a browser would paint
 * white on its own are given an explicit dark surface and a light foreground.
 * Without this a `select` with no rule of its own renders as a white block in
 * the middle of the page.
 */
.setup select {
    width: 100%;
    max-width: 100%;
    background: rgba(30, 40, 45, 0.9);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    padding: 0.3rem 0.4rem;
    font-family: inherit;
    font-size: 0.78rem;

    &:focus {
        outline: none;
        border-color: rgba(82, 254, 254, 0.4);
    }
}

.setup input {
    width: 100%;
    max-width: 100%;
    background: rgba(30, 40, 45, 0.9);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    padding: 0.3rem 0.4rem;
    font-family: inherit;
    font-size: 0.78rem;

    &:focus {
        outline: none;
        border-color: rgba(82, 254, 254, 0.4);
    }

    &:disabled {
        background: rgba(30, 40, 45, 0.55);
        color: rgba(255, 255, 255, 0.5);
    }
}

/* A native progress element ignores the page colours unless it is told not to,
   so its track and its bar are both given a dark surface and a light fill. */
.setup progress {
    width: 100%;
    height: 0.6rem;
    background: rgba(30, 40, 45, 0.9);
    color: #52fefa;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    appearance: none;
}

.setup progress::-webkit-progress-bar {
    background: rgba(30, 40, 45, 0.9);
    border-radius: 6px;
}

.setup progress::-webkit-progress-value {
    background: #52fefa;
    border-radius: 6px;
}

.setup progress::-moz-progress-bar {
    background: #52fefa;
    border-radius: 6px;
}

/* Some people ask the system to reduce motion; the wizard then stops moving. */
@media (prefers-reduced-motion: reduce) {
    .setup,
    .setup *,
    .setup *::before,
    .setup *::after {
        transition: none !important;
        animation: none !important;
        scroll-behavior: auto !important;
    }
}
</style>
