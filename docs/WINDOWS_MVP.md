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
| Single instance | yes | no | **no** — a second launch was not performed | — | **Ready in code, unverified** |
| System tray | yes | yes (menu + states, 26 core tests) | **no** — no click on a real desktop | — | **Ready in code, native path unverified** |
| Autostart | yes | yes (controller + fake backend) | **no** — no sign-out/sign-in was performed | the plugin writes the per-user Run key | **Ready in code, unverified after a real logon** |
| Installer | **no** | **no** | no | WiX/NSIS toolchain | **Not implemented** |
| Lifecycle manager | yes | yes (10 unit tests) | wired into the GUI exit path; exit not run here | — | **Ready in code** |
| Runtime diagnostics | yes | yes (core tests + interface tests) | export not run against a real file dialog | — | **Ready** |
| First-run wizard | yes | yes (state + interface tests) | build verified | — | **Ready** |
| Unified settings page | yes | yes (interface tests) | build verified | — | **Ready** — one settings page with the required sections, adding Startup and tray, Privacy, Diagnostics, and About |
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
* **The desktop shell** (`docs/DESKTOP_SHELL.md`, `docs/ADR_TRAY_AUTOSTART.md`):
  the tray with a state-carrying menu, the close behaviour with its dialog, one
  exit route through the lifecycle, single instance, opt-in autostart for the
  current user, the microphone session with stale-event protection, and the
  first-run wizard (`docs/FIRST_RUN.md`). 26 core tests and 20 interface tests.
* **Diagnostics in the window** (`docs/RUNTIME_DIAGNOSTICS.md`): a section of the
  settings page with the component table, the preview, an export that runs the
  core's screen first, and a summary for the clipboard.

## Not implemented, and not claimed

These are listed so that nobody reads this document as a promise:

1. **Windows installer.** No MSI, no NSIS, no bundler configuration. Nothing was
   installed, so nothing is verified as an installation. The next stage owns it.
2. **Whole-application backup/restore.** No versioned container, no manifest, no
   atomic restore. What exists today is per feature: notes export/import, vault
   export/import, memory export/import, the autocorrect word list export, and the
   Windows-actions audit export. The stage after this one owns it.
3. **Notifications with a registered identity.** Timers and reminders show an
   in-application notification and attempt a system toast only when the build has
   an AUMID. With no installer there is no AUMID, so the honest state is
   "in-application notification only", and the diagnostics report says
   `version_unknown` for it rather than claiming a toast works.
4. **Two exit steps are not registered.** `stop-vosk` belongs to the voice host
   in `jarvis-app`, and `checkpoint-databases` has no implementation in the
   stores. An exit logs them as absent instead of pretending they ran.
5. **The settings tree has no search.** The sections are there; a search box
   would need an index over four settings documents, and it was not built.
6. **Autostart cannot detect an entry pointing at another copy**, because the
   plugin in use reports existence rather than contents. The mitigation is a
   rewrite on every start, and the limitation is documented.
7. **Smoke tests against a real desktop, models, and microphone.** None were run:
   no tray click, no sign-in with autostart enabled, no real dictation, and no
   second launch of the application.

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
implemented and covered by 715 Rust tests and 173 interface tests. It is **not** yet an installable
product: there is no tray, no autostart, no wizard, no installer, and no
whole-application backup, and a public release is blocked by a licence conflict
that only the copyright holder can resolve.
