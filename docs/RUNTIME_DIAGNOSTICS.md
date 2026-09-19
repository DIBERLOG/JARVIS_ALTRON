# Runtime diagnostics

Diagnostics answer one question: *what is present, what is missing, and what
could not be checked?* — without answering any question about the person using
the machine.

The core is `crates/jarvis-core/src/diagnostics.rs`, with the text rules in
`crates/jarvis-core/src/text.rs`. The window does not expose the export yet; this
page describes what the report is, what it refuses to carry, and how to read it.

## What the report contains

| Field | Source | Notes |
|---|---|---|
| `application_version` | the caller | the version string the window was built with |
| `operating_system`, `architecture` | `std::env::consts` | `windows` / `x86_64`; the detailed Windows build number is not read |
| `components[]` | the caller | one line per dependency: name, state, and a short explanation |
| `schema_versions[]` | the stores | the version of each database and settings document |
| `health_checks[]` | the caller | what was tried, and whether it passed |
| `database_sizes[]` | the stores | **file name only**, with a human-readable size |
| `model_sizes[]` | the settings | **file name only** |
| `memory_total_mb`, `memory_free_mb` | `sysinfo` | numbers, not usage history |
| `disk_free_mb` | `sysinfo` | free space on the data drive |
| `recent_errors[]` | the logs | **codes and counts**, never messages |
| `licenses[]` | `docs/LICENSING_STATUS.md` | component, licence, and status |
| `notes[]` | the caller | plain sentences about what could not be read |

## The component states

```text
Ready               the dependency is present and usable
Missing             it is not there
WrongArchitecture   it is built for another architecture (a 32-bit DLL, for example)
VersionUnknown      it is there, and its version could not be read
Incompatible        it is there, and it cannot work with this build
PermissionDenied    it is there, and this user may not use it
NotConfigured       nothing was chosen, which is a choice, not a fault
Disabled            the feature is switched off
```

`Ready`, `Disabled`, and `NotConfigured` do not "need attention".
`Missing`, `WrongArchitecture`, `VersionUnknown`, `Incompatible`, and
`PermissionDenied` do, and the report lists those separately
(`needs_attention()`).

## What the report cannot carry

By construction — the struct has no field for it:

* a user name, a home directory, or any absolute path;
* the text of a note, a vault entry, a fact, or a conversation;
* a password, a token, a key, or `key.dpapi`;
* a transcript from dictation;
* a window title;
* the content of the audit log;
* a prompt sent to a model, or an answer received from one.

Two mechanisms keep it that way:

1. **Redaction on the way in.** Every `detail` and every note goes through
   `text::redact`, which replaces a drive path, a UNC path, and a `/`-separated
   path with `<path>`. File names are taken with `text::file_label`, so a size is
   reported as `notes.db`, never as the folder it lives in.
2. **A screen on the way out.** `DiagnosticReport::screen()` walks the rendered
   text and refuses a report that mentions `C:\`, `\\`, `/users/`, `key.dpapi`,
   or a secret-shaped assignment (`api_key=`, `Bearer `, `-----BEGIN`). The JSON
   export calls the screen and fails instead of writing.

Both are covered by tests, including a test that a path-shaped field added by
mistake is caught rather than exported.

## The preview

`preview()` returns the rendered report line by line, and it is what a window
should show **before** anything is written: the same text that would be exported,
so the person can see exactly what they are about to save. The report is bounded
to 400 lines.

## How a person will use it

1. Open the diagnostics page (when it is wired), read the component list, and
   fix what says `Missing` or `NotConfigured`.
2. Look at the preview. If it names a file you do not recognise, that is a real
   finding about your configuration.
3. Export it to a file only if you are reporting a problem; the file is safe to
   share, which is the point of the screen.

## What the window does with it

The settings page has a **Diagnostics** tab
(`frontend/src/components/desktop/DiagnosticsPanel.svelte`) and four commands:

```text
diagnostics_run       builds the report and its preview
diagnostics_preview   the lines, without building a view
diagnostics_export    writes the JSON to a file the user picks
diagnostics_summary   a short, already-screened summary for the clipboard
```

The panel shows the component table with a state per row, the file sizes by name,
the recent error categories by code and count, and the full preview. The export
runs the screen first: a report that carries a path or a secret-shaped line is
refused with the line number, not written. The summary is the preview, so the
clipboard gets the same text the person just read.

`diagnostics_run` collects its facts from the running application:

| Check | How it is read | What it does not do |
|---|---|---|
| data directory | a write probe, which is deleted immediately | never writes anything else |
| SQLite, stores | whether the store file exists, and whether the shared session is unlocked | never decrypts anything to find out |
| model server, local model | the configured paths and a PE-header check for the executable | never starts the server |
| Whisper executable and model | the same checks the dictation page uses | never runs a transcription |
| Vosk runtime, dictionaries | whether the folder exists, and how many files it holds | never loads a model |
| microphone | the input device list | **never opens the microphone** |
| Windows commands, Core Audio, screenshots | the backend capability report | never executes an action |
| autostart, tray | the entry state and whether the icon exists | never writes the entry |
| WebView2, notifications | reported as `version_unknown`, with the reason | does not claim a toast works without an installed build |
| memory, disk | `sysinfo` | — |

## What is not implemented

* **The version of an external binary is not read.** `whisper-cli.exe` and
  `llama-server.exe` are checked for existence, size, and architecture, not for a
  version, so they report what the file check knows and nothing more.
* **The Windows build number is not read.** The report says `windows x86_64`,
  which is true, rather than calling `RtlGetVersion` it does not use elsewhere.
* **The `target` directory size is not reported.** The stage asked for it in dev
  mode only; nothing reads it, and no command would be honest about it in a
  release build.
* **A hash of a public binary is not reported.** The stage allows it; the code
  does not compute one, because nothing verifies it against a published value,
  and an unverified hash invites a false sense of checking.
