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

## Restore procedure (intended flow)

1. Install JARVIS on the new machine and let it create its own local DPAPI blob.
2. Import the portable envelope and supply the original master password; the
   master key is unwrapped in memory.
3. Replace the new local DPAPI blob with a fresh protection of the restored
   master key (`dpapi_protect`).
4. Existing encrypted payloads now decrypt, because they were encrypted under the
   restored master key; the previous machine's local blob is irrelevant and
   cannot unlock them.

Importing a backup does not modify or delete the existing local blob until step 3
succeeds, so a wrong password leaves the current installation untouched.
