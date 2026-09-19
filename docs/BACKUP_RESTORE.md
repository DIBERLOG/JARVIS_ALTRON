# Backup and restore

Two things share this name in the project, and they must not be confused:

1. **What exists today**: a per-feature export and import, one encrypted file at
   a time (notes, vault, AI memory, the autocorrect word list, the Windows-actions
   audit log).
2. **What the stage describes**: one versioned, encrypted container that carries
   all of it plus a portable master-key envelope.

**The container is not implemented.** This page describes what works now, so a
person can back their data up today, and what the container has to be, so that
the next stage implements the described thing rather than an approximation of it.

## What works today

| Data | Export | Import | Protection |
|---|---|---|---|
| Notes | `notes_export_backup_file` (native save dialog) | `notes_import_backup_file` | the notes key, derived from the master key |
| Vault | `vault_export_backup_file` | `vault_import_backup_file` | the vault key |
| AI memory | `memory_export_backup` / `memory_import_backup` | same | the memory key |
| Autocorrect word list | `autocorrect_dictionary_export_backup` | `autocorrect_dictionary_import_backup` | the autocorrect key |
| Windows-actions audit log | `windows_actions_export_audit_log` | — (a log, not state) | plain JSONL, content-free by design |
| Portable master key | `export_backup` in `sync::crypto` (Argon2id + a wrapping key) | `import_backup` | **a password the user types**; the password is not stored |

Each export is written by a Rust command, never by the interface, and each file
is encrypted with the same construction the stores use (Argon2id key derivation,
XChaCha20-Poly1305, a versioned record format). The interface never receives a
key, and the password for the portable key envelope is not kept anywhere.

What this does **not** give you: a single file that restores everything at once,
or a way to restore onto a fresh machine without also remembering which feature
export you made when.

## What the container has to be

The design, recorded so it can be implemented as described:

**Format:** one file, encrypted as a whole with a key derived from a password the
user types (Argon2id, the same parameters as the portable key envelope), with a
versioned manifest inside.

**Manifest fields:**

```text
format_version
application_version
created_at
components            # which parts are inside, and their schema versions
file_hashes           # SHA-256 of every payload file
database_schema_versions
encryption_metadata   # algorithm, parameters, salt, nonce — never the password
```

**Inside:** notes, vault, AI memory, the autocorrect word list, the settings that
are not machine-specific, the allowed-applications list (as disabled entries or
as metadata only), the portable master-key envelope, and the manifest.

**Never inside:** `key.dpapi`, an unsealed master key, absolute local paths,
models, `llama-server.exe`, DLLs, temporary files, caches, the audit log by
default, screenshots, or recorded audio.

**How it must be built:**

* SQLite is *snapshotted*, not copied: `VACUUM INTO` or the backup API, so a
  write in progress cannot produce a torn file. Copying a live database with
  `std::fs::copy` is not acceptable and is the mistake the stage exists to
  prevent.
* Every payload file gets a SHA-256 in the manifest.
* The output is written to a temporary file and renamed into place, so a failure
  leaves no half-written backup.
* A cancelled or failed backup must not damage the original data.

**How restore must work, in order:**

1. choose the file;
2. check the structure and the format version, refusing an unknown newer version
   (no downgrade of a schema this build does not know);
3. verify every hash;
4. unlock the portable key envelope with the password;
5. show the components and their schema versions;
6. check that the schemas are compatible with this build;
7. make a safety backup of the current state;
8. stop the services that hold the stores;
9. restore into a temporary directory;
10. open and check each restored database;
11. replace atomically;
12. create a **new** local DPAPI blob for this machine;
13. restart the stores;
14. show a report of what was restored.

**On failure:** put the previous state back, leave no partially restored
database, keep the original backup file untouched, and never print a secret. A
path to an executable, a model, or a dictionary that came from another computer
is restored as *untrusted metadata*, never as a ready-to-use setting.

## What a person should do today

Until the container exists, a backup that is actually restorable needs three
things, in this order:

1. **The data:** export notes, vault, and memory through their own commands, into
   a folder that is not the data directory.
2. **The key:** export the portable key envelope and choose a password you will
   remember. **Without it, the exports are unreadable on a new machine.**
3. **The knowledge:** write down which export belongs to which feature and which
   password was used. This document is that note; keep a copy of it with the
   files.

An export of `key.dpapi` is useless on another machine (DPAPI is per user and per
machine) and is explicitly *not* a backup of the key.
