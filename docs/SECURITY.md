# Security baseline

## Local command execution

TOML CLI commands are launched as an executable plus separate arguments. The
executor no longer invokes `cmd /C` or `sh -c`, so command arguments are not
reparsed as shell syntax. Existing Lua and AutoHotkey command packs remain a
trusted local extension surface and require a separate hardening audit before
third-party packs are accepted.

Commands use `safe`, `confirmation_required`, or `forbidden` risk levels. A
confirmation expires after 15 seconds and releases only the stored command ID.
Voice confirmation is not authentication, so destructive and elevated actions
need a future GUI confirmation flow. `browser_close` is currently `forbidden`:
its AutoHotkey implementation elevates, closes applications, and restarts
JARVIS.

## Planned GUI confirmation contract

The confirmation dialog will show the action name, target program, normalized
arguments, consequence, and an expiry countdown, with **Cancel** and
**Confirm** controls. Confirming must release an immutable prepared-action ID;
the UI must not accept a replacement executable or argument list. Voice is not
an acceptable confirmation channel for application termination, elevation,
reboot/shutdown, file deletion, system-settings changes, passwords, or data
transmission.

## AI boundary

AI providers are chat-only. They must not receive a shell, arbitrary code
runner, clipboard contents, files, notes, Vault data, passwords, API keys, or
system diagnostics without an explicit feature-specific user request. A future
tool call must be schema-validated, allow-listed, and passed through SafetyGate.

## Known risks

`Cargo.toml` declares GPL-3.0-only while `LICENSE.txt` and README describe
CC-BY-NC-SA-4.0. This licensing conflict must be resolved by the copyright
holder before distribution. It was recorded, not changed.

## Local AI runtime

The local AI runtime (`jarvis_core::ai::local`) manages one `llama-server` child
process and streams completions from it. These rules are enforced in code, not by
convention, and each is covered by a test:

* **Loopback only, twice.** `loopback_address()` refuses any host that is not
  `127.0.0.1`, `::1`, or `localhost` before a socket is created, and refuses
  `0.0.0.0`, `::`, `192.168.1.10`, `10.0.0.1`, `example.com`, `8.8.8.8`, and the
  empty string. Configuration shape validation independently refuses wildcard and
  LAN hosts, and refuses `allow_lan: true`. There is no telemetry and no network
  access beyond this loopback endpoint; the client implements no TLS, redirects,
  proxy, or cookie handling.
* **Per-argument launch, no shell.** `server_arguments()` returns the arguments as
  a list, and `RealProcessRunner::spawn` passes each one to `Command::arg`. There
  is no `cmd /C`, no `sh -c`, and no string concatenation, so spaces and shell
  metacharacters in a model path cannot add arguments (covered by a test using a
  path containing spaces, `&`, `|`, `;`, `$( )`, backticks, `%PATH%`, and quotes).
  stdin is `Stdio::null()`, stdout is discarded, and on Windows the child is
  created with `CREATE_NO_WINDOW`.
* **Only the own child process is stopped.** `ManagedServer` kills the process it
  spawned, with a bounded wait (`KILL_GRACE`, 5 s) and a second kill attempt.
  There is no `taskkill /IM`, no process enumeration, and no kill by name, so a
  server the user started by hand is untouched. Application exit calls
  `LocalAiHandle::shutdown()` so no child outlives the window.
* **No prompt, completion, stderr, or secret in logs.** `ChatError` variants carry
  no body text: a non-success HTTP status becomes `HttpStatus(u16)` and the
  response body (which may echo the prompt) is discarded. The SSE client keeps no
  content in an error, the generation events carry only content-free failure
  messages, and the command module logs `ChatError::to_string()` and nothing else.
  The `stderr_tail` is llama.cpp's own diagnostics, reduced to a bounded tail.
* **Bounded stderr tail.** At most `STDERR_TAIL_LINES` (200) lines, each stripped
  of control characters and truncated at `STDERR_LINE_CHARS` (400) characters, so
  a chatty or hostile server cannot grow memory or inject control sequences into
  the interface.
* **No AI access to the encrypted storages.** The gateway is constructed with a
  configuration only; it receives no `VaultStore`, `VaultSession`, note store, key
  material, or process-execution capability. `crates/jarvis-core/tests/vault_ai_isolation.rs`
  scans `src/ai` (and the voice and scripting surfaces) for any reference to the
  vault identifiers, and separately scans
  `crates/jarvis-gui/src/tauri_commands/local_ai.rs` for `vault`, `notes`,
  `notestore`, `vaultsession`, and `master_key`. The chat request that crosses the
  socket is checked to carry only `model`, `messages`, `stream`, `max_tokens`,
  `temperature`, `top_p`, `chat_template_kwargs`, and `stream_options`, with no
  credential-shaped field and no credential header.
* **No model download without explicit user action.** Nothing is fetched from the
  network: `server_path` and `model_path` are files the user selects through a
  file picker, the model must already exist on disk, and a missing or unusable
  file blocks the start instead of triggering a download.
* **Model output is never executed.** The model has no tool surface and no shell.
  Generated text is rendered as text in the chat panel; it is not passed to a
  command executor, `SafetyGate`, Lua, or AutoHotkey. The system prompt states the
  same rule, but the enforcement is the missing capability, not the prompt.

Limits of these controls: the model file is not hashed or verified, so a user can
select any file that carries a usable GGUF header; `llama-server` itself is a
native binary the application does not attest; the loopback rule protects the
endpoint from the network, not from another local process that connects to the
port while the server runs; and a generation cancellation sets a flag, so a
server that keeps computing after the client disconnects is not interrupted.
