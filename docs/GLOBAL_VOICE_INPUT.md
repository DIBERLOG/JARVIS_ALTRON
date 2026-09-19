# Global voice input

Say the phrase, speak, and the text appears in the field the cursor was in. The
person stays in their own application: Notepad, a browser, an editor, a document.

The engine lives in `crates/jarvis-core/src/dictation/`. It reuses what already
exists and adds no second of anything:

| Piece | Where it comes from |
| --- | --- |
| The spoken trigger (Vosk) | the voice host, through [`VoiceHost`] |
| The dictation (Whisper) | the existing `WhisperSession`, through [`VoiceTranscriber`] |
| One microphone | `recorder::MicrophoneLease`, released by the host before the dictation asks for it |
| The spelling pass | the local autocorrect layer, through [`TextCorrector`] |
| The confirmation | the existing TTS, through the `speak` callback |
| Where the text may go | `windows_actions` for the window facts, and the gate in `target.rs` |

## The sequence

```text
Vosk intent ─► capture the focused element (never its text)
            ─► speak the confirmation
            ─► VoiceHost::release_microphone()      (Vosk stopped, device free)
            ─► VoiceTranscriber::transcribe()       (the existing Whisper session)
            ─► TextCorrector::correct()
            ─► voice punctuation
            ─► the gate, then the delivery
            ─► VoiceHost::restore_listener()        (on every path, including a failure)
```

The listener is restored by a guard, so a failed transcription, a refused target
and a cancellation all leave the assistant listening again. A second request
while one is running is refused with `busy` before anything happens.

`DictationStage` is what the tray shows: `idle`, `preparing`, `confirming`,
`handover`, `recording`, `transcribing`, `correcting`, `inserting`, `delivered`,
`failed`, `cancelled`. `DictationEngine::cancel()` is what Escape and the tray
item call; it is bounded, and a cancelled request delivers nothing.

## The phrases

| Said | Meaning |
| --- | --- |
| «Джарвис, голосовой ввод» | start |
| «Джарвис, начни голосовой ввод» | start |
| «Джарвис, включи диктовку» | start |
| «Джарвис, продиктую текст» | start |
| «Jarvis voice input» | start |
| «Стоп», «готово», «stop» | stop the recording and take the text |

The confirmation is «Да, сэр. Начинаю голосовой ввод» (and the English and
Ukrainian equivalents). Matching is normalization, not fuzzy: lower case, no
punctuation, collapsed whitespace, compared against the phrase list.

## Where the text may and may not go

The gate is `target::decide`. Every rule has a name, and the name is what the log
and the notification carry.

| Rule | Refused because |
| --- | --- |
| `password_field` | a dictated word must never land in a secret field |
| `own_window` | this application's own windows, including the vault and the notes |
| `secure_desktop` | the UAC prompt and the lock screen belong to Windows |
| `elevated_target` | a normal process may not drive an elevated one |
| `read_only_field` | the field exists to be read |
| `element_disabled` | the field cannot accept input now |
| `unknown_element` | a type this build cannot prove is a text field |

A refusal is a refusal: nothing is typed, nothing is copied, and the person is
told which rule refused it.

The one case that is **not** a refusal is `no_text_capability`: the element is a
text field the build can prove is safe, but it offers no way to be written into.
There the text goes to the clipboard, because refusing would leave the person
with nothing.

The window is checked **twice**: once when the request starts, and again
immediately before the text is delivered. If the foreground window changed in
between — which is what happens when someone alt-tabs during a dictation — the
text is not delivered anywhere, and the outcome is `window_changed`.

## Delivery

1. **UI Automation** (`DeliveryMethod::UiAutomation`) writes into the element.
   The text never leaves the application, and nothing is left behind.
2. **The clipboard** (`DeliveryMethod::Clipboard`) copies it and the person
   pastes. This is the fallback, and it is the path in use today: see
   "What is not wired yet" below.

There is **no keyboard synthesis** anywhere in this route. Typing a transcript by
synthesizing keystrokes means driving whatever has the focus, keystroke by
keystroke, with no way to prove afterwards where the text went, and it is the
same primitive that makes a keylogger indistinguishable from an assistant. If UI
Automation cannot do it, the clipboard does. Narrowing that prohibition is a
separate decision with its own threat model, and it has not been taken.

## Voice punctuation

Spoken marks become marks, locally, in three languages: «точка», «запятая»,
«двоеточие», «точка с запятой», «открой кавычки» / «закрой кавычки»,
«вопросительный знак», «восклицательный знак», «новая строка», «новый абзац»,
«открой скобку», «закрой скобку», «тире» (and the Ukrainian and English words).

* a mark is recognized as a whole word, so «точками» is untouched;
* a mark that follows a preposition — «попал в точку» — stays a word;
* a mark attaches to the word before it («привет,»), an opening bracket attaches
  to the word after it («список (один)»), and a sentence break capitalizes the
  next word;
* **limitation:** dictated *quotes* are not detected. «Он сказал: точка» cannot be
  told apart from a real full stop without understanding the sentence, and
  pretending otherwise would corrupt text. The setting that turns the pass off
  exists for exactly this reason.

The result is never sent to the local model on its own. An AI improvement is a
separate, explicit action with a preview, and it is not part of this route.

## Settings (`GlobalDictationSettings`)

| Field | Default | Meaning |
| --- | --- | --- |
| `enabled` | `false` | nothing listens for the phrase until it is switched on |
| `phrase` | «джарвис голосовой ввод» | the phrase the person uses |
| `speak_confirmation` | `true` | whether the confirmation is spoken |
| `language` | `auto` | the Whisper language hint |
| `autocorrect` | `true` | whether the local spelling pass runs |
| `preference` | `ui_automation` | `ui_automation` (with the clipboard as fallback) or `clipboard` |
| `clipboard_seconds` | 30 | how long a copied text stays before it is wiped |
| `preview_before_insert` | `false` | whether the text is shown before it is delivered |

A settings document that a person or a version got wrong is repaired by
`normalized()` rather than refused: an empty phrase becomes the default one, an
unknown language becomes `auto`, and the clipboard timeout is clamped into the
range the vault's clipboard guard already enforces.

## Command diagnostics and the phrase checker

"Ordinary commands are not accepted" had three causes, and all three are fixed
and covered by tests:

1. **A slot-bearing phrase could not match.** `какая погода в {city}` was compared
   literally, placeholder and all, so the score never reached
   `CMD_RATIO_THRESHOLD` (75) and the weather command was unreachable by voice.
   The placeholder now stands for any run of spoken words — one for `Москве`, two
   for `Нижнем Новгороде`, none when the value is left out — and the rest of the
   phrase is compared with the same score and the same threshold.
2. **An unclear safe action ended the search.** The Windows-action router ran
   before the command packs and answered for phrases it had not acted on:
   `открой браузер` was recognised as a launch, found no allowed application, and
   the configured `browser_open` command was never consulted. An `Ambiguous`
   answer now falls through to the packs, and its reason is reported only if the
   packs have no answer either. Nothing was executed on that path, so falling
   through cannot run the wrong thing.
3. **A confident AI intent that named nothing ended the search.** If
   `intent::classify` returned an identifier that is not in the loaded packs (a
   stale training cache, an edited pack), the fuzzy matcher was never asked. The
   phrase matcher is now the fallback for that case as well.

Every phrase passes seven stages, named by
`jarvis_core::commands::CommandStage` so the voice host, the window and a log
line use the same words: `listener_received_phrase`, `wake_word_detected`,
`normalized_length`, `command_match`, `rejection_code`, `execution_started`,
`execution_result`. Each stage is one log line and one `command_diagnostic` event
carrying the stage name, the **length** of the text the matcher saw, a reason
code, a command identifier and an outcome. The transcript itself is never part of
a stage, and the log no longer quotes a recognized phrase anywhere — not in the
voice host, not in the matcher.

Refusal codes: `too_short`, `empty_after_strip`, `no_match`, `no_commands`,
`awaiting_confirmation`, `forbidden`, and `action_*` for an undecided safe
action.

One normalizer serves both sides: `commands::normalize_phrase` (lower case, `ё`
folded to `е`, punctuation dropped, the wake word and its comma removed) is what
the listener matches with, and it is what the settings button
**«Проверить фразу без выполнения»** calls through `commands::check_phrase` —
which is `fetch_command`, not a second matcher. The phrase travels no further
than that call: it is not executed, not stored, not logged, and no shell or
keyboard path is reached. The answer names the command and its slot names, never
its path or its arguments. Normalization is used for matching only; the phrase
that goes to the executor is the one the person said, so a slot value keeps its
own letters.

## Privacy

Never logged, never stored, never put in a URL or in browser storage: the
recognized text, the content of the field, the document title, the clipboard, a
file name. What the log gets is the stage, the character count, the delivery
method, and a content-free code. The transcript of the last request is held in
memory for a preview and is cleared by the next request, by `forget`, and by the
exit.

`crates/jarvis-core/src/dictation/tests.rs` checks this structurally: the route's
own source is read as text and refused if it contains browser storage, a window
text API, a shell, keyboard synthesis, or a path to the vault, the notes or the
memory.

## What is not wired yet

The engine, the gate, the punctuation, the settings and the intents are complete
and tested (24 tests). Three pieces remain, and each is a seam that does not
change the route:

1. **The Windows UI Automation probe** (`ForegroundProbe`, `TextInserter`). The
   production probe currently reports `Unavailable("ui_automation")`, which is
   what makes the clipboard the working delivery path. The native reader needs
   `Win32_UI_Accessibility` in the `windows` crate and a COM wrapper that reads
   the focused element (control type, `IsPassword`, `IsReadOnly`, `IsEnabled`,
   value/text pattern support, process id, elevation) and writes with
   `IUIAutomationValuePattern::SetValue`. Everything on the other side of the
   trait is already in place and already tested against fakes.
2. **The Tauri commands, the tray states and the settings section.** The engine's
   `DictationStatusView` and `GlobalDictationSettings` are the shapes those three
   need.
3. **The Vosk trigger.** The voice host recognises the phrase and calls
   `release_microphone`, then the engine. `is_start_request` and
   `GlobalDictationSettings::matches` are the matcher it should use.

**Readiness (updated).** The feature is now reachable: the phrase the listener recognises, a button in the settings, and two tray rows all start the same route, and the result goes to the protected clipboard with the notice «Text ready. Press Ctrl+V». The production mode of the personal MVP is the **clipboard**: nothing is typed, there is no automatic Ctrl+V, and the UI Automation reader stays a documented experimental extension.

The feature is still required by check 7 of
`docs/WINDOWS_MVP.md` ("Checks on this machine") and is therefore **open** until
the three pieces above are wired and the manual scenarios below have been run. It
does not block the rest of personal use, and it is not part of the public-release
blockers — those are the four in `docs/RELEASE_CHECKLIST.md`, section 3.

## Manual checks on Windows

Run these once the three pieces above are wired. Each one names what must happen,
so a failure is unambiguous.

| Check | What must happen |
| --- | --- |
| Notepad | put the cursor in the text area, say «Джарвис, голосовой ввод», say a sentence, stop: the sentence appears in Notepad, punctuated |
| Browser field | the same in a search box and in a text area; the text appears where the cursor was |
| VS Code | the editor surface is a document element: either it is written into, or the text is copied and the notification says so |
| Word | the document surface behaves as VS Code, and the paragraph mark produces a paragraph |
| Password field | the text must **not** be inserted and must **not** be copied; the reason is `password_field` |
| A window that changes during the recording | alt-tab away before stopping: nothing is typed anywhere, and the reason is `window_changed` |
| A second dictation while one runs | refused with `busy`, and the first one is unaffected |
| UI Automation refuses at the last moment | the text is on the clipboard, the notification says "press Ctrl+V", and the clipboard is wiped after the configured timeout |
| The clipboard was changed by someone else | the wipe must not remove what the person put there |
| The microphone | after the text is delivered, the listener is listening again; the log shows the listener stopping before the dictation and starting after the text |
| Escape | cancels during the recording and during the transcription; nothing is delivered |
| The tray | shows recording, transcribing, inserted, copied, and the exact code for a refusal |

Log lines to look for: `dictation: delivered (stage=delivered method=… characters=… audio_ms=…)`,
`dictation: the listener has the microphone again`, and on a failure
`dictation: request failed (stage=… error_code=…)`.

## Limits

* the phrase match is exact after normalization. A mispronounced phrase does not
  start anything, which is the safe failure;
* a model that produces nothing yields `audio_empty`, and a model that fails
  yields `transcription_failed` with the Whisper code;
* the clipboard fallback is visible by nature: the text sits in the clipboard
  until it is replaced or wiped, and any process on the machine can read it while
  it is there. That is why it is the fallback and not the primary path;
* a dictated *quote* cannot be protected from voice punctuation, as described
  above;
* an element whose type cannot be read is refused rather than guessed at, so a
  custom control with no accessible text pattern goes to the clipboard.
