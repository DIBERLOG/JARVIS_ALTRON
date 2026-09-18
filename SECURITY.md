# Local storage security

Scope: the local encrypted storage layer (`crates/jarvis-core/src/sync/crypto.rs`)
and the durable sync repository that stores its output. There is no network
transport in this layer and none is planned for this stage. The password vault
built on top of it has its own document: `VAULT.md`.

## Key hierarchy

```text
master password
  -> Argon2id (random 16-byte salt)
  -> password-derived KEK
  -> unwraps the random 256-bit master key
  -> HKDF-SHA256 per purpose (JARVIS/notes/v1, JARVIS/vault/v1, JARVIS/ai-memory/v1)
  -> the purpose key encrypts the records of that feature
```

The master password is never used as a record-encryption key, no feature encrypts
with the raw master key, and no two features share a working key. A leaked notes
key does not expose the vault key or the master key, and vice versa.

* KDF: Argon2id, `Variant` from the `argon2` crate (no custom construction).
* Default parameters: 19 MiB memory, 2 iterations, parallelism 1, 32-byte output.
  They are stored in every backup envelope so a different machine can restore.
* Purpose keys: HKDF-SHA256 (`hkdf` crate, RFC 5869) with a fixed application salt
  and a versioned domain label. Derived in memory on unlock; never written to
  disk. Labels are part of the on-disk contract and must never be reused.
* Master passwords shorter than `MIN_PASSWORD_BYTES` (8) are refused with a
  controlled error.
* Parameters read from an untrusted backup are bounded (memory 8 MiB..1 GiB,
  iterations 1..16, parallelism 1..8) so a hostile envelope cannot demand
  unbounded memory or CPU.
* The vault store accepts only `PurposeKeyProvider`, which holds a derived key and
  has no accessor for the master key; the notes store uses
  `MasterKeyCryptoProvider`, which also keeps the master key so that records
  written before key separation still decrypt.

## Record encryption

* AEAD: XChaCha20-Poly1305 with a fresh 24-byte system-RNG nonce per record.
  Nonces are never reused for the same key.
* Every record carries an explicit `format_version`; unknown versions are
  rejected before any decryption is attempted. Version 2 records are encrypted
  with a purpose key; version 1 records predate key separation and are opened
  with the raw master key so existing notes keep working. The version byte is
  authenticated, so it cannot be downgraded by tampering.
* Authenticated associated data binds the record to its version **and** to its
  logical location (entity type plus entity ID). Moving a ciphertext to another
  entity or another entity type makes decryption fail instead of silently
  returning the wrong plaintext.
* A wrong key, a wrong password, a truncated ciphertext, and a flipped byte all
  produce the same controlled error, so callers cannot use error text as an
  oracle for "correct key but damaged data".
* The serialized envelope is `format_version || nonce || ciphertext`; it contains
  no plaintext, no key material, and no lengths that reveal content structure.

## Windows key protection (DPAPI)

* `CryptProtectData` / `CryptUnprotectData` from `windows_sys`, compiled under
  `cfg(windows)` only.
* Scope is the **current Windows user**: `CRYPTPROTECT_LOCAL_MACHINE` is never
  requested, so the blob is not usable by other accounts on the machine.
* `CRYPTPROTECT_UI_FORBIDDEN` is always set, so DPAPI can never raise a system
  prompt or require user interaction.
* Failure is reported as `CryptoError::PlatformProtectionUnavailable { os_error }`
  where `os_error` is a Windows error code. No blob content, key material, or
  plaintext ever appears in the error.
* The buffer returned by Windows is copied and then released with `LocalFree`.
* This blob is a convenience for local unlock only. It is **not** a backup: it
  cannot be restored on another computer or another Windows account. See
  `LOCAL_STORAGE_BACKUP.md`.
* On non-Windows targets the functions return `CryptoError::UnsupportedPlatform`
  instead of pretending to protect anything.

## Memory hygiene

* `MasterKey` zeroizes its bytes on drop (`zeroize::ZeroizeOnDrop`) and has no
  `Display`; its `Debug` prints `MasterKey(<redacted>)`.
* Intermediate plaintext buffers (for example the unwrapped master key during
  backup import) are zeroized as soon as they are copied.
* `Debug` is manually implemented for `EncryptedRecord`, `DpapiProtectedKey`,
  `PortableKeyBackup`, `MasterKeyCryptoProvider`, `PurposeKeyProvider`,
  `EncryptedPayload`, `SyncMutation`, `SyncRecord`, the vault DTOs, the vault
  payload, the clipboard guard, and the generator policy so accidental
  diagnostics can never print metadata, ciphertext, keys, or secrets.
* Payload bytes never reach `log` output: the storage layer maps SQLite failures
  to opaque `SyncError` variants, and tests assert that fictional note and vault
  markers do not appear in the database file or in `Debug` output.
* Known gap: the `hkdf`/`hmac` crates do not zeroize their internal PRK on drop.
  Our own keys, the clipboard value, and temporary plaintext buffers are
  zeroized; that intermediate hash state is not.

## Password vault

The password vault (`crates/jarvis-core/src/vault/`, interface in
`frontend/src/routes/vault/`) is built on this layer with a derived working key of
its own. `VAULT.md` describes it in full; the security-relevant points are:

* its database (`vault.sqlite3`) is separate from the notes database, and its
  payload cipher is `PurposeKeyProvider` for `JARVIS/vault/v1`, which never holds
  the master key;
* lists and detail payloads cannot carry a password, and a metadata-only save
  cannot erase a secret that was never revealed;
* copying a secret happens in Rust, the command returns clipboard state only, and
  the timed wipe clears the clipboard only while it still holds our own value;
* the generator uses the system CSPRNG with rejection sampling, never
  `Math.random`, and never folds bytes with a modulo;
* the vault is unreachable from the AI, voice, and scripting surfaces, which a
  source-scanning test enforces.

## Notes on Windows

The notes feature (`crates/jarvis-core/src/notes/`) is the first consumer of this
layer. It stores each note and folder as an encrypted entity payload, so title,
body, tags, and folder names exist in plaintext only in memory.

* Key material lives beside the database in the application data directory:
  `key.backup.json` (portable envelope) and `key.dpapi` (DPAPI blob for the
  current Windows user). The master password is never stored.
* Locking the storage drops the master key, and while locked the interface shows
  no note content and the backend refuses every content read with
  `StorageLocked`.
* If the database survives but the key files do not, the storage reports
  `key_missing` and refuses to generate a new key, because that would orphan the
  existing ciphertext. Recovery is only possible through a portable backup.
* Content never reaches a log line or an error message: `NoteError` carries
  stable, content-free text, and `Debug` redacts titles, bodies, tags, folder
  names, and excerpts.
* Search runs in memory over decrypted notes; no plaintext index or plaintext
  search column exists. The cost of that choice is documented in `NOTES.md`.
* The only temporary file the notes layer writes is the staging file used for
  atomic key-file replacement, and it contains ciphertext only.
* Notes are not sent to the AI layer, and this stage has no network path.

## What this layer does not do

* It does not hide *existence* or *shape*: metadata listed in
  `ADR_SQLITE_SYNC_STORAGE.md` (entity IDs, entity types, device IDs, revisions,
  cursors, timestamps, ciphertext lengths) is stored in the clear. For notes that
  means the number of notes, their approximate size, and their edit times are
  visible without the key.
* It does not protect against an attacker who already runs code as the logged-in
  Windows user; DPAPI-protected keys are recoverable by that user by design, and
  decrypted notes live in that user's process memory while unlocked.
* It does not implement its own ciphers, KDFs, or randomness. Only audited crates
  (`argon2`, `chacha20poly1305`, `getrandom`) and the OS API are used.
* It does not yet cover key rotation or password change; changing the master
  password means exporting a new backup envelope.
* It does not scrub the decrypted note set from memory when locking beyond
  dropping the key and the cache: process memory that held plaintext is reused,
  not explicitly wiped.
