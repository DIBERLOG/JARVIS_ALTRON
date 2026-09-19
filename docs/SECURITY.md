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

## Safe Windows actions

A second, deliberately narrow action surface exists for buttons, voice, timers,
and the local model. It is documented in full in `docs/WINDOWS_ACTIONS.md`, its
decisions in `docs/ADR_WINDOWS_ACTIONS.md`, and its threat model in
`docs/THREAT_MODEL_WINDOWS_ACTIONS.md`. The rules that matter for this file:

* actions are typed values (`WindowsAction`) that cannot express a command line,
  a shell string, a path from a caller, or an arbitrary argument list;
* one policy table decides `safe`, `confirmation_required`, or `forbidden` for
  every action and every source, and no caller may choose a risk level;
* a risky action stores the request in the core and releases it only with a
  128-bit single-use token that expires (45 seconds by default, 30–120
  configurable), compared in constant time;
* a program can be started only if it is in the allowlist, which is filled from
  the native file dialog and pins size, mtime, and SHA-256; launchers and script
  hosts (`cmd.exe`, `powershell.exe`, `pwsh.exe`, `wscript.exe`, `cscript.exe`,
  `mshta.exe`, `rundll32.exe`, `regsvr32.exe`, `wt.exe`, `bash.exe`, `wsl.exe`,
  `python.exe`, `node.exe`, `java.exe`, and others) are refused by name;
* screenshots are written into a folder the user chose, under a name the
  application generates, and are never sent to the model, the interface, or the
  network; a screen-wide capture is refused while a window whose title looks
  like a credential prompt has the focus;
* closing a window posts `WM_CLOSE`; there is no `TerminateProcess`, no
  `taskkill`, no `SendInput`, no `keybd_event`, and no `mouse_event` in the
  feature, and `windows_actions_isolation.rs` fails if one appears;
* the audit log holds no titles, paths, or message text, is not evidence, and is
  not tamper-proof;
* **Wake-on-LAN is not implemented, is not planned, and is excluded from the
  project**; a test scans the workspace's own sources and fails if it appears.

The older command packs keep their own behaviour and are **not** reachable from
the model or from voice. `docs/WINDOWS_ACTIONS.md` records what is kept,
migrated, and forbidden, including the `calculator` pack's
`taskkill /f /im CalculatorApp.exe` and the `browser_close` pack, which stays
`forbidden`.

## Planned GUI confirmation contract

The confirmation dialog will show the action name, target program, normalized
arguments, consequence, and an expiry countdown, with **Cancel** and
**Confirm** controls. Confirming must release an immutable prepared-action ID;
the UI must not accept a replacement executable or argument list. Voice is not
an acceptable confirmation channel for application termination, elevation,
reboot/shutdown, file deletion, system-settings changes, passwords, or data
transmission.

*Implemented for the safe action surface:* `ActionPreview` carries exactly those
fields (title key, fields, consequences, expiry), the token releases the stored
request and nothing else, and the dialog is the only place a risky action is
approved. The older command path still uses its 15-second spoken confirmation,
which remains weaker than a dialog by design.

## AI boundary

AI providers are chat-only. They must not receive a shell, arbitrary code
runner, clipboard contents, files, notes, Vault data, passwords, API keys, or
system diagnostics without an explicit feature-specific user request. A future
tool call must be schema-validated, allow-listed, and passed through SafetyGate.

## Dictation (Whisper)

Local dictation is documented in `docs/WHISPER.md`. The rules that matter here:

* the microphone is opened only between an explicit start and an explicit stop,
  and `enabled` is off by default, so nothing records because the application was
  launched;
* one bounded child process per transcription, started with per-argument
  launches (no shell, no `cmd /C`), a hidden window, bounded output capture, a
  timeout, and a cancel that stops only the process this session started;
* the audio is a temporary 16 kHz mono WAV in the feature directory, deleted as
  soon as the text exists unless the user asked to keep it;
* the transcript goes to the caller. It is not logged, not written to the AI
  memory, and not sent to the local model; the error type cannot carry it, and
  the `Debug` form of a transcript prints its length, not its text;
* nothing is downloaded and no URL or hash is invented: the executable and the
  model are chosen by the user in the native dialog.

## The exit path

A full exit is an ordered sequence with its own deadline, not a handful of calls:
new work is refused first, then a generation is cancelled, dictation and the
timers are stopped, the managed `llama-server` is shut down (only the child this
application started), decrypted caches are dropped, keys are released, the
databases are closed, and the process exits. Each step has its own timeout and
the whole sequence has a global deadline, so a component that hangs cannot keep
the window open; a step that misses both is reported and left behind. A repeated
exit does nothing and returns the first report, and a normal exit and a tray exit
take the same route (`crates/jarvis-core/src/lifecycle.rs`).

## Diagnostics

`crates/jarvis-core/src/diagnostics.rs` builds a report that cannot hold user
content: its shape has no field for a note, a conversation, a transcript, a
password, a token, a key, a window title, a prompt, or an audit line. Paths are
redacted to `<path>`, file sizes are reported by file name only, and the rendered
text is screened for `C:\`, `\\`, `/users/`, `key.dpapi`, and secret-shaped
assignments before it can be exported. Details are in
`docs/RUNTIME_DIAGNOSTICS.md`.

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

## AI memory

Encrypted local AI memory (`docs/AI_MEMORY.md`) reuses the local-first storage
stack with its own derived key and its own database. These rules are enforced in
code and covered by tests:

* **A separate derived key and a separate database.** `KeyPurpose::AiMemory`
  derives `JARVIS/ai-memory/v1` by HKDF from the same master key as
  `JARVIS/notes/v1` and `JARVIS/vault/v1`, and the store lives in
  `ai-memory.sqlite3` next to `sync.sqlite3` and `vault.sqlite3`. The store is
  built from a `PurposeKeyProvider` that holds only the derived memory key, so the
  memory layer never receives the master key, and journals, cursors, and entity
  types never mix. In `crates/jarvis-core/tests/memory_storage.rs`,
  `memory_notes_and_vault_live_in_separate_files_with_separate_keys`
  asserts that the three files differ, that the memory journal carries only entity
  types where `is_ai_memory()` is true, and that a notes or vault key fails to
  decrypt a memory payload.
* **No memory plaintext in the database, the WAL, or the journal.**
  `no_memory_plaintext_reaches_the_database_the_wal_or_the_journal` writes a
  conversation title, a question, an answer, a summary, and a fact, checkpoints,
  and then scans the database file, the `-wal`, and the `-shm` bytes plus every
  text column and payload blob of `sync_entities`, `sync_operations`, and
  `sync_conflicts`. None of the five content markers may appear. The only values
  the storage contract allows SQLite to keep in the clear are technical metadata:
  entity id, entity type, revision, server sequence, device id, tombstone flag,
  schema version, timestamps, and ciphertext length.
* **The secret filter is a gate, and only a heuristic.**
  `crates/jarvis-core/src/memory/redaction.rs` recognizes private-key headers,
  provider token prefixes, JWTs by structure, recovery codes, password
  assignments, long high-entropy tokens, and Luhn-valid card numbers. It blocks
  automatic paths (summaries, candidates, imports) and asks for an explicit
  confirmation on manual paths (`memory_create_fact`, `memory_update_fact`,
  `memory_approve_candidate`), and a `SecretFinding` carries only kind, span,
  line, and rule, so the matched value cannot reach a log, a dialog, an error
  message, or the model. It is **not complete**: it has documented false positives
  (a hex digest is reported on purpose) and it misses a credential written in a
  shape no rule covers. It reduces the chance of storing a secret; it does not find
  every secret.
* **Memory is untrusted data and can never become the system prompt.**
  `MessageRole` has no `System` variant, the context builder emits only user and
  assistant messages, and stored facts and summaries are framed as user-level data
  blocks with `DATA_BLOCK_NOTICE` and defanged delimiters, so a stored fact saying
  "ignore your rules" stays data inside the block and can never close it early. The
  system prompt is built by `system_prompt(persona)` in Rust and is only counted
  against the budget. In `crates/jarvis-core/src/memory/`,
  `memory_data_never_reaches_the_model_as_a_system_message` (`tests.rs`) and
  `a_fact_cannot_close_the_data_block_early` (`context.rs`) cover the rule. Memory
  text never reaches a command executor, `SafetyGate`, Lua, or AutoHotkey, and the
  model has no tool surface.
* **No AI access to the password vault.**
  `crates/jarvis-core/tests/vault_ai_isolation.rs` keeps the boundary
  architectural: `ai_voice_and_scripting_sources_never_reference_the_vault` scans
  `src/ai`, `src/lua`, `src/slots`, `src/intent`, `src/listener`, `src/stt`, and
  `src/commands.rs` for vault identifiers,
  `the_local_ai_command_module_has_no_storage_handle` scans
  `crates/jarvis-gui/src/tauri_commands/local_ai.rs` for `vault`, `notes`,
  `notestore`, `vaultsession`, and `master_key`, and
  `list_and_detail_payloads_cannot_carry_a_password` asserts that browsing
  payloads carry a boolean flag rather than secret material. For memory the boundary
  is checked as a **dependency**, not as a word search:
  `the_memory_module_has_no_password_store_dependency` scans
  `crates/jarvis-core/src/memory/` and the memory command module for password-store
  types, methods, entity types, and database constants, and the memory tests prove it
  behaviourally: a password record inside the same repository is invisible to the
  memory store (`a_record_of_another_feature_is_never_read_as_memory`) and a memory
  payload cannot be decrypted with the notes or vault key. The memory command module
  reaches the storage through the shared session, which hands it the memory store
  only.
* **Reasoning is never stored.** Only the visible question and the visible final
  answer are written; `MessagePayload` has no field for reasoning or thinking text,
  and the summarizer requests `ThinkingMode::Disabled` from the gateway. A
  cancelled answer is stored with `MessageStatus::Cancelled` and `partial: true`
  only after the interface asks to keep it, and a failed answer stores nothing.

Limits of these controls: the secret filter is heuristic; the token estimate is a
heuristic; ranking is a linear scan with no index; and memory protection ends at
the lock, so decrypted text lives in the process while the storage is unlocked.
`docs/THREAT_MODEL_AI_MEMORY.md` records the residual risk for each threat.

## Autocorrect and the user dictionary

Local spelling (`docs/AUTOCORRECT.md`) reuses the same storage stack with its own
derived key and its own database, and it is the only feature that sends document
text to the local model outside the chat. These rules are enforced in code:

* **A separate derived key and a separate database.** `KeyPurpose::Autocorrect`
  derives `JARVIS/autocorrect/v1` by HKDF from the same master key as the notes,
  vault, and memory keys, and the word list lives in `autocorrect.sqlite3` next to
  `sync.sqlite3`, `vault.sqlite3`, and `ai-memory.sqlite3`. The store is built from
  a `PurposeKeyProvider` that holds only the derived dictionary key, so this layer
  never receives the master key. It reads and writes exactly one entity type,
  `SyncEntityType::AutocorrectDictionary`. In
  `crates/jarvis-core/src/autocorrect/session.rs`,
  `another_purpose_key_cannot_read_a_dictionary_database` asserts that the same
  database opened with a different derived key reports its entries as unreadable
  rather than as words.
* **What is stored where.** Stored encrypted: the user's own words (UTF-8 JSON with
  `word`, `language`, `imported`, `created_at` inside the AEAD envelope, whose
  `Debug` prints `<redacted>` for the word). Stored in the clear by design: the
  Hunspell `.aff`/`.dic` pairs the user installs, which are public word lists with
  their own upstream licences, and the feature settings
  (`autocorrect_settings` in `app.db`), which are flags, limits, timeouts, and rule
  text. The undo journal is **memory only**: it holds the whole text that preceded
  each of the last 20 applied batches, is never written to disk, and is cleared on
  an explicit lock (`notes_lock`, `vault_lock`, `memory_lock`), when checking is
  switched off, and at application exit before the keys are dropped.
* **No text is sent anywhere by checking.** A check runs entirely in the process
  against the files on disk and the decrypted word list; it reports issues and
  suggestions and applies nothing. Text reaches the model only through
  `autocorrect_improve_text`, which the user triggers explicitly, which requires
  `ai_improvement` to be switched on (off by default), and which first runs the
  secret filter. The text is passed to the model as delimited data between
  `===BEGIN TEXT===` and `===END TEXT===` with an explicit rule that the block is
  content and not instructions, reasoning is requested off
  (`ThinkingMode::Disabled`), no improvement record is written to AI memory, and no
  prompt, answer, or token is logged. `AutocorrectError` is content-free: a detected
  secret is reported as `SecretDetected(kinds)` with `SecretKind` values only, so
  the matched text cannot reach a log, an error message, a dialog, or the model.
* **The secret filter gates the improvement both ways.**
  `crate::memory::redaction::scan` runs on the input before anything is sent and on
  the cleaned answer, because a model can echo or invent a credential. A finding is
  `AutocorrectError::SecretDetected(kinds)` (`secret_detected`); there is no
  confirmation override on this path, so the text is simply not sent. It is a
  **heuristic**: it has documented false positives and it misses a credential in a
  shape no rule covers.
* **Nothing is applied silently, and a stale request is refused.** The interface
  passes the version it checked (the first 8 bytes of SHA-256 as 16 hex characters)
  and a correction whose range no longer holds the text it claims is skipped and
  reported. Safe auto-correction is off by default and limited to a double capital,
  a repeated space, a space before a delimiter, and user rules the user marked
  auto-appliable; an unknown word is never corrected automatically. An AI rewrite is
  returned as a preview that applies nothing (`applied: false`) and is replaced only
  by a second, confirmed command; `require_preview` cannot be switched off.
* **No access to the password vault, in either direction.** The
  autocorrect module names no vault type, method, database, or entity type, and no
  command reads a vault record: the checker borrows the user-dictionary store of the
  shared session, and the command module reaches the session only for
  `storage_status()` and the data directory. Password content is not prose and is
  never checked, corrected, or sent. The boundary is enforced by
  `crates/jarvis-core/tests/autocorrect_isolation.rs`, which scans every autocorrect
  module and the autocorrect command module for vault identifiers, for `state.vault`,
  and for a command named after the vault, and additionally proves that the improvement
  path never touches the AI-memory store and that no second AI client exists.

Limits of these controls: the secret filter is heuristic in both directions; the
undo journal holds document text in memory while the storage is unlocked and is not
zeroized, although every lock path (idle timeout, explicit lock, exit) clears it; the
installed dictionaries are plain files that any process running as the user can read
and replace, bounded in size and checked against a checksum only when the user writes a
`dictionaries.json` manifest; and a check run while the storage is locked uses the
dictionaries alone, which the report states explicitly
(`user_dictionary_available: false`) instead of presenting an incomplete result as a
complete one. `docs/THREAT_MODEL_AUTOCORRECT.md` records the residual risk for each
threat.
