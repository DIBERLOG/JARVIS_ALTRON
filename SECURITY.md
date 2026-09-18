# Local storage security

Scope: the local encrypted storage layer (`crates/jarvis-core/src/sync/crypto.rs`)
and the durable sync repository that stores its output. There is no network
transport in this layer and none is planned for this stage.

## Key hierarchy

```text
master password
  -> Argon2id (random 16-byte salt)
  -> password-derived KEK
  -> unwraps the random 256-bit master key
  -> master key encrypts individual records
```

The master password is never used as a record-encryption key, and a leaked record
key does not expose the master key or the password.

* KDF: Argon2id, `Variant` from the `argon2` crate (no custom construction).
* Default parameters: 19 MiB memory, 2 iterations, parallelism 1, 32-byte output.
  They are stored in every backup envelope so a different machine can restore.
* Master passwords shorter than `MIN_PASSWORD_BYTES` (8) are refused with a
  controlled error.
* Parameters read from an untrusted backup are bounded (memory 8 MiB..1 GiB,
  iterations 1..16, parallelism 1..8) so a hostile envelope cannot demand
  unbounded memory or CPU.

## Record encryption

* AEAD: XChaCha20-Poly1305 with a fresh 24-byte system-RNG nonce per record.
  Nonces are never reused for the same key.
* Every record carries an explicit `format_version`; unknown versions are
  rejected before any decryption is attempted.
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
  `PortableKeyBackup`, `MasterKeyCryptoProvider`, `EncryptedPayload`,
  `SyncMutation`, and `SyncRecord` so accidental diagnostics can never print
  metadata, ciphertext, or payload bytes.
* Payload bytes never reach `log` output: the storage layer maps SQLite failures
  to opaque `SyncError` variants, and tests assert that fictional note and vault
  markers do not appear in the database file or in `Debug` output.

## What this layer does not do

* It does not hide *existence* or *shape*: metadata listed in
  `ADR_SQLITE_SYNC_STORAGE.md` (entity IDs, entity types, device IDs, revisions,
  cursors, timestamps, ciphertext lengths) is stored in the clear.
* It does not protect against an attacker who already runs code as the logged-in
  Windows user; DPAPI-protected keys are recoverable by that user by design.
* It does not implement its own ciphers, KDFs, or randomness. Only audited crates
  (`argon2`, `chacha20poly1305`, `getrandom`) and the OS API are used.
* It does not yet cover key rotation or password change; changing the master
  password means exporting a new backup envelope.
