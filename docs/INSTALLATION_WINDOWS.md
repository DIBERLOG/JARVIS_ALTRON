# Installation on Windows

This page describes how to get the application running on a Windows machine
**today**, which is from source, and what a real installer will have to do when
one exists. It is written so that nothing here can be mistaken for "there is an
installer".

## There is no installer in this stage

No MSI, no NSIS package, and no bundler configuration exists in this repository
yet. Nothing was installed on a machine as part of this stage, so installing is
currently a build step, and the honest name for any package produced later is
**installer candidate** until someone installs it on a real system and confirms
it works.

## What has to be on the machine

| Requirement | Why | Notes |
|---|---|---|
| Windows 10 or 11, x64 | the application is Windows-only in its process handling and its audio backend | a 32-bit or ARM build is not produced |
| WebView2 runtime | the interface is a WebView | present on Windows 11 and on most Windows 10 machines; the Microsoft installer provides it otherwise |
| Visual C++ runtime (x64) | every Rust binary needs it | installed by many programs; if the application does not start at all, this is the first thing to check |
| Node.js 20+ and npm | only to build the interface | not needed to run a built application |
| Rust (stable) with the MSVC toolchain | only to build the Rust side | not needed to run a built application |

Optional, each supplied by the user and never bundled:

| Component | Enables | Where it goes |
|---|---|---|
| `llama-server.exe` (from a `llama.cpp` release) | local chat and the AI action catalogue | any folder; the settings page asks for the path |
| a GGUF model (for example a Qwen3-8B `Q4_K_M`) | the local model itself | any folder; the settings page asks for the path |
| `whisper-cli.exe` (from a `whisper.cpp` release) | dictation | any folder; the dictation settings ask for the path |
| a Whisper `ggml-*.bin` model | dictation | any folder; the dictation settings ask for the path |
| Vosk runtime (`libvosk`) and a Vosk model | the wake-word and command listener | as the upstream documentation describes |
| Hunspell dictionaries (RU/EN) | spelling checks | any folder; the autocorrect settings ask for the folder |

None of these is downloaded by the application, and no URL or hash for them is
invented anywhere in this repository.

## Build and run from source

```powershell
# 1. interface dependencies and bundle
cd frontend
npm ci
npm run build

# 2. the Rust workspace (the first build is long: it compiles Tauri, SQLite, Lua,
#    and the ONNX runtime that the intent backend fetches at build time)
cd ..
cargo build --workspace

# 3. run the window
cargo run -p jarvis-gui
```

Two notes about the first build:

* `cargo build` with the `intent-classifier` feature fetches ONNX Runtime
  binaries through `ort-download-binaries`. That is a build-time download by the
  crate, not a download by the application, and it needs network access.
* Building the whole workspace in parallel can exhaust memory on a small machine;
  `-j 1` is the safe setting and is what this project uses.

## Where the application keeps its data

Everything is under the per-user application data directory
(`%APPDATA%\com.priler.jarvis` by default, resolved by `platform-dirs`):

| File | What it is |
|---|---|
| `app.db` | the settings database |
| encrypted stores | notes, vault, and AI memory, each under its own derived key |
| `key.dpapi` | the master key sealed with DPAPI for the current user |
| the Windows-actions directory | `allowed-applications.json`, `timers.json`, `actions-audit.jsonl`, `settings.json`, screenshots |
| `whisper-settings.json` | the dictation settings (no transcript is ever stored) |

**`key.dpapi` must never be copied to another machine or into a backup.** A full
backup is meant to carry a *portable* key envelope protected by a password
instead; that container is not implemented yet (see `docs/BACKUP_RESTORE.md`).

## What a future installer must and must not contain

Written down now so the decision is not made by accident later:

**Must contain:** the application binary, the frontend bundle, the icon and tray
assets, and a declared dependency on the Visual C++ runtime (either bundled
under Microsoft's redistributable terms or stated as a prerequisite).

**Must not contain, without a separate licence check:** any model (`GGUF`,
Whisper `ggml-*.bin`, Vosk model), `llama-server.exe`, `whisper-cli.exe`,
Hunspell dictionaries, ONNX Runtime binaries, or any file whose licence has not
been confirmed. `docs/LICENSING_STATUS.md` lists what is confirmed and what is
not.

**Must not contain, ever:** a user's data directory, `key.dpapi`, a portable key
backup, a transcript, an audit log, a screenshot, a test secret, or an absolute
path from the machine it was built on.

**Uninstall:** removes the application. It must not delete the user's data by
default; deleting it is a separate, explicit option with a warning.

## Verifying a build before using it

```powershell
# the interface and the Rust side must both pass before the build means anything
cd frontend; npm run test:ui; npm run build; cd ..
cargo test --workspace -j 1
cargo clippy --workspace --all-targets -j 1
```

A build that fails one of these is not a build to test by hand.
