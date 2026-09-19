import { test } from "node:test"
import assert from "node:assert/strict"

import {
    DEFAULT_CONTEXT_SIZE,
    DEFAULT_HOST,
    DEFAULT_MAX_TOKENS,
    DEFAULT_PORT,
    DEFAULT_TEMPERATURE,
    DEFAULT_TOP_P,
    MAX_CONTEXT_SIZE,
    MAX_GPU_LAYERS,
    MAX_MAX_TOKENS,
    MAX_THREADS,
    MIN_CONTEXT_SIZE,
    PROFILES,
    THINKING_MODES,
    applyGenerationEvent,
    beginExchange,
    buildRequest,
    canGenerate,
    canSend,
    canStart,
    canStop,
    defaultSettings,
    describeModel,
    emptyChatView,
    failExchange,
    fieldLabelKey,
    formatBytes,
    formatDuration,
    formatUsage,
    hasWarning,
    isBlocked,
    normalizeSettings,
    profileLabelKey,
    reportLevelKey,
    serverSummary,
    stateLabelKey,
    thinkingAvailable,
    thinkingLabelKey,
    validateDraft,
    worstLevel
} from "../src/lib/local-ai-model.ts"

/** A complete, valid configuration, as the settings page would produce. */
function configured(overrides = {}) {
    const settings = defaultSettings()
    settings.server.server_path = "C:\\tools\\llama-server.exe"
    settings.server.model_path = "C:\\models\\Qwen3-8B-Q4_K_M.gguf"
    return { ...settings, ...overrides, server: { ...settings.server, ...(overrides.server ?? {}) } }
}

/** A status as `local_ai_status` returns it. */
function status(state, overrides = {}) {
    return {
        state,
        host: "127.0.0.1",
        port: 8080,
        pid: state === "stopped" ? null : 4321,
        model_file: "Qwen3-8B-Q4_K_M.gguf",
        server_file: "llama-server.exe",
        profile: "jarvis",
        thinking: "auto",
        capabilities: {
            endpoint_available: state !== "stopped",
            model_id: "qwen3-8b",
            models: ["qwen3-8b"],
            streaming: state !== "stopped",
            thinking_switch: false,
            reasoning_field_observed: false,
            build_info: null,
            notes: []
        },
        last_error: null,
        stderr_tail: [],
        stderr_truncated: false,
        out_of_memory_hint: false,
        uptime: null,
        generating: state === "generating",
        ...overrides
    }
}

// ------------------------------------------------------------------ settings

test("the default settings are conservative and loopback only", () => {
    const settings = defaultSettings()
    assert.equal(settings.server.host, DEFAULT_HOST)
    assert.equal(settings.server.port, DEFAULT_PORT)
    assert.equal(settings.server.context_size, DEFAULT_CONTEXT_SIZE)
    assert.equal(settings.server.cpu_threads, 0)
    assert.equal(settings.server.gpu_layers, 0)
    assert.equal(settings.allow_lan, false)
    assert.equal(settings.profile, "jarvis")
    assert.equal(settings.thinking, "auto")
    assert.equal(settings.max_tokens, DEFAULT_MAX_TOKENS)
    assert.equal(settings.temperature, DEFAULT_TEMPERATURE)
    assert.equal(settings.top_p, DEFAULT_TOP_P)
    // No path is guessed: the user must choose the executable and the model.
    assert.equal(settings.server.server_path, "")
    assert.equal(settings.server.model_path, "")
    assert.deepEqual([...PROFILES], ["jarvis", "altron"])
    assert.deepEqual([...THINKING_MODES], ["auto", "enabled", "disabled"])
})

test("settings from the backend are repaired rather than trusted", () => {
    const repaired = normalizeSettings({
        server: {
            server_path: "  C:\\tools\\llama-server.exe ",
            model_path: "C:\\models\\m.gguf",
            host: "  127.0.0.1 ",
            port: Number.NaN,
            context_size: 999999,
            cpu_threads: -5,
            gpu_layers: 99999,
            startup_timeout_seconds: 1
        },
        temperature: Number.POSITIVE_INFINITY,
        top_p: 4,
        max_tokens: 0,
        profile: "skynet",
        thinking: "maybe",
        allow_lan: "yes",
        schema_version: 99
    })
    assert.equal(repaired.server.host, "127.0.0.1")
    assert.equal(repaired.server.port, DEFAULT_PORT)
    assert.equal(repaired.server.context_size, MAX_CONTEXT_SIZE)
    assert.equal(repaired.server.cpu_threads, 0)
    assert.equal(repaired.server.gpu_layers, MAX_GPU_LAYERS)
    assert.equal(repaired.server.startup_timeout_seconds, 5)
    assert.equal(repaired.temperature, DEFAULT_TEMPERATURE)
    assert.equal(repaired.top_p, 1)
    assert.equal(repaired.max_tokens, 1)
    assert.equal(repaired.profile, "jarvis")
    assert.equal(repaired.thinking, "auto")
    // Only a real boolean may switch the loopback rule off.
    assert.equal(repaired.allow_lan, false)
    assert.equal(repaired.schema_version, 1)
    // A path is trimmed but never rewritten.
    assert.equal(repaired.server.server_path, "C:\\tools\\llama-server.exe")
})

test("a missing or unusable settings object falls back to the defaults", () => {
    for (const value of [null, undefined, {}, { server: null }, 42]) {
        assert.deepEqual(normalizeSettings(value), defaultSettings())
    }
})

test("the local checks report each problem on its own field", () => {
    assert.deepEqual(validateDraft(configured()), [])
    assert.equal(worstLevel(validateDraft(configured())), "ok")

    const missing = validateDraft(defaultSettings())
    assert.deepEqual(
        missing.map((issue) => issue.field).sort(),
        ["model_path", "server_path"]
    )
    assert.ok(missing.every((issue) => issue.level === "blocked"))

    const wrong_extension = validateDraft(configured({ server: { model_path: "C:\\models\\model.txt" } }))
    assert.equal(worstLevel(wrong_extension), "warning")
    assert.ok(wrong_extension.some((issue) => issue.field === "model_path"))

    // Every refused value is caught locally, so the form explains it while the
    // user types rather than only when the server is started.
    const cases = [
        [{ server: { host: "0.0.0.0" } }, "host"],
        [{ server: { host: "::" } }, "host"],
        [{ server: { host: "192.168.1.10" } }, "host"],
        [{ server: { port: 80 } }, "port"],
        [{ server: { context_size: 16 } }, "context_size"],
        [{ server: { context_size: MAX_CONTEXT_SIZE + 1 } }, "context_size"],
        [{ server: { cpu_threads: MAX_THREADS + 1 } }, "cpu_threads"],
        [{ server: { gpu_layers: MAX_GPU_LAYERS + 1 } }, "gpu_layers"],
        [{ server: { startup_timeout_seconds: 1 } }, "startup_timeout_seconds"],
        [{ server: { startup_timeout_seconds: 5000 } }, "startup_timeout_seconds"],
        [{ temperature: 9 }, "temperature"],
        [{ top_p: 0 }, "top_p"],
        [{ max_tokens: 0 }, "max_tokens"],
        [{ max_tokens: MAX_MAX_TOKENS + 1 }, "max_tokens"],
        [{ allow_lan: true }, "allow_lan"]
    ]
    for (const [patch, field] of cases) {
        const issues = validateDraft(configured(patch))
        assert.ok(
            issues.some((issue) => issue.field === field),
            `expected a local issue on ${field} for ${JSON.stringify(patch)}`
        )
        assert.equal(isBlocked(issues), true, `${field} must block starting`)
    }

    // Loopback aliases are accepted, and a wide context is allowed.
    for (const host of ["127.0.0.1", "::1", "localhost"]) {
        assert.deepEqual(validateDraft(configured({ server: { host } })), [])
    }
    assert.deepEqual(validateDraft(configured({ server: { context_size: MIN_CONTEXT_SIZE } })), [])
})

test("warnings and blocks are summarised by the worst level", () => {
    assert.equal(worstLevel([{ level: "ok" }]), "ok")
    assert.equal(worstLevel([{ level: "ok" }, { level: "warning" }]), "warning")
    assert.equal(worstLevel([{ level: "warning" }, { level: "blocked" }]), "blocked")
    assert.equal(hasWarning([{ level: "warning" }]), true)
    assert.equal(isBlocked([{ level: "warning" }]), false)
})

test("every field maps to a translatable label", () => {
    const fields = [
        "server_path",
        "model_path",
        "host",
        "port",
        "context_size",
        "cpu_threads",
        "gpu_layers",
        "startup_timeout_seconds",
        "temperature",
        "top_p",
        "max_tokens",
        "allow_lan",
        "schema_version"
    ]
    const keys = fields.map(fieldLabelKey)
    assert.equal(new Set(keys).size, fields.length, "each field needs its own label")
    assert.ok(keys.every((key) => key.startsWith("ai-settings-field-")))
    assert.equal(fieldLabelKey("something-new"), "ai-settings-field-generic")
})

test("label helpers produce the keys the components render", () => {
    assert.equal(stateLabelKey("generating"), "ai-chat-state-generating")
    assert.equal(profileLabelKey("altron"), "ai-chat-profile-altron")
    assert.equal(thinkingLabelKey("disabled"), "ai-chat-thinking-disabled")
    assert.equal(reportLevelKey("blocked"), "ai-settings-report-blocked")
})

// -------------------------------------------------------------------- status

test("the controls follow the lifecycle state", () => {
    for (const state of ["stopped", "starting", "ready", "generating", "stopping", "failed"]) {
        const value = status(state)
        assert.equal(canGenerate(value), state === "ready" || state === "generating", state)
        assert.equal(canStart(value), state === "stopped" || state === "failed", state)
        assert.equal(canStop(value), state !== "stopped", state)
    }
    assert.equal(canGenerate(null), false)
    assert.equal(canStart(null), true)
    assert.equal(canStop(null), false)
})

test("a reasoning preference is only available when the server reports a switch", () => {
    const without = status("ready")
    assert.equal(thinkingAvailable(without, "auto"), true)
    assert.equal(thinkingAvailable(without, "enabled"), false)
    assert.equal(thinkingAvailable(without, "disabled"), false)

    const with_switch = status("ready", {
        capabilities: { ...without.capabilities, thinking_switch: true }
    })
    assert.equal(thinkingAvailable(with_switch, "enabled"), true)
    assert.equal(thinkingAvailable(with_switch, "disabled"), true)
    assert.equal(thinkingAvailable(null, "auto"), true)
    assert.equal(thinkingAvailable(null, "enabled"), false)
})

test("the server summary shows the model and the loopback endpoint", () => {
    assert.equal(serverSummary(status("ready")), "qwen3-8b · 127.0.0.1:8080")
    assert.equal(
        serverSummary(status("ready", { capabilities: { ...status("ready").capabilities, model_id: null } })),
        "Qwen3-8B-Q4_K_M.gguf · 127.0.0.1:8080"
    )
    assert.equal(serverSummary(null), "")
})

// ----------------------------------------------------------------- chat view

test("an exchange starts with the question and an empty answer", () => {
    const view = beginExchange(emptyChatView(), "Привет, JARVIS", 1000)
    assert.equal(view.entries.length, 2)
    assert.deepEqual(view.entries[0], { role: "user", text: "Привет, JARVIS", thinking: "" })
    assert.deepEqual(view.entries[1], { role: "assistant", text: "", thinking: "" })
    assert.equal(view.generating, true)
    assert.equal(view.startedAtMs, 1000)
})

test("streamed tokens accumulate in one answer, including non-Latin text", () => {
    let view = beginExchange(emptyChatView(), "вопрос", 0)
    const before = view.entries
    view = applyGenerationEvent(view, { type: "token", text: "От" }, 10)
    view = applyGenerationEvent(view, { type: "token", text: "вет" }, 20)
    view = applyGenerationEvent(view, { type: "token", text: ": 42" }, 30)
    assert.equal(view.entries[1].text, "Ответ: 42")
    assert.equal(view.entries[0].text, "вопрос", "the question is untouched")
    // Each token produced a new array, which is what makes Svelte re-render the
    // bubble while the answer streams.
    assert.notEqual(view.entries, before)
    assert.notEqual(view.entries[1], before[1])
})

test("reasoning output is kept apart from the answer", () => {
    let view = beginExchange(emptyChatView(), "why", 0)
    view = applyGenerationEvent(view, { type: "thinking", text: "потому" }, 5)
    view = applyGenerationEvent(view, { type: "thinking", text: " что" }, 6)
    view = applyGenerationEvent(view, { type: "token", text: "Because." }, 7)
    assert.equal(view.entries[1].thinking, "потому что")
    assert.equal(view.entries[1].text, "Because.")
})

test("the started event records what the server actually accepted", () => {
    let view = beginExchange(emptyChatView(), "hi", 500)
    view = applyGenerationEvent(
        view,
        {
            type: "started",
            id: "c0ffee",
            model: "qwen3-8b",
            profile: "altron",
            thinking: "enabled",
            thinking_applied: false,
            stream: true
        },
        510
    )
    assert.equal(view.generationId, "c0ffee")
    assert.equal(view.model, "qwen3-8b")
    assert.equal(view.profile, "altron")
    assert.equal(view.thinking, "enabled")
    assert.equal(view.thinkingApplied, false)
    assert.equal(view.streamed, true)
    assert.equal(view.generating, true)
})

test("completion, cancellation, and failure each settle the view", () => {
    const answered = applyGenerationEvent(
        beginExchange(emptyChatView(), "hi", 100),
        {
            type: "completed",
            usage: { prompt_tokens: 12, completion_tokens: 30, total_tokens: 42 },
            finish_reason: "stop",
            duration_ms: 900,
            cancelled: false
        },
        1450
    )
    assert.equal(answered.generating, false)
    assert.equal(answered.finishReason, "stop")
    assert.deepEqual(answered.usage, { prompt_tokens: 12, completion_tokens: 30, total_tokens: 42 })
    assert.equal(answered.elapsedMs, 1350)
    assert.equal(answered.cancelled, false)
    assert.equal(answered.error, null)

    const cancelled = applyGenerationEvent(
        beginExchange(emptyChatView(), "hi", 100),
        { type: "cancelled", partial: true, duration_ms: 250 },
        300
    )
    assert.equal(cancelled.generating, false)
    assert.equal(cancelled.cancelled, true)
    assert.equal(cancelled.elapsedMs, 200)

    const failed = applyGenerationEvent(
        beginExchange(emptyChatView(), "hi", 100),
        { type: "failed", error: "the model server is not reachable" },
        200
    )
    assert.equal(failed.generating, false)
    assert.equal(failed.error, "the model server is not reachable")
    assert.equal(failed.elapsedMs, 100)

    // A completion without a start time still reports the core duration.
    const late = applyGenerationEvent(
        emptyChatView(),
        { type: "cancelled", partial: false, duration_ms: 777 },
        0
    )
    assert.equal(late.elapsedMs, 777)
})

test("a failure before any event is shown on the view", () => {
    const view = failExchange(beginExchange(emptyChatView(), "hi", 0), "write a message first")
    assert.equal(view.generating, false)
    assert.equal(view.error, "write a message first")
    assert.equal(view.elapsedMs, 0)
})

test("a token with no open answer is still rendered", () => {
    const view = applyGenerationEvent(emptyChatView(), { type: "token", text: "orphan" }, 0)
    assert.equal(view.entries.length, 1)
    assert.equal(view.entries[0].text, "orphan")
})

// ------------------------------------------------------------------ requests

test("the request carries the history, the draft, and the sampling settings", () => {
    let view = beginExchange(emptyChatView(), "first", 0)
    view = applyGenerationEvent(view, { type: "token", text: "answer one" }, 1)
    view = applyGenerationEvent(
        view,
        { type: "completed", usage: null, finish_reason: "stop", duration_ms: 5, cancelled: false },
        6
    )

    const settings = configured({ profile: "altron", temperature: 0.2, top_p: 0.5, max_tokens: 64 })
    const request = buildRequest(view, settings, "second")
    assert.deepEqual(request.messages, [
        { role: "user", content: "first" },
        { role: "assistant", content: "answer one" },
        { role: "user", content: "second" }
    ])
    assert.equal(request.profile, "altron")
    assert.equal(request.thinking, "auto")
    assert.equal(request.stream, true)
    assert.equal(request.max_tokens, 64)
    assert.equal(request.temperature, 0.2)
    assert.equal(request.top_p, 0.5)
    // The system prompt is never sent from here: the core adds it from the
    // profile, so the interface cannot replace the constraints.
    assert.equal(request.messages.some((message) => message.role === "system"), false)
})

test("an empty history is not sent as an empty message", () => {
    let view = beginExchange(emptyChatView(), "only", 0)
    view = applyGenerationEvent(view, { type: "token", text: "" }, 1)
    const request = buildRequest(view, configured(), "next")
    assert.deepEqual(request.messages, [{ role: "user", content: "only" }, { role: "user", content: "next" }])
    assert.equal(request.messages.every((message) => message.content.length > 0), true)
})

test("sending is refused while generating, without text, or without a server", () => {
    const settings = configured()
    const view = emptyChatView()
    assert.equal(canSend(view, status("ready"), "hello", settings), true)
    assert.equal(canSend(view, status("ready"), "   ", settings), false)
    assert.equal(canSend(view, status("stopped"), "hello", settings), false)
    assert.equal(canSend(view, status("starting"), "hello", settings), false)
    assert.equal(canSend(view, null, "hello", settings), false)
    assert.equal(canSend({ ...view, generating: true }, status("generating"), "hello", settings), false)
    // An incomplete configuration cannot send either.
    assert.equal(canSend(view, status("ready"), "hello", defaultSettings()), false)
    assert.equal(canSend(view, status("ready"), "hello", configured({ allow_lan: true })), false)
})

// ---------------------------------------------------------------- formatting

test("byte counts read like the core report", () => {
    assert.equal(formatBytes(0), "0 B")
    assert.equal(formatBytes(512), "512 B")
    assert.equal(formatBytes(2048), "2 KiB")
    assert.equal(formatBytes(5 * 1024 * 1024), "5 MiB")
    assert.equal(formatBytes(3 * 1024 * 1024 * 1024), "3.0 GiB")
    assert.equal(formatBytes(Number.NaN), "0 B")
    assert.equal(formatBytes(-1), "0 B")
})

test("durations and token counts are only shown when they exist", () => {
    assert.equal(formatDuration(0), "0 ms")
    assert.equal(formatDuration(320), "320 ms")
    assert.equal(formatDuration(1500), "1.5 s")
    assert.equal(formatDuration(Number.NaN), "0 ms")

    assert.equal(formatUsage(null), "")
    assert.equal(formatUsage({ prompt_tokens: 12, completion_tokens: 30 }), "12 in · 30 out")
    assert.equal(formatUsage({ total_tokens: 42 }), "42 total")
    assert.equal(formatUsage({}), "")
})

test("the model line never claims the file was verified", () => {
    const model = {
        path: "C:\\models\\m.gguf",
        file_name: "Qwen3-8B-Q4_K_M.gguf",
        size_bytes: 5 * 1024 * 1024 * 1024,
        gguf: {
            version: 3,
            tensor_count: 291,
            metadata_entries: 24,
            architecture: "qwen3",
            name: "Qwen3 8B",
            quantisation: "Q4_K_M"
        }
    }
    const line = describeModel(model)
    assert.equal(line, "Qwen3-8B-Q4_K_M.gguf (Q4_K_M, 5.0 GiB, qwen3)")
    assert.equal(line.toLowerCase().includes("verified"), false)
    assert.equal(describeModel(null), "")

    const bare = describeModel({
        ...model,
        gguf: { ...model.gguf, architecture: null, quantisation: null }
    })
    assert.equal(bare, "Qwen3-8B-Q4_K_M.gguf (5.0 GiB)")
})
