# Local dictation (Whisper)

Dictation turns speech into text **on this machine**. The user supplies a
Whisper build and a model; the application runs it once for each recording,
reads the text it produces, and deletes the audio. Nothing is uploaded, no
account is involved, and no model is downloaded.

The interface for it is the "Диктовка" settings tab; the core is
`crates/jarvis-core/src/whisper/`.

## What the feature does

1. **Records only when asked.** `enabled` is off in the settings by default, and
   every recording starts from a button the user pressed. Nothing opens a
   microphone because the application was installed, launched, or autostarted.
2. **Writes one temporary file.** The recording is written to
   `dictation.wav` in the application data directory: 16 kHz, mono, 16-bit PCM,
   which is exactly what the recorder produces and what Whisper expects.
3. **Runs one bounded process.** The user's own `whisper-cli.exe` (or `main.exe`)
   is started with an argument list this application builds, one argument at a
   time — never through a shell. The process is stopped when it finishes, when
   the timeout expires, or when the user cancels.
4. **Reads the text.** The JSON report Whisper writes next to the audio is
   parsed; a build that prints the transcription instead is read from its
   standard output. Anything that is neither is reported as
   `invalid_response`, not guessed at.
5. **Deletes the audio.** Unless the user turned on "keep the recorded audio",
   the WAV and the report are removed as soon as the text exists.
6. **Shows the text and stops there.** The transcript is returned to the window
   that asked for it. It is not logged, not written to the AI memory, and not
   sent to the local model.

## What the feature does not do

* **It does not download anything.** There is no URL and no hash in this
  project: not for `whisper.cpp`, not for a model, not for a dictionary. A
  missing file is a `NotConfigured` state, and the wizard asks for it.
* **It does not convert audio.** `whisper_transcribe_file` accepts 16 kHz mono
  16-bit WAV and reports anything else. Decoding an arbitrary audio file is a
  parser this feature deliberately does not have.
* **It does not verify a model.** The container magic (`ggml`/`GGUF`) and the
  size the *file name* claims are checked, and the status says in plain words
  that the contents were not verified against a hash. A file renamed to
  `ggml-large-v3.bin` is reported as the name it carries, not as a fact.
* **It does not bundle or ship a binary or a model.** Both are the user's, and
  both are chosen through the native file dialog.
* **It is not a background listener.** There is no wake word and no always-on
  transcription; Vosk keeps its own, separate role.

## Settings

| Setting | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Whether dictation is allowed at all |
| `binary_path` | empty | The `whisper-cli.exe` the user picked |
| `model_path` | empty | The `ggml-*.bin` the user picked |
| `language` | `auto` | `auto`, `ru`, `en`, `ua`, `de`, `fr`, `es` |
| `translate` | `false` | Translate into English instead of transcribing |
| `threads` | `4` | Threads handed to the model (1–32) |
| `max_seconds` | `30` | Longest recording (1–300) |
| `silence_ms` | `1500` | Silence that ends a recording (500–10000) |
| `timeout_seconds` | `120` | How long the process may run (5–600) |
| `keep_audio` | `false` | Keep the WAV after a transcription |
| `allow_from_window` | `true` | Whether the window's own button may dictate |

The document is `whisper-settings.json` in the application data directory. It is
written to a temporary file and renamed, so an interrupted write cannot leave
half a file behind, and a damaged document falls back to the defaults (dictation
off, no paths) instead of refusing to load.

## The checks a file has to pass

`probe_binary`:

* it exists, is a regular file, and is named `.exe`;
* its PE header says x86-64. A 32-bit build is reported as
  `wrong_architecture` *before* a start is attempted, instead of failing as a
  mysterious process error. Only the first 4 KiB is read, so a 200 MB executable
  is not loaded to look at eight bytes.

`probe_model`:

* it exists, is a regular file, and is between 40 MB and 8 GB;
* it starts with a `ggml` or `GGUF` container;
* the size its name states is read from the name (`tiny`, `base`, `small`,
  `medium`, `large-v3`, and the English-only variants).

## Lifecycle and cancellation

* one transcription runs at a time; a second request is refused with `busy`;
* `whisper_cancel` sets a flag the record loop reads and the runner checks
  before it kills **its own child** — no `taskkill`, no process enumeration, so
  a Whisper the user started by hand is untouched;
* the session returns to `idle` on every path, including a failure and a
  cancellation;
* switching `enabled` off while something is running stops it;
* `WhisperHandle::shutdown` is safe to call twice and runs at application exit,
  so a closing window never leaves a microphone open.

## States the interface shows

Recording state: `idle`, `recording`, `transcribing`.

Readiness: `whisper-readiness-ready`, `whisper-readiness-disabled`,
`whisper-readiness-empty` (nothing chosen), `whisper-readiness-binary`,
`whisper-readiness-model`. Each one is a sentence, not a code.

Errors are content-free codes: `not_configured`, `binary_unavailable`,
`model_unavailable`, `wrong_architecture`, `audio_unavailable`, `audio_empty`,
`busy`, `process_unavailable`, `process_failed`, `timed_out`,
`invalid_response`, `cancelled`, `unsupported_language`, `storage`. A variant
never carries a transcript, an audio buffer, or a path, so an error can be shown
and logged safely (covered by a test).

## Honest limitations

* **No real model was run in this environment.** The whole session is tested
  against a fake runner: the arguments, the JSON and stdout parsing, the silence
  and length rules, the cancellation, the deletion of the audio, and the state
  machine. The round trip through a real `whisper.cpp` build is **unverified**,
  and the JSON shape the parser expects is the one the `whisper.cpp` builds this
  feature targets produce (`transcription[].timestamps.from/to`, `result.language`).
* **Recording was not exercised with a microphone here.** The frame source is
  real (`recorder::read_microphone`, the same PvRecorder path Vosk uses), but no
  hardware test was performed.
* **The model's contents are not verified.** A wrong or hostile model is a model
  the user chose; the name-based size check is a convenience, not a guarantee.
* **Dictation does not correct what it heard.** The text goes to the window as
  the model wrote it; the autocorrect feature is a separate step the user asks
  for.
* **A very long recording is capped at 300 seconds**, and anything beyond that is
  refused rather than split: chunking audio is a later decision, not a silent
  behaviour.
