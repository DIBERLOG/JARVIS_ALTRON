//! The route, with fakes for everything the platform provides.
//!
//! Every test here is deterministic: no desktop, no microphone, no model. What it
//! asserts is the order of the calls, the rule that refused an insertion, and the
//! fact that the text never reaches a log, a file, or the window's storage.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::error::DictationError;
use super::insertion::{ClipboardWriter, DeliveryMethod, ForegroundProbe, TextInserter};
use super::punctuation::apply_voice_punctuation;
use super::session::{
    DictationEngine, DictationRequest, DictationStage, EngineDeps, TextCorrector, TranscribedText,
    VoiceHost, VoiceTranscriber,
};
use super::target::{
    decide, ElementKind, InsertionMethod, TargetSnapshot, VoiceInputPreference, WindowIdentity,
};

/// A marker the tests use to prove the text never leaks out of them.
const SECRET: &str = "FICTIONAL_SECRET_DICTATION";

/// Every call the route made, in order. This is what "the listener gives the
/// microphone up before the dictation" is asserted against.
#[derive(Clone, Default)]
struct Trace(Arc<Mutex<Vec<String>>>);

impl Trace {
    fn push(&self, call: impl Into<String>) {
        self.0.lock().unwrap().push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// The voice host: the listener that holds the microphone.
struct FakeHost {
    trace: Trace,
    free: Arc<AtomicUsize>,
    restore_fails: bool,
}

impl VoiceHost for FakeHost {
    fn release_microphone(&self) -> Result<(), DictationError> {
        self.trace.push("vosk.stop");
        self.free.store(1, Ordering::SeqCst);
        Ok(())
    }

    fn restore_listener(&self) -> Result<(), DictationError> {
        self.trace.push("vosk.start");
        if self.restore_fails {
            return Err(DictationError::VoiceHostUnavailable);
        }
        self.free.store(0, Ordering::SeqCst);
        Ok(())
    }
}

/// The dictation: one recording, one text, and the observation that the
/// microphone was free when it started.
struct FakeTranscriber {
    trace: Trace,
    host_free: Arc<AtomicUsize>,
    text: String,
    audio_ms: u64,
    fail: Option<DictationError>,
    started_while_busy: Arc<AtomicUsize>,
}

impl VoiceTranscriber for FakeTranscriber {
    fn transcribe(&self) -> Result<TranscribedText, DictationError> {
        self.trace.push("whisper.start");
        if self.host_free.load(Ordering::SeqCst) == 0 {
            self.started_while_busy.fetch_add(1, Ordering::SeqCst);
        }
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        Ok(TranscribedText {
            text: self.text.clone(),
            audio_ms: self.audio_ms,
        })
    }
}

/// The spelling layer, recorded so a test can see it ran.
struct FakeCorrector {
    trace: Trace,
}

impl TextCorrector for FakeCorrector {
    fn correct(&self, text: &str) -> String {
        self.trace.push("autocorrect");
        text.to_string()
    }
}

/// The focused element, and what the foreground window is when the text arrives.
struct FakeProbe {
    target: TargetSnapshot,
    /// The identity reported at delivery time; `None` means "no foreground".
    current: Option<WindowIdentity>,
    available: bool,
}

impl ForegroundProbe for FakeProbe {
    fn focused_target(&self) -> Result<TargetSnapshot, DictationError> {
        if self.available {
            Ok(self.target.clone())
        } else {
            Err(DictationError::Unavailable("ui_automation"))
        }
    }

    fn foreground_identity(&self) -> Option<WindowIdentity> {
        self.current
    }

    fn is_available(&self) -> bool {
        self.available
    }
}

/// UI Automation: writes into the field, or refuses.
struct FakeInserter {
    trace: Trace,
    fail: bool,
    written: Arc<Mutex<Option<String>>>,
}

impl TextInserter for FakeInserter {
    fn insert(&self, text: &str) -> Result<(), DictationError> {
        self.trace.push("uia.insert");
        if self.fail {
            return Err(DictationError::Unavailable("ui_automation"));
        }
        *self.written.lock().unwrap() = Some(text.to_string());
        Ok(())
    }
}

/// The clipboard, where a person pastes from.
#[derive(Default)]
struct FakeClipboard {
    trace: Trace,
    written: Arc<Mutex<Option<String>>>,
}

impl ClipboardWriter for FakeClipboard {
    fn write(&self, text: &str) -> Result<(), DictationError> {
        self.trace.push("clipboard.write");
        *self.written.lock().unwrap() = Some(text.to_string());
        Ok(())
    }
}

struct Harness {
    trace: Trace,
    started_while_busy: Arc<AtomicUsize>,
    host: FakeHost,
    transcriber: FakeTranscriber,
    corrector: FakeCorrector,
    probe: FakeProbe,
    inserter: FakeInserter,
    clipboard: FakeClipboard,
    engine: Arc<DictationEngine>,
    spoken: Mutex<Vec<String>>,
}

impl Harness {
    fn new(text: &str) -> Self {
        let trace = Trace::default();
        let free = Arc::new(AtomicUsize::new(1));
        let started_while_busy = Arc::new(AtomicUsize::new(0));
        let target = TargetSnapshot::edit(1001, 4242);
        let current = WindowIdentity {
            window_id: target.window_id,
            process_id: target.process_id,
        };
        Self {
            trace: trace.clone(),
            started_while_busy: Arc::clone(&started_while_busy),
            host: FakeHost {
                trace: trace.clone(),
                free: Arc::clone(&free),
                restore_fails: false,
            },
            transcriber: FakeTranscriber {
                trace: trace.clone(),
                host_free: Arc::clone(&free),
                text: text.to_string(),
                audio_ms: 1_500,
                fail: None,
                started_while_busy: Arc::clone(&started_while_busy),
            },
            corrector: FakeCorrector {
                trace: trace.clone(),
            },
            probe: FakeProbe {
                target,
                current: Some(current),
                available: true,
            },
            inserter: FakeInserter {
                trace: trace.clone(),
                fail: false,
                written: Arc::new(Mutex::new(None)),
            },
            clipboard: FakeClipboard {
                trace: trace.clone(),
                written: Arc::new(Mutex::new(None)),
            },
            engine: Arc::new(DictationEngine::new()),
            spoken: Mutex::new(Vec::new()),
        }
    }

    fn run(
        &self,
        autocorrect: bool,
        punctuation: bool,
    ) -> Result<super::session::DictationOutcome, DictationError> {
        let speak = |phrase: &str| self.spoken.lock().unwrap().push(phrase.to_string());
        self.engine.run(
            DictationRequest {
                autocorrect,
                punctuation,
                preference: VoiceInputPreference::UiAutomation,
                speak_confirmation: true,
            },
            EngineDeps {
                host: &self.host,
                transcriber: &self.transcriber,
                corrector: &self.corrector,
                probe: &self.probe,
                inserter: &self.inserter,
                clipboard: &self.clipboard,
                speak: &speak,
            },
        )
    }

    fn written(&self) -> Option<String> {
        self.inserter.written.lock().unwrap().clone()
    }

    fn copied(&self) -> Option<String> {
        self.clipboard.written.lock().unwrap().clone()
    }
}

// -------------------------------------------------------------- the sequence

#[test]
fn a_voice_request_runs_one_dictation_and_writes_into_the_field() {
    let harness = Harness::new("привет мир");
    let outcome = harness.run(true, true).expect("the request");
    assert_eq!(outcome.stage, DictationStage::Delivered);
    assert_eq!(outcome.characters, 10);
    assert_eq!(outcome.method, Some(DeliveryMethod::UiAutomation));
    assert_eq!(outcome.audio_ms, 1_500);
    assert_eq!(harness.written().as_deref(), Some("Привет мир"));
    // The confirmation was spoken.
    assert_eq!(harness.spoken.lock().unwrap().len(), 1);
    assert!(harness.spoken.lock().unwrap()[0].contains("голосовой ввод"));
    // The route, in order.
    assert_eq!(
        harness.trace.calls(),
        vec![
            "vosk.stop",
            "whisper.start",
            "autocorrect",
            "uia.insert",
            "vosk.start"
        ]
    );
}

#[test]
fn the_listener_gives_the_microphone_up_before_the_dictation_starts() {
    let harness = Harness::new("привет");
    harness.run(false, false).expect("the request");
    let calls = harness.trace.calls();
    let stop = calls.iter().position(|call| call == "vosk.stop");
    let start = calls.iter().position(|call| call == "whisper.start");
    assert!(stop.is_some() && start.is_some());
    assert!(stop < start, "the microphone must be free first: {calls:?}");
    assert_eq!(
        harness.started_while_busy.load(Ordering::SeqCst),
        0,
        "the dictation never started while the listener held the device"
    );
}

#[test]
fn the_listener_comes_back_after_the_text_and_after_every_failure() {
    let harness = Harness::new("привет");
    harness.run(true, true).expect("the request");
    assert_eq!(harness.trace.calls().last().unwrap(), "vosk.start");

    let mut harness = Harness::new("привет");
    harness.transcriber.fail = Some(DictationError::TranscriptionFailed("process_failed"));
    let error = harness.run(false, false).unwrap_err();
    assert_eq!(error.code(), "transcription_failed");
    assert_eq!(harness.trace.calls().last().unwrap(), "vosk.start");
    assert_eq!(
        harness.written(),
        None,
        "nothing is written after a failure"
    );

    let mut harness = Harness::new("привет");
    harness.probe.target.password = true;
    let error = harness.run(false, false).unwrap_err();
    assert_eq!(error.code(), "target_refused");
    assert_eq!(harness.written(), None);
    assert_eq!(harness.trace.calls().last().unwrap(), "vosk.start");

    // A listener that will not come back is reported, and the failure the request
    // hit is still the one the person sees.
    let mut harness = Harness::new("привет");
    harness.host.restore_fails = true;
    harness.transcriber.fail = Some(DictationError::TranscriptionFailed("process_failed"));
    assert_eq!(
        harness.run(false, false).unwrap_err().code(),
        "transcription_failed"
    );
}

#[test]
fn a_second_request_while_one_is_running_is_refused() {
    // An overlapping request, made from inside the transcription: this is what a
    // second trigger, a tray click, or a window button would do.
    struct Overlap {
        engine: Arc<DictationEngine>,
        observed: Arc<Mutex<Option<&'static str>>>,
    }
    impl VoiceTranscriber for Overlap {
        fn transcribe(&self) -> Result<TranscribedText, DictationError> {
            let second = self.engine.run(
                DictationRequest {
                    autocorrect: false,
                    punctuation: false,
                    preference: VoiceInputPreference::Clipboard,
                    speak_confirmation: false,
                },
                EngineDeps {
                    host: &Passive,
                    transcriber: &Nothing,
                    corrector: &Nothing,
                    probe: &NoFocus,
                    inserter: &Nothing,
                    clipboard: &Nothing,
                    speak: &|_: &str| {},
                },
            );
            *self.observed.lock().unwrap() = Some(match second {
                Err(error) => error.code(),
                Ok(_) => "none",
            });
            Err(DictationError::Cancelled)
        }
    }
    struct Passive;
    impl VoiceHost for Passive {
        fn release_microphone(&self) -> Result<(), DictationError> {
            Ok(())
        }
        fn restore_listener(&self) -> Result<(), DictationError> {
            Ok(())
        }
    }
    struct Nothing;
    impl VoiceTranscriber for Nothing {
        fn transcribe(&self) -> Result<TranscribedText, DictationError> {
            Ok(TranscribedText {
                text: "нет".to_string(),
                audio_ms: 0,
            })
        }
    }
    impl TextCorrector for Nothing {
        fn correct(&self, text: &str) -> String {
            text.to_string()
        }
    }
    impl TextInserter for Nothing {
        fn insert(&self, _: &str) -> Result<(), DictationError> {
            Ok(())
        }
    }
    impl ClipboardWriter for Nothing {
        fn write(&self, _: &str) -> Result<(), DictationError> {
            Ok(())
        }
    }
    struct NoFocus;
    impl ForegroundProbe for NoFocus {
        fn focused_target(&self) -> Result<TargetSnapshot, DictationError> {
            Err(DictationError::Unavailable("ui_automation"))
        }
        fn foreground_identity(&self) -> Option<WindowIdentity> {
            None
        }
        fn is_available(&self) -> bool {
            false
        }
    }

    /// A focus that can be read, for the tests that need to get past the gate.
    struct SimpleFocus;
    impl ForegroundProbe for SimpleFocus {
        fn focused_target(&self) -> Result<TargetSnapshot, DictationError> {
            Ok(TargetSnapshot::edit(5, 6))
        }
        fn foreground_identity(&self) -> Option<WindowIdentity> {
            Some(WindowIdentity {
                window_id: 5,
                process_id: 6,
            })
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    let engine = Arc::new(DictationEngine::new());
    let observed = Arc::new(Mutex::new(None));
    let speak = |_: &str| {};
    let outcome = engine.run(
        DictationRequest {
            autocorrect: false,
            punctuation: false,
            preference: VoiceInputPreference::Clipboard,
            speak_confirmation: false,
        },
        EngineDeps {
            host: &Passive,
            transcriber: &Overlap {
                engine: Arc::clone(&engine),
                observed: Arc::clone(&observed),
            },
            corrector: &Nothing,
            probe: &SimpleFocus,
            inserter: &Nothing,
            clipboard: &Nothing,
            speak: &speak,
        },
    );
    assert_eq!(*observed.lock().unwrap(), Some("busy"));
    assert_eq!(outcome.unwrap_err(), DictationError::Cancelled);
    // The engine is idle again, and a cancel of an idle engine does nothing.
    assert_eq!(engine.stage(), DictationStage::Idle);
    assert!(!engine.cancel());
}

// ------------------------------------------------------------------ the gate

#[test]
fn a_password_field_is_refused() {
    let mut target = TargetSnapshot::edit(1, 1);
    target.password = true;
    let verdict = decide(&target, VoiceInputPreference::UiAutomation);
    assert!(!verdict.is_allowed());
    assert_eq!(verdict.rule, Some("password_field"));

    let mut target = TargetSnapshot::edit(1, 1);
    target.element_kind = ElementKind::Password;
    assert_eq!(
        decide(&target, VoiceInputPreference::UiAutomation).rule,
        Some("password_field")
    );

    let mut harness = Harness::new("привет");
    harness.probe.target.password = true;
    let error = harness.run(false, false).unwrap_err();
    assert_eq!(error, DictationError::TargetRefused("password_field"));
    assert_eq!(error.code(), "target_refused");
    assert_eq!(harness.written(), None);
    assert_eq!(harness.copied(), None);
}

#[test]
fn elevated_unknown_read_only_disabled_secure_and_own_windows_are_refused() {
    for (rule, mutate) in [
        ("elevated_target", 0),
        ("unknown_element", 1),
        ("read_only_field", 2),
        ("element_disabled", 3),
        ("secure_desktop", 4),
        ("own_window", 5),
    ] {
        let mut target = TargetSnapshot::edit(1, 1);
        match mutate {
            0 => target.elevated_target = true,
            1 => target.element_kind = ElementKind::Unknown,
            2 => target.read_only = true,
            3 => target.enabled = false,
            4 => target.secure_desktop = true,
            _ => target.own_window = true,
        }
        let verdict = decide(&target, VoiceInputPreference::UiAutomation);
        assert!(!verdict.is_allowed(), "{rule} must refuse");
        assert_eq!(verdict.rule, Some(rule));
    }
}

#[test]
fn an_element_without_a_writable_pattern_goes_to_the_clipboard_instead_of_refusing() {
    let mut target = TargetSnapshot::edit(1, 1);
    target.supports_value_pattern = false;
    let verdict = decide(&target, VoiceInputPreference::UiAutomation);
    assert!(verdict.is_allowed(), "the fallback is not a refusal");
    assert_eq!(verdict.method(), Some(InsertionMethod::Clipboard));
    assert_eq!(verdict.rule, Some("no_text_capability"));

    let mut target = TargetSnapshot::edit(1, 1);
    target.element_kind = ElementKind::Document;
    target.supports_value_pattern = false;
    target.supports_text_pattern = true;
    assert_eq!(
        decide(&target, VoiceInputPreference::UiAutomation).method(),
        Some(InsertionMethod::UiAutomation)
    );

    // A control type this build knows and does not write into.
    let mut target = TargetSnapshot::edit(1, 1);
    target.element_kind = ElementKind::Other;
    assert_eq!(
        decide(&target, VoiceInputPreference::UiAutomation).method(),
        Some(InsertionMethod::Clipboard)
    );
}

#[test]
fn a_changed_window_means_nothing_is_typed() {
    let mut harness = Harness::new("привет");
    harness.probe.current = Some(WindowIdentity {
        window_id: 9999,
        process_id: 777,
    });
    let error = harness.run(false, false).unwrap_err();
    assert_eq!(error, DictationError::WindowChanged);
    assert_eq!(harness.written(), None);
    assert_eq!(harness.copied(), None);
    assert_eq!(harness.trace.calls().last().unwrap(), "vosk.start");

    let mut harness = Harness::new("привет");
    harness.probe.current = None;
    assert_eq!(
        harness.run(false, false).unwrap_err(),
        DictationError::WindowChanged
    );
    assert_eq!(harness.written(), None);
}

#[test]
fn ui_automation_unavailable_falls_back_to_the_clipboard() {
    // The probe cannot inspect the focus: the gate refuses, nothing is typed, and
    // the listener is not even disturbed — there is no reason to stop it before
    // the request can be served.
    let mut harness = Harness::new("привет");
    harness.probe.available = false;
    let error = harness.run(false, false).unwrap_err();
    assert_eq!(error.code(), "unavailable");
    assert_eq!(harness.written(), None);
    assert!(
        harness.trace.calls().is_empty(),
        "an unreadable focus is refused before the microphone changes hands"
    );
    assert_eq!(harness.copied(), None);

    // The probe works and UI Automation refuses at the last moment: the text goes
    // to the clipboard rather than being lost.
    let mut harness = Harness::new("привет мир");
    harness.inserter.fail = true;
    let outcome = harness.run(false, false).expect("the clipboard takes over");
    assert_eq!(outcome.method, Some(DeliveryMethod::Clipboard));
    assert_eq!(outcome.rule, Some("ui_automation_failed"));
    assert_eq!(harness.copied().as_deref(), Some("привет мир"));
    assert_eq!(harness.written(), None);
    assert!(harness
        .trace
        .calls()
        .contains(&"clipboard.write".to_string()));

    // The person prefers the clipboard: UI Automation is never attempted.
    let harness = Harness::new("привет");
    let speak = |_: &str| {};
    let outcome = harness
        .engine
        .run(
            DictationRequest {
                autocorrect: false,
                punctuation: false,
                preference: VoiceInputPreference::Clipboard,
                speak_confirmation: false,
            },
            EngineDeps {
                host: &harness.host,
                transcriber: &harness.transcriber,
                corrector: &harness.corrector,
                probe: &harness.probe,
                inserter: &harness.inserter,
                clipboard: &harness.clipboard,
                speak: &speak,
            },
        )
        .expect("the request");
    assert_eq!(outcome.method, Some(DeliveryMethod::Clipboard));
    assert!(!harness.trace.calls().contains(&"uia.insert".to_string()));
    assert_eq!(harness.copied().as_deref(), Some("привет"));
    // The outcome says how it was delivered, and never what was delivered.
    let rendered = serde_json::to_string(&outcome).expect("json");
    assert!(!rendered.contains("привет"), "{rendered}");
}

#[test]
fn a_cancelled_request_delivers_nothing() {
    struct CancelDuringTranscription {
        engine: Arc<DictationEngine>,
    }
    impl VoiceTranscriber for CancelDuringTranscription {
        fn transcribe(&self) -> Result<TranscribedText, DictationError> {
            self.engine.cancel();
            Ok(TranscribedText {
                text: SECRET.to_string(),
                audio_ms: 500,
            })
        }
    }

    let harness = Harness::new("привет");
    let engine = Arc::clone(&harness.engine);
    let speak = |_: &str| {};
    let error = engine
        .run(
            DictationRequest {
                autocorrect: false,
                punctuation: false,
                preference: VoiceInputPreference::Clipboard,
                speak_confirmation: false,
            },
            EngineDeps {
                host: &harness.host,
                transcriber: &CancelDuringTranscription {
                    engine: Arc::clone(&engine),
                },
                corrector: &harness.corrector,
                probe: &harness.probe,
                inserter: &harness.inserter,
                clipboard: &harness.clipboard,
                speak: &speak,
            },
        )
        .unwrap_err();
    assert_eq!(error, DictationError::Cancelled);
    assert_eq!(harness.written(), None);
    assert_eq!(harness.copied(), None);
    assert_eq!(engine.stage(), DictationStage::Idle);
    assert_eq!(engine.last_text(), None);
    assert_eq!(engine.last_characters(), 0);
    assert_eq!(harness.trace.calls().last().unwrap(), "vosk.start");
}

// ------------------------------------------------------------- punctuation

#[test]
fn voice_punctuation_writes_the_marks_a_person_says() {
    let (text, report) = apply_voice_punctuation("привет точка как дела вопросительный знак");
    assert_eq!(text, "Привет. Как дела?");
    assert_eq!(report.marks, 2);
    assert!(report.changed);

    let (text, _) = apply_voice_punctuation("первая строка новая строка вторая строка");
    assert_eq!(text, "Первая строка\nВторая строка");

    let (text, _) = apply_voice_punctuation("первый абзац новый абзац второй абзац");
    assert_eq!(text, "Первый абзац\n\nВторой абзац");

    let (text, _) = apply_voice_punctuation("список открой скобку один закрой скобку");
    assert_eq!(text, "Список (один)");

    let (text, _) = apply_voice_punctuation("осторожно восклицательный знак");
    assert_eq!(text, "Осторожно!");

    let (text, _) = apply_voice_punctuation("да запятая нет");
    assert_eq!(text, "Да, нет");

    let (text, _) = apply_voice_punctuation("hello comma world period");
    assert_eq!(text, "Hello, world.");
    let (text, _) = apply_voice_punctuation("first new paragraph second");
    assert_eq!(text, "First\n\nSecond");

    let (text, _) = apply_voice_punctuation("привіт кома як справи знак питання");
    assert_eq!(text, "Привіт, як справи?");
}

#[test]
fn a_mark_that_is_really_a_word_stays_a_word() {
    // A preposition before the word keeps it a word, which is the honest
    // limitation this pass can handle.
    let (text, report) = apply_voice_punctuation("он попал в точку");
    assert_eq!(text, "Он попал в точку");
    assert_eq!(report.marks, 0);
    assert!(!report.changed);

    let (text, _) = apply_voice_punctuation("мы встретились с точками отсчёта");
    assert_eq!(text, "Мы встретились с точками отсчёта");

    let (text, _) = apply_voice_punctuation("запятаятая история");
    assert_eq!(text, "Запятаятая история");

    let (text, report) = apply_voice_punctuation("обычный текст без знаков");
    assert_eq!(text, "Обычный текст без знаков");
    assert_eq!(report.marks, 0);
    assert!(!report.changed);
}

// -------------------------------------------------------------- the privacy

#[test]
fn the_outcome_never_carries_the_text() {
    let harness = Harness::new(SECRET);
    let outcome = harness.run(false, false).expect("the request");
    let rendered = serde_json::to_string(&outcome).expect("json");
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(!format!("{outcome:?}").contains(SECRET));
    // The text is held in memory for a preview, and nowhere else.
    assert_eq!(harness.engine.last_text().as_deref(), Some(SECRET));
    harness.engine.forget();
    assert_eq!(harness.engine.last_text(), None);
    assert_eq!(harness.engine.last_characters(), 0);
}

#[test]
fn the_route_has_no_storage_and_no_field_reading() {
    // A structural check of the route's own source: no browser storage, no
    // window-text API, and no file written from a transcript.
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("session.rs", include_str!("session.rs")),
        ("insertion.rs", include_str!("insertion.rs")),
        ("target.rs", include_str!("target.rs")),
        ("error.rs", include_str!("error.rs")),
        ("punctuation.rs", include_str!("punctuation.rs")),
    ];
    for (name, source) in sources {
        for forbidden in [
            "localStorage",
            "sessionStorage",
            "indexedDB",
            "location.hash",
            "document.cookie",
            "GetWindowText",
            "window.title",
            "Command::new",
            "SendInput",
            "keybd_event",
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} must not contain {forbidden}"
            );
        }
    }
    // The fixture text lives in the tests and nowhere in the route.
    for (name, source) in sources {
        assert!(
            !source.contains(SECRET),
            "{name} must not mention the fixture"
        );
    }
    // What is logged is a length, a stage, a method, and a code.
    let session = include_str!("session.rs");
    assert!(session.contains("characters={"));
    assert!(session.contains("error_code={}"));
    assert!(session.contains("method={}"));
}

#[test]
fn the_route_cannot_reach_the_vault_the_notes_or_the_memory() {
    for (name, source) in [
        ("mod.rs", include_str!("mod.rs")),
        ("session.rs", include_str!("session.rs")),
        ("insertion.rs", include_str!("insertion.rs")),
        ("target.rs", include_str!("target.rs")),
    ] {
        for forbidden in [
            "VaultSession",
            "EncryptedNoteStore",
            "MemoryStore",
            "MasterKey",
            "VaultPaths",
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} must not reach {forbidden}"
            );
        }
    }
}

#[test]
fn the_start_phrases_are_the_ones_the_command_answers_to() {
    for phrase in [
        "Джарвис, голосовой ввод",
        "Джарвис, начни голосовой ввод",
        "Джарвис, включи диктовку",
        "Jarvis voice input",
    ] {
        assert!(super::is_start_request(phrase), "{phrase} must start it");
    }
    for phrase in [
        "джарвис голосовой ввод!",
        "ДЖАРВИС НАЧНИ ГОЛОСОВОЙ ВВОД",
        "  джарвис,  голосовой   ввод  ",
    ] {
        assert!(super::is_start_request(phrase), "{phrase} must start it");
    }
    for phrase in [
        "джарвис который час",
        "голосовой ввод текста",
        "включи музыку",
        "",
    ] {
        assert!(
            !super::is_start_request(phrase),
            "{phrase} must not start it"
        );
    }

    assert!(super::is_stop_request("Стоп"));
    assert!(super::is_stop_request("готово"));
    assert!(!super::is_stop_request("останови музыку"));

    assert_eq!(super::confirmation("ru"), "Да, сэр. Начинаю голосовой ввод");
    assert!(super::confirmation("en").contains("Yes, sir"));
    assert!(super::confirmation("ua").contains("Так, сер"));
}

#[test]
fn the_settings_are_off_until_asked_for_and_repair_themselves() {
    let settings = super::GlobalDictationSettings::default();
    assert!(!settings.enabled, "global voice input is off by default");
    assert!(settings.clipboard_seconds >= crate::vault::clipboard::MIN_CLEAR_SECONDS);
    assert!(
        !settings.matches("Джарвис, голосовой ввод"),
        "off means off"
    );

    let enabled = super::GlobalDictationSettings {
        enabled: true,
        ..super::GlobalDictationSettings::default()
    };
    assert!(enabled.matches("Джарвис, голосовой ввод"));
    assert!(enabled.matches("jarvis voice input"));
    assert!(!enabled.matches("джарвис открой блокнот"));

    // A custom phrase is accepted as well as the built-in ones.
    let custom = super::GlobalDictationSettings {
        enabled: true,
        phrase: "Джарвис, пиши".to_string(),
        ..super::GlobalDictationSettings::default()
    };
    assert!(custom.matches("джарвис пиши"));
    assert!(custom.matches("Джарвис, голосовой ввод"));

    // A nonsense document is repaired rather than refused.
    let repaired = super::GlobalDictationSettings {
        enabled: true,
        phrase: "   ".to_string(),
        language: "klingon".to_string(),
        clipboard_seconds: 5,
        ..super::GlobalDictationSettings::default()
    }
    .normalized();
    assert_eq!(repaired.phrase, super::START_PHRASES[0]);
    assert_eq!(repaired.language, "auto");
    assert_eq!(
        repaired.clipboard_seconds,
        crate::vault::clipboard::MIN_CLEAR_SECONDS
    );
}

#[test]
fn a_recognized_phrase_becomes_a_typed_intent() {
    assert_eq!(
        super::intent_of("Джарвис, голосовой ввод"),
        super::VoiceIntent::StartGlobalDictation
    );
    assert_eq!(
        super::intent_of("Джарвис, начни голосовой ввод"),
        super::VoiceIntent::StartGlobalDictation
    );
    assert_eq!(
        super::intent_of("Джарвис, включи диктовку"),
        super::VoiceIntent::StartGlobalDictation
    );
    assert_eq!(
        super::intent_of("Джарвис, продиктую текст"),
        super::VoiceIntent::StartGlobalDictation
    );
    assert_eq!(super::intent_of("Стоп"), super::VoiceIntent::StopDictation);
    assert_eq!(
        super::intent_of("готово"),
        super::VoiceIntent::StopDictation
    );
    // Everything else is not an intent, and it is not a command either.
    assert_eq!(
        super::intent_of("джарвис который час"),
        super::VoiceIntent::None
    );
    assert_eq!(super::intent_of("открой блокнот"), super::VoiceIntent::None);
    assert_eq!(super::intent_of(""), super::VoiceIntent::None);
    // The names are stable and carry no content.
    assert_eq!(
        super::VoiceIntent::StartGlobalDictation.as_str(),
        "start_global_dictation"
    );
    assert_eq!(super::VoiceIntent::StopDictation.as_str(), "stop_dictation");
    assert_eq!(super::VoiceIntent::None.as_str(), "none");

    // The settings gate the intent: switched off means no intent at all.
    let off = super::GlobalDictationSettings::default();
    assert_eq!(
        super::intent_with_settings(&off, "Джарвис, голосовой ввод"),
        super::VoiceIntent::None
    );
    let on = super::GlobalDictationSettings {
        enabled: true,
        ..super::GlobalDictationSettings::default()
    };
    assert_eq!(
        super::intent_with_settings(&on, "Джарвис, продиктую текст"),
        super::VoiceIntent::StartGlobalDictation
    );
    assert_eq!(
        super::intent_with_settings(&on, "стоп"),
        super::VoiceIntent::StopDictation
    );
    assert_eq!(
        super::intent_with_settings(&on, "включи музыку"),
        super::VoiceIntent::None
    );
}

#[test]
fn the_intent_type_carries_no_command_and_no_path() {
    // A type, not a string the rest of the application could execute.
    let source = include_str!("mod.rs");
    let start = source
        .find("pub enum VoiceIntent")
        .expect("the intent type");
    let end = source[start..].find('}').unwrap() + start;
    let body = &source[start..end];
    for forbidden in ["String", "Path", "Command", "args", "shell"] {
        assert!(
            !body.contains(forbidden),
            "an intent must not be able to carry {forbidden}"
        );
    }
    // And the route has no process API at all.
    for (name, text) in [
        ("mod.rs", include_str!("mod.rs")),
        ("session.rs", include_str!("session.rs")),
        ("insertion.rs", include_str!("insertion.rs")),
        ("target.rs", include_str!("target.rs")),
    ] {
        for forbidden in ["Command::new", "std::process", "powershell", "cmd.exe"] {
            assert!(
                !text.contains(forbidden),
                "{name} must not contain {forbidden}"
            );
        }
    }
}

#[test]
fn the_newer_marks_are_written_the_way_a_person_says_them() {
    let (text, report) = apply_voice_punctuation("список двоеточие один точка с запятой два");
    assert_eq!(text, "Список: один; два");
    assert_eq!(report.marks, 2);

    let (text, _) = apply_voice_punctuation("он сказал открой кавычки привет закрой кавычки");
    assert_eq!(text, "Он сказал «привет»");

    let (text, _) = apply_voice_punctuation("note colon first semicolon second");
    assert_eq!(text, "Note: first; second");

    let (text, _) = apply_voice_punctuation("він сказав відкрий лапки привіт закрий лапки");
    assert_eq!(text, "Він сказав «привіт»");

    // A bare "кавычки" is not interpreted: the limit is deliberate.
    let (text, report) = apply_voice_punctuation("слово кавычки слово");
    assert_eq!(text, "Слово кавычки слово");
    assert_eq!(report.marks, 0);
}
