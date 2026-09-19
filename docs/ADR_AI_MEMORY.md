# ADR: encrypted local AI memory

## Status

Accepted and implemented. This decision covers the local memory stage only; it adds no sync
server, no network transport, no second storage engine, and no embedding model.

## Context

The local AI slice (`docs/LOCAL_AI.md`) generates answers but keeps no conversation: a chat
panel that forgets everything makes the assistant useful only inside one generation, and users
who want continuity are forced into cloud chat clients that retain their text off-device.

The application already has the pieces a memory layer needs:

* one local-first storage stack — `SyncEngine` over `SyncRepository`, with per-entity
  revisions, repository-assigned cursors, base-revision conflict detection, tombstones, and
  encrypted payloads (`docs/ADR_LOCAL_FIRST_SYNC.md`);
* a production AEAD cipher with purpose-derived keys (`KeyPurpose`, `PurposeKeyProvider`);
* one master-key session (`NotesVault`) that already gates notes and the password vault, with
  an idle lock and a DPAPI unlock path;
* one local model gateway with two profiles and a fixed profile system prompt.

What was missing was a decision about what memory is allowed to be. Three risks dominate:
storing text that must never be stored (credentials, reasoning, the system prompt), letting
stored text act as instructions, and storing more than the model's context window can hold.

## Decision

Add a fourth purpose key and a fourth database on the existing stack, and treat memory as
bounded, labelled data.

1. **One more purpose key, one more database.** `KeyPurpose::AiMemory` derives
   `JARVIS/ai-memory/v1` from the same master key by HKDF, and the store lives in
   `ai-memory.sqlite3` next to `sync.sqlite3` and `vault.sqlite3`. The store is built from a
   `PurposeKeyProvider`, so it never receives the master key, and a key of one purpose cannot
   open another purpose's ciphertext.
2. **The same sync contracts, reused unchanged.** Four new entity types
   (`SyncEntityType::AiMemoryConversation`, `AiMemoryMessage`, `AiMemorySummary`,
   `AiMemoryFact`) use the existing revision, cursor, conflict, and tombstone rules. Payloads
   are UTF-8 JSON inside the AEAD envelope, each with its own `schema_version` (currently `1`),
   and an unknown version is refused rather than guessed.
3. **A separate bounded context builder instead of sending the history.** `build_context`
   plans the window: system prompt, safety reserve, response reserve come off the top; facts
   get at most the memory cap and 20% of what remains, the summary 25%, and the recent messages
   the rest with a floor. Memory is emitted as a labelled **user-level data block**, never as a
   system message, and the system prompt is never emitted or trimmed by this code.
4. **Candidates instead of automatic extraction.** A model may propose durable facts only when
   `suggest_facts` is on, each anchored to a user message that was actually sent, and only the
   user's approval makes a candidate usable. Nothing a model writes becomes memory by itself.
5. **A transparent ranking instead of embeddings.** Facts are scored by scope, pinning, term
   overlap, category, recency, and usage. There is no vector index, so every ranking decision
   can be explained and tested — at the cost of a linear scan.
6. **The secret filter as a gate, not a promise.** `redaction::scan` blocks automatic paths,
   asks for confirmation on manual ones, and never logs or sends the matched value. It is
   documented as a heuristic with false positives and false negatives.

## Alternatives considered and rejected

* **A second database engine** (a dedicated embedded store for memory). Rejected: it would
  duplicate encryption, migrations, locking, and backup handling, and split the local-first
  contract in two. The existing stack already provides revisions, cursors, and conflicts.
* **Plaintext SQLite with a column-level cipher.** Rejected: SQL text columns, the write-ahead
  log, and journal tables would carry titles and message fragments outside the AEAD, so a
  stolen file or a leftover `-wal` would leak memory. The integration test scans the database,
  the `-wal`, the `-shm`, and every journal text column for plaintext markers.
* **Embeddings or a vector store.** Rejected for this stage: it adds a model, an index that
  must itself be encrypted and rebuilt, and an opaque ranking. The linear keyword score is
  weaker but explainable, and its cost is reported through the linear-search warning.
* **Automatic fact extraction on by default.** Rejected: the model would decide what to
  remember, and a wrong or invented sentence would become durable memory. Extraction is opt-in
  (`suggest_facts: false`), candidates are `Pending`, and each one must cite the user message
  it came from.
* **Sending the whole history.** Rejected: it grows without bound, it would silently exceed the
  context window, and it would leave the oldest part of a long conversation unprotected. The
  builder caps the recent window, summarizes the older part, and drops in a defined order.
* **Storing reasoning.** Rejected outright: only the visible question and the visible final
  answer are stored. Reasoning is not in the payload type, is never read from the gateway's
  thinking channel, and summarization requests `ThinkingMode::Disabled`.
* **Storing memory in the browser** (`localStorage`, IndexedDB, or the webview profile).
  Rejected: it would be unencrypted, outside the master-key session, and outside the lock,
  backup, and deletion rules the rest of the data follows.
* **A memory-specific master password.** Rejected: it would create a second secret to forget
  and a second key lifecycle. Memory shares the one unlock state, while keeping its own derived
  key and its own database.

## Consequences

Positive:

* Text is encrypted with a domain-separated key; a wrong purpose key fails to open it, and a
  foreign master key reports records as unreadable instead of failing the listing.
* Nothing about memory can replace the system prompt: `MessageRole` has no `System` variant,
  the builder emits only user and assistant messages, and stored text is defanged inside
  labelled blocks.
* Memory is deletion-aware and portable: soft delete, purge, clear-one-conversation,
  clear-all-history (facts kept), and a ciphertext export that needs the master key.
* Losing the key loses the memory, but the rest of the application keeps working, because the
  memory database is separate and its absence is reported rather than fatal.
* The feature can be switched off completely: `enabled: false` refuses writes, stops using
  facts, and drops the memory key and its cache at once (`VaultSession::drop_memory()`), while
  the notes and the password vault stay unlocked because the unlock state is shared.

Negative:

* **Search is a linear scan.** The store decrypts the whole set into memory to search it, so a
  large memory costs time and RAM on the first read. Only a warning above 500 facts or 5000
  messages is offered; there is no index.
* **The secret filter is a heuristic.** It both over-reports (a hash or a long identifier) and
  under-reports (a credential in an unusual shape), and it asks the user to decide on the
  manual path.
* **The token estimate is a heuristic.** Three ASCII characters per token and one token per
  non-ASCII character is a budget model, not a tokenizer, so real usage can differ.
* **The summary is lossy.** The oldest part of a long conversation exists only as a model
  summary, and a summary that no longer matches a covered message is marked stale and then
  left out of the context.
* **One summarization job at a time per conversation, and none without a model.** Without a
  running gateway there are no summaries, and the messages simply stay verbatim.
* Memory is scoped per profile, but the underlying key is one key: separate scopes are an
  application rule, not a cryptographic separation between JARVIS and ALTRON.

## References

* `docs/AI_MEMORY.md` — what is stored, the context builder, the secret filter, deletion.
* `docs/LOCAL_AI.md` — the gateway, profiles, and how the chat panel uses memory.
* `docs/SECURITY.md` — the security baseline and the AI memory boundary rules.
* `docs/THREAT_MODEL_AI_MEMORY.md` — threats, enforced mechanisms, and residual risk.
* `docs/ADR_LOCAL_FIRST_SYNC.md` — the revision, cursor, conflict, and tombstone model reused
  here.
