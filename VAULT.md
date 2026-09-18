# Local password vault (Windows)

Status: **experimental**. This is the first version of the password vault. It has
not been audited, it cannot be made unbreakable, and a forgotten master password
cannot be recovered. Do not treat it as a replacement for a mature, independently
reviewed password manager until it has been reviewed and used for a while.

Out of scope for this stage: TOTP, bank cards, documents, passkeys, browser
autofill/extension, device-to-device sync, and master-key rotation.

## Key separation

The vault uses the same random master key as the notes feature, but **never the
same working key**:

```text
master password
  -> Argon2id (random salt, 19 MiB, t=2, p=1)
  -> password-derived KEK
  -> unwraps the random 256-bit master key
       |-> HKDF-SHA256, info "JARVIS/notes/v1"  -> notes database
       `-> HKDF-SHA256, info "JARVIS/vault/v1"  -> vault database
```

* Derivation is HKDF-SHA256 (RFC 5869) from the `hkdf` crate with a fixed
  application salt and a versioned domain label. No custom KDF is implemented.
* Purpose keys are derived in memory on unlock and are never written to disk.
* `PurposeKeyProvider` is the only cipher the vault store accepts. It has no
  accessor for the master key and no legacy fallback, so the separation is
  enforced by the type system: a vault-domain provider cannot open a note record
  and a notes-domain provider cannot open a vault record, even if handed the
  ciphertext.
* Records also carry the entity type in their authenticated data, so a payload
  cannot be replayed into another entity or another domain.
* The vault lives in its own database file (`vault.sqlite3`) with its own journal,
  cursors, and conflicts. Notes use `sync.sqlite3`.

### Compatibility with notes written before key separation

Records written before domain separation used the raw master key with envelope
format version 1. The notes provider keeps the master key for exactly that case
and upgrades every record it rewrites to version 2 (HKDF-derived notes key). A
fixture captured from the previous release proves this in
`crates/jarvis-core/src/notes/tests.rs`
(`notes_written_before_key_separation_still_decrypt`).

## Stored data

Each entry is one synchronization entity of type `vault_record`; the payload is
JSON, encrypted before it reaches SQLite:

```json
{
  "schema_version": 1,
  "name": "...",
  "username": "...",
  "password": "...",
  "urls": ["..."],
  "notes": "...",
  "tags": ["..."],
  "favorite": false,
  "created_at": "RFC 3339",
  "updated_at": "RFC 3339",
  "deleted_at": "RFC 3339 | null"
}
```

Everything a user typed is inside the ciphertext. The plaintext metadata is only:
entity ID, entity type, revision, journal cursor, device ID, tombstone flag,
payload schema version, timestamps, and ciphertext length. In other words, the
number of entries, their approximate size, and their edit times are visible to
anyone who can read the file.

Trash is a soft delete (`deleted_at`); permanent deletion writes a tombstone and
drops the payload. Deleting is the only way to remove an entry's ciphertext.

## Interface boundaries

* **Lists and details never contain a password.** `VaultItemSummary` and
  `VaultItemDetails` have no password field, so the IPC payload cannot carry one.
  This is asserted by `crates/jarvis-core/tests/vault_ai_isolation.rs`.
* **`SecretRevealResult` is the only carrier** and is produced solely by
  `vault_reveal`, an explicit user action while the vault is unlocked.
* **Saving without revealing cannot erase a secret.** Renaming, retagging,
  changing the URL, or toggling the favourite uses a metadata-only update; the
  password and the free-form notes are only written by an explicit secret update
  after a reveal.
* **The notes field is treated as sensitive too**: it is not part of the details
  payload and only arrives with a reveal.

## Clipboard

Copying happens in Rust; the value never travels to the interface for that
purpose.

* The copy command returns clipboard state (armed, remaining seconds, timeout) and
  never the value.
* The clipboard is cleared automatically after a configurable delay: 15, 30, 45,
  or 60 seconds, default 30.
* The pending copy is remembered, and the timed wipe only clears the clipboard
  while it still contains **our** value. If the user copied something else in the
  meantime, their text is left untouched.
* A new copy replaces the timer, so an older timer can never wipe a newer value.
* An unreadable clipboard (another process holding it, or no text on it) is
  treated as "not our value" and is never cleared blindly.
* Locking the vault cancels the timer and clears our value immediately.
* A watchdog thread checks every 250 ms; the timer keeps running while the window
  is open, independently of the interface.
* Clipboard failures are reported without the value that failed to be copied.

## Automatic locking

* Options: 1, 5, 15, or 30 minutes, or never. Default: 5 minutes.
* Enforcement is twofold. The page tracks local activity and locks after the
  timeout, and the backend checks the same timeout before every vault command, so
  the master key is dropped even if the page stops running its timers.
* Locking drops the vault key, the decrypted vault cache, the master key, any
  revealed secret in the interface, and the pending clipboard timer.
* Closing the application also locks the storage.
* Lock-on-focus-loss exists as a manual action only: it is not wired to a setting
  in this version, so it cannot surprise the user while they work.

## Password generator

* The generator lives in the core and uses the system CSPRNG (`getrandom`).
  `Math.random` is never used, and the fill/selection logic cannot be reached from
  the interface with a weaker source.
* Index selection uses rejection sampling, so no character is more likely than
  another; modulo folding is never used. A deterministic test feeds out-of-range
  bytes and proves they are rejected instead of folded.
* Options: length (8–128, default 20), lowercase, uppercase, digits, symbols,
  exclude look-alike characters, and "one of every selected category".
* A generated password is placed in the editor and is **not** stored until the
  user saves the entry. It can also be generated straight onto the clipboard, in
  which case it never enters the interface at all.
* Entropy is reported as an estimate, not a guarantee.

## Changing the master password

```text
verify the current password          (fails -> nothing changed)
build the new envelope and DPAPI blob in memory
atomically replace key.backup.json
refresh key.dpapi                    (Windows; skipped elsewhere)
```

* The current password is always verified first.
* The new password must be confirmed, be at least 8 characters, and differ from
  the current one.
* The master key is **re-wrapped**, not replaced, so the stored data is not
  re-encrypted and its revisions do not change.
* Every write goes through a temporary file plus a rename, so a failure leaves the
  previous envelope fully usable. The DPAPI blob protects the same master key, so
  a failure while refreshing it cannot lock the user out.
* No step writes an unwrapped master key to disk.

**Not implemented:** rotating the master key itself. That would require
re-encrypting every record and is a separate future feature. Changing the
password does not change any derived key either; it only changes the wrapper.

## Threats and limitations

Honest scope of what this protects against, and what it does not:

* **Protects** against someone reading the database file, a backup of it, or a
  stolen disk image without the master password. File contents are
  XChaCha20-Poly1305 ciphertext with a fresh nonce per record.
* **Protects** the vault domain from a compromise of the notes domain, and vice
  versa, as far as key usage goes.
* **Does not protect** against malware or another process running as the same
  Windows user while the vault is unlocked. While unlocked, decrypted entries live
  in the process memory, and DPAPI-protected keys can be unwrapped by that user.
* **Does not protect** against a keylogger, screen capture, or clipboard reading
  while a secret is revealed or copied.
* **Does not hide** the number of entries, their approximate size, or their edit
  times.
* **JavaScript memory cannot be reliably wiped.** A password that is shown in the
  interface exists as a JavaScript string, which cannot be zeroized. The interface
  therefore minimizes how long a secret is displayed, clears its state when the
  secret is hidden or the vault locks, and never writes a secret into
  `localStorage`, `sessionStorage`, IndexedDB, the URL, history state, or a log.
  A frontend test asserts those prohibitions on the source.
* **Rust side**: master keys, derived keys, the clipboard value, and payload
  buffers are zeroized (`zeroize`). HKDF's internal PRK is not explicitly zeroized
  by the `hkdf 0.12` / `hmac 0.12` crates, which is a documented limitation.
* **No audit.** The cryptographic primitives come from audited RustCrypto crates,
  but this feature has not been reviewed by a third party.
* **No recovery.** Losing the master password means the entries are
  unrecoverable. Keep the portable backup envelope and the password safe.
* **No browser integration and no autofill.** Secrets are only ever placed on the
  clipboard by an explicit action.

## Where the vault must never reach

The vault is a Rust-only capability and is deliberately unreachable from the AI,
voice, and scripting surfaces:

* `ChatProvider` and the AI layer never receive a `VaultStore` or a
  `VaultSession`;
* the voice command catalogue (`resources/commands`) contains no vault entry, so
  no spoken phrase can list, copy, or reveal an entry;
* there is no Lua API for the vault;
* a test (`crates/jarvis-core/tests/vault_ai_isolation.rs`) scans those surfaces
  for any vault reference and fails if one appears.

## Tests

Backend:

* `crates/jarvis-core/src/vault/tests.rs` – store CRUD, metadata-only saves that
  cannot erase a secret, trash/tombstone, favourites, tags, search, conflict
  resolution (all three choices), wrong-key handling, cross-domain decryption
  failures, generator behavior (categories, look-alike exclusion, rejection
  sampling, uniformity sanity, uniqueness, entropy), clipboard timing and
  protection of foreign clipboard content, idle-lock policy, and Debug redaction.
* `crates/jarvis-core/tests/vault_storage.rs` – session lifecycle, lock/unlock,
  restart, wrong key, portable backup recovery in a new data directory, master
  password change (success, refusal cases, and that the old password keeps
  working after a failed change), DPAPI after a change, separate databases, and a
  scan proving no secret reaches the database, the write-ahead log, or the
  diagnostics.
* `crates/jarvis-core/tests/vault_ai_isolation.rs` – the architectural boundary.

Frontend (`npm run test:ui`):

* `frontend/tests/vault-model.test.mjs` – masking, clearing state on hide, dirty
  detection, autosave, save indicator, idle and clipboard options, generator
  validation and entropy, URL parsing, and master-password validation.
* `frontend/tests/vault-i18n.test.mjs` – every message exists in all three
  locales, locales stay in sync, no message is unused, no key is built by string
  concatenation, and no secret can be persisted in browser storage.
