# Backup and restore

One encrypted file with everything this application stores on behalf of the
person using it, and one sequence that puts it back on this or another Windows
computer.

The panel is **Settings → Backup and restore**. Nothing leaves the machine: the
container is written where the person chooses in the native dialog, and the
restore reads the file they pick there.

## What is in a backup

| Component | Logical name | Where it lives |
| --- | --- | --- |
| Notes | `notes/sync.sqlite3` | data directory |
| Password vault | `vault/vault.sqlite3` | data directory |
| AI memory | `memory/ai-memory.sqlite3` | data directory |
| Spelling dictionary | `autocorrect/autocorrect.sqlite3` | data directory |
| Portable key envelope | `key/portable-envelope.json` | data directory |
| Whisper settings | `settings/whisper-settings.json` | data directory |
| Desktop settings | `settings/desktop.json` | data directory |
| First-run state | `settings/setup.json` | data directory |
| Windows actions | `settings/windows-actions.json` | data directory |
| Timers and reminders | `settings/windows-timers.json` | data directory |
| Application settings document | `settings/app.db` | configuration directory |

The list is a list, not a directory walk. A component that does not exist on this
machine is simply absent, and the four databases plus the key envelope must exist
for a container to be written at all.

### What is deliberately not in a backup

* **the DPAPI key blob** (`key.dpapi`). It is bound to one Windows account on one
  computer; carrying it would produce a file that cannot be opened anywhere else.
  It is recreated for the current account after a restore;
* **the master key in the clear** — it is never written anywhere, in the container
  or out of it;
* **GGML/GGUF models, `whisper-cli`, Vosk DLLs and other runtime files**;
* **logs** (`log.txt`, `actions-audit*.jsonl`), **caches** (the webview profile,
  `screenshot/`, `dictionaries.json` with its absolute paths);
* **temporary files** (the dictation WAV, the JSON report of a transcription);
* **results of dictation** — a transcript is shown and forgotten, never stored;
* **absolute user paths of any kind**: a container entry is a logical
  `component/file` name and nothing else;
* **clipboard content**;
* `target/`, `node_modules/`.

## Container format, version 1

```text
magic          8 bytes   "JARVISBK"
format         u16 LE    1
header_len     u32 LE    length of the JSON header
header         JSON      cleartext (see below)
entries        repeated, in manifest order:
  record_len   u32 LE    length of the record
  record       version byte, 24-byte nonce, AEAD ciphertext
```

The header:

```json
{
  "format": "jarvis-backup",
  "format_version": 1,
  "created_at": "2026-01-01T12:00:00+00:00",
  "app_version": "0.1.0",
  "cipher": "xchacha20poly1305",
  "kdf": "JARVIS/backup/v1",
  "envelope": "<hex of the portable key envelope JSON>",
  "manifest": {
    "entries": [
      { "name": "notes/sync.sqlite3", "kind": "sqlite", "bytes": 40960,
        "sha256": "<64 hex>", "schema_version": 2 }
    ],
    "total_bytes": 40960
  },
  "manifest_sha256": "<64 hex>"
}
```

* `manifest_sha256` is the SHA-256 of the canonical (serde) JSON of `manifest`;
* the payload is split into 1 MiB chunks. Each chunk is one XChaCha20-Poly1305
  record with its own random 24-byte nonce and its own tag, and its associated
  data binds `manifest_sha256`, the entry name, the chunk index, whether it is
  the last chunk, and the chunk's plaintext length. Entries cannot be reordered,
  renamed, truncated, or swapped;
* every entry also carries the SHA-256 of its whole plaintext in the manifest, so
  a payload that decrypts but does not match is refused.

### Bounds

At most 64 entries, at most 256 MiB per entry, at most 512 MiB in total, at most
64 KiB of header. A container that declares more is refused before anything is
allocated, read, or written. A declared chunk length must equal the length the
manifest implies, so a container cannot ask the reader for an unbounded amount of
memory.

### Entry names

Lowercase ASCII letters, digits, `.`, `_`, `-`, and one separator `/` between a
component and a file. No leading or trailing separator, no empty or `.`/`..`
component, no `\`, `:`, or NUL (so no NTFS alternate data stream), no reserved
Windows device name (`con`, `nul`, `lpt1`, …), no duplicate name. A name that
does not pass is refused as `unsafe_entry_name`, and the path it would produce is
checked again before it is used.

## The cryptography

Nothing new was invented. The container uses the primitives the application
already uses for its own storage:

* **Argon2id** (19 MiB, 2 iterations, 1 lane) derives a wrapping key from the
  master password, exactly as the portable key envelope already does;
* **XChaCha20-Poly1305** wraps the master key into that envelope, and encrypts
  the payload under a key derived from it;
* **HKDF-SHA256** derives the payload key from the master key with the purpose
  label `JARVIS/backup/v1` — a fifth `KeyPurpose`, separate from notes, vault, AI
  memory and autocorrect, so a feature key can never read a container and a
  container key can never read a feature record.

**How a backup is opened on another computer:** the container carries the
portable key envelope (an Argon2id + XChaCha20-Poly1305 wrapping of the master
key by the master password), and its payload is encrypted with a key derived from
that same master key. On the other machine the person types the **master
password** — the same one the vault uses — and nothing else. A DPAPI blob is not
in the container and is not needed: after a restore the application creates a new
blob for the account that performed the restore.

Keys and plaintext buffers are zeroized (the master key type is `Zeroize` +
`ZeroizeOnDrop`). No password, key, envelope, note name, vault record, or path is
ever logged, put in a DTO, or shown in the interface.

## Consistent snapshots

The four databases run in WAL mode, and their WAL is not assumed to be
checkpointed: at the moment a backup starts, committed data can live only in
`-wal`. Copying `db`, `db-wal` and `db-shm` by hand produces a mixture of
moments, and copying only `db` silently loses the WAL.

Every database is therefore snapshotted with **`VACUUM INTO`**, SQLite's own
documented backup path: one read transaction, committed WAL frames included, a
complete and defragmented copy out, the live file untouched. It runs on a
connection the backup opens for the snapshot alone, so no live store has to be
reached into, and a backup can be taken while the application is running. Every
snapshot then has to pass `PRAGMA integrity_check` before it is trusted.

**What "consistent" means here, and what it does not.** Each database is
consistent on its own, and no WAL content is lost. The four are separate files
and SQLite has no distributed transaction across them, so a write that lands
between two of them appears in the later one and not the earlier one. The window
therefore closes its own writing paths for the duration of an export and says so
in the interface; the guarantee is per database, not across all four.

All four are checkpointed with `PRAGMA wal_checkpoint(TRUNCATE)` as the
`checkpoint-databases` step of the single `LifecycleManager`, so a full exit
leaves complete databases behind.

## Export

1. the destination is chosen in the native save dialog, and the extension is
   `.jarvisbak`;
2. the master password is checked against the key envelope *before* anything is
   written, so a wrong password fails immediately and costs nothing;
3. each component is snapshotted into a staging directory inside the data
   directory;
4. the container is written to a temporary neighbour
   (`<destination>.part-<pid>`), flushed, and `sync_all`ed;
5. the temporary file is renamed over the destination — atomically — and the
   staging directory is removed;
6. on any failure the temporary file and the staging directory are removed, and
   an existing destination is refused unless the person switched on "allow
   replacing an existing file".

One export or restore runs at a time. A second call is refused with `busy`.

## Restore

1. **verify** — magic, version, bounds, names, manifest hash, envelope, every
   chunk's AEAD tag and every entry's SHA-256, all before anything on this machine
   is touched. A wrong password, one changed byte, or a changed manifest is
   refused here;
2. **stage** — the entries are written into a staging directory inside the data
   directory, every SQLite file is opened there and made to pass
   `PRAGMA integrity_check`, and a database whose schema is newer than this build
   understands is refused as `schema_too_new`;
3. **safety backup** — the current state is exported into a full container of its
   own, with the same password, under `backups/`. It is never deleted
   automatically;
4. the encrypted storage is closed (`notes.lock()`), which closes all four
   database connections;
5. **prepared** → the journal is written;
6. **old_moved** → the current files are moved into `restore-previous/`, not
   deleted;
7. **new_installed** → the staged files are moved into place, and two settings
   are forced off: dictation (`enabled`) and autostart (`autostart_enabled`);
8. **verified** → every installed database passes `integrity_check` again, and
   the local key is bound to this Windows account (a fresh DPAPI blob);
9. **committed** → the journal is removed. `restore-previous/` and the safety
   backup stay on disk;
10. the storage is opened again **locked**: the vault and the notes ask for the
    master password before anything is decrypted.

A restore is never partial: if the person chose a full restore and any step
fails, the rollback runs and the previous state is what remains.

## Rollback and crash recovery

The journal (`restore-journal.json` in the data directory) holds the stage, the
container's file *name*, the safety backup's file *name*, and one line per
component: logical name, kind, file name, and whether the file existed before.
It never holds a path, a key, or a password.

| Stage | What a crash there leaves |
| --- | --- |
| `prepared` | nothing was moved; the rollback removes nothing and restores nothing |
| `old_moved` | the old files are in `restore-previous/`; the rollback moves them back |
| `new_installed` | the new files are in place; the rollback removes what was not there before and moves the old files back |
| `verified` | as above; the DPAPI blob is the only thing bound to the new key, and it is created again after a later successful restore |
| `committed` | the restore finished; the journal is removed on sight |

On start-up, before any store is opened, the application reads the journal. A
journal that is not `committed` means the new state may be half in place, so the
answer is always the old state: the person still has the container and can try
again. A rollback that cannot finish is reported as `recovery_failed` and the
journal is kept, so the next start tries again; the application never leaves a
mixture of old and new databases without saying so.

`restore-previous/` and the safety backup are shown in the panel with their file
names and a delete button. They are the only way back, so nothing removes them
without the person asking.

## Diagnostics and privacy

The diagnostics report says only: whether the feature can run, the format
version, whether a key envelope exists, the last operation and its code, whether
a restore was interrupted, and whether the replaced state is still on disk.
Never a path, a password, a key, a note name, or a vault record name.

## Limits and honest caveats

* a backup is only as strong as the master password. There is no recovery: losing
  the password loses the backup;
* anyone who has the container **and** the password has the data. Anyone who can
  run code as this user while the storage is unlocked can read what the user can
  read, backup or no backup;
* the four databases are consistent individually but not as one distributed
  transaction. Export while nothing is writing for the strongest result;
* a container is not compressed. A backup is roughly as large as the data;
* the format is versioned. A container written by a newer format version, or
  holding a newer storage schema, is refused rather than half-read;
* the safety backup is a full container, so a restore needs free disk space for
  both the container and the state it replaces;
* this document does not claim that a backup cannot be broken. It claims that the
  container is authenticated, that a wrong password or a single changed byte is
  refused before anything is restored, and that a failure never leaves a mixture
  of old and new data.

## Manual check on Windows

1. unlock the storage (create or enter the master password) so a key envelope
   exists;
2. **Settings → Backup and restore**: the status line says a full backup can be
   made, and the list of components names features, not files;
3. type the master password, press **Create a full backup**, choose a location:
   the panel shows the file name, the component count and the size;
4. **Open a backup…** and pick the file: the panel lists what it holds. Press
   **Restore this backup** and confirm;
5. the panel reports the restore, that the storage is locked again, and shows the
   safety backup's name. Check in the log:
   `backup: restore finished (components=… safety=true rolled_back=None)`;
6. unlock the vault with the master password and check that a note and a vault
   record are the ones from the backup. Dictation must be **off** and autostart
   must be **off**, whatever the container said;
7. press **Delete the replaced state** and **Delete** next to the safety backup
   when they are no longer needed;
8. for the crash path: start a restore and kill the application between the
   stages (a debugger, or `taskkill /F`), then start it again. The log must say
   `an interrupted restore was rolled back (stage=…)` and the data must be the
   state from before the restore.
