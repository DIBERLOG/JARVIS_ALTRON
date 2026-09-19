# Licensing status

This document records what is known about the licence of every component this
project uses, what is contradictory, and what that means for using and shipping
the application. It does not resolve anything: only the copyright holder can do
that, and this file must not pretend otherwise.

**Bottom line: the conflict below blocks a public release. Personal use of a
build you make for yourself is a different question, and this document says what
is known so that decision can be made by the person who owns the rights.**

## The project's own licence — a conflict that is not resolved

| Where | What it says |
|---|---|
| `Cargo.toml` (`[workspace.package]`) | `license = "GPL-3.0-only"` |
| `LICENSE.txt`, `README.md` (upstream) | CC-BY-NC-SA-4.0 |

GPL-3.0-only and CC-BY-NC-SA-4.0 are not two spellings of one licence:

* **GPL-3.0-only** is a software licence that grants commercial use and requires
  source disclosure for derived works;
* **CC-BY-NC-SA-4.0** forbids commercial use of the licensed work.

A single work cannot be offered under both as if they were one grant: a
downstream user cannot satisfy "commercial use is allowed" and "commercial use
is forbidden" at the same time. The correct fix is a decision by the copyright
holder — either the code is GPL-3.0-only (and the README/LICENSE files are
corrected to say so), or it is CC-BY-NC-SA-4.0 (and the Cargo metadata is
corrected, accepting that it is then not an OSI-approved software licence).

**What this repository does about it:** nothing is rewritten. This file records
the conflict, `docs/RELEASE_CHECKLIST.md` lists it as a blocker, and the build
produced by this stage is named **Personal Windows MVP build**, not a release.

## Components and their licences

The table separates what is **confirmed** (the component states it, and the
statement is well known), what is **not distributed** by this project (so its
licence governs the user's own copy, not our distribution), and what is
**unknown and must be verified** before anything is shipped to anyone else.

| Component | Role | Licence as declared | Distributed by this project? | Status |
|---|---|---|---|---|
| This application (the source in this repository) | the application | GPL-3.0-only in `Cargo.toml`; CC-BY-NC-SA-4.0 in `LICENSE.txt`/`README.md` | yes (source) | **conflicting** |
| `llama.cpp` / `llama-server.exe` | local model server | MIT | **no** — the user supplies it | not distributed; the wizard asks for the path |
| Qwen GGUF model (for example Qwen3-8B) | local model weights | Apache-2.0 for the published Qwen releases; **each file must be checked** | **no** — the user supplies it | not distributed; the version and terms are the user's |
| `whisper.cpp` (`whisper-cli.exe`) | local dictation | MIT | **no** — the user supplies it | not distributed |
| Whisper model (`ggml-*.bin`) | dictation model weights | MIT for the OpenAI-released weights (verify the mirror you download from) | **no** — the user supplies it | not distributed |
| Vosk / `libvosk` | speech recognition runtime | Apache-2.0 | **no** — not bundled by this stage | not distributed; if it is ever bundled, its `NOTICE` requirements apply |
| Vosk models | recognition models | Apache-2.0 for the models published by Alpha Cephei | **no** | not distributed |
| Hunspell dictionaries (RU/EN) | spelling dictionaries | varies by dictionary: the RU and EN dictionaries commonly carry MIT/BSD/LGPL/GPL alternatives | **no** | **unknown per dictionary** — must be checked file by file before distribution |
| `spellbook` (Rust crate) | local spelling engine | as declared by the crate (verify on crates.io before distribution) | yes (compiled in) | **verify** |
| `mlua` (Lua 5.5, vendored) | the command-pack sandbox | MIT (Lua itself is MIT) | yes (compiled in) | confirmed for Lua; verify the crate's own metadata |
| `rusqlite` (bundled SQLite) | local storage | MIT (SQLite is public domain) | yes (compiled in) | confirmed |
| `chacha20poly1305`, `argon2`, `hkdf`, `sha2`, `getrandom`, `zeroize` | cryptography | MIT or Apache-2.0 (dual) | yes (compiled in) | confirmed |
| `vortaro` (Tauri 2) and its plugins | window, dialogs, files | MIT or Apache-2.0 | yes | confirmed |
| `svelte`, `vite`, `routify`, `@svelteuidev/*`, `radix-icons-svelte`, `howler`, `worker-timers` | interface | MIT (radix-icons-svelte: MIT; `@svelteuidev/*`: MIT) | yes (bundled into the frontend) | confirmed for the ones listed; re-check on version bumps |
| `pv_recorder`, `rustpotter` (both from `Priler`'s repositories) | audio capture and wake word | as declared by those repositories | yes (compiled in) | **verify** — these are git dependencies, so their licence travels with the commit that is pinned in `Cargo.lock` |
| `fastembed` / `ort` | embeddings for the intent backend | Apache-2.0 (fastembed), MIT/Apache-2.0 (ort), **and ONNX Runtime binaries under their own terms** | yes (a binary is downloaded at build time by `ort-download-binaries`) | **verify** — the ONNX Runtime binary has its own licence and is fetched by a build script |
| Visual C++ runtime | required to run the application | Microsoft's redistributable terms | **no** — declared as a prerequisite | not distributed |

## What is not enumerated here

The list above covers the **direct** dependencies and the external binaries a
person supplies. It is not a full transitive audit: `cargo-license`,
`cargo-deny`, and `cargo-audit` are not installed in this environment, and a
transitive list produced by hand would be a guess. `npm audit` was run and is
recorded in `docs/WINDOWS_MVP.md`.

If a tool is available, the command that produces the full list is:

```powershell
cargo install cargo-license cargo-deny cargo-audit
cargo license --json > licences.json
cargo deny check licenses
```

## What may be done with this build today

* **Personal use on your own machine:** the components you supply yourself
  (`llama-server`, the GGUF model, `whisper-cli`, the Whisper model, Vosk) are
  governed by their own licences, which you accept by downloading them. Nothing
  in this project redistributes them.
* **Sharing the source:** the conflict in `Cargo.toml` versus `LICENSE.txt` must
  be resolved by the copyright holder first. Until then, a person who receives
  the source does not know which terms apply.
* **Sharing a built installer:** **blocked.** An installer would distribute
  compiled code and third-party binaries, which is exactly what the unresolved
  conflict and the unverified entries forbid.

## The name of this build

Until the conflict is resolved by the copyright holder, the result of this stage
is called:

```text
Personal Windows MVP build
```

It is not a release, not a beta, and not a distribution. `docs/RELEASE_CHECKLIST.md`
states the same thing in the form of a checklist.
