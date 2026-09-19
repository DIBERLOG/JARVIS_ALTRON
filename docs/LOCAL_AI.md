# Local AI runtime

## What this is

A complete local-AI vertical slice for the Windows desktop application: one `LocalAiGateway`
in Rust owns configuration, validation, machine resources, a managed `llama-server` child
process, a loopback-only HTTP/SSE client, streaming generation events, cancellation, and two
profiles. The Windows GUI only talks to that gateway.

No model and no server binary ship with the repository, nothing is downloaded automatically,
and no automated test runs a real model. The intended Windows model is a Qwen3-8B Q4_K_M-class
GGUF file that the user selects, but nothing verifies that a file is that model (see "Model
and resource validation").

## Module map

* Contracts: `crates/jarvis-core/src/ai/{mod,conversation,error,prompt}.rs`.
* Gateway: `crates/jarvis-core/src/ai/local/gateway.rs` — lifecycle, events, cancellation.
* Configuration: `crates/jarvis-core/src/ai/local/config.rs` — shape validation, launch arguments.
* Model: `crates/jarvis-core/src/ai/local/model.rs` — GGUF inspection and file checks.
* Resources: `crates/jarvis-core/src/ai/local/resources.rs` — the memory estimate.
* Transport: `http.rs` (minimal HTTP/1.1) and `client.rs` (OpenAI shapes, SSE) in `ai/local/`.
* Process: `process.rs` — spawn, readiness, bounded stop, stderr tail.
* Commands: `crates/jarvis-gui/src/tauri_commands/local_ai.rs`.
* Interface: `frontend/src/lib/local-ai{,-model}.ts`, `frontend/src/components/ai/Local*.svelte`.
* Tests: `crates/jarvis-core/tests/{local_ai_gateway,vault_ai_isolation}.rs` and
  `frontend/tests/local-ai-{model,i18n}.test.mjs`.

## Architecture

```text
interface (Tauri commands + Svelte)
        |
        v
LocalAiGateway            config + validation + resources
        |                 process lifecycle + stderr tail
        |                 streaming client + cancellation
        v
llama-server              127.0.0.1:<port>, OpenAI-compatible HTTP
```

The rule is structural: the GUI never spawns a process and never speaks HTTP. It asks the
gateway for a status, a validation report, and a generation, and receives tokens as events.
`LocalAiHandle` holds one `Arc<LocalAiGateway>`, so two windows share one gateway and therefore
one managed server. The AI layer has no handle to the encrypted notes or password storages (see
`docs/SECURITY.md`).

## How `llama-server` is started

`LocalAiConfig::server_arguments()` returns exactly these arguments, in this order: `--model`
(the trimmed `server.model_path`), `--host`, `--port`, `--ctx-size` (`server.context_size`), and
then `--threads` (`server.cpu_threads`) and `--n-gpu-layers` (`server.gpu_layers`) only when
those values are greater than `0`. The baseline is eight arguments and no others; `0` means
"leave it to llama.cpp", so no flag is emitted. The model path is its own argument, passed
verbatim: never quoted, escaped, or joined into a shell string, so a path with spaces or shell
metacharacters cannot add arguments or change the command. `RealProcessRunner::spawn` calls
`Command::new(program)` and then `command.arg()` once per element — no shell, no `cmd /C`, no
string concatenation. stdin is `Stdio::null()`, stdout is discarded so a full pipe cannot block
the server, and stderr is piped. On Windows the child is created with `CREATE_NO_WINDOW`
(`0x0800_0000`), so no console window appears.

Loopback is enforced twice. `LocalAiConfig::validate_shape()` accepts only `127.0.0.1`, `::1`, or
`localhost`, refuses `0.0.0.0` and `::` as "binding to all interfaces is refused", and refuses
every other value with "only a loopback host is accepted in this version". `allow_lan` is kept in
the settings format but is rejected whenever it is true. `loopback_address()` independently
refuses `0.0.0.0`, `::`, `192.168.1.10`, `10.0.0.1`, `example.com`, `8.8.8.8`, and the empty
string before a socket is created, so no LAN address reaches the client.

`start()` is idempotent: from `Ready`, `Generating`, `Starting`, or `Stopping` it returns the state
instead of spawning a second process, and it fails with `ChatError::InsufficientResources` before
anything is spawned when the configuration is blocked. The states are `Stopped`, `Starting`,
`Ready`, `Generating`, `Stopping`, and `Failed`. A start moves `Stopped` or `Failed` to `Starting`
and then `Ready`; a generation moves `Ready` to `Generating` and back to `Ready` when the worker
settles, or to `Failed` on an error; a stop moves through `Stopping` back to `Stopped`; an
unexpected child exit is noticed by `status()` and reported as `Failed`.

Readiness is a `GET /health` probe: `200` is `Ready`, `503` is `Loading`, any other status is
`Unknown`, and a refused connection is `ServerNotRunning` or `TimedOut`. Probing repeats every
`READY_POLL_INTERVAL` (400 ms) until `startup_timeout_seconds` elapses (default 120, allowed
5–900); a timeout is `ChatError::StartupTimedOut` and an early exit is `ChatError::ServerStopped`.
A refused connection while starting is expected and not fatal. Probing, streaming, and stopping
all happen outside the state mutex, so a slow server never blocks a status query.

stderr is drained by a reader thread into a bounded buffer: at most `STDERR_TAIL_LINES` (200)
lines, each cleaned of control characters and truncated at `STDERR_LINE_CHARS` (400) characters,
with a total count and a truncation flag. The buffer reports an out-of-memory hint when a line
contains "out of memory", "failed to allocate", or "insufficient memory".

`stop()` is bounded: kill, wait up to `KILL_GRACE` (5 s) polling every `EXIT_POLL_INTERVAL`
(50 ms), kill again if the process is still there, wait once more, then give up. `Drop for
ManagedServer` kills the child too, so an early return cannot leave one behind. Only the child
this process started is ever touched: no `taskkill`, no process enumeration, no kill by name, so
a server the user started by hand is left alone. On exit, `main.rs` calls
`LocalAiHandle::shutdown()` for `RunEvent::ExitRequested` and `RunEvent::Exit`.

## Model and resource validation

`validate_shape()` checks every field that needs no disk access and reports each problem as a
`ConfigIssue { field, message }`: `schema_version` (must equal `CONFIG_SCHEMA_VERSION`, currently
`1`), `allow_lan`, `host`, `port` (1024–65535), `context_size` (512–262144), `cpu_threads` (at
most 256), `gpu_layers` (at most 1000), `startup_timeout_seconds` (5–900), `temperature` (0–2,
finite), `top_p` (0.05–1, finite), `max_tokens` (1–32768), `server_path`, and `model_path`.

`validate_model()` then checks what is really on disk, and only this: the server file exists, is a
regular file, and is not empty (a warning if it does not end in `.exe`); the model file exists, is
a regular file, is not empty, and ends in `.gguf` (a warning below 1 MiB); the GGUF magic `GGUF`
and a version in `1..=4`; and `general.architecture`, `general.name`, and `general.file_type`
mapped to a quantisation name (`0`=`F32` ... `15`=`Q4_K_M`) when the metadata section parses.

What is not checked: that the executable really is `llama-server` or a compatible version; file
integrity beyond the header; tensor contents; and that a file declaring `qwen3` is Qwen3. The
report says "declared architecture"; a non-Qwen declaration is a warning, not a block. No hash is
computed. Metadata parsing is bounded by `MAX_METADATA_BYTES` (32 MiB), `MAX_STRING_BYTES` (4096),
and `MAX_ARRAY_ELEMENTS` (65536); a truncated or unknown-layout metadata section downgrades to a
warning — only the header was checked — and never turns a blocked file into a usable one.

`resources.rs` is explicitly a heuristic, not a calculation. The estimate is `model_size + KV cache
+ RUNTIME_OVERHEAD_BYTES` (512 MiB), where the KV cache is `KV_BYTES_PER_TOKEN_ESTIMATE`
(`2 * 36 * 8 * 128 * 2` = 147456 bytes per context token, documented as an 8B-class figure) times
the context size. It is compared with `sysinfo`'s available memory: needing more than is available
is `Blocked`, needing more than 1/`HEADROOM_WARNING_RATIO` (1.25) of what is available is a
`Warning`, otherwise `Ok`; unreadable memory is a `Warning`, and no model size at all is `Blocked`.
GPU/VRAM usage is not estimated — the report says so — and the exact RAM need is not measured.

## Streaming and cancellation

`http.rs` is a deliberately minimal HTTP/1.1 client for exactly one peer: plain TCP to `127.0.0.1`
or `::1`, and no TLS, redirects, proxy, cookies, or authentication headers. Bodies are read in
three framings: `Content-Length` (the declared length wins and everything after it is not body),
`Transfer-Encoding: chunked` (chunk extensions after `;` are ignored), and close-delimited. The
head and the chunk framing are read one byte at a time so framing bytes never mix into the body
buffer, which keeps the incremental reader correct for chunked streams. Limits: `MAX_BODY_BYTES`
(16 MiB), `MAX_LINE_BYTES` (4 MiB), `MAX_HEAD_LINE_BYTES` (16 KiB), `MAX_HEADERS` (128). Bodies are
never logged.

The client sends `POST /v1/chat/completions` with `Accept: text/event-stream`, `Connection: close`,
`User-Agent: JARVIS-Altront-local-ai`, `Content-Type: application/json`, and a `Content-Length`
body. The body carries exactly `model`, `messages`, `stream`, `max_tokens`, `temperature`, `top_p`,
and optionally `chat_template_kwargs` and `stream_options`. A test asserts that no
credential-shaped field or header is ever sent. The messages themselves are built by the memory
context builder when memory is available (see "Chat panel and AI memory" below).

`SseParser` implements the parts of Server-Sent Events that matter: `data:` lines are accumulated
(multiple lines joined with newlines, one leading space stripped), `event:` names are kept, and
comment lines and unknown field names are ignored. `interpret_chunk()` turns each payload into a
`ChunkOutcome`: `Token`, `Thinking`, `Completed` (on `finish_reason`, a `usage` object, or the
literal `[DONE]`), `Empty`, or `Malformed`. A malformed chunk is ignored while usable tokens
arrive; a stream that produced nothing and contained unparsable data is
`ChatError::InvalidStream`, and tokens that already arrived are always kept.

The socket read timeout is `DEFAULT_READ_TIMEOUT` (300 ms), so the reader wakes regularly and a
cancellation flag set by another thread is noticed between tokens, not only when the server sends
more. `DEFAULT_STALL_TIMEOUT` (90 s, 5 s for the short probes) bounds how long no data at all is
tolerated; the connect timeout is `DEFAULT_CONNECT_TIMEOUT` (3 s). Cancellation is a flag checked
between lines and tokens, not a socket shutdown: `ChatError::Cancelled` ends the stream promptly
and the partial text stays with the caller.

Generation events are `Started` (id, model, profile, thinking, whether the thinking preference was
applied, streaming), `Token`, `Thinking`, `Completed` (usage, finish reason, duration, cancelled),
`Cancelled` (whether partial text existed), and `Failed` (a content-free message). The worker thread
`jarvis-local-ai` runs the generation; `GenerationEvent::kind()` gives a content-free label.

The frontend uses a Tauri `Channel<GenerationEvent>` instead of polling because tokens arrive as
they are produced: `applyGenerationEvent()` folds each event into the chat view and the panel
re-renders per token. A 5-second status timer in `LocalChat.svelte` tracks uptime, capabilities,
and a crashed server — it is not used to follow a generation.

## Profiles and reasoning

`Persona` has two variants, `Jarvis` and `Altron` (`AiProfile` is a type alias for it, not a
parallel enum). Both system prompts are built in Rust by `system_prompt(persona)` as the profile
instructions followed by the shared `SAFETY_CONSTRAINTS`, so a chat message can never replace or
weaken them; `PROMPT_VERSION` is `local-ai-prompt-1`. The distinction is tone and depth only:
JARVIS is steady and concise, ALTRON is critical and analytical, and ALTRON never gains a
permission. The system message is inserted by `ChatCompletionRequest::build`, not by the
interface.

`ThinkingMode` is `auto` (default), `enabled`, or `disabled`. A preference is only sent when the
running server's chat template was seen to mention a thinking switch: `probe()` reads `/props` and
sets `thinking_switch_in_template` when the `chat_template` string contains `enable_thinking` or
`thinking` (a heuristic that depends on the llama.cpp build). Without that, `chat_template_kwargs`
is omitted and `Started` reports `thinking_applied: false`. `auto` never sends a preference.

Reasoning output is surfaced only when the server sends a separate `reasoning_content` field in a
delta, which becomes `GenerationEvent::Thinking` and is rendered in a collapsible block. Text that
merely looks like reasoning is never split out by pattern matching, so the interface cannot claim a
distinction the backend did not make. A delta that carries `content` and `reasoning_content` at the
same time yields `ChunkOutcome::TokenAndThinking`, and the gateway emits the reasoning event and
then the answer event, so neither half is lost. `reasoning_field_observed` records that such a field
has been seen at least once.

## Chat panel and AI memory

`LocalChat.svelte` is the chat surface of the encrypted local AI memory (`docs/AI_MEMORY.md`). When
memory is on, the settings say history is stored, and the shared encrypted storage is unlocked, the
panel writes exactly two things: the **visible user question** (`memory_append_user_message`) and
the **visible final answer** (`memory_append_assistant_message`). Reasoning is never stored:
`MessagePayload` has no field for a thinking channel, the panel's collapsible reasoning block is
display only, and the answer that is written is the visible text only.

The request context comes from the memory context builder when it is available
(`memory_build_context`, which returns the `ContextPlanView` the panel then passes to
`local_ai_generate`), and from the panel's own local view of the current chat otherwise. The plan
never contains the system prompt: the gateway adds it from the profile, so the panel cannot
replace it.

* A **cancelled** answer is stored only on request: the partial text appears with a "keep partial"
  action, and `memory_append_assistant_message` is called with `MessageStatus::Cancelled` and
  `partial: true` only after the user asks to keep it
  (`KEEP_PARTIAL_ANSWER_DEFAULT = false`).
* A **failed** answer stores nothing: `finishAnswer` records `failed` in the local view and
  refreshes the memory status, but a failed generation is never written as a completed answer.
* A question that could not be stored is reported with the `memory-not-saved` notice, and the chat
  continues locally: storage failure never blocks a generation.
* Summaries and memory candidates are produced after the answer is stored, and only when the
  settings allow them (`auto_summaries`, `suggest_facts`); a candidate is `Pending` until the user
  approves it and picks its scope.

While the storage is locked, the panel shows the storage gate instead of conversation memory and
does not attempt a write.

## Text improvement through the same gateway

The chat panel is not the only caller. The explicit "improve text" action of the local
autocorrect feature (`docs/AUTOCORRECT.md`) drives the same gateway, so there is no second AI
client, no second `llama-server`, and no second configuration. `LocalAiTextImprover` in
`crates/jarvis-core/src/autocorrect/improvement.rs` is the consumer, and
`crates/jarvis-gui/src/tauri_commands/autocorrect.rs` runs it on the blocking pool against the
shared `Arc<LocalAiGateway>` (`state.local_ai.shared()`).

A text improvement is a `GenerationRequest` with exactly one **user** message, the profile
from the request, `thinking: Some(ThinkingMode::Disabled)`, `stream: true`,
`temperature: 0.2`, and `max_tokens` derived from the text (`chars / 2 + 256`, clamped to
256…4096). The system prompt is still added by `ChatCompletionRequest::build` from the
profile: the improvement prompt is a task sentence plus the text between `===BEGIN TEXT===`
and `===END TEXT===` with an explicit rule that the block is data, so the caller cannot
replace or weaken the constraints. As with the chat, `ThinkingMode::Disabled` is a request
that is only forwarded when the running server's template was seen to mention a thinking
switch; the reasoning channel is ignored by this path either way.

Streaming is used so a rewrite that is going the wrong way can be stopped between tokens:
`autocorrect_cancel_improvement` calls `LocalAiGateway::cancel()`, and because one gateway
serves one generation at a time that flag belongs to the improvement in flight. A cancelled
generation becomes `AutocorrectError::Cancelled`; any other gateway failure becomes
`AiUnavailable`. Both are content-free, no token is logged, and the result is a preview the
user must confirm.

The improvement path never writes AI memory. It has no memory store, no conversation, and no
fact; it borrows the gateway and returns text to its caller, and it uses only three pure
helpers from the memory module — `scan_for_secrets`, `looks_like_instruction`, and
`clean_model_text` — so nothing about the request can appear in stored history. The secret
filter gates the text **before** it is sent (`check_improvement_input`) and the answer after
it is cleaned (`check_improvement_answer`); a finding is
`AutocorrectError::SecretDetected(kinds)` (`secret_detected`) carrying only `SecretKind`
values, and the text is not sent.

## Settings

Defaults are conservative: host `127.0.0.1`, port `8080`, context `8192` tokens, `cpu_threads` `0`,
`gpu_layers` `0` (CPU only), `startup_timeout_seconds` `120`, `temperature` `0.7`, `top_p` `0.95`,
`max_tokens` `1024`, profile `jarvis`, thinking `auto`, `allow_lan` `false`, schema version `1`. No
path is guessed: `server_path` and `model_path` start empty.

The settings live under the key `local_ai_config` (`SETTINGS_KEY`) inside the existing application
settings store, written to `app.db` in the application config directory. The value is a JSON
object; it is not secret and never holds conversation text, notes, or key material. Writes go
through `fsutil::write_json_atomic` (temporary file plus rename), and a missing, empty, or damaged
value falls back to defaults instead of failing: the application always starts and the form always
has something usable.

| Command | Returns |
| --- | --- |
| `local_ai_get_config` | The `LocalAiConfig` the runtime is using |
| `local_ai_validate` | A `LocalAiReport` (level, issues, model, resources); starts nothing |
| `local_ai_save_config` | A `LocalAiReport`; stores the settings and applies them |
| `local_ai_export_config` | The written path, or an empty string when cancelled |
| `local_ai_import_config` | The imported `LocalAiConfig`, or `null` when cancelled |
| `local_ai_start` | The `LocalAiStatus` once the server answered (or was already running) |
| `local_ai_stop` | The `LocalAiStatus` after the managed child stopped |
| `local_ai_restart` | The `LocalAiStatus` after stop-then-start |
| `local_ai_status` | The `LocalAiStatus`: state, host, port, pid, files, profile, errors |
| `local_ai_generate` | The generation id as a string; events arrive on the passed channel |
| `local_ai_cancel` | `true` when a generation was running and was flagged |
| `local_ai_select_server` | The chosen `llama-server` executable path, or `null` |
| `local_ai_select_model` | The chosen `.gguf` file path, or `null` |

`local_ai_start`, `local_ai_stop`, and `local_ai_restart` run on a blocking worker pool because
readiness can wait for the whole startup timeout. Export and import use the file pickers, and only
paths, numbers, and the profile are ever written.

## How to run it

```powershell
# interface unit tests and type checks
cd frontend
npm ci
npm run build      # routify + svelte-check + vite build
npm run test:ui    # node --test tests/**/*.test.mjs

# Rust
cargo check --workspace
cargo test --workspace
cargo clippy --workspace

# AI memory, separately: the store over real files and the vault boundary
cargo test -p jarvis-core --test memory_storage
cargo test -p jarvis-core --test vault_ai_isolation
```

To try it end to end you need your own `llama-server.exe` (from a llama.cpp Windows release) and
your own GGUF model file, for example a Qwen3-8B `Q4_K_M` file. This repository ships no model and
no server binary, nothing is downloaded automatically, and no automated test exercises a real
model: the integration tests use a mock loopback HTTP server, synthetic GGUF headers, and a fake
process runner. Point the settings page at both files, start the server, and wait for `Ready`.

## Limitations and unverified claims

* No model or `llama-server` binary is bundled, fetched, or updated by the application. Without both
  files the slice cannot generate anything.
* Nothing proves that a `.gguf` file is the model it declares. Only the magic, version, and metadata
  strings are read; a non-Qwen architecture is a warning, and a file whose metadata cannot be parsed
  is accepted on its header alone.
* The memory estimate is a heuristic with fixed constants. It does not measure the real footprint,
  and GPU/VRAM usage is not estimated at all.
* No hash, signature, or provenance check is performed on the model file. `docs/ADR_LOCAL_AI.md`
  mentions SHA-256 as a later step; it is not implemented.
* `LocalAiCapabilities::streaming` mirrors reachability rather than probing the streaming endpoint,
  and the thinking switch is a substring heuristic over `/props`, so both can be wrong for an
  unusual server build.
* Only one generation runs per gateway at a time, and "stop" flags the stream rather than
  guaranteeing the server aborts the request.
* The conversation is only persisted through the encrypted AI memory, and only while memory is
  enabled and the storage is unlocked (`docs/AI_MEMORY.md`); with memory off, nothing is stored
  anywhere. There is still no tool surface: model output is never executed. Context-window
  management exists only on the memory path, and the token budget there is an estimate, so a
  request can still exceed the real window.
* Errors are content-free by design, which also means a diagnosis is often just a status code plus
  the bounded stderr tail.
* The runtime is Windows-specific in its process handling (`CREATE_NO_WINDOW`), and
  `RealProcessRunner` itself is not exercised by any automated test — only the fake runner is.
* The text-improvement path is not exercised end to end either. Its unit tests use a fixture
  `TextImprovementProvider`, so `build_improvement_prompt` and the preview rules are covered
  while the `GenerationRequest` that `LocalAiTextImprover::generate` actually sends is not:
  no test runs it against a real or mock server, and a stopped or missing server surfaces as
  `ai_unavailable` rather than as a tested path.
