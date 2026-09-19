# ADR: local autocorrect and the encrypted user dictionary

## Status

Accepted and implemented in Rust, with the command surface registered. This decision covers
local spelling, the encrypted word list, the correction and undo path, and the explicit AI
text improvement. It adds no cloud service, no shipped or downloaded dictionary, no second
AI client, no network transport, and no second database engine. The Svelte interface for the
feature is not part of this decision and is not implemented yet.

## Context

The application has an editor (notes), a chat draft, and a local model, but nothing checked
the text a user actually types. Two shapes of help were plausible and they have very
different risk profiles: a *checker*, which is deterministic, local, and explainable, and a
*rewriter*, which is a model and can change meaning. Both were wanted, and neither was
allowed to touch the user's text without the user deciding.

The application already has the pieces this needs:

* one master-key session (`NotesVault`) that gates notes, the password vault, AI memory, and
  an idle lock, with purpose-derived keys (`KeyPurpose`, `PurposeKeyProvider`);
* one local-first storage stack — `SyncEngine` over `SyncRepository`, with per-entity
  revisions, cursors, base-revision conflicts, and tombstones
  (`docs/ADR_LOCAL_FIRST_SYNC.md`);
* one local model gateway (`docs/LOCAL_AI.md`) with one managed `llama-server`, streaming
  events, cancellation, and profile system prompts built in Rust;
* one secret filter (`crate::memory::redaction`) that reports by kind and never by value.

What was missing was a decision about three things: where the words come from, what a
correction is allowed to do on its own, and what may leave the process.

## Decision

Check locally, store the user's own words encrypted, apply nothing silently, and make an AI
rewrite an explicit, previewed action.

1. **`spellbook` as the engine, and no shipped dictionary.** The checker is
   [`spellbook`](https://crates.io/crates/spellbook) (`spellbook = "0.4"`, pinned to 0.4.2 in
   `Cargo.lock`): a pure-Rust, Hunspell-compatible rewrite of Nuspell, MPL-2.0, with no
   native library, no DLL to ship, and no build step beyond `cargo`. It reads the standard
   `.aff`/`.dic` pairs a user already has from LibreOffice or a distribution package, so the
   application ships no word list and downloads none. A missing pair is reported with the
   exact paths that were looked for (`DictionaryMissing`), a pair that exists but does not
   parse is `Invalid`, and an encoding that is neither UTF-8 nor ISO-8859-1 is refused rather
   than mis-decoded. An optional `dictionaries.json` manifest lets a user pin SHA-256
   checksums and the origin of the files they installed.
2. **One more purpose key, one more database, and the same sync contracts.**
   `KeyPurpose::Autocorrect` derives `JARVIS/autocorrect/v1` by HKDF from the same master
   key, and the word list lives in `autocorrect.sqlite3` next to the other stores, written
   and read only as `SyncEntityType::AutocorrectDictionary`. The store is built from a
   `PurposeKeyProvider`, so this layer never receives the master key, and a key of one
   purpose cannot open another purpose's ciphertext. Payloads are UTF-8 JSON inside the AEAD
   envelope with their own `schema_version` (currently `1`); an unknown version is refused
   rather than guessed. Nothing about the word list becomes memory, a note, or a password
   record, because the entity type is the only type this store reads or writes.
3. **Unicode scalar offsets in Rust, UTF-16 ranges for the interface, never byte offsets.**
   Every range the engine produces is in `char`s, which is what Rust can slice safely, and
   each reported issue carries the matching `Utf16Range` computed by the same pass, which is
   what JavaScript indices and `setSelectionRange` expect. An emoji (one `char`, two UTF-16
   units) or a combining mark therefore cannot shift a highlight, and a range that no longer
   fits the text is refused (`RangeMismatch`) instead of being clamped onto the wrong
   characters.
4. **Corrections are confirmed, versioned, and reversible.** A check reports; nothing is
   applied by the checker. `apply_corrections` takes the corrections the user accepted, the
   text, and the version the interface checked, refuses the whole request on a version
   mismatch (`StaleText`), skips and reports an individual correction whose text no longer
   matches the range it claims, and returns the new text plus a `CorrectionBatch`. The
   journal keeps the **last 20 applied batches in memory**, each with the whole text that
   preceded it, so an undo restores those exact bytes; it refuses when the text changed
   afterwards (`UndoConflict`) rather than throwing away the user's own edit.
5. **Safe auto-correction is a narrow, opt-in exception.** It is off by default
   (`safe_autocorrect: false`) and can apply only reasons with exactly one sensible repair:
   a double capital, a repeated space, a space before a delimiter, and a user rule the user
   themselves marked `auto_apply`. An unknown word is never auto-fixed, whatever the settings
   say, because a name or a term can be a valid word the dictionary does not know. Even then
   the backend only proposes: the text changes only through the confirmed apply path.
6. **The AI improvement is explicit and previewed, and it reuses the existing gateway.**
   `preview_improvement` sends the text through the same shared `LocalAiGateway` the chat
   uses, with `ThinkingMode::Disabled`, a streaming request, and a low temperature. The text
   is passed as delimited data (`===BEGIN TEXT===` … `===END TEXT===`) with an explicit rule
   that the block is content and not instructions; the answer is cleaned, bounded, compared
   word by word, and returned as a `TextImprovementPreview` whose `applied` field is always
   false. Only `apply_improvement`, after the user confirms, replaces the text, and it
   refuses a preview whose source text changed. The secret filter runs on the input **and**
   on the answer, and nothing is written to AI memory.
7. **The password vault is outside the feature.** The autocorrect module has no vault store,
   no vault handle, and no command that reads a vault record; no vault content is checked,
   corrected, or sent anywhere.

## Alternatives considered and rejected

* **`zspell` as the engine.** Rejected: when the decision was made, its crates.io licence
  metadata did not state a licence, which is a poor fit for a component of a shipped
  application, and its suggestion generation sat behind an unstable feature. `spellbook`
  offers the same Hunspell-compatible surface with a stated licence and usable suggestions.
  (That upstream metadata is not recorded anywhere in this repository; the licence facts
  above are what the decision rested on, not something a test here verifies.)
* **`hunspell-*` FFI bindings.** Rejected: they require the native Hunspell C/C++ library,
  which on Windows means building or shipping an extra DLL, dealing with a C toolchain, and
  carrying a native dependency into a Tauri bundle. The whole point of choosing a pure-Rust
  engine is that `cargo build` is the entire build.
* **Shipping a word list.** Rejected: Russian and English Hunspell lists are multi-megabyte
  files with their own upstream licences (Russian dictionaries are commonly LGPL- or
  BSD-style, SCOWL-derived `en_US` is permissive), and this project's own licence question is
  already recorded as unresolved in `docs/SECURITY.md`. Committing or downloading them would
  make the repository the distributor of someone else's data and add a large binary to every
  clone. The user installs the files, and the licence of the list stays with the list.
* **Reusing the AI-memory database for the word list.** Rejected: the memory database is
  owned by memory's settings, switch, and lifecycle, and its entity types are the memory
  types. Putting words there would either widen `is_ai_memory()` — making the memory store
  able to read dictionary records — or make memory's own list and delete operations touch
  something that is not memory. A separate file with a separate derived key and one entity
  type keeps each store unable to read the other's records.
* **Keeping the word list in a plain file** (or in a column of an unencrypted database).
  Rejected: a word list says a lot about the user — names, brands, employers, projects,
  medication, places — so it gets the same AEAD envelope, purpose key, lock behaviour, and
  conflict model as the rest of the encrypted data. A plain file would also have no revision,
  tombstone, or conflict semantics, so two devices could only ever clobber each other.
* **Byte offsets in the public range type.** Rejected: an interface slicing UTF-8 by bytes
  can split a character and panic or corrupt the text, and a JavaScript index is counted in
  UTF-16 units anyway, so a byte offset would have to be converted at the boundary — with an
  emoji in front of the word, a naive conversion highlights the wrong characters. Char
  offsets plus a computed UTF-16 range keeps the safe unit inside Rust and the correct unit
  at the boundary.
* **A persisted correction history.** Rejected: the journal stores the text that preceded
  each batch, which means it stores document text. Persisting it would put document content
  on disk in a store that is not the document, outside the "the text is where the user put
  it" rule, and it would need its own encryption, retention, and export story. Keeping it in
  memory and clearing it on lock and exit bounds the exposure to the unlocked session.
* **Correcting dictionary suggestions automatically.** Rejected outright: a wrong automatic
  replacement destroys user text, and the case where a dictionary is least reliable — an
  unknown word that is a real name — is exactly the case an automatic corrector would hit
  most often. Automatic action is limited to rules with one repair, and everything else is a
  question the user answers.
* **An automatic rewrite by the model.** Rejected outright, for the same reason plus a worse
  failure mode: a model can drop a detail, change a number, soften a commitment, or invent a
  sentence, and the user would have no way to see which. The preview requirement is what
  makes the AI path a decision instead of a surprise, and `normalized()` forces
  `require_preview` on so no settings file can turn it off.
* **A second AI client or a second model server for improvements.** Rejected: it would double
  the process, the configuration, the resource estimate, and the attack surface for one
  occasional task. The improvement path borrows the same `Arc<LocalAiGateway>` through
  `state.local_ai.shared()` and asks for `ThinkingMode::Disabled`, so it inherits the
  loopback-only rule, the argument-list launch, the bounded stop, and the content-free error
  surface that already exist.
* **A browser-side spell check** (`spellcheck` attributes, `localStorage`, or the webview's
  own dictionary). Rejected: it would be outside the master-key session and outside the lock,
  it cannot see the user's encrypted words, it cannot offer the same rules, and it cannot be
  audited or tested from this repository. The checker is Rust code with unit tests.

## Consequences

Positive:

* The word list is encrypted with a domain-separated key in its own file, with the existing
  revision, cursor, tombstone, and conflict rules. A wrong purpose key reports entries as
  unreadable instead of showing wrong words, and a foreign entity type is never read.
* Nothing is applied silently: a check reports, a correction is an explicit batch with a
  version, an AI change is a preview with a word-level difference, and every applied batch is
  undoable while the text matches.
* Everything can be explained to the user: which dictionaries are installed, which words
  were skipped and why, what changed, and what a warning means.
* Dictionaries are replaceable data rather than code: no native library, no build step, no
  download, and an optional manifest for anyone who wants checksums.
* The feature is separable. Switching `enabled` off drops the derived key and the decrypted
  words (`drop_autocorrect()`) while the shared unlock state stays, so notes, the vault, and
  memory remain usable; the ciphertext on disk is untouched.
* Failure is reported, never fatal: a missing, broken, or mis-encoded dictionary, an
  unreadable entry, and a stopped model server all produce a message and leave the rest of
  the application working.

Negative:

* **A dictionary is context-free.** Names, brands, and professional terminology are reported
  as errors until the user teaches them, a word used in the wrong sense is accepted, and a
  language the user did not install a dictionary for is not checked at all.
* **The user has to install the dictionaries**, and until then the feature reports
  `DictionaryMissing` and checks nothing. The licence of each word list stays the user's
  problem, which is deliberate but is a real setup cost.
* **The engine is upstream.** Suggestion quality, Hunspell affix coverage, and the crate's
  own maintenance are `spellbook`'s, and a change there is a dependency update; the licence is
  MPL-2.0, which is file-level copyleft and recorded here rather than audited.
* **Undo is short-lived and shallow.** Twenty batches, memory only, lost on lock or exit, and
  refused after the user edits the text. That is the intended trade for not persisting
  document text.
* **The AI can still change meaning.** The prompt, the warnings, the size check, and the
  preview reduce the risk; nothing verifies that the rewrite means the same thing.
* **A check can run without the word list.** With both dictionaries installed a check works
  while the encrypted storage is locked; it then consults the dictionaries alone, and the
  report says so (`user_dictionary_available: false`) rather than reporting the user's own
  words as misspelled.
* **The interface is narrow by design.** The panels list issues, apply one suggestion at a
  time, and show a whole-difference preview before an AI change; there are no inline
  squiggles in the textarea and no page-level keyboard shortcut for undo, because both
  would hide which text is being changed.

## References

* `docs/AUTOCORRECT.md` — the feature guide: pipeline, tokenizer, dictionaries, word list,
  undo, AI improvement, settings, limits, and limitations.
* `docs/THREAT_MODEL_AUTOCORRECT.md` — assets, adversaries, and residual risk.
* `docs/SECURITY.md` — the security baseline and the autocorrect section.
* `docs/ADR_AI_MEMORY.md` — the same purpose-key and separate-database decision for memory.
* `docs/ADR_LOCAL_FIRST_SYNC.md` — the revision, cursor, conflict, and tombstone model.
* `docs/LOCAL_AI.md` — the gateway the improvement path reuses.
