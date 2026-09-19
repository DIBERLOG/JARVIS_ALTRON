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

## What is not implemented

* **No window command yet.** The core builds, screens, previews, and serialises a
  report; no Tauri command exposes it, so today the report can only be produced
  from Rust (a test, or a small program using the crate).
* **Component collection is the caller's job.** The module deliberately does not
  know how to probe Vosk, Whisper, or the model server: it holds the vocabulary,
  the redaction, the screen, and the rendering. The probing belongs next to each
  feature, which is where the states will come from when the command is wired.
* **`VersionUnknown` is under-used.** Nothing reads a version out of
  `whisper-cli.exe` or `llama-server.exe` today, so those report what the file
  check knows and nothing more.
