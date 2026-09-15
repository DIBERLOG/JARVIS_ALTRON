# Local storage security

Record content is encrypted with a random 256-bit master key using XChaCha20-Poly1305. Each encryption uses a system-RNG nonce. The master password is used only to derive an Argon2id KEK; it is never a record-encryption key. Key and temporary plaintext buffers are zeroized where practical, and Debug output redacts secret values.

On Windows, the local master-key copy is protected for the current user by DPAPI (`CryptProtectData` / `CryptUnprotectData`) with UI disabled. DPAPI errors do not include secret material. This copy is intentionally machine/user-context bound.

This layer encrypts payloads only; metadata listed in `ADR_SQLITE_SYNC_STORAGE.md` remains visible. Consumers must not write notes, AI memory, or vault data except as encrypted payloads.
