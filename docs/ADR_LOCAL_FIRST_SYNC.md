# ADR: local-first synchronization core

## Status

Accepted for stage 1. This decision defines local data contracts only; it does
not add a sync server, Android client, pairing flow, transport, SQLite storage,
or production encryption.

## Context

The current application persists desktop settings as a local JSON file and has
AI conversation structs, but no shared mobile/desktop synchronization model.
Copying a database file would make independent offline edits unsafe.

## Decision

Synchronize individual operations. Each operation has a UUID, `device_id`,
`base_revision`, next `revision`, entity type, ISO-8601 metadata timestamp,
and tombstone state. Server/client time is informational only; revision checks
decide ordering.

The supported entity types are notes, note folders and tags, user-approved AI
memory, autocorrect dictionaries, safe UI settings, selected JARVIS and ALTRON
settings, vault metadata, and encrypted vault records. Model files, audio,
caches, diagnostic logs, plaintext secrets, master passwords, keys, hardware
identifiers, and third-party tokens are not sync entities.

`SyncRecord` separates `SyncRecordMetadata` from its opaque
`EncryptedPayload`. Sync logs contain only identifiers, entity types and
outcomes. Payload debug output is redacted.

`SyncEngine` records local operations, applies received operations once, and
queries changes after a monotonically increasing cursor. `SyncRepository` and
`CryptoProvider` are interfaces for later persistent and production adapters.
The only in-memory repository and reversible provider are test-only and cannot
be compiled into a production configuration.

Incoming operations validate device identifiers, operation/revision shape,
RFC 3339 timestamps, tombstone/payload consistency, and a 1 MiB encrypted
payload limit. Transport-level request-size limits and authenticated device
authorization remain the responsibility of the future sync service.

When an incoming operation's `base_revision` differs from the current record,
the record is unchanged and a `SyncConflict` stores both the current and
incoming versions. Tombstones are operations with no payload and remain in the
record; they are not immediately purged.

## Consequences

This stage has no network endpoint, device authorization, pairing, Tailscale,
Android UI, automatic conflict merge, password storage, or real cryptography.
Future production storage must implement `SyncRepository`; a vetted AEAD-based
provider must implement `CryptoProvider` without changing the operation model.
