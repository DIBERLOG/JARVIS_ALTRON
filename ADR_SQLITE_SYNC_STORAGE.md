# ADR: SQLite sync storage

## Status

Accepted. Implemented in `crates/jarvis-core/src/sync.rs` (protocol),
`crates/jarvis-core/src/sync/sqlite.rs` (durable storage), and
`crates/jarvis-core/src/sync/crypto.rs` (payload encryption).

## Context

Notes, AI memory, and vault records need local-first storage that can later be
replicated between devices without a server that assigns order. Wall-clock
timestamps cannot be trusted for ordering, and clients must not be able to
choose the revision or position of the record they write.

## Decision

### One write entry point

`SyncRepository::apply_mutation` is the only write path. A client submits a
`SyncMutation` and the repository decides everything else. All of the following
happen inside **one transaction**:

```text
operation_id check -> device sequence check -> read entity -> base revision check
-> assign entity revision and server sequence -> write journal
-> write entity or conflict -> COMMIT
```

A conflict, an idempotent repeat, and a refusal inside that transaction leave no
partial rows. A refusal (`ApplyOutcome::Rejected`) writes nothing at all, so a
repeated refused request stays refused without consuming a cursor.

### What the client supplies, what the repository assigns

| Client-supplied               | Repository-assigned        |
| ----------------------------- | -------------------------- |
| `operation_id`, `entity_id`   | `entity_revision`          |
| `entity_type`, `device_id`    | `server_sequence` (cursor) |
| `device_sequence`             |                            |
| `base_revision`, `kind`       |                            |
| `timestamp`, `schema_version` |                            |
| `encrypted_payload`           |                            |

`entity_revision` and `server_sequence` are deliberately absent from
`SyncMutation`, so a client cannot forge either.

### Revision and cursor rules

```text
new entity:  base_revision = 0                  -> entity_revision = 1
update:      current_revision = N, base = N      -> entity_revision = N + 1
conflict:    current_revision != base_revision   -> entity unchanged, incoming version kept
```

Additional invariants:

* A repeated `operation_id` is idempotent: the original outcome is returned and
  the cursor does not move.
* `(device_id, device_sequence)` is unique; reusing a sequence with a different
  operation is refused.
* `server_sequence` is assigned monotonically by the repository (`AUTOINCREMENT`),
  never by a timestamp. Two devices can compare progress without trusting clocks.
* A tombstone produces a new revision, and a later non-delete mutation cannot
  resurrect a tombstoned entity.
* `kind` is a client hint. Conflict detection is revision-based, so a stale
  mutation that is mislabelled (an update sent with `base_revision = 0` because
  the client never saw the entity) is preserved as a conflict instead of being
  discarded as malformed.
* `page_after` returns applied entries only. Conflicts consume a server sequence
  and advance `next_cursor`, but they are never replicated to peers.
* A retained conflict is resolved by an explicit operator action
  (`discard_conflict` after keeping the current version, accepting the incoming
  one, or storing it as a second note). Only the pending conflict row is removed:
  the journal entry and its payload survive, so a version is never destroyed
  silently. Notes use this in `crates/jarvis-core/src/notes/`; see `NOTES.md`.

### Schema

`SCHEMA_VERSION = 2`. Migrations run inside one transaction and are idempotent:

* `v1` – metadata, entities, operations journal, conflicts, devices, cursors.
* `v2` – `sync_entities.device_id`, journal `outcome`/`conflict_id`, conflict
  snapshot columns (`base_revision`, `server_sequence`, `current_device_id`,
  `current_updated_at`), plus backfill of the new columns from data the older
  schema already had.

A database declaring a version newer than this build is refused with
`SyncError::UnsupportedSchemaVersion` rather than opened or downgraded.

### Connection settings

Foreign keys are enabled and verified to be on. `synchronous = FULL`. WAL is
requested and the pragma result is checked; if SQLite does not report `wal`, the
open fails instead of silently running in another journal mode. A busy timeout
(default five seconds) is configured, and `SQLITE_BUSY`/`SQLITE_LOCKED` surface
as `SyncError::StorageBusy`. Closing runs `wal_checkpoint(TRUNCATE)` before the
connection is released.

### Location

`SqliteSyncRepository::production_path()` resolves the per-user application data
directory (`platform_dirs::AppDirs` with the bundle identifier). The database is
never created inside the repository checkout.

## Metadata that stays readable

Ciphertext protects content, not existence. The following columns are plaintext
by design and must be treated as non-secret:

* entity ID and entity type;
* device ID and device sequence;
* operation ID, operation kind, base revision, entity revision, server sequence;
* timestamps, tombstone flag, payload schema version;
* ciphertext length.

## Consequences

* Storage is replication-ready without a network layer: `page_after` plus device
  cursors describe exactly what a peer has already seen.
* Encryption is enforced at the API level: `EncryptedPayload` has no public
  constructor, so only a `CryptoProvider` (in practice
  `MasterKeyCryptoProvider`) can produce a storable payload. There is no
  production path that stores note, memory, or vault plaintext.
* Existing conflicts are retained for review; resolving them (accepting the
  incoming version) is a later, explicit stage.
* If a master password is lost without a portable backup, encrypted payloads are
  unrecoverable; see `LOCAL_STORAGE_BACKUP.md`.
