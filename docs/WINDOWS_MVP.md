# Windows MVP: what is ready for cautious personal testing

This document is the honest state of the "prepare a Personal Windows MVP build"
stage. It separates what is implemented and covered by tests, what was verified
by hand, what could not be verified in this environment, and what is not
implemented at all.

**Name of the result:** `Personal Windows MVP build` — a build for the author's
own machine. It is not a release, and a public release is blocked by the licence
conflict in `docs/LICENSING_STATUS.md`.

**Installer:** not built in this stage, and no installer was installed on a
Windows machine here, so what exists is an **installer candidate** at best — and
at the moment there is no installer configuration to point at either. See
"Not implemented" below.

## Readiness matrix

`Code` = the module exists and compiles. `Autotests` = covered by tests in this
repository. `Manual test` = a person ran it against real Windows APIs, real
hardware, or a real model. `Native deps` = what has to be present outside the
application. A fake backend is **not** a manual test.

| Component | Code | Autotests | Manual test | Native dependencies | Readiness |
|---|---|---|---|---|---|
| Notes (encrypted) | yes | yes (storage, conflicts, trash, import/export) | not in this stage | none beyond the app | **Ready** |
| Vault (encrypted) | yes | yes (storage, secrets, clipboard timers) | not in this stage | none beyond the app | **Ready** |
| AI memory (encrypted) | yes | yes (storage, context, secret filter) | not in this stage | none beyond the app | **Ready** |
| Autocorrect | yes | yes (engine, word list, isolation) | not in this stage | dictionaries optional | **Ready** (dictionaries `NotConfigured`) |
| Local AI gateway | yes | yes (config, process, streaming, probe) | not in this stage | `llama-server.exe` + GGUF, user-supplied | **Ready in code**, model missing here |
| Windows Actions | yes | yes (119 unit tests, 7 isolation, 8 contract) | **no** — fake backend only | a real desktop session | **Ready in code**, native path unverified |
| Vosk | yes (upstream) | partial (model listing) | not in this stage | `libvosk` + a model | unchanged from upstream; **unverified here** |
| Whisper (dictation) | yes | yes (58 unit tests + interface tests) | **no** | `whisper-cli.exe` + a `ggml-*.bin`, user-supplied | **Ready in code**, real round trip unverified |
| Backup / restore (whole application) | **no** | **no** | no | — | **Not implemented** (per-feature export/import exists) |
| Frontend (Svelte + Routify) | yes | yes (173 Node tests) | build verified | — | **Ready** |
| Tauri shell | yes | no | build compiles; window not run here | WebView2 | **Ready in code**, unverified |
| System tray | **no** | **no** | no | — | **Not implemented** |
| Autostart | **no** | **no** | no | — | **Not implemented** |
| Installer | **no** | **no** | no | WiX/NSIS toolchain | **Not implemented** |
| Lifecycle manager | yes | yes (10 unit tests) | wired into the GUI exit path; exit not run here | — | **Ready in code** |
| Runtime diagnostics | yes (core) | yes (11 unit tests) | export command not wired to the window | — | **Ready in core**, UI pending |
| First-run wizard | **no** | **no** | no | — | **Not implemented** |
| Unified settings page | partial | yes (interface tests) | build verified | — | **Partial** — the panels exist, the single tree does not |
| Licensing analysis | yes (documents) | — | — | — | **Ready** — see `docs/LICENSING_STATUS.md` |
| Dependency audit | partial | — | `npm audit` run; `cargo audit`/`cargo deny` unavailable | — | **Partial** — see below |

## What was done in this stage

* **Whisper dictation** (`crates/jarvis-core/src/whisper/`, `docs/WHISPER.md`):
  a bounded one-shot process, a native architecture check, a format check, a
  session with silence/length/cancel, and a settings panel. 58 unit tests.
* **Lifecycle manager** (`crates/jarvis-core/src/lifecycle.rs`): the exit as an
  ordered, deadline-bounded, idempotent sequence with a per-step report; wired
  into the Tauri exit path so a normal exit and a tray exit take one route.
* **Runtime diagnostics** (`crates/jarvis-core/src/diagnostics.rs` and
  `crates/jarvis-core/src/text.rs`): a report whose *shape* cannot hold user
  content, with path redaction, a screen that refuses a path or a secret, a
  preview, and a JSON export. 17 unit tests.
* **Licensing documents**: `docs/LICENSING_STATUS.md`, `THIRD_PARTY_NOTICES.md`,
  and `docs/RELEASE_CHECKLIST.md`.

## Not implemented, and not claimed

These are listed so that nobody reads this document as a promise:

1. **System tray.** No tray icon, no tray menu, no "close to tray". The exit
   path is ready for it (one route, one report), but the tray itself is absent.
2. **Autostart.** No registry entry, no `Run` key, no Tauri autostart plugin
   call. Autostart is therefore trivially "off by default", but it also cannot
   be switched on.
3. **First-run wizard.** No wizard. The individual settings pages exist
   (general, local AI, memory, autocorrect, Windows commands, dictation,
   devices), and every feature can already be left unconfigured, but there is no
   guided sequence and no `NotConfigured` overview page.
4. **Windows installer.** No MSI, no NSIS, no bundler configuration. Nothing was
   installed, so nothing is verified as an installation.
5. **Whole-application backup/restore.** No versioned container, no manifest, no
   atomic restore. What exists today is per feature: notes export/import, vault
   export/import, memory export/import, the autocorrect word list export, and the
   Windows-actions audit export.
6. **Unified settings tree.** The settings page groups the panels in tabs, which
   is close, but it is not the single tree the stage describes, and there is no
   search.
7. **Diagnostics export in the window.** The core can build, screen, preview, and
   serialise a report; no command exposes it to the interface yet.
8. **Notifications with a registered identity.** Timers and reminders show an
   in-application notification and attempt a system toast only when the build
   has an AUMID. With no installer, there is no AUMID, so the honest state is
   "in-application notification only".
9. **Smoke tests against a real desktop, models, and microphone.** None were run.

## Dependency audit

* `npm audit`: **0 vulnerabilities** in production dependencies. Development
  dependencies report advisories (`brace-expansion` high, `esbuild` moderate via
  `vite` ≤ 0.24.2, `immutable` high). They are build-time and development-only
  packages; `npm audit`'s own advice for `esbuild` is `--force`, which would jump
  `vite` to a new major version, and that was **not** done.
* `cargo audit` and `cargo deny`: **not installed**, so no Rust advisory scan was
  run in this environment. This is a real gap, and the commands that close it are
  in `docs/LICENSING_STATUS.md`.
* A full transitive licence list was not produced, for the same reason.

## Manual smoke tests: what a person should run first

In this order, with the machines and files available; each line is a check a
person can complete in a few minutes.

1. Build: `npm ci`, `npm run build`, `cargo build --workspace`.
2. Start the window; Notes: create, edit, close, reopen, confirm the text is there.
3. Vault: add a `FICTIONAL_*` entry, lock, unlock, confirm the secret is intact.
4. Autocorrect: pick a dictionary folder, check a sentence, undo.
5. Windows commands: volume up/down, a screenshot into a chosen folder, a
   30-second reminder, lock the session, and the confirmation dialog for a
   risky action.
6. Dictation: point the settings at a local `whisper-cli.exe` and a
   `ggml-*.bin`, allow dictation, press "Начать диктовку", speak one sentence,
   confirm the text and that no WAV is left behind.
7. Local AI: start `llama-server` from the settings, ask a question, then ask for
   an action and confirm the dialog that appears.
8. Exit from the window and check the log: the exit report should list every step
   as done.

Anything that cannot be run on the machine at hand stays **unverified**, not
"works".

## Honest summary

The application is a working local-first assistant whose encrypted storages,
local model gateway, autocorrect, safe Windows actions, and dictation are
implemented and covered by 670+ automated tests. It is **not** yet an installable
product: there is no tray, no autostart, no wizard, no installer, and no
whole-application backup, and a public release is blocked by a licence conflict
that only the copyright holder can resolve.
