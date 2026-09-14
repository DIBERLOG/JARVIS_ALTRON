# Threat model: stage-1 local-first synchronization

## Assets

Notes, approved AI-memory entries, password-vault records, device identities,
operation history, and future encryption keys are sensitive. This stage stores
no real passwords and introduces no persistent store or production keys.

## Threats and controls

| Threat | Stage-1 control | Deferred control |
| --- | --- | --- |
| Note or vault content in logs | Opaque payload and redacted `Debug`; audit entries contain metadata only | Structured logging policy and production log review |
| Replay of an operation | UUID outcomes make re-application idempotent | Authenticated transport and durable replay journal |
| Offline concurrent edits | `base_revision` mismatch creates a conflict retaining both versions | UI/manual conflict resolution and server coordination |
| Deleted record returns after sync | Tombstone operation is retained | Tombstone retention and authenticated garbage collection |
| Untrusted mobile client | No transport is exposed in this stage | Pairing, per-device keys, authorization and rate limits |
| Compromise of local storage | No real persistence is added | SQLCipher/OS key storage and vetted AEAD provider |

## Non-goals and assumptions

This module does not claim confidentiality. Its test-only reversible provider
is explicitly not cryptography and is unavailable outside tests. A future
transport must authenticate a paired device before accepting operations and
must never treat network membership alone as authorization.
