# ADR: safe Windows actions

Status: accepted, implemented on the `feature/ai-memory-windows-commands`
branch. Supersedes nothing; it is the first decision record about what this
application may do to the machine it runs on.

## Context

The application already had two ways to act on Windows: command packs
(`resources/commands/*`) executed by `jarvis-app`, `jarvis-cli`, and a GUI
listener, and a Lua sandbox that exposes `jarvis.system.exec`. Both are wide:
a pack carries an executable path and arguments, an AutoHotkey pack carries a
script, and the Lua sandbox carries arbitrary commands. Risk levels existed in
the pack files, but only `jarvis-app` enforced them, and only for 15 seconds by
a *command id*.

At the same time the project gained a local model, a memory layer, and voice
input. The obvious next step — "let the model run commands" — is the one that
turns every earlier boundary into a suggestion: a model that can emit a shell
string can do anything the user can, and a prompt injection becomes remote code
execution.

## Decision

Add a second, deliberately narrow surface, and make it the **only** surface the
model and voice can reach.

1. **Actions are typed values.** `WindowsAction` is a closed enum with one
   variant per capability. No variant carries a command line, a shell string, an
   executable path from a caller, or an arbitrary argument list. The shapes
   `RawCommand`, `Shell`, `PowerShell`, `ExecutablePathFromAi`, and
   `ArbitraryArguments` cannot be expressed, so they cannot be requested by
   accident or by a modified window.
2. **One policy, one place.** A single table answers `Safe`, `Confirm`, or
   `Forbidden` for every action kind and source. Callers do not choose a risk
   level, and a caller cannot raise or lower one.
3. **One gate, typed and single-use.** The request itself is stored in the core
   with a 128-bit random token, a TTL (45 seconds by default, 30–120
   configurable), and a constant-time comparison. Confirming returns only the
   token; the request that runs is the one that was stored. This is why
   `SafetyGate` was generalised into `ConfirmationGate<A>` instead of being
   duplicated: the older command path keeps its API as a thin wrapper.
4. **The platform sits behind one trait.** `WindowsBackend` has a native
   implementation and a fake. Every rule above the trait — policy, gate,
   preview, audit, executor — is tested without a desktop session, without
   launching a program, and without capturing a screen.
5. **Programs come from a file dialog, once.** The allowlist records the
   canonical path, size, mtime, SHA-256, and the fixed arguments the user chose.
   A changed file is refused until the user accepts the new version. Launchers
   and script hosts are refused by name.
6. **Screenshots are written, never sent.** The image goes to a directory the
   user chose, under a name the application generates, and nowhere else.
7. **The model path is a catalogue, not a prompt.** Tools are offered with
   strict JSON schemas and the capability is probed from the server's own
   `/props`. An answer in prose is shown and never parsed. There is no fallback
   that reads a command out of text, and there never will be one.
8. **Voice is a router, not a parser of commands.** It produces the same typed
   actions, through the same policy and the same gate, and it reports what it
   did not understand instead of guessing.
9. **The log is content-free.** No titles, no paths, no reminder text. It is a
   convenience, and it says so.
10. **Wake-on-LAN is excluded from the project.** It is not implemented, not
    planned, and a test fails if the words appear anywhere in the workspace's
    own code.

## Consequences

* The feature is small and reviewable: one policy table, one gate, one executor,
  one audit log. Adding a capability means adding a variant, a policy row, a
  schema, and tests — four places that the policy test forces to stay in step.
* The older command packs are unchanged and still wide. They remain a trusted
  local extension surface and are *not* reachable from the model or from voice.
  Migrating them is a separate decision, recorded in `WINDOWS_ACTIONS.md` as a
  table of keep/migrate/forbid.
* Some capabilities are deliberately missing: no shutdown or reboot, no
  registry, no file deletion, no elevation, no typing, no clipboard, no
  process termination, no browser control. Each of them would need its own
  threat model, and several cannot be done safely at all from a chat window.
* `windows = "0.61"` and `image = "0.25"` (PNG only) are new Windows-only
  dependencies of `jarvis-core`. They are used in one module,
  `windows_actions::backend::native`, and nothing above the backend trait sees a
  Win32 type.
* A screenshot can still contain a password: the guard reads window titles and
  is a courtesy, not a boundary. This is stated in the user documentation, in
  the threat model, and in the code that implements it.

## Alternatives considered

* **Reuse the command packs for the model.** Rejected: a pack is a command line,
  so a model that can choose a pack can choose a command, and the risk levels
  are advisory.
* **Parse a command out of the model's answer.** Rejected outright. Text is
  text; treating an answer as a request means every prompt injection is an
  execution.
* **Let the interface pass a path or arguments.** Rejected: the interface is a
  webview, and the pending request must survive a modification of it. The path
  comes from the native dialog; the arguments come from the stored entry.
* **Keep the pending request in the interface.** Rejected: a reload would either
  lose it silently or resurrect an approval whose TTL has passed. It lives in
  the core, and a reload forgets it.
* **Confirm by voice for everything.** Rejected: the older command path uses a
  spoken "подтверждаю" for its own actions, and voice is not authentication.
  The new feature's confirmation is a dialog with the exact fields, and voice
  confirmation is limited to a request the user just made.
* **A system-wide hook to detect screenshots by other programs.** Rejected: it
  would need elevation, would be trivially bypassed, and would create the
  illusion of a boundary that does not exist.
