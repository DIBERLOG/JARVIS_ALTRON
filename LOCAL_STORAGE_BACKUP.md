# Portable local-key backup

A portable backup moves the master key to another computer. It is the only
supported way to recover encrypted notes, AI memory, and vault records after
reinstalling Windows or changing machines.

## Envelope

`PortableKeyBackup` is a versioned, self-describing envelope:

| Field                  | Purpose                                              |
| ---------------------- | ---------------------------------------------------- |
| `format_version`       | envelope version (currently `1`)                     |
| `kdf`                  | KDF identifier, `argon2id`                           |
| `memory_kib`           | Argon2id memory cost used for this envelope          |
| `iterations`           | Argon2id time cost used for this envelope            |
| `parallelism`          | Argon2id parallelism used for this envelope          |
| `salt`                 | random 16-byte KDF salt                              |
| `aead`                 | AEAD identifier, `xchacha20poly1305`                 |
| `nonce`                | random 24-byte nonce for the key wrap                |
| `encrypted_master_key` | the random master key, encrypted under the KEK       |
| `metadata`             | authenticated metadata string (see below)            |

`metadata` is a canonical string
`jarvis-key-backup|<version>|<kdf>|<memory>|<iterations>|<parallelism>|<aead>|<salt>`
and is passed as AEAD associated data. It is stored alongside the envelope so a
restore can verify that the parameters it is about to use are the exact ones the
envelope was created with. Changing any field, including swapping in weaker KDF
parameters, invalidates the tag.

Binary fields are encoded as byte arrays by `to_json()`; `from_json()` parses the
same envelope back. The envelope never contains the master key, the master
password, or any plaintext record.

## Guarantees

* The master key is written only as AEAD ciphertext.
* A wrong master password is rejected; there is no partial or best-effort decode.
* Altered metadata, altered salt, altered nonce, truncated fields, and a flipped
  ciphertext byte are all detected.
* An unknown `format_version`, `kdf`, or `aead` identifier is refused before any
  key derivation, so a future format cannot be silently misread by this build.
* KDF parameters arriving from an untrusted envelope are range-checked before use.
* Restore is impossible without the master password. Losing it means the data is
  unrecoverable; JARVIS has no recovery key and no backdoor.

## Restore procedure

Implemented for the notes storage (`crates/jarvis-core/src/notes/vault.rs`):

1. Install JARVIS on the new machine. On first start the storage is
   `uninitialized`, so no key file exists yet.
2. Copy the encrypted database (`sync.sqlite3`) if the notes are being moved, or
   start empty to restore only the key.
3. Open the notes page. Because the database has entities but no key file, the
   storage reports `key_missing` and refuses to create a new key: generating one
   would orphan the existing ciphertext.
4. Import the portable backup, either from a file (`Import backup file`) or by
   pasting the envelope into the form, and supply the original master password.
   The master key is unwrapped in memory and both local key files are re-created:
   a fresh DPAPI blob for this machine plus a copy of the envelope.
5. Existing encrypted payloads now decrypt, because they were encrypted under the
   restored master key. The previous machine's local blob is irrelevant and cannot
   unlock them.

A wrong password changes nothing: the import fails before any file is rewritten.

## Saving a portable copy from the interface

Both the notes page and the vault page offer `Save a backup copy`. It re-wraps the
in-memory master key with a password of the user's choice and writes a new
envelope to a chosen file. It needs the storage to be unlocked, because the master
key must be in memory, but it never stores or returns the master password. The
locally stored envelope can also be copied verbatim
(`copy_local_backup_to`), which needs no key material because the file is already
password-protected.

## One envelope covers notes and passwords

Notes and the password vault share one master key and therefore one portable
envelope. Restoring it recovers both databases: each derives its own working key
(`JARVIS/notes/v1`, `JARVIS/vault/v1`) from the restored master key. Copy both
`sync.sqlite3` and `vault.sqlite3` to move the data; the envelope alone is not
enough without the ciphertext, and the ciphertext is useless without the
envelope.

## Changing the master password

Changing the master password rewrites this envelope with a new wrapper around the
**same** master key, and refreshes the local DPAPI blob. Stored data is not
re-encrypted and keeps its revisions fine. The replacement is atomic (temporary
file plus rename), so a failure leaves the previous envelope usable — that
behaviour is covered by an integration test. Rotating the master key itself is not
implemented yet; see `VAULT.md`.

## DPAPI is not a backup

Windows DPAPI protection (`crates/jarvis-core/src/sync/crypto.rs`) exists so the
application can unlock the local database without retyping the master password on
every start. The resulting blob is:

* bound to the current Windows user on the current machine;
* explicitly excluded from the portable envelope;
* never used to restore data on another computer.

The DPAPI blob and the portable backup are separate Rust types
(`DpapiProtectedKey` and `PortableKeyBackup`), so the two cannot be confused at
compile time.
