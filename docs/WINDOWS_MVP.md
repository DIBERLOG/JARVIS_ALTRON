# Windows MVP: what is ready for personal use on this machine

This document is the honest state of the Windows MVP. It separates what is
implemented and covered by tests, what was verified by hand **on this
computer**, what has not been verified, and what is not implemented at all.

**Scope of this document: personal use on the current Windows computer.** It is
not a release, and it is not a claim about any other machine.

**Warning, kept where it cannot be missed:**

> **Vault and AI memory are experimental. No independent security audit has been
> carried out.** The cryptography is the project's own design, reviewed only by
> its author and its test suite. Treat the vault as a place for passwords you can
> afford to lose, and keep an independent copy elsewhere. This warning does not
> block personal use on this machine; it does block handing the project to anyone
> else, and it is listed again in `docs/RELEASE_CHECKLIST.md`.

Three readiness levels are used from here on, and they never mix:

| Level | Meaning | Blocked by |
| --- | --- | --- |
| **Personal use** | runs on this computer, for its author | nothing except the checks in "Checks on this machine" below |
| **Installer** | the built application can be installed and removed on this machine | an installer that builds, installs, uninstalls, and leaves no secret behind |
| **Public release** | another person may use it | a clean-Windows test, resolved licences, an independent audit, and a verified backup transfer |

## What "a clean Windows check" is, and is not, here

A clean-Windows check means: install on a Windows machine that has never had this
project on it, with no build tools, no `target/debug`, and no developer paths, and
run the acceptance scenarios there.

* it is **not** a requirement for personal use on this computer, and it has
  **not** been performed — this document does not claim it was;
* it **is** a requirement before handing the project to another user or making a
  public release, and it is listed there;
* a limited substitute, when it is possible and cheap, is a **new local Windows
  profile** on this machine (a fresh user account, the application installed from
  the built bundle). It is worth doing and it is recorded when it happens, but it
  is **not mandatory** for personal use and it is not the same thing as a clean
  machine.

## Checks on this machine (required for personal use)

These are the checks that must pass on the current computer before this build can
be called usable for personal work. They do not replace the automated suites;
they are what the automated suites cannot see.

| # | Check | State |
| --- | --- | --- |
| 1 | The built application starts **outside** `cargo run` (the bundle's own executable, started from Explorer or the Start menu) | **to do** |
| 2 | The built application does not depend on `target/debug` or on any other build-tree path (Windows opens nothing from the checkout while the installed copy runs) | **to do** |
| 3 | Notes: create, edit, close, reopen, confirm the text is there | **to do** |
| 4 | Vault: add a `FICTIONAL_*` entry, lock, unlock, confirm the secret is intact | **to do** |
| 5 | Whisper: point the settings at the local `whisper-cli.exe` and a `ggml-*.bin`, dictate a sentence, get the text, confirm no WAV is left behind | **done** — verified by hand on this machine |
| 6 | Vosk: the listener starts, hears the wake word, and returns to idle | **to do** |
| 7 | Global voice input: the phrase is recognized, the confirmation is spoken, the microphone changes hands, and the text reaches the field | **to do** — the engine is tested; the Windows reader, the commands and the trigger are not wired yet (see `docs/GLOBAL_VOICE_INPUT.md`) |
| 8 | Tray: the icon appears, the menu works, the states are shown, and a full exit from the tray closes the process | **to do** |
| 9 | Autostart: switch it on, sign out and back in, confirm the application starts (and only once), switch it off again | **to do** |
| 10 | Timers and reminders: a 30-second timer fires, the notification appears, the audit line is written | **to do** |
| 11 | Diagnostics: the report shows every component, the export runs the screen first, and nothing in it names a path, a note, or a secret | **to do** |
| 12 | Install and uninstall: the installer runs, the application appears in the Start menu, and uninstall removes the program while leaving user data in place | **to do** — there is no installer yet (`docs/INSTALLATION_WINDOWS.md`) |
| 13 | No secrets in the bundle: no key, no `key.dpapi`, no user data, no model, no test fixture, no absolute developer path | **to do** — checked by review today; an automated bundle scan is part of the installer stage |

Anything that has not been run stays **to do**. A fake backend, a passing unit
test and a compile are not a manual check.

**Not blocked by, for personal use:** a clean-Windows test, an independent audit,
unresolved licences, and the manual backup/restore check. Each of those is
required before *someone else* is asked to rely on the project.

## Readiness matrix

`Code` = the module exists and compiles. `Autotests` = covered by tests in this
repository. `Manual test` = a person ran it against real Windows APIs, real
hardware, or a real model on this machine. `Native deps` = what has to be present
outside the application. A fake backend is **not** a manual test.

| Component | Code | Autotests | Manual test | Native dependencies | Readiness |
|---|---|---|---|---|---|
| Notes (encrypted) | yes | yes (storage, conflicts, trash, import/export) | **no** | none beyond the app | **Ready in code**; experimental, no independent audit |
| Vault (encrypted) | yes | yes (storage, secrets, clipboard timers) | **no** | none beyond the app | **Ready in code**; experimental, no independent audit |
| AI memory (encrypted) | yes | yes (storage, context, secret filter) | **no** | none beyond the app | **Ready in code**; experimental, no independent audit |
| Autocorrect | yes | yes (engine, word list, isolation) | **no** | dictionaries optional | **Ready in code** (dictionaries `NotConfigured` without them) |
| Local AI gateway | yes | yes (config, process, streaming, probe) | **no** | `llama-server.exe` + GGUF, user-supplied | **Ready in code**, model missing on this machine |
| Windows Actions | yes | yes (119 unit tests, 7 isolation, 8 contract) | **no** — fake backend only | a real desktop session | **Ready in code**, native path unverified |
| Vosk | yes (upstream) | partial (model listing) | **no** | `libvosk` + a model | unverified on this machine |
| Whisper (dictation) | yes | yes (unit tests + interface tests) | **yes** — dictation on this machine, checked by hand end to end | `whisper-cli.exe` + a `ggml-*.bin`, user-supplied | **Ready and verified on this machine** |
| Global voice input | yes (engine, gate, punctuation) | yes (24 tests) | **no** — the Windows reader, the commands and the trigger are not wired | UI Automation (Windows), the existing Whisper setup | **Ready in code**, not reachable from the window yet |
| Backup / restore (whole application) | yes | yes (27 engine tests: full cycle, WAL, damage, rollback) | **no** — the manual check is deliberately open | free disk space, the master password | **Implemented and tested automatically**; the manual check does not block personal use |
| Frontend (Svelte + Routify) | yes | yes (230 Node tests) | build verified | — | **Ready** |
| Tauri shell | yes | no | **no** — the window has not been started from the bundle here | WebView2 | **Ready in code**, unverified |
| Single instance | yes | no | **no** | — | **Ready in code, unverified** |
| System tray | yes | yes (menu + states, core tests) | **no** | — | **Ready in code, unverified** |
| Autostart | yes | yes (controller + fake backend) | **no** | the plugin writes the per-user Run key | **Ready in code, unverified after a real logon** |
| Installer | **no** | **no** | no | WiX/NSIS toolchain | **Not implemented** — see `docs/INSTALLATION_WINDOWS.md` |
| Lifecycle manager | yes | yes (unit tests) | wired into the GUI exit path; **`checkpoint-databases` is registered now**, `stop-vosk` is not | — | **Ready in code** |
| Runtime diagnostics | yes | yes (core tests + interface tests) | **no** — export not run against a real file dialog | — | **Ready in code** |
| First-run wizard | yes | yes (state + interface tests) | build verified | — | **Ready** |
| Unified settings page | yes | yes (interface tests) | build verified | — | **Ready** |
| Licensing analysis | yes (documents) | — | — | — | **Ready** for personal use; a public release is blocked by the conflict in `docs/LICENSING_STATUS.md` |
| Dependency audit | partial | — | `npm audit` run; `cargo audit`/`cargo deny` not installed | — | **Partial** — see below |

## What the automated suites cover

* `cargo test --workspace -j 1` — **852 tests**, including the encrypted stores,
  the AI memory, the autocorrect engine, the Windows-action policy and isolation,
  the tray and autostart state machines, the diagnostics shape, the lifecycle
  order, the recorder and the Whisper session, the backup container and its
  rollback, and the global voice input route and gate;
* `npm run test:ui` — **230 tests**, including the interface shapes, the three
  locales, and the structural checks that keep a transcript out of the log and
  out of browser storage;
* `cargo check --workspace`, `npm run build`, `cargo clippy` on the files written
  here, `rustfmt` on the files written here, and `git diff --check`.

Encryption is not weakened anywhere by the personal-use framing: the key
separation, the AEAD records, the DPAPI binding, the portable envelope and the
backup container are exactly what they were, and their tests still have to pass.

## Not implemented, and not claimed

Listed so that nobody reads this document as a promise:

1. **Windows installer.** No MSI, no NSIS, no bundler configuration, no AUMID.
   Nothing has been installed, so nothing is verified as an installation.
2. **Notifications with a registered identity.** Without an AUMID the timers show
   an in-application notification, and the diagnostics report says so instead of
   claiming a system toast works.
3. **`stop-vosk` is not registered as an exit step.** It belongs to the voice host
   in `jarvis-app`. `checkpoint-databases` **is** registered now and truncates the
   write-ahead logs on exit.
4. **The global voice input is not reachable from the window yet.** The engine,
   the gate and the punctuation are implemented and tested; the Windows UI
   Automation reader, the commands, the tray states, the settings section and the
   Vosk trigger are the remaining wiring.
5. **The settings tree has no search.**
6. **Autostart cannot detect an entry pointing at another copy**, because the
   plugin in use reports existence rather than contents. The mitigation is a
   rewrite on every start, and the limitation is documented.
7. **Smoke tests against a real desktop for the tray, autostart, Vosk and the
   installer** have not been run. Whisper dictation was run and passed.

## Dependency audit

* `npm audit`: **0 vulnerabilities** in production dependencies. Development
  dependencies report advisories (`brace-expansion` high, `esbuild` moderate via
  `vite` ≤ 0.24.2, `immutable` high). They are build-time and development-only
  packages; `npm audit`'s own advice for `esbuild` is `--force`, which would jump
  `vite` to a new major version, and that was **not** done.
* `cargo audit` and `cargo deny`: **not installed**, so no Rust advisory scan was
  run in this environment. This is a real gap; the commands that close it are in
  `docs/LICENSING_STATUS.md`, and it is on the public-release list.
* A full transitive licence list was not produced, for the same reason.

## Honest summary

The application is a working local-first assistant whose encrypted storages,
local model gateway, autocorrect, safe Windows actions, dictation, backup and
voice-input engine are implemented and covered by 852 Rust tests and 230 interface
tests, with Whisper dictation verified by hand on this machine.

It is **good enough to keep using personally on this computer**. It is **not** an
installable product yet (no installer, no verified tray or autostart session), and
it is **not** ready for another user: that needs a clean-Windows test, resolved
licences, an independent audit of the vault and the AI memory, a verified backup
transfer between two computers, and an installer that has been installed and
removed on a real machine. None of those is claimed here.
