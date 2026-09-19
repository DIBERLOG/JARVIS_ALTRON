# Third-party notices

This file lists the third-party components the application uses, what they are
for, and where their terms come from. It is a notice file, not a licence grant:
each component remains under its own licence, held by its own authors.

The application itself is distributed under the terms stated by the copyright
holder. Those terms are currently **contradictory** — see
`docs/LICENSING_STATUS.md` — and until that is resolved this file must be read as
"what the components require", not as "what this project grants".

## Bundled or compiled in

These components become part of the application binary or of the interface
bundle that ships with it. Each one's licence requires that its notice and
licence text travel with the distribution.

| Component | Used for | Licence (as declared) |
|---|---|---|
| Rust standard library and crates (`serde`, `serde_json`, `toml`, `regex`, `uuid`, `once_cell`, `log`, `parking_lot`, `chrono`, `rand`, `sysinfo`, `tempfile`, `image`) | general runtime | MIT or Apache-2.0 (dual) |
| `rusqlite` with bundled SQLite | encrypted local storage | MIT; SQLite itself is in the public domain |
| `argon2`, `chacha20poly1305`, `hkdf`, `sha2`, `getrandom`, `zeroize` | key derivation and authenticated encryption | MIT or Apache-2.0 |
| `fluent` / `fluent-bundle` / `unic-langid` | interface translations | Apache-2.0 or MIT |
| `mlua` with vendored Lua 5.5 | the local command-pack sandbox | MIT |
| `hound` | reading and writing WAV audio | Apache-2.0 |
| `kira`, `rodio` | sound playback | MIT or Apache-2.0 |
| `pv_recorder` | microphone capture | see the pinned revision of `Priler/pvrecorder` |
| `rustpotter` | wake-word detection | see the pinned revision of `Priler/rustpotter` |
| `nnnoiseless` | noise suppression | see the crate's own metadata |
| `fastembed`, `ort`, `ndarray`, `tokenizers` | the intent-classification backend, **and the ONNX Runtime binary that `ort-download-binaries` fetches at build time** | fastembed: Apache-2.0; ort: MIT or Apache-2.0; **ONNX Runtime binaries: Microsoft's own terms — verify before distributing** |
| `vosk` (Rust crate) and `libvosk` | speech recognition | Apache-2.0 |
| `spellbook` | local spelling correction | see the crate's own metadata |
| `windows`, `windows-sys`, `winrt-notification`, `clipboard-win` | Windows APIs, notifications, clipboard | MIT or Apache-2.0 |
| `vortaro` (Tauri 2), `@tauri-apps/api`, the dialog/fs/shell plugins, `@vortaro/cli` | the window and its build | MIT or Apache-2.0 |
| `svelte`, `vite`, `@roxi/routify`, `@svelteuidev/core` and friends, `radix-icons-svelte`, `howler`, `worker-timers`, `typescript`, `sass` | the interface | MIT |

## Supplied by the user, never redistributed here

The application is built to work with these, but no copy of them is in this
repository, in the source tree, or in any build produced by this stage. The
person who installs the application chooses and downloads each one, and accepts
its terms by doing so.

| Component | Used for | Licence (as declared) | Where it comes from |
|---|---|---|---|
| `llama.cpp` / `llama-server.exe` | running a local model | MIT | the user's own download; the wizard asks for the path |
| GGUF model weights (for example Qwen3-8B) | the local model | Apache-2.0 for the published Qwen releases; **check the file you download** | the user's own download |
| `whisper.cpp` / `whisper-cli.exe` | local dictation | MIT | the user's own download |
| Whisper `ggml-*.bin` weights | dictation model | MIT for the OpenAI-released weights; check the mirror | the user's own download |
| Vosk models | speech recognition | Apache-2.0 for the models published by Alpha Cephei | the user's own download |
| Hunspell dictionaries (RU/EN) | spelling dictionaries | varies per dictionary, and some are offered under a choice of MIT/BSD/LGPL/GPL | the user's own download |
| Visual C++ runtime | required to run a Rust binary on Windows | Microsoft's redistributable terms | the prerequisite declared in `docs/INSTALLATION_WINDOWS.md` |

## What this file does not claim

* It does not claim that the list of compiled-in crates is exhaustive: it names
  the components a person would ask about, not every transitive dependency.
  `docs/LICENSING_STATUS.md` explains why a complete transitive list was not
  produced in this environment, and gives the commands that produce one.
* It does not claim that a component's declared licence is the only term that
  applies. A crate can be dual-licensed, and a downloaded binary can carry an
  additional agreement.
* It does not resolve the conflict in the project's own licence. Nothing in this
  file grants rights to the application itself.
