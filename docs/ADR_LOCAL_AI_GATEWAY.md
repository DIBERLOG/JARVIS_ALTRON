# ADR: local AI gateway

## Status

Accepted.

## Context

The desktop application needs a local chat model without becoming a network client. The
earlier plan in `docs/ADR_LOCAL_AI.md` decided to run `llama-server` as a child process and
speak its OpenAI-compatible HTTP API on loopback, with Qwen3-8B Q4_K_M and an 8192-token
context as the Windows defaults. That plan left the shape of the runtime open: who owns the
process, who validates the model, who talks HTTP, and where cancellation lives.

Constraints that shaped the decision:

* The GUI is a web view. It must not be able to spawn a process or reach a socket on its own,
  and it must not be handed a capability the rest of the application does not trust it with.
* A local model may be slow: loading a GGUF file from disk can take minutes, and a token
  stream can stall. A blocking call on the window thread would freeze the application.
* The model must never gain a tool surface. The encrypted notes and password storages are
  Rust-only capabilities with their own key session, and the AI layer must not be able to
  reach them.
* The peer is one known `llama-server` on `127.0.0.1`. A general HTTP stack, with TLS,
  redirects, proxies, and a cookie jar, would be capability the runtime does not need.

## Decision

**One gateway in Rust.** `LocalAiGateway` (`crates/jarvis-core/src/ai/local/gateway.rs`) is
the only layer that manages the server. It owns:

* configuration and validation (`config.rs`, `model.rs`, `resources.rs`);
* the state machine `Stopped → Starting → Ready → Generating → Stopping → Stopped`, plus
  `Failed`, and the `last_error` that goes with it;
* the lifecycle of exactly one `ManagedServer` child, including its bounded stderr tail;
* the loopback HTTP client, the SSE parser, and the typed generation events;
* cancellation, which is an `AtomicBool` checked between tokens.

**The interface only asks.** Tauri commands (`crates/jarvis-gui/src/tauri_commands/local_ai.rs`)
forward a configuration, a request for a status or a report, and a generation request. They
never contain a `Command`, a socket, or a URL. The frontend (`frontend/src/lib/local-ai.ts`)
calls those commands and nothing else.

**Loopback only.** `loopback_address()` refuses any host that is not `127.0.0.1`, `::1`, or
`localhost` before a socket is created, and configuration validation refuses `0.0.0.0`, `::`,
and every LAN address. `allow_lan` exists in the settings format but is rejected in this
version, so the field is explicit rather than silently absent.

**No tool surface and no storage access.** The AI contracts in `crates/jarvis-core/src/ai`
never receive a vault handle, a note store, a Lua sandbox, or a process-execution capability,
and the gateway is constructed with a configuration only. The isolation test
(`crates/jarvis-core/tests/vault_ai_isolation.rs`) scans the AI sources, the voice and
scripting surfaces, and the AI command module for any reference to the encrypted storages, so
the boundary fails loudly if a later change weakens it.

**A hand-written HTTP/1.1 client.** `http.rs` supports the three framings the endpoint uses
(`Content-Length`, `Transfer-Encoding: chunked`, close-delimited), incremental line reading
for SSE, a bounded body, and — the reason it exists — a socket-level read timeout
(`DEFAULT_READ_TIMEOUT`, 300 ms) plus a stall timeout. It sends no `Authorization` header and
no credential-shaped body field, and it never logs a body.

## Alternatives considered and rejected

* **An HTTP client library without a per-read timeout.** A blocking read would have to return
  before the cancellation flag could be noticed, so a cancel would only take effect when the
  server sent the next chunk — or never, on a stalled stream.
* **Adding the needed timeouts to such a library.** A new network dependency for one known peer,
  to be maintained and audited, buys nothing over a small client that already has to implement
  the line reader that SSE needs.
* **Spawning `llama-server` from the frontend, or through a shell.** The web view would own a
  process, and a model path would pass through shell parsing. Rejected on capability grounds.
* **A per-token IPC polling loop.** N round trips per answer, a status channel doing the work of
  an event channel, and no prompt way to notice a stall. The Tauri `Channel<GenerationEvent>`
  delivers one event per token and closes the loop when the worker settles.
* **Adding rules or tools to the model.** The model is text-only by design; actions stay behind
  the structured command path and `SafetyGate`. A model that could call a tool would need schema
  validation, an allow-list, and confirmation first.
* **A separate helper process for the AI layer.** One more thing to package, supervise, and
  secure, for a boundary the Rust module already enforces in-process.

## Consequences

The positive consequences are the ones the constraints asked for: the window never blocks
(readiness and generation run off the window thread), the process is supervised and stopped on
application exit, the endpoint is unreachable from the network, cancellation is prompt, and
the encrypted storages stay unreachable from the AI layer.

The costs and the deliberate gaps:

* A hand-written HTTP client is small on purpose. It does not implement keep-alive, redirects,
  compression, `Expect: 100-continue`, or HTTP/2, and it would need review before it talked to
  anything but this endpoint.
* One gateway means one managed server per application, not one per window, and one generation
  at a time. That is a current limit, not a claimed feature.
* The AI layer has no conversation memory: the interface holds the visible history and sends it
  again with each request, and nothing is persisted. Approved AI memory remains a later stage.
* Validation is shallow by design: a `.gguf` file's magic, version, and declared metadata are
  checked, but tensor contents, file integrity, and the claimed architecture are not. The
  memory estimate is a heuristic with fixed constants, not a measurement.
* Stopping flags the running stream; it does not guarantee that `llama-server` aborts the
  request on its side.
* Nothing downloads a model, and no automated test runs a real one: the tests use a mock
  loopback server, synthetic GGUF headers, and a fake process runner.

## References

* `docs/ADR_LOCAL_AI.md` — the earlier plan for the local provider, its profiles, and its
  command-safety rules.
* `docs/LOCAL_AI.md` — the implemented surface: module map, launch arguments, validation,
  streaming, profiles, settings, and the commands.
* `docs/SECURITY.md` — the boundary rules the local AI runtime enforces and the tests that
  prove them.
