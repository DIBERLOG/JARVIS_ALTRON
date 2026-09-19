# Release checklist

This is the list that has to be true before anything leaves this machine. It is
written as checks with a state, not as intentions.

Legend: **yes** — done and verified here; **partial** — done for some parts;
**no** — not done; **blocked** — cannot be done by this project alone.

**Current status: a Personal Windows MVP build for the author's own machine. A
public release is BLOCKED by the licence conflict in `docs/LICENSING_STATUS.md`.**

## 1. Licence

| Check | State |
|---|---|
| The project's own licence is stated once, unambiguously | **no** — `Cargo.toml` says GPL-3.0-only, `LICENSE.txt`/`README.md` say CC-BY-NC-SA-4.0 |
| Third-party notices exist | **yes** — `THIRD_PARTY_NOTICES.md` |
| Every bundled component's licence is confirmed | **partial** — confirmed for the crates listed as such; `spellbook`, `nnnoiseless`, the two `Priler` git dependencies, and the ONNX Runtime binary still need a check |
| Every user-supplied component is clearly *not* distributed | **yes** — stated in `docs/LICENSING_STATUS.md` and `docs/INSTALLATION_WINDOWS.md` |
| A complete transitive dependency licence list exists | **no** — `cargo-license`/`cargo-deny` are not installed here |
| Public release permitted | **blocked** |

## 2. Security

| Check | State |
|---|---|
| `key.dpapi` is never exported, bundled, or logged | **yes** — stated in the backup and installation documents; the diagnostics screen refuses `key.dpapi` |
| No test secret in the tree | **yes** — `git grep` finds only `FICTIONAL_*` fixtures |
| No absolute developer path in shipped files | **partial** — checked by review; no automated check exists yet |
| The AI and voice paths cannot reach a shell | **yes** — `windows_actions_isolation.rs` (7 tests) and the legacy audit in `docs/WINDOWS_ACTIONS.md` |
| Legacy commands that use `taskkill` are disabled or migrated | **partial** — the only `taskkill` entry is in the `calculator` `.yaml` pack, which the loader does not read (it reads `command.toml`), and the entry is now marked `risk_level: forbidden`; the loaded packs contain no `taskkill` |
| `browser_close` stays `forbidden` | **yes** — it is `forbidden` and refused by the gate |
| Wake-on-LAN absent from the project | **yes** — a workspace-wide test fails if it appears |
| Audit log carries no content | **yes** — the `AuditEntry` shape is checked by a test |

## 3. Data safety

| Check | State |
|---|---|
| Whole-application backup container | **no** |
| Restore with rollback and atomic replacement | **no** |
| Portable key envelope inside the backup | **no** (the envelope itself exists; the container does not) |
| Per-feature export/import works | **yes** — notes, vault, memory, word list |
| No live-database file copy anywhere | **yes** — no such code exists yet, which is why the container is a design and not an approximation |

## 4. Build and tests

| Check | State |
|---|---|
| `npm ci` | **yes** |
| `npm run build` | **yes** — 0 errors |
| `npm run test:ui` | **yes** — 173 tests |
| `cargo check --workspace -j 1` | **yes** |
| `cargo test --workspace -j 1` | **yes** — all suites pass |
| `cargo clippy --workspace --all-targets -j 1` | **partial** — clean for the code written here; the workspace still reports pre-existing upstream warnings |
| `cargo fmt --all -- --check` | **partial** — the files written here are clean; 89 upstream files fail and were deliberately not reformatted |
| `cargo audit` | **no** — not installed |
| `cargo deny check` | **no** — not installed |
| `npm audit` | **yes** — 0 production vulnerabilities; dev-only advisories recorded and not force-fixed |

## 5. Packaging

| Check | State |
|---|---|
| Installer format chosen and recorded in an ADR | **no** — not chosen yet |
| Installer builds | **no** |
| Bundle contains no model, no `llama-server`, no dictionary | **yes, vacuously** — there is no bundle |
| Bundle contains no user data, key, or test secret | **yes, vacuously** |
| Architecture is x64 only | **yes** — no other target is configured |
| Uninstall keeps user data by default | **no** — nothing to uninstall |
| Installed on a real Windows machine and verified | **no** |
| **Therefore: the package is an installer candidate, not a verified installer** | — |

## 6. Runtime behaviour

| Check | State |
|---|---|
| Ordered exit, bounded, idempotent | **yes** — `lifecycle.rs`, 10 tests, wired into the Tauri exit path |
| Tray icon and menu | **no** |
| Autostart, off by default, user-controlled | **no** |
| First-run wizard | **no** |
| `NotConfigured` states for every optional feature | **partial** — dictation and Windows actions report them; the wizard that would use them does not exist |
| Diagnostics report, redacted and previewed | **yes** in the core; **no** window command |
| Notifications honest about what the build can do | **yes** — in-application notification; no AUMID without an installer, and the capability flag says `false` on an uninstalled build |

## 7. Manual smoke tests on a real machine

| Check | State |
|---|---|
| Window starts | **no** |
| Notes create/edit/restart | **no** |
| Vault with a `FICTIONAL_*` entry, lock/unlock | **no** |
| Volume, screenshot, timer, window action | **no** |
| Dictation end to end | **no** |
| Local AI with a real server and model | **no** |
| Vosk | **no** |
| Exit report clean | **no** |

**Every "no" here is a reason not to call this build tested.** They are listed so
that the next session starts from the truth.

## 8. The minimum before a public release

1. The copyright holder resolves `Cargo.toml` versus `LICENSE.txt`.
2. `cargo audit`, `cargo deny check`, and `cargo license` are run and their
   findings are addressed or accepted in writing.
3. Every bundled component's licence is confirmed, including the ONNX Runtime
   binary and the two `Priler` git dependencies.
4. The installer is built, installed, and verified on a clean Windows machine,
   including uninstall behaviour.
5. The legacy `taskkill` command is removed or migrated, and the remaining Lua
   full-sandbox route is documented as a deliberate, local-only surface.
6. The whole-application backup and restore exist and are tested, because a
   user's data must be recoverable before anyone else is asked to rely on it.
7. The manual smoke tests in section 7 have been run and recorded.
