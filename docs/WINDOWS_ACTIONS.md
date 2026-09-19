# Safe Windows actions

This document describes what the "Команды Windows" feature can do, what it
deliberately cannot do, and how each request is checked. It is written for a
person who wants to know whether the feature is allowed to do something before
it does it.

The short answer: the feature can change the volume, take a screenshot, start a
timer or a reminder, act on a window of the current list, lock the session, and
start a program the user allowed — and nothing else. There is no command line,
no shell, no PowerShell, no file deletion, no registry, no shutdown, no
elevation, and no way for the local model or a spoken phrase to ask for any of
those.

## The flow every request goes through

```text
DirectGui ─┐
Voice ─────┤
LocalAi ───┼─► ActionRequest ─► ActionPolicy ─► ConfirmationGate ─► Executor ─► Win32 / Core Audio / GDI
Timer ─────┘   (typed)          (Safe/Confirm/           (single-use      │
                                 Forbidden)               token, TTL)      └─► AuditLog
                                                                          └─► ActionPreview ─► dialog
```

1. **A request is a typed value.** `WindowsAction` has one variant per thing the
   feature can do, and every field is a bounded number, an identifier this
   application minted, or a short string. There is no variant with a command
   line, a path from a caller, or an argument list, so there is nothing to
   smuggle in.
2. **One policy decides.** `ActionPolicy` answers `Safe`, `Confirm`, or
   `Forbidden` for every kind of action and every source. A tool call from the
   model is a request, not a permission.
3. **A risky action is confirmed.** The request is stored in the core, the
   interface receives a preview (fields, risk, consequences, expiry) and a
   128-bit single-use token, and only that token releases the *stored* request.
   Everything the interface sends back is ignored.
4. **The executor is the only place that touches the system.** It re-validates a
   window identifier, refuses a screenshot while a window that looks like it may
   show credentials has the focus, and writes an image only under a name it
   generated itself.
5. **Everything is logged content-free.** Timestamps, action names, sources,
   decisions, durations, and a bounded target label. No window titles, no paths,
   no reminder text.

## The rules, one by one

### Allowed applications

A program can be started only if the user added it by choosing the file in the
native Windows file dialog. At that moment the feature records the canonical
path, the file size, the modification time, and the SHA-256 of the file, plus
the arguments the user fixed. A launch uses exactly that stored entry: if the
file changed since it was allowed, the launch is refused with
`executable_changed` until the user accepts the new version in the settings.

The path never comes from the interface, from a tool call, or from a spoken
phrase, and it is never assembled from a name. Command-line interpreters and
script hosts are refused by name whatever the user picks:
`cmd.exe`, `powershell.exe`, `pwsh.exe`, `wscript.exe`, `cscript.exe`,
`mshta.exe`, `rundll32.exe`, `regsvr32.exe`, `conhost.exe`, `wt.exe`,
`bash.exe`, `sh.exe`, `wsl.exe`, `python.exe`, `pythonw.exe`, `node.exe`,
`java.exe`, `javaw.exe`. Network paths, environment variables, relative paths,
and non-`.exe` files are refused as well.

### Volume

Absolute and relative changes are bounded: one relative action moves the level
by at most 25 points, and the quick buttons use 10. Mute and unmute are exact.
The level is read through Core Audio; there is no audio capture and no audio
content is ever read.

### Screenshots

A screenshot is written into a directory the user chose, under a name this
application generates, and an existing file is never overwritten. The image is
written to disk and never sent anywhere: not to the model, not to the interface,
not over the network. The path is shown to the user once.

While a window whose title looks like it may show credentials (`jarvis`,
`altron`, `vault`, `пароль`, `password`, `credential`) has the focus, a
screen-wide capture is refused with `sensitive_window`. This is a courtesy
guard, not a security boundary — see the threat model.

### Windows

The list of visible windows is taken on demand. Each entry carries an opaque
identifier that lives for 90 seconds and is replaced by every new listing; an
action on an identifier from an earlier listing is refused with
`window_expired`. The window is looked up again immediately before the action,
and the process id, visibility, and window class are re-checked, so a stale
identifier cannot reach a different window.

Closing a window posts `WM_CLOSE`. The feature never calls `TerminateProcess`,
never calls `taskkill`, and never synthesizes input: there is no `SendInput`,
no `keybd_event`, and no `mouse_event` anywhere in it. A window that belongs to
this application is refused, and so are the system's own shell windows.

### Timers and reminders

A timer is 5 seconds to 24 hours; a reminder is 30 seconds to 30 days and at
most 500 characters. Timers live in `timers.json`; a reminder's text is sealed
with DPAPI for the current user, so a copy of the file does not reveal it. One
scheduler thread wakes on a condition variable, fires an item exactly once, and
each item is written as fired *before* anything is announced, so a notification
can never be the only record of what fired. A reminder whose text looks like a
credential needs a confirmation before it is stored.

### Lock

Locking uses `LockWorkStation`. There is no shutdown, no reboot, no sleep, and
no way to unlock, change a password, or touch another user's session.

### Spoken phrases

The voice router understands a small vocabulary: quieter/louder, mute, a timer
or a reminder with a duration, minimize/maximize/restore/close/move a window,
lock, take a screenshot, list windows, and "открой <name>" for an allowed
program. It answers `NotAnAction`, `Ambiguous { reason }`, `Disabled`, or an
actual request — and ambiguity is always reported, never guessed at. A phrase
never becomes a command line, and a dictated sentence cannot reach the
executor.

### The local model

The model is offered a catalogue of tools with strict JSON schemas
(`additionalProperties: false`) and answers with a tool call. The catalogue is
offered only when the server's own chat template can carry a tool call
(`/props`); when it cannot, the model is not offered anything and the interface
explains why. Text answers are shown and never parsed for actions.

An unknown tool name, an extra field, a `command`/`path`/`args`/`shell`
argument, or a value outside its range is refused.

## The audit log

`actions-audit.jsonl` in the feature directory, rotated at 512 KiB to
`actions-audit.1.jsonl` (one old file kept), with the newest 500 entries in
memory for the interface. Each line holds a timestamp, an action name, the
source, the risk, the decision, a content-free result label, a duration, an
error category, and a bounded target (an application id, a window identifier, a
timer identifier, or a monitor number).

The log is a convenience, **not evidence** and not a security control. It is
append-only until the user clears it, clearing and exporting need an explicit
confirmation, and the file is not signed, not chained, and not protected
against a program running as the same user.

## Existing commands: what is kept, migrated, and forbidden

The feature did not replace the older command packs; it added a second,
narrower surface next to them. This table is the audit of the older surface and
of the new one.

| Command / surface | Source | Mechanism | Arguments | Risk today | Confirmation | Decision |
|---|---|---|---|---|---|---|
| `resources/commands/*/command.toml` with `action = "cli"` | command packs on disk | `std::process::Command` with `exe_path` + `exe_args` | fixed by the pack, plus extracted slots | `risk_level` from the file | `SAFETY_GATE` in `jarvis-app` only, 15 s; not enforced in `jarvis-cli` or the GUI listener | **migrate**: keep reading packs, route every execution through one policy; do not add new CLI packs |
| `action = "ahk"` command packs | command packs on disk | launches AutoHotkey with a script | script path from the pack | `risk_level` | as above | **migrate**: a script is a command line; needs its own audit before it is offered as a feature |
| `action = "lua"` command packs | command packs on disk | full Lua sandbox | script from the pack | `risk_level` | as above | **migrate**: `jarvis.system.exec` exists only here; it must never be connected to the model |
| `action = "voice"` command packs | command packs on disk | plays an audio file | none | `safe` | not required | **keep**: no system access |
| `calculator` pack: `taskkill /f /im CalculatorApp.exe` | command pack, **`.yaml`** | forced process termination through a shell | fixed | was unset; now `risk_level: forbidden` | the gate refuses it | **forbid, and it is not loaded**: the loader reads `command.toml` only, so no `.yaml` pack reaches the executor. The entry is marked `forbidden` so that a future YAML loader cannot pick it up silently; replace it with the typed window-close action |
| `browser_close` | command pack | AutoHotkey, elevates, closes applications, restarts JARVIS | script | `risk_level = "forbidden"` | refused by the gate | **forbid**: it must stay forbidden; the new feature never elevates and never ends a process |
| `jarvis.system.exec` | Lua full sandbox | arbitrary command execution | anything | not exposed by this feature | none | **forbid**: it is not reachable from the new feature, from the model, or from voice |
| The new actions | buttons, voice, the local model, timers | typed `WindowsAction` only | typed fields only | central table | one gate, single-use token, 45 s by default | **new**: the surface described above |
| Wake-on-LAN | — | — | — | — | — | **excluded from the project**: not implemented, not planned, and `windows_actions_isolation.rs` fails if the words appear in the workspace's own sources |

## Legacy command packs: what is actually loaded

The audit was repeated for this stage with the loader in front of it, and the
result changes what the earlier table means:

* **Loaded packs** are the ones with `command.toml`: `browser`, `counter`,
  `weather`, and `test_slots`. Of these, `browser_close` is `forbidden` and
  refused by the gate; `browser_open` launches an AutoHotkey executable from its
  pack directory, which is a script launcher, not a shell, and it stays a
  trusted local extension;
* **`.yaml` packs are not loaded at all.** `parse_commands` reads
  `command.toml` and nothing else, so the seven `.yaml` packs — `calculator`,
  `jarvis`, `steam`, `stop`, `terminate`, `volume`, `windows` — are dead
  configuration in this codebase. `serde_yaml` is declared as a dependency and
  never used, which is the same fact seen from the other side;
* therefore the `taskkill` entry in the `calculator` pack **is not reachable
  today**. It has been marked `risk_level: forbidden` so that a future YAML
  loader cannot pick it up silently, and the honest description is "dead
  configuration, marked to stay refused", not "a live command that was
  disabled";
* `jarvis.system.exec` exists only in the Lua **full** sandbox
  (`crates/jarvis-core/src/lua/api/system.rs`). Nothing in the AI path, the
  voice router, or the safe action surface can reach it: a test fails if that
  path appears in the feature's sources.

Any change to the loader that starts reading `.yaml` packs must revisit this
section first.
## Honest limitations

* **The sensitive-window guard is a courtesy, not a boundary.** The screen
  belongs to the session: any program running as the user can capture it at any
  moment, and the feature's guard only stops *its own* capture while a
  suspicious window has the focus. It also relies on window titles, which a
  program chooses freely.
* **The allowlist is not a sandbox.** An allowed program runs with the user's
  full rights. Allowing a browser, for example, allows anything the browser can
  do.
* **The audit log is not tamper-proof.** A program running as the same user can
  edit or delete it.
* **Confirmation is not authentication.** A person at the keyboard can approve;
  nothing proves *who* approved.
* **The window list can change between the listing and the action.** The
  identifier, the re-validation, and the 90-second lifetime make a silent
  substitution very unlikely, but a window that is still alive and still matches
  the re-check is acted on.
* **Reminders fire only while the application runs.** The item is stored, and it
  fires late on the next start, but nothing wakes the machine.
* **No system toast on an uninstalled build.** A toast needs a registered
  application id; the interface shows its own notification instead, and
  `Capabilities::notifications` says `false` rather than pretending.
