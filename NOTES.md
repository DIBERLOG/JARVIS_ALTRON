# Encrypted notes on Windows

Status: implemented for the Windows desktop build. The password vault, AI chat,
AI memory, dictation, and device-to-device sync are **not** part of this stage.

## What is stored

Each note is one synchronization entity of type `note`; each folder is one entity
of type `note_folder`. The entity payload is a JSON document that is encrypted
before it reaches SQLite:

```json
{
  "schema_version": 1,
  "title": "...",
  "body": "...",
  "folder_id": "uuid | null",
  "tags": ["..."],
  "pinned": false,
  "created_at": "RFC 3339",
  "updated_at": "RFC 3339",
  "deleted_at": "RFC 3339 | null"
}
```

Title, body, folder name, and tags are therefore **ciphertext at rest**. The
plaintext that remains visible is only the technical metadata defined in
`ADR_SQLITE_SYNC_STORAGE.md`: entity ID, entity type, revision, journal cursor,
device ID, timestamps, tombstone flag, payload schema version, and ciphertext
length.

Consequences that are accepted on purpose:

* The number of notes, their size, and their change frequency are visible to
  anyone who can read the database file.
* There is no SQL search over note text, because that would require plaintext
  columns or an unencrypted index. Search runs in memory (below).

## Lifecycle of a note

| Action | Storage effect |
| --- | --- |
| Create | new `note` entity, revision 1 |
| Edit / autosave | new revision (N+1) of the same entity |
| Pin / unpin | new revision |
| Move to trash | new revision with `deleted_at` set; content is kept |
| Restore | new revision with `deleted_at` cleared |
| Delete forever | tombstone; the encrypted payload is dropped from the entity |

Deleting a folder never deletes notes: the notes that referenced it are rewritten
with `folder_id = null`.

## Search, sorting, and memory limits

The first read after unlocking walks the applied journal, decrypts every entity,
and keeps the note set in memory. Every write invalidates that cache; the next
read reloads it.

* Search is a case-insensitive substring match over title, body, and tags, run in
  memory on the decrypted set. There is no index and no plaintext copy on disk.
* Cost is linear in the total decrypted text per query, and memory use is
  proportional to the size of all notes at once.
* This is deliberately the first implementation. It is comfortable for thousands
  of ordinary notes; a large corpus (hundreds of MB of text) will need an
  encrypted index or paging, which is a later stage.
* A note whose payload cannot be decrypted is never silently dropped: list
  results carry an `unreadable` count that the interface shows.

## Key material

```text
sync.sqlite3      encrypted payloads, revisions, cursors
key.backup.json   portable envelope: master key wrapped with Argon2id(password)
key.dpapi         DPAPI blob of the master key for the current Windows user
device.id         stable device identifier (not a secret)
```

All of it lives in the per-user application data directory
(`SqliteSyncRepository::production_path()`), never inside the checkout.

* First start: the interface asks for a master password, generates a random
  master key, writes the portable envelope, and additionally writes the DPAPI
  blob when the platform supports it.
* Later starts: unlock with Windows (DPAPI) or with the master password.
* The master password is never stored; it is verified by unwrapping the portable
  envelope.
* If the key files are gone but the database is not, the storage reports
  `key_missing` and refuses to create a new key, because that would orphan the
  existing ciphertext. Only importing a portable backup recovers it.
* "Save a backup copy" re-wraps the in-memory master key with a password of the
  user's choice and writes a new envelope to a chosen file.

## Conflict handling

The synchronization layer records a conflict when an incoming version was written
against a base revision that is no longer current. Nothing is discarded
implicitly, and the interface offers three explicit choices:

| Choice | Effect |
| --- | --- |
| Keep current | the pending conflict is dismissed; the journal still holds the incoming mutation |
| Accept incoming | the stored entity gets a **new** revision containing the incoming content |
| Keep both | the incoming content becomes a second note with a new ID; the original is untouched |

`discard_conflict` only removes the pending conflict row; the journal entry and
its payload remain, so both versions stay auditable.

## Interface

The notes page follows the existing application style and is designed for the
550 px window:

* storage gate: create master password, unlock with Windows, unlock with the
  master password, import a portable backup file, paste an envelope;
* note list with search, sort, pin indicator, tags, and change time;
* editor with title, body, tags, folder, pin switch, save indicator, and
  revision display;
* folder rail with create, rename, and delete;
* tag rail built from the current notes;
* trash view with restore and permanent deletion behind an inline confirmation;
* autosave with a trailing debounce of 700 ms, flushed when changing notes,
  blurring a field, locking, or leaving the page;
* a conflict panel with the three resolutions.

All note commands run on a worker thread, so encryption, SQLite writes, and
Argon2id derivation never block the window.

## What the notes layer never does

* It never writes a note title, body, tag, or folder name as plaintext.
* It never logs note content; errors are content-free messages.
* It never stores the master password or an unencrypted copy of the master key.
* It never writes note content to a temporary file. The only temporary file is
  the atomic-rename staging file for key material, which holds ciphertext.
* It never sends notes anywhere. There is no network path in this stage, and the
  notes layer is not connected to the AI layer.

## Tests

* `crates/jarvis-core/src/notes/tests.rs` – model and store unit tests: CRUD,
  trash/restore/purge, folders, tags, pinning, sorting, search, wrong key,
  restart, conflict detection, and all three resolutions.
* `crates/jarvis-core/tests/notes_storage.rs` – vault integration tests:
  initialize, unlock (DPAPI and password), lock, restart, plaintext scan of the
  database and WAL, portable backup recovery in a new directory, damaged DPAPI
  blob, and re-import on a machine with a different local key.
* `frontend/tests/notes-model.test.mjs` – interface logic: autosave debounce and
  flush, save-indicator state machine, relative time buckets, tag parsing, dirty
  detection, option keys, folder options, and path shortening. Run with
  `npm run test:ui`.
* `npm run build` type-checks every component with `svelte-check`.
