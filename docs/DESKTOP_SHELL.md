# The Windows desktop shell

The shell is what makes the application a desktop program rather than a window:
a tray icon, a close button that asks, one exit route, autostart for the current
user, and a single answer to "is the microphone open".

The logic lives in `crates/jarvis-core/src/desktop.rs` (26 tests) and the Tauri
layer in `crates/jarvis-gui/src/desktop.rs`. The core owns every decision; the
Tauri file renders the menu, dispatches on the identifiers the core returns, and
holds nothing the core does not describe.

## The tray

Built with Tauri's own tray API (`tauri::tray::TrayIconBuilder`) — no hand-written
Win32 tray. One icon, identifier `jarvis-main`; a second is a bug, and
`install_tray` runs once, in `setup`.

| Row | What it does | When it is disabled |
|---|---|---|
| Open JARVIS / Focus window | shows, unminimises, and focuses the window | never |
| Hide window | hides it, leaving the process running | while the window is hidden |
| AI: ready / stopped / working / not configured | a **status**, not a button | always (a status row) |
| Start Vosk / Stop Vosk | starts or stops the voice listener | starting, while dictation holds the microphone |
| Start dictation | starts a dictation on a worker thread | Whisper not configured, microphone busy, or Vosk listening |
| Stop dictation | stops the recording or the transcription in flight | nothing is running |
| Microphone: idle / listening / recording / transcribing / stopping / failed | the visible indicator | always (a status row) |
| Active timers: 0, 1, 2 … | a count, so "0" is visible | always (a status row) |
| Lock storage | locks the vault, notes, and memory, and drops the spelling journals | while the storages are locked |
| Settings | shows the window and asks it to open the settings page | never |
| Exit | the full exit | never — a person must always be able to stop it |

A left click on the icon opens or focuses the window. The tooltip carries the AI
state, the microphone state, and the timer count, so a hidden window is never
silent about what it is doing: `JARVIS — AI stopped, recording, 2 timer(s)`.

The menu is rebuilt after every tray action and after every state poll from the
window (once every two seconds while the window is open). There is no background
thread for it, and no second source of truth: `AudioSession` decides what the
microphone row says.

**Not verified:** the tray has not been clicked on a real desktop in this
environment. The menu, its rows, and their enabled states are covered by tests
against `tray_menu`; the Tauri rendering of them is *ready in code, native path
unverified*.

## Close behaviour

Three settings, and the default is **ask**:

| Setting | What a close does |
|---|---|
| Ask (default) | the core prevents the close, shows the window, and emits `desktop-close-requested`; the window shows its own dialog |
| Hide to tray | the window is hidden; timers keep running |
| Exit | the full exit runs |

The dialog offers **Hide to tray**, **Exit**, and **Cancel**, plus **Remember
this choice**. It is a window dialog rather than a native message box because
"remember" is the point, and a native box cannot offer it. The core answers the
close in exactly one place (`on_close_requested`), so the dialog, the setting,
and the tray cannot disagree.

`tray_explained` records that the user has seen the explanation once, so the
first hide is never silent.

## One exit route

The tray's Exit, the dialog's Exit, the settings page, and a system shutdown all
call `desktop::request_exit`, which runs `LifecycleManager::shutdown` and exits.
The lifecycle is idempotent (a second call returns the first report), bounded
(each step has a timeout, the whole sequence has a deadline), and ordered — the
order is in `jarvis_core::lifecycle::SHUTDOWN_ORDER` and is checked by a test:

```text
refuse-new-work → cancel-generation → stop-dictation → stop-vosk → stop-timers
→ stop-llama-server → drop-decrypted-caches → zeroize-keys → checkpoint-databases
→ close-databases → exit
```

**What is not wired yet, said plainly:** the GUI registers seven of those steps
(cancel-generation, stop-dictation, stop-timers, drop-decrypted-caches,
stop-llama-server, close-databases, zeroize-keys) plus the final `exit` marker.
`stop-vosk` belongs to the voice host in `jarvis-app`, and `checkpoint-databases`
has no implementation in the stores: neither is registered, so an exit logs them
as absent rather than pretending they ran.

## Single instance

`tauri-plugin-single-instance` (2.3.7). A second launch hands its arguments to
the running copy, which shows and focuses its window; no second scheduler, tray,
database, or microphone session is created. No local TCP server is involved, and
no secret is passed between the copies.

## Autostart

`tauri-plugin-autostart` (2.5.1), which uses the supported per-user mechanism on
Windows (the `Run` key) and never a shell. The entry is written with one
argument, `--start-minimized`, from the core's own constant.

* **off by default**, and only a person turns it on;
* written for the current user; no administrator rights are requested;
* **rewritten on every start where the setting says it should exist**, so an
  update or a move cannot leave an entry pointing at nothing;
* a refusal (group policy, denied access) is reported as `Unavailable` with one
  short sentence, and the setting is kept as the user asked so the state can
  explain the refusal instead of lying;
* an entry that exists is reported as pointing at this copy: the plugin reports
  whether an entry exists, not what command it holds, so the command is rewritten
  rather than read back. This is stated here because it is a real limitation of
  the mechanism in use.

What a login is allowed to start is decided by three switches, all off:

| Setting | Default | Meaning |
|---|---|---|
| `start_minimized` | false | the window starts hidden in the tray |
| `start_vosk` | false | the voice listener starts |
| `start_local_ai` | false | the managed model server starts |

**Whisper has no such switch, and the core has no field for one.** A login never
starts a dictation, and no code path in the shell can start one.

**Not verified:** no sign-out and sign-in was performed in this environment, so
"starts with Windows" is *ready in code, unverified after a real logon*. The
decision logic — which switches exist, their defaults, and what a login may
start — is covered by tests.

## The microphone

`AudioSession` is the single source of truth, with five states the interface and
the tray both render: `Idle`, `VoskListening`, `WhisperDictation`,
`TranscribingFile`, `Stopping`, `Failed`. Two rules make it trustworthy:

* **one session at a time.** A session takes a ticket; a second `begin` while one
  holds the device is refused (`MicrophoneBusy`), so two consumers of one device
  cannot happen;
* **a stale event is ignored.** Every state change presents the ticket, and a
  ticket from a session that has already ended is refused (`StaleSession`). A late
  "stopped" from an old session can neither clear nor overwrite the state of the
  running one — which is what stops the tray from claiming a microphone is open
  when it is not, and the reverse.

`release_for_exit` invalidates every ticket and returns the state to `Idle`, so a
full exit never leaves the device claimed.

## Settings documents

Two files in the application data directory, both written atomically (a temporary
file and a rename) and both falling back to safe defaults when damaged:

* `desktop.json` — `close_behavior`, `autostart_enabled`, `start_minimized`,
  `start_local_ai`, `start_vosk`, `tray_explained`, `schema_version`;
* `setup.json` — see `docs/FIRST_RUN.md`.

There is one settings document per feature and one writer per document. This is
not a second settings format: it is the shell's own document, next to the ones
the other features own.

## Commands

```text
desktop_get_state            desktop_show_window        desktop_hide_window
desktop_request_exit         desktop_get_close_behavior desktop_set_close_behavior
desktop_update_settings      desktop_lock_storage
autostart_get_state          autostart_enable           autostart_disable
setup_get_state              setup_complete_step        setup_skip_step
setup_finish                 setup_reset
diagnostics_run              diagnostics_preview        diagnostics_export
diagnostics_summary
```

The window cannot write a registry key, drive the tray, create a second
lifecycle, build a diagnostics document, or store a password: no command accepts
one, and a test scans the interface sources for the words that would mean
otherwise.
