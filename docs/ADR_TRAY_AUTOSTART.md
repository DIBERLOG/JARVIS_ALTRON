# ADR: tray, autostart, and the exit route

Status: accepted and implemented on `feature/ai-memory-windows-commands`. It
extends the decisions in `docs/ADR_WINDOWS_ACTIONS.md` to the desktop shell.

## Context

The application was a window that owned everything: closing it ended the process,
so a reminder could not fire while the window was hidden, and there was no way to
ask for the window back. The stage asks for a tray, a background mode, autostart,
a first-run wizard, a single settings tree, and a diagnostics page.

Two things were already decided and must not be re-litigated here: **the exit is
an ordered, bounded sequence** (`docs/DESKTOP_SHELL.md` and
`crates/jarvis-core/src/lifecycle.rs`), and **one feature owns one settings
document**. This record is about the shell's own choices.

## Decisions

### 1. Tauri's tray, not a hand-written Win32 tray

Tauri 2.9.5 exposes `tauri::tray` when the `tray-icon` feature is enabled. Writing
our own `Shell_NotifyIcon` loop would mean our own message window, our own icon
lifecycle, and our own click handling — three things that already exist, tested,
in a dependency the project already has.

Consequence: the `tauri` dependency gained the `tray-icon` and `image-png`
features. The tray icon reuses the window icon from `resources/icons`.

### 2. The menu is data, built by the core

`jarvis_core::desktop::tray_menu(&snapshot) -> Vec<TrayMenuRow>` returns rows with
an identifier, a label key, an enabled flag, and an optional value. The Tauri
layer only renders them and dispatches on the identifier.

Consequence: what the tray offers, when a row is disabled, and what the timer
count says are all unit-tested without a desktop, and the menu cannot drift from
the state. 26 core tests cover this.

### 3. Status rows are rows, not buttons

"AI: stopped", the microphone state, and the timer count are rows that cannot be
clicked. A status that looks like a button invites a click that does nothing, and
a menu that hides its state behind an action is a menu a person cannot read.

### 4. The close button asks by default, in the window's own dialog

A native message box cannot offer "remember this choice", and asking is the safe
default: hiding a window is a decision a person should make once and understand.
The core prevents the close and emits an event; the window shows a dialog with
Hide to tray / Exit / Cancel and a **Remember this choice** checkbox.

Consequence: the core's `on_close_requested` is the single place a close is
interpreted, so the dialog, the setting, and the tray cannot disagree. The
`tray_explained` flag makes the first hide non-silent.

### 5. One exit route

The tray's Exit, the dialog's Exit, the settings page, and a system shutdown call
the same `request_exit`, which runs the lifecycle and exits. The lifecycle is
idempotent, so a second request — two clicks, or an exit during an exit — returns
the first report instead of running the sequence twice.

### 6. Autostart through the supported plugin, off by default

`tauri-plugin-autostart` writes the per-user entry with the current user's rights
and no shell. Hand-writing the `Run` key with `windows-sys` would be more code
for the same behaviour and would have to re-implement the path quoting.

Decisions inside that:

* **off by default**: nothing is written because the application was launched;
* the entry carries one argument, `--start-minimized`, taken from the core's
  constant so the writer and the reader cannot disagree;
* **the entry is rewritten on every start where the setting says it should
  exist**, which is what makes a move or an update safe without reading the
  command back — the plugin reports existence, not contents;
* the three "what a login starts" switches live in the desktop settings document,
  not in the entry, so changing them does not rewrite the registry;
* **Whisper has no switch at all.** The core's `AutostartActions` has a
  `start_whisper` field that is always `false`, so a caller cannot forget the
  rule.

### 7. Single instance through the plugin

`tauri-plugin-single-instance` hands a second launch to the running copy, which
shows its window. No local TCP port is opened, and no secret is passed. A second
scheduler, a second tray, and a second database handle are what this prevents.

### 8. One microphone state, with tickets

`AudioSession` refuses a second session while one holds the device, and every
state change presents a ticket. A stale event is refused (`StaleSession`) instead
of being applied.

Consequence: the tray cannot show "recording" after a session ended, and a slow
worker cannot clear the state of a session that replaced it. This is the fix for
the failure mode where a hidden window claims to be recording for ever.

### 9. The shell's settings are its own document

`desktop.json` holds the close behaviour, the autostart switches, and the
explained flag; `setup.json` holds the wizard's progress. Both are written
atomically. This is not a second settings *format*: it is the third of four
documents that each have one owner (`settings.json` for the safe actions,
`whisper-settings.json` for dictation, `desktop.json`, `setup.json`), and no
setting is stored in two of them.

### 10. The interface asks, the core acts

Every command in `docs/DESKTOP_SHELL.md` either reads state or asks the core to
change something. The window cannot write a registry key, drive the tray, create
a second lifecycle, build a diagnostics document, or keep a password; a test scans
the interface sources for the words that would mean otherwise.

## Consequences

* The shell is testable where it matters: 26 core tests for the menu, the close
  behaviour, autostart, the audio session, and the wizard's state; 193 interface
  tests overall, of which 20 are new for the shell.
* Two features are *ready in code, unverified on a real desktop*: the tray
  rendering and the autostart behaviour after a real sign-in. Both are named as
  such in `docs/WINDOWS_MVP.md` and `docs/RELEASE_CHECKLIST.md`.
* Autostart cannot detect an entry that points at another copy, because the
  plugin does not report the command. The mitigation is a rewrite on every start,
  and the limitation is documented rather than hidden behind a failed check.
* The exit sequence has two steps that are not registered (the voice host's Vosk
  stop, and a database checkpoint that does not exist). They are listed as gaps in
  `docs/DESKTOP_SHELL.md`, and the exit report shows them as absent.

## Alternatives rejected

* **Hand-written Win32 tray.** More code, more untested surface, no benefit.
* **Closing the window ends the process, with no tray.** Then a reminder cannot
  fire while the window is closed, which is how the feature is actually used.
* **Hiding to the tray without asking.** An application that appears to have quit
  but has not is the classic desktop complaint; the default answer is "ask".
* **A native message box for the close.** It cannot offer "remember".
* **Writing the `Run` key by hand.** The supported mechanism already does it, with
  the quoting rules for paths that contain spaces.
* **Putting the autostart switches in the registry command line.** Changing a
  switch would rewrite the entry, and the entry would then be a second settings
  document.
* **A background thread that refreshes the tray every second.** The menu is
  refreshed when something changes and when the window polls; a second timer is
  another thing that can disagree with the state.
