# The first-run wizard

The wizard is ten short steps. Every one of them can be skipped, nothing is
configured behind the user's back, and it can be run again from the settings.

* Core: `SetupState`, `SETUP_STEPS`, and the atomic store, in
  `crates/jarvis-core/src/desktop.rs` (covered by the desktop tests).
* Interface: `frontend/src/components/desktop/FirstRunWizard.svelte`, shown by
  the shell whenever `needsWizard` is true, with
  `frontend/src/lib/desktop-model.ts` holding the order and the progress rules.

## Where it got to, and how that is stored

`setup.json` in the application data directory. It is **versioned**, because a
single boolean cannot be migrated:

```json
{
  "setup_version": 1,
  "completed_at": "2026-01-01T12:00:00Z",
  "completed_steps": ["language", "local_ai"],
  "skipped_steps": ["storage", "whisper"],
  "language": "ru"
}
```

* `setup_version` is compared with the application's own version of the wizard.
  Bumping it asks every existing profile to see the new step **once**, which is
  how a step is added without a manual migration;
* `completed_at` being absent means the wizard has not been finished, whatever
  the version says;
* a step is **completed** or **skipped**, never both. A step that is completed
  after being skipped stops being skipped: the later answer is the one that
  counts;
* **finishing marks every step the user never saw as skipped**, so the summary
  can say "not configured" instead of leaving it pending forever;
* a damaged document falls back to "the wizard has not run" — which shows the
  wizard again rather than breaking anything.

## The steps

| # | Step | What it does | If skipped |
|---|---|---|---|
| 1 | Language | RU / EN / UA, applied at once so the rest is readable | the system language stays |
| 2 | Encrypted storage | explains the choice and **opens the storage page**; the master password is typed there and never passes through the wizard | the storages stay uninitialised; nothing else is affected |
| 3 | Local model | points at the local AI settings for `llama-server.exe` and a GGUF file | the local model stays `NotConfigured` |
| 4 | Dictation | points at the Whisper settings for the executable and the model | dictation stays off |
| 5 | Microphone | shows the microphone state; **no recording is started** | the device list is still read by diagnostics |
| 6 | Voice listener | explains that Vosk needs its runtime and a model, and that it holds the microphone while listening | Vosk stays off |
| 7 | Dictionaries | explains that spelling dictionaries come from different projects with different licences | the spell checker runs without a dictionary |
| 8 | Allowed programs | points at the allowlist page, where a program is chosen in the native file dialog | nothing can be started |
| 9 | Autostart | explains the three switches and that all of them are off | autostart stays off |
| 10 | Summary | what is ready, what is missing, what was skipped; the diagnostics page is one click away | — |

Two rules the steps follow:

* **no step is a dead end.** A skipped feature is left switched off, and the
  feature it belongs to can be configured later from its own page — no step is
  required for the application to be usable;
* **no step can start a background process.** Nothing is downloaded, no model is
  fetched, no server is started, no recording is taken, and no microphone is
  opened. The steps point at the pages that own those decisions.

## Password handling

The wizard **never** handles a master password. The storage step explains the
choice and links to the page where the vault session derives its key. That keeps
one implementation of "create a master key" and one place where a password is
typed; the wizard's own state carries no secret, which a test checks by scanning
the component for the words that would mean otherwise.

## Re-running it

`setup_reset` clears `setup.json` and nothing else: no setting, no store, and no
key is touched. The wizard then starts at step 1 on the next render. This is the
supported way to walk through it again, and it is offered in the settings.

## What is not done

* A step that configures a feature in place would be nicer than one that points
  at the page that does. The current design is deliberate — one implementation
  per setting — but it does make steps 3, 4, 8, and 10 into signposts.
* The microphone step shows the state and the device count; the level test
  described in the stage (`a short level test`) is a button on the voice page,
  not inside the wizard.
* No screenshot or walkthrough exists; the text is the documentation.
