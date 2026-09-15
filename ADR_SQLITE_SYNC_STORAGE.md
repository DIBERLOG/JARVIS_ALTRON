# SQLite sync storage

JARVIS stores local sync state in the application's data directory, never in the checkout. The database uses SQLite with foreign keys, a five-second busy timeout, `synchronous=FULL`, and WAL only after SQLite confirms that `PRAGMA journal_mode=WAL` returned `wal`.

`sync_operations` is an append-only journal. A client supplies operation ID, entity ID, device ID, device sequence, base revision, timestamp, schema version, kind, and an encrypted payload. The repository assigns entity revision and monotonic server sequence (the sync cursor). One transaction performs de-duplication, sequence validation, current-version read, conflict detection, journal write, entity/conflict write, and commit. Tombstones cannot be overwritten by a later non-delete operation.

Schema migration is transactional. A database declaring a newer schema version is rejected rather than opened or downgraded.

Unencrypted metadata is: opaque UUIDs, entity type, device ID, device sequence, operation kind, revisions, server sequence, timestamps, tombstone flag, schema version, and encrypted-blob lengths. Payload bytes are never plaintext in this repository.
