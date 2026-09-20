import { test } from "node:test"
import assert from "node:assert/strict"

import {
    COMPONENT_STATES,
    COMPONENTS,
    ERROR_CODES,
    STAGES,
    STEPS,
    UNKNOWN_LABEL_KEY,
    WARNING_CODES,
    applyEvent,
    canRemoveComponent,
    componentLabelKey,
    confirmKeys,
    describeWarningKeys,
    errorLabelKey,
    formatBytes,
    formatDecimalBytes,
    formatSpeed,
    isActiveStage,
    isTerminalStage,
    missingSpaceMessage,
    normalizeView,
    originLabelKey,
    percent,
    planSummary,
    shouldPoll,
    spaceIsTight,
    stageLabelKey,
    stageOrder,
    stageOutcome,
    stageProgressStages,
    stateLabelKey,
    stepLabelKey,
    stepOfStage,
    warningLabelKey
} from "../src/lib/local-ai-setup-model.ts"

/** The model size the wizard's space table is pinned to. */
const MODEL_BYTES = 5027783488

function view(overrides = {}) {
    return {
        stage: "idle",
        step: "preflight",
        step_index: 1,
        component: null,
        downloaded_bytes: 0,
        total_bytes: 0,
        required_bytes: 12 * 1024 * 1024 * 1024,
        available_bytes: 200 * 1024 * 1024 * 1024,
        missing_bytes: 0,
        download_bytes: 5 * 1024 * 1024 * 1024,
        temporary_bytes: 6 * 1024 * 1024 * 1024,
        installed_bytes: 5 * 1024 * 1024 * 1024,
        rollback_bytes: 0,
        safety_reserve_bytes: 1024 * 1024 * 1024,
        runtime_state: "absent",
        runtime_version: null,
        runtime_bytes: 0,
        model_state: "absent",
        model_bytes: 0,
        active_origin: "unset",
        runtime_pre_release: false,
        running: false,
        can_start: true,
        can_cancel: false,
        can_retry: false,
        can_cleanup: false,
        error_code: null,
        warning_codes: [],
        recovery: {
            interrupted: false,
            interrupted_stage: null,
            previous_error_code: null,
            partial_download_bytes: 0,
            staging_ready: false,
            runtime_installed: false,
            model_installed: false
        },
        offer: {
            runtime: {
                component: "runtime",
                display_name: "runtime build",
                version: "b7000",
                source_label: "official build",
                license: "MIT",
                download_bytes: 1024 * 1024 * 1024,
                installed_bytes: 2 * 1024 * 1024 * 1024,
                pre_release: false,
                checksum_verified: true,
                quantization: null,
                architecture: "x64",
                ram_recommendation_bytes: null,
                context_recommendation: 8192
            },
            model: {
                component: "model",
                display_name: "local model",
                version: null,
                source_label: "official build",
                license: "Apache-2.0",
                download_bytes: MODEL_BYTES,
                installed_bytes: MODEL_BYTES,
                pre_release: false,
                checksum_verified: true,
                quantization: "Q4_K_M",
                architecture: "qwen3",
                ram_recommendation_bytes: 8 * 1024 * 1024 * 1024,
                context_recommendation: 8192
            },
            install_root: "user data folder",
            internet_needed_for_download_only: true,
            works_offline_after_install: true,
            steps: [...STEPS],
            stage_codes: [...STAGES]
        },
        ...overrides
    }
}

// ------------------------------------------------------------------ label keys

test("every known code maps to its own label key", () => {
    const cases = [
        [STAGES, stageLabelKey, "ai-setup-stage-"],
        [STEPS, stepLabelKey, "ai-setup-step-"],
        [COMPONENTS, componentLabelKey, "ai-setup-component-"],
        [COMPONENT_STATES, stateLabelKey, "ai-setup-state-"],
        [ERROR_CODES, errorLabelKey, "ai-setup-error-"],
        [WARNING_CODES, warningLabelKey, "ai-setup-warning-"]
    ]
    for (const [codes, build, prefix] of cases) {
        const keys = codes.map(build)
        assert.equal(
            new Set(keys).size,
            codes.length,
            `each code needs its own label, collided in ${prefix}`
        )
        assert.ok(keys.every((key) => key.startsWith(prefix)), prefix)
        assert.ok(keys.every((key) => !key.endsWith("-")), prefix)
    }
    assert.equal(stageLabelKey("complete"), "ai-setup-stage-complete")
    assert.equal(stepLabelKey("launch_test"), "ai-setup-step-launch_test")
    assert.equal(componentLabelKey("model"), "ai-setup-component-model")
    assert.equal(stateLabelKey("damaged"), "ai-setup-state-damaged")
    assert.equal(errorLabelKey("hash_mismatch"), "ai-setup-error-hash_mismatch")
    assert.equal(warningLabelKey("space_is_tight"), "ai-setup-warning-space_is_tight")
})

test("an unknown code falls back to the generic message", () => {
    // A core that adds a code must not make the panel print a key that does not
    // exist, and must not print an empty line either.
    for (const build of [stageLabelKey, stepLabelKey, componentLabelKey, stateLabelKey, errorLabelKey, warningLabelKey]) {
        assert.equal(build("a-code-from-the-future"), UNKNOWN_LABEL_KEY)
        assert.equal(build(""), UNKNOWN_LABEL_KEY)
    }
    assert.equal(UNKNOWN_LABEL_KEY, "ai-setup-unknown")
    assert.equal(originLabelKey("managed"), "ai-setup-origin-managed")
    assert.equal(originLabelKey("user_provided"), "ai-setup-origin-user_provided")
    assert.equal(originLabelKey("mixed"), "ai-setup-origin-mixed")
    assert.equal(originLabelKey("unset"), "ai-setup-origin-unset")
    assert.equal(originLabelKey(null), UNKNOWN_LABEL_KEY)
    assert.equal(originLabelKey(undefined), UNKNOWN_LABEL_KEY)
})

// ---------------------------------------------------------------- formatting

test("byte counts use binary units and keep one decimal at GiB", () => {
    assert.equal(formatBytes(0), "0 B")
    assert.equal(formatBytes(-5), "0 B")
    assert.equal(formatBytes(Number.NaN), "0 B")
    assert.equal(formatBytes(Number.POSITIVE_INFINITY), "0 B")
    assert.equal(formatBytes(512), "512 B")
    assert.equal(formatBytes(2048), "2 KiB")
    assert.equal(formatBytes(2 * 1024 * 1024), "2 MiB")
    assert.equal(formatBytes(3 * 1024 * 1024 * 1024), "3.0 GiB")
    // The pinned model size, which the panel shows as both readings.
    assert.equal(formatBytes(MODEL_BYTES), "4.7 GiB")
})

test("byte counts also read in decimal units", () => {
    assert.equal(formatDecimalBytes(0), "0 B")
    assert.equal(formatDecimalBytes(-5), "0 B")
    assert.equal(formatDecimalBytes(Number.NaN), "0 B")
    assert.equal(formatDecimalBytes(1500), "1.5 kB")
    assert.equal(formatDecimalBytes(2_500_000), "2.5 MB")
    assert.equal(formatDecimalBytes(MODEL_BYTES), "5.0 GB")
    // The two readings describe the same file, so a vendor's number matches.
    assert.notEqual(formatBytes(MODEL_BYTES), formatDecimalBytes(MODEL_BYTES))
})

test("a transfer rate is empty until there is one to report", () => {
    assert.equal(formatSpeed(2_500_000), "2.5 MB/s")
    assert.equal(formatSpeed(0), "0 B/s")
    assert.equal(formatSpeed(-1), "")
    assert.equal(formatSpeed(Number.NaN), "")
    assert.equal(formatSpeed(Number.POSITIVE_INFINITY), "")
})

test("a percentage is an integer between 0 and 100, and 0 otherwise", () => {
    assert.equal(percent(0, 100), 0)
    assert.equal(percent(50, 100), 50)
    assert.equal(percent(1, 3), 33)
    assert.equal(percent(200, 100), 100)
    assert.equal(percent(10, 0), 0)
    assert.equal(percent(0, 0), 0)
    assert.equal(percent(Number.NaN, 100), 0)
    assert.equal(percent(10, Number.NaN), 0)
    assert.equal(percent(-10, 100), 0)
})

// -------------------------------------------------------------- stage helpers

test("the stage helpers follow the documented order", () => {
    assert.equal(stageOrder("idle"), 1)
    assert.equal(stageOrder("preflight"), 2)
    assert.equal(stageOrder("complete"), 14)
    assert.equal(stageOrder("a-code-from-the-future"), 0)

    assert.equal(stepOfStage("preflight"), 1)
    assert.equal(stepOfStage("download_runtime"), 2)
    assert.equal(stepOfStage("activate_runtime"), 2)
    assert.equal(stepOfStage("download_model"), 3)
    assert.equal(stepOfStage("configure"), 4)
    assert.equal(stepOfStage("launch_test"), 5)
    assert.equal(stepOfStage("test_inference"), 6)
    assert.equal(stepOfStage("complete"), 6)
    assert.equal(stepOfStage("a-code-from-the-future"), 1)
    // Every stage must land inside the six steps the wizard renders.
    for (const code of STAGES) {
        const step = stepOfStage(code)
        assert.ok(step >= 1 && step <= STEPS.length, `${code} mapped to step ${step}`)
    }

    assert.equal(isTerminalStage("complete"), true)
    assert.equal(isTerminalStage("cancelled"), true)
    assert.equal(isTerminalStage("failed"), true)
    assert.equal(isTerminalStage("preflight"), false)
    assert.equal(isTerminalStage("idle"), false)

    assert.equal(isActiveStage("preflight"), true)
    assert.equal(isActiveStage("download_model"), true)
    assert.equal(isActiveStage("idle"), false)
    assert.equal(isActiveStage("complete"), false)
    assert.equal(isActiveStage("failed"), false)
})

test("the progress list drops idle and keeps the stage order", () => {
    assert.deepEqual(stageProgressStages(STAGES), STAGES.filter((code) => code !== "idle"))
    assert.deepEqual(stageProgressStages(["idle", "preflight", "complete"]), ["preflight", "complete"])
    // A code the panel cannot name is dropped instead of rendered blank.
    assert.deepEqual(stageProgressStages(["preflight", "from-the-future"]), ["preflight"])
    assert.deepEqual(stageProgressStages(["preflight", "preflight"]), ["preflight"])
    assert.deepEqual(stageProgressStages(null), [])
    assert.deepEqual(stageProgressStages(undefined), [])
})

test("a stage outcome is derived from the code alone", () => {
    assert.equal(stageOutcome("complete"), "ok")
    assert.equal(stageOutcome("cancelled"), "cancelled")
    assert.equal(stageOutcome("failed"), "failed")
    assert.equal(stageOutcome("download_runtime"), "running")
    assert.equal(stageOutcome("idle"), "running")
})

// -------------------------------------------------------------- normalization

test("a hostile payload is repaired into a usable view", () => {
    const repaired = normalizeView({
        // Numbers arrive as strings, as `serde` sends a wide integer.
        required_bytes: "5368709120",
        available_bytes: null,
        download_bytes: Number.NaN,
        runtime_state: "ready",
        // Everything below is missing, null, or the wrong shape entirely.
        offer: null,
        recovery: null,
        warning_codes: null,
        error_code: 42,
        component: undefined,
        runtime_version: null,
        active_origin: "somewhere",
        // A number that arrived as text is repaired in two stages: the text is
        // read when it parses, and the unusable remainder falls back to zero.
        step_index: "2",
        runtime_bytes: "not a number"
    })

    assert.equal(repaired.required_bytes, 5368709120)
    assert.equal(repaired.available_bytes, 0)
    assert.equal(repaired.download_bytes, 0)
    assert.equal(repaired.step_index, 2)
    assert.equal(repaired.runtime_bytes, 0)
    assert.equal(repaired.runtime_state, "ready")
    assert.equal(repaired.error_code, null)
    assert.equal(repaired.component, null)
    assert.equal(repaired.runtime_version, null)
    assert.equal(repaired.active_origin, "unset")
    assert.deepEqual(repaired.warning_codes, [])
    assert.equal(repaired.recovery.interrupted, false)
    assert.equal(repaired.recovery.partial_download_bytes, 0)
    // A payload that lost the lists still renders a complete wizard.
    assert.deepEqual(repaired.offer.steps, [...STEPS])
    assert.deepEqual(repaired.offer.stage_codes, [...STAGES])
    assert.equal(repaired.offer.runtime.display_name, "")
    assert.equal(repaired.offer.model.download_bytes, 0)
    // Every number the markup can reach is finite.
    for (const [key, value] of Object.entries(repaired)) {
        if (typeof value === "number") assert.ok(Number.isFinite(value), `${key} is not finite`)
    }
})

test("a payload that is not an object, and unknown codes, stay safe", () => {
    for (const value of [null, undefined, 42, "nope"]) {
        const repaired = normalizeView(value)
        assert.equal(repaired.stage, "idle")
        assert.equal(repaired.step, "preflight")
        assert.equal(repaired.step_index, 1)
        assert.equal(repaired.running, false)
        assert.deepEqual(repaired.offer.steps, [...STEPS])
    }
    // A warning code the panel cannot name is dropped, not shown as a raw code.
    const repaired = normalizeView(view({ warning_codes: ["space_is_tight", "from-the-future", 7] }))
    assert.deepEqual(repaired.warning_codes, ["space_is_tight"])
})

test("a complete payload survives normalization unchanged", () => {
    const repaired = normalizeView(view())
    assert.deepEqual(repaired, view())
})

// -------------------------------------------------------------------- progress

test("two channel messages produce a speed from the byte delta", () => {
    const first = applyEvent(
        { downloaded: 0, total: 10_000_000, startedAt: 1000, now: 1000 },
        { stage: "download_model", component: "model", downloaded_bytes: 1_000_000, total_bytes: 10_000_000 }
    )
    assert.equal(first.downloaded, 1_000_000)
    assert.equal(first.total, 10_000_000)
    // The clock did not move between the start and this message, so there is no
    // rate to report yet.
    assert.equal(first.bytesPerSecond, 0)

    const second = applyEvent(
        { downloaded: 1_000_000, total: 10_000_000, startedAt: 1000, now: 2000 },
        { stage: "download_model", component: "model", downloaded_bytes: 3_000_000, total_bytes: 10_000_000 }
    )
    assert.equal(second.downloaded, 3_000_000)
    assert.equal(second.bytesPerSecond, 2_000_000)
})

test("a channel message that advanced nothing reports no speed", () => {
    const stalled = applyEvent(
        { downloaded: 2_000_000, total: 10_000_000, startedAt: 1000, now: 5000 },
        { stage: "download_model", component: null, downloaded_bytes: 2_000_000, total_bytes: 10_000_000 }
    )
    assert.equal(stalled.bytesPerSecond, 0)
    assert.equal(stalled.downloaded, 2_000_000)
})

test("a clock that did not move reports no speed rather than an infinite one", () => {
    const sameTick = applyEvent(
        { downloaded: 1_000_000, total: 10_000_000, startedAt: 4000, now: 4000 },
        { stage: "download_model", component: null, downloaded_bytes: 3_000_000, total_bytes: 10_000_000 }
    )
    assert.equal(sameTick.downloaded, 3_000_000)
    assert.equal(sameTick.bytesPerSecond, 0)
    assert.ok(Number.isFinite(sameTick.bytesPerSecond))
})

test("a message that would move the count backwards is not a negative speed", () => {
    const backwards = applyEvent(
        { downloaded: 5_000_000, total: 10_000_000, startedAt: 0, now: 1000 },
        { stage: "download_model", component: null, downloaded_bytes: 1_000_000, total_bytes: 10_000_000 }
    )
    assert.equal(backwards.bytesPerSecond, 0)
})

// --------------------------------------------------------------------- space

test("the missing-space message carries the shortfall", () => {
    const short = normalizeView(view({ required_bytes: 10 * 1024 ** 3, available_bytes: 4 * 1024 ** 3 }))
    assert.equal(missingSpaceMessage(short).key, "ai-setup-missing-space")
    // The core's own figure wins when it sent one.
    assert.equal(missingSpaceMessage(short).bytes, 6 * 1024 ** 3)
    const reported = normalizeView(
        view({ required_bytes: 10 * 1024 ** 3, available_bytes: 4 * 1024 ** 3, missing_bytes: 123 })
    )
    assert.equal(missingSpaceMessage(reported).bytes, 123)
    // When the plan fits there is nothing missing.
    const roomy = normalizeView(view())
    assert.equal(missingSpaceMessage(roomy).bytes, 0)
    assert.equal(missingSpaceMessage(roomy).key, "ai-setup-missing-space")
})

test("space is tight only when the plan fits with little to spare", () => {
    // Plenty of room.
    assert.equal(spaceIsTight(normalizeView(view({ required_bytes: 10, available_bytes: 1000 }))), false)
    // Fits, but with less than double the peak free.
    assert.equal(spaceIsTight(normalizeView(view({ required_bytes: 10, available_bytes: 15 }))), true)
    // Exactly double is no longer tight.
    assert.equal(spaceIsTight(normalizeView(view({ required_bytes: 10, available_bytes: 20 }))), false)
    // Does not fit at all: that is the missing-space warning, not this one.
    assert.equal(spaceIsTight(normalizeView(view({ required_bytes: 10, available_bytes: 5 }))), false)
    // Nothing planned yet.
    assert.equal(spaceIsTight(normalizeView(view({ required_bytes: 0, available_bytes: 0 }))), false)
})

test("the warning list is deterministic and translated", () => {
    const many = normalizeView(
        view({
            warning_codes: ["space_is_tight", "runtime_pre_release", "model_already_installed"]
        })
    )
    assert.deepEqual(describeWarningKeys(many), [
        "ai-setup-warning-runtime_pre_release",
        "ai-setup-warning-model_already_installed",
        "ai-setup-warning-space_is_tight"
    ])
    // The order follows the code list, not the payload order.
    const reordered = normalizeView(
        view({
            warning_codes: ["model_already_installed", "space_is_tight", "runtime_pre_release"]
        })
    )
    assert.deepEqual(describeWarningKeys(reordered), describeWarningKeys(many))
    assert.deepEqual(describeWarningKeys(normalizeView(view())), [])
    // One key per warning, even when a code arrives twice.
    const duplicated = normalizeView(view({ warning_codes: ["space_is_tight", "space_is_tight"] }))
    assert.deepEqual(describeWarningKeys(duplicated), ["ai-setup-warning-space_is_tight"])
})

test("the plan summary reports every number the table renders", () => {
    const summary = planSummary(normalizeView(view({
        download_bytes: 1,
        temporary_bytes: 2,
        installed_bytes: 3,
        rollback_bytes: 4,
        safety_reserve_bytes: 5,
        required_bytes: 6,
        available_bytes: 7
    })))
    assert.deepEqual(summary, {
        download: 1,
        temporary: 2,
        installed: 3,
        rollback: 4,
        reserve: 5,
        peak: 6,
        available: 7,
        missing: 0
    })
    const short = planSummary(normalizeView(view({ required_bytes: 10, available_bytes: 4 })))
    assert.equal(short.missing, 6)
    assert.equal(short.peak, 10)
    assert.equal(short.available, 4)
    for (const value of Object.values(summary)) assert.ok(Number.isFinite(value))
})

// ------------------------------------------------------------------ controls

test("the page polls only while a run is in flight", () => {
    assert.equal(shouldPoll(normalizeView(view({ running: true }))), true)
    assert.equal(shouldPoll(normalizeView(view({ running: false }))), false)
    assert.equal(shouldPoll(null), false)
})

test("a component can only be removed when it is there and nothing is running", () => {
    const removable = normalizeView(view({ can_start: true, running: false, runtime_state: "ready" }))
    assert.equal(canRemoveComponent(removable, "runtime"), true)
    assert.equal(canRemoveComponent(removable, "model"), false)

    const both = normalizeView(view({ runtime_state: "ready", model_state: "partial" }))
    assert.equal(canRemoveComponent(both, "model"), true)

    assert.equal(canRemoveComponent(normalizeView(view({ running: true, runtime_state: "ready" })), "runtime"), false)
    assert.equal(canRemoveComponent(normalizeView(view({ can_start: false, runtime_state: "ready" })), "runtime"), false)
    // An updating component is still removable; an absent one is not.
    assert.equal(canRemoveComponent(normalizeView(view({ runtime_state: "updating" })), "runtime"), true)
    assert.equal(canRemoveComponent(normalizeView(view({ runtime_state: "damaged" })), "runtime"), true)
    assert.equal(canRemoveComponent(normalizeView(view({ runtime_state: "absent" })), "runtime"), false)
    assert.equal(canRemoveComponent(null, "runtime"), false)
})

test("the removal confirmation is a pair of keys per component", () => {
    assert.deepEqual(confirmKeys("runtime"), {
        titleKey: "ai-setup-confirm-remove-runtime",
        confirmKey: "ai-setup-remove-runtime"
    })
    assert.deepEqual(confirmKeys("model"), {
        titleKey: "ai-setup-confirm-remove-model",
        confirmKey: "ai-setup-remove-model"
    })
    assert.notEqual(confirmKeys("runtime").titleKey, confirmKeys("model").titleKey)
})
