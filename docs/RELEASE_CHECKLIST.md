# Release checklist

This is the list that has to be true before something leaves this machine. It is
written as checks with a state, not as intentions, and it is split by what the
check actually blocks.

Legend: **yes** — done and verified here; **partial** — done for some parts;
**no** — not done; **n/a** — does not apply at this level; **blocked** — cannot be
done by this project alone.

**Current status: a Personal Windows MVP build for the author's own computer.**

> **Warning.** The vault and the AI memory are experimental. No independent
> security audit has been carried out. This does not block personal use on this
> machine; it does block handing the project to anyone else.

## 0. The three readiness levels

| Level | Blocking checks are in | Honest state |
| --- | --- | --- |
| Personal use on this computer | section 1 | **Code-ready**; the manual checks in section 1.3 are still open |
| Installer for this computer | section 2 | **Not started**: no installer exists |
| Public release / another user | section 3 | **Blocked**: clean-Windows test, licences, independent audit, backup transfer |

A check that is not in a section's list does not block that section. A clean
Windows machine, an independent audit and resolved licences are **not** personal-use
blockers, and this document does not claim any of them was done.

## 1. Personal use on this computer

### 1.1 Build and automated suites

| Check | State |
|---|---|
| `npm ci` | **yes** |
| `npm run build` | **yes** — 0 errors, 1 pre-existing a11y warning |
| `npm run test:ui` | **yes** — 230 tests |
| `cargo check --workspace -j 1` | **yes** |
| `cargo test --workspace -j 1` | **yes** — 852 tests, all suites pass |
| `cargo clippy --workspace --all-targets -j 1` | **partial** — clean for the files written here; the workspace still reports pre-existing upstream warnings |
| `cargo fmt --all -- --check` | **partial** — the files written here are clean; 89 upstream files fail and were deliberately not reformatted |
| `npm audit` | **yes** — 0 production vulnerabilities; dev-only advisories recorded and not force-fixed |

### 1.2 Security and data safety (not weakened by the personal-use framing)

| Check | State |
|---|---|
| `key.dpapi` is never exported, bundled, or logged | **yes** — the backup container excludes it, and the diagnostics screen refuses the name |
| No test secret in the tree | **yes** — `git grep` finds only `FICTIONAL_*` fixtures |
| No absolute developer path in shipped files | **partial** — checked by review; the automated bundle scan comes with the installer |
| The AI and voice paths cannot reach a shell | **yes** — `windows_actions_isolation.rs` (7 tests), and the voice-input route has no process API at all |
| The voice-input route cannot reach the vault, the notes or the memory | **yes** — structural tests in `crates/jarvis-core/src/dictation/tests.rs` |
| Legacy commands that use `taskkill` are disabled or migrated | **partial** — the only `taskkill` entry is in the `calculator` `.yaml` pack, which the loader does not read, and it is marked `risk_level: forbidden` |
| `browser_close` stays `forbidden` | **yes** |
| Wake-on-LAN absent from the project | **yes** — a workspace-wide test fails if it appears |
| Audit log carries no content | **yes** — the `AuditEntry` shape is checked by a test |
| Encryption unchanged by this document | **yes** — key separation, AEAD records, DPAPI binding, portable envelope and the backup container keep their tests |

### 1.3 Checks on this machine

Each of these is a person, a real desktop and a few minutes. They are the
personal-use gate, and they are the ones the automated suites cannot see.

| Check | State |
|---|---|
| The built application starts **outside** `cargo run` | **no** |
| No dependency on `target/debug` or any build-tree path while the installed copy runs | **no** |
| Notes: create, edit, close, reopen | **no** |
| Vault: add a `FICTIONAL_*` entry, lock, unlock | **no** |
| Whisper: dictate a sentence to the real build and get the text | **yes** — verified by hand on this machine |
| Vosk: the listener starts and returns to idle | **no** |
| Global voice input: the phrase, the confirmation, the microphone handover, the text in the field | **no** — the engine is tested; the Windows reader, the commands and the trigger are not wired |
| Tray: icon, menu, states, and a full exit that closes the process | **no** |
| Autostart: on, sign out, sign in, starts once, off again | **no** |
| Timers: a 30-second timer fires and writes its audit line | **no** |
| Diagnostics: the report names no path, note or secret, and the export runs the screen first | **no** |
| Install and uninstall on this machine | **no** — section 2 |
| No secrets in the bundle | **no** — section 2 |

### 1.4 Not blocked, and honestly open

| Item | Why it does not block personal use | What it would need |
|---|---|---|
| A clean-Windows check | the application runs on this computer, which is the only machine it is for | section 3 |
| An independent audit of the vault and the AI memory | the risk is the author's own, and the warning above is in the interface documents | section 3 |
| Unresolved licences | nothing is distributed | section 3 |
| The manual backup/restore check | the container, its validation, its rollback and its tests are in place; the round trip on real data has not been walked through by hand | run the manual script in `docs/BACKUP.md`; it stays **open** and does not block this build |
| A new local Windows profile as a limited substitute for a clean machine | worth doing, cheap, and not mandatory | optional, recorded when it happens |

## 2. Installer for this computer

| Check | State |
|---|---|
| Installer format chosen and recorded in an ADR | **no** |
| Installer builds | **no** |
| Bundle contains no model, no `llama-server`, no dictionary | **no** — there is no bundle |
| Bundle contains no user data, no key, no test secret, no absolute developer path | **no** — the check is part of this stage |
| Architecture is x64 only | **yes** — no other target is configured |
| Install and uninstall verified on this machine | **no** |
| Uninstall keeps user data by default | **no** — nothing to uninstall |
| **Therefore: the package is an installer candidate, not a verified installer** | — |

An installer is not required to keep using the application personally. It is
required before calling anything "installed", and it is listed here rather than in
section 3 because installing and removing it on **this** machine is a sensible
step for the author.

## 3. Before handing to another user, or a public release

Everything here is required before anyone else is asked to rely on the project.
None of it blocks personal use, and none of it has been done.

1. **A clean Windows test.** Install on a machine that has never had this project
   on it, with no build tools and no developer paths, and run the acceptance
   scenarios from section 1.3 there. Not performed, and not claimed.
2. **The licences.** The copyright holder resolves the `Cargo.toml` (GPL-3.0-only)
   versus `LICENSE.txt`/`README.md` (CC-BY-NC-SA-4.0) conflict, `cargo audit`,
   `cargo deny check` and `cargo license` are run, and every bundled component's
   licence is confirmed — including the ONNX Runtime binary and the two `Priler`
   git dependencies. See `docs/LICENSING_STATUS.md`.
3. **An independent security audit** of the vault and the AI memory: the key
   handling, the purpose-key derivation, the record format, the backup container
   and the restore path. Until then the warning at the top of this file stands,
   and it is repeated in `docs/SECURITY.md`, `docs/AI_MEMORY.md` and the vault
   section of the interface.
4. **A verified backup transfer between two computers**: export on one machine,
   restore on a different one, with the notes, the vault, the memory and the
   dictionary compared. The container carries the portable envelope for exactly
   this, and the round trip has not been performed.
5. The manual backup/restore check from section 1.4, on real data.
6. The legacy `taskkill` command is removed or migrated, and the remaining Lua
   full-sandbox route is documented as a deliberate, local-only surface.
7. The installer from section 2 is built, installed, removed, and verified on a
   clean machine.
8. The manual smoke tests in section 1.3 are all **yes**, with the date and the
   machine recorded.
