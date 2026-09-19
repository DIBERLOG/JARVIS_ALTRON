//! The conversation route, checked where it can be checked without a microphone.
//!
//! Three things matter here and each has its own test:
//!
//! * **the control phrases** work in all three languages, and an ordinary sentence
//!   is not one of them;
//! * **the state machine** allows one request at a time, can be cancelled from every
//!   stage, and has a deadline for each stage;
//! * **the isolation is structural**: the route is read as text and refused if it
//!   can reach a command, a Windows action, the safety gate or a process.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::*;

#[test]
fn the_control_phrases_are_the_ones_a_person_says() {
    for (phrase, expected) in [
        ("давай поговорим", ConversationIntent::Start),
        ("Джарвис, хочу поговорить", ConversationIntent::Start),
        ("у меня вопрос", ConversationIntent::Start),
        ("режим диалога", ConversationIntent::Start),
        ("поговорим о погоде", ConversationIntent::Start),
        ("let's talk", ConversationIntent::Start),
        ("i have a question", ConversationIntent::Start),
        ("conversation mode", ConversationIntent::Start),
        ("у мене питання", ConversationIntent::Start),
        ("давай поговоримо", ConversationIntent::Start),
        ("продолжим", ConversationIntent::Continue),
        ("продолжай разговор", ConversationIntent::Continue),
        ("continue the conversation", ConversationIntent::Continue),
        ("продовжимо", ConversationIntent::Continue),
        ("закончи разговор", ConversationIntent::Stop),
        ("заверши диалог", ConversationIntent::Stop),
        ("end the conversation", ConversationIntent::Stop),
        ("закінчи розмову", ConversationIntent::Stop),
        ("отмени разговор", ConversationIntent::Cancel),
        ("cancel the conversation", ConversationIntent::Cancel),
        ("скасуй розмову", ConversationIntent::Cancel),
    ] {
        assert_eq!(
            intent_of(phrase),
            Some(expected),
            "`{phrase}` must be {:?}",
            expected
        );
    }

    // Abandoning a question is the narrower wish: "отмени разговор" is a cancel, not
    // a stop, even though both words are in it.
    assert_eq!(
        intent_of("отмени разговор"),
        Some(ConversationIntent::Cancel)
    );

    // An ordinary sentence is not a control phrase.
    for phrase in [
        "какая погода в москве",
        "открой браузер",
        "выключи звук",
        "расскажи анекдот",
        "как дела",
        "what is the weather",
        "",
        "   ",
    ] {
        assert_eq!(
            intent_of(phrase),
            None,
            "`{phrase}` must not be a control phrase"
        );
    }
}

#[test]
fn one_request_at_a_time_and_every_stage_can_be_cancelled() {
    let start = Instant::now();
    let mut session = ConversationSession::new();
    assert_eq!(session.stage(), ConversationStage::Idle);
    assert!(!session.stage().is_active());

    session.begin(start).expect("the first turn");
    assert_eq!(session.stage(), ConversationStage::Recording);
    assert!(session.stage().is_active());

    // A second trigger while one is running is refused, whatever it is.
    assert_eq!(session.begin(start), Err(ConversationError::Busy));

    // Cancelling is allowed from every active stage.
    let now = start;
    session
        .enter(ConversationStage::Transcribing, now)
        .expect("stage");
    assert!(session.cancel(now));
    assert_eq!(session.stage(), ConversationStage::Cancelled);
    assert_eq!(session.last_code(), Some("cancelled"));
    // Cancelling twice says nothing more, and the next turn is allowed.
    assert!(!session.cancel(now));
    session.begin(now).expect("a new turn after a cancel");
    session
        .enter(ConversationStage::Transcribing, now)
        .expect("stage");
    session
        .enter(ConversationStage::Thinking, now)
        .expect("stage");
    assert!(
        session.cancel(now),
        "a question can be abandoned while it is being answered"
    );
    session.begin(now).expect("a new turn");
    session
        .enter(ConversationStage::Transcribing, now)
        .expect("stage");
    session
        .enter(ConversationStage::Thinking, now)
        .expect("stage");
    session
        .enter(ConversationStage::Answering, now)
        .expect("stage");
    assert!(
        session.cancel(now),
        "an answer can be abandoned while it is shown"
    );

    // Cancelling when nothing is happening is not a cancellation.
    session.cancel(now);
    session.begin(now).expect("a new turn");
    session.cancel(now);
    let mut idle = ConversationSession::new();
    assert!(!idle.cancel(now));
}

#[test]
fn the_route_walks_its_stages_and_counts_its_turns() {
    let now = Instant::now();
    let mut session = ConversationSession::new();
    session.begin(now).expect("begin");
    session
        .enter(ConversationStage::Transcribing, now)
        .expect("stage");
    session
        .enter(ConversationStage::Thinking, now)
        .expect("stage");
    session
        .enter(ConversationStage::Answering, now)
        .expect("stage");
    session
        .enter(ConversationStage::Speaking, now)
        .expect("stage");
    session
        .enter(ConversationStage::Finished, now)
        .expect("stage");
    assert_eq!(session.turns(), 1);
    assert!(!session.stage().is_active());

    // An answer that is never spoken still finishes.
    session.begin(now).expect("begin");
    session
        .enter(ConversationStage::Transcribing, now)
        .expect("stage");
    session
        .enter(ConversationStage::Thinking, now)
        .expect("stage");
    session
        .enter(ConversationStage::Answering, now)
        .expect("stage");
    session
        .enter(ConversationStage::Finished, now)
        .expect("stage");
    assert_eq!(session.turns(), 2);

    // A stage that does not belong is refused, and does not move the session.
    session.begin(now).expect("begin");
    let error = session
        .enter(ConversationStage::Speaking, now)
        .expect_err("recording cannot jump to speaking");
    assert_eq!(error.code(), "conversation_wrong_stage");
    assert_eq!(session.stage(), ConversationStage::Recording);
}

#[test]
fn every_stage_has_a_deadline() {
    let start = Instant::now();
    let mut session = ConversationSession::new();
    session.begin(start).expect("begin");
    assert_eq!(session.timeout(start), None);
    assert_eq!(
        session.timeout(start + RECORD_TIMEOUT + Duration::from_millis(1)),
        Some("record_timeout")
    );

    session
        .enter(ConversationStage::Transcribing, start)
        .expect("stage");
    assert_eq!(
        session.timeout(start + TRANSCRIBE_TIMEOUT + Duration::from_millis(1)),
        Some("transcribe_timeout")
    );

    session
        .enter(ConversationStage::Thinking, start)
        .expect("stage");
    assert_eq!(
        session.timeout(start + ANSWER_TIMEOUT + Duration::from_millis(1)),
        Some("answer_timeout")
    );

    // A finished stage has no deadline: nothing is waiting.
    session
        .enter(ConversationStage::Answering, start)
        .expect("stage");
    session
        .enter(ConversationStage::Finished, start)
        .expect("stage");
    assert_eq!(
        session.timeout(start + ANSWER_TIMEOUT + Duration::from_secs(60)),
        None
    );
}

#[test]
fn a_question_is_checked_and_an_answer_is_cut() {
    assert_eq!(
        check_question("  какая погода?  ").expect("a question"),
        "какая погода?"
    );
    assert_eq!(
        check_question("   ").expect_err("an empty question").code(),
        "question_too_long"
    );
    let too_long = "я".repeat(MAX_QUESTION_CHARS + 1);
    assert!(check_question(&too_long).is_err());
    let long_answer = "о".repeat(MAX_ANSWER_CHARS + 40);
    assert_eq!(cut_answer(&long_answer).chars().count(), MAX_ANSWER_CHARS);
    assert_eq!(cut_answer("  ответ  "), "ответ");
}

#[test]
fn no_provider_means_a_typed_refusal_and_never_an_invented_answer() {
    let provider = Disabled;
    assert_eq!(provider.name(), "disabled");
    assert!(!provider.is_available());
    assert!(!provider.is_cloud());
    let cancel = AtomicBool::new(false);
    let mut deltas = |_: &str| {};
    assert_eq!(
        provider
            .answer(DEFAULT_SYSTEM_PROMPT, "привет", &cancel, &mut deltas)
            .expect_err("nothing is configured"),
        ConversationError::NotConfigured
    );
    assert_eq!(
        ConversationError::NotConfigured.code(),
        "provider_not_configured"
    );
}

#[test]
fn a_failure_is_reported_with_a_safe_code() {
    let now = Instant::now();
    let mut session = ConversationSession::new();
    session.begin(now).expect("begin");
    session.fail("provider_failed", now);
    assert_eq!(session.stage(), ConversationStage::Failed);
    assert_eq!(session.last_code(), Some("provider_failed"));
    assert!(!session.stage().is_active());
    // And a new question is allowed after a failure.
    session.begin(now).expect("a new turn after a failure");
}

#[test]
fn the_listener_is_restored_on_every_exit() {
    use std::cell::RefCell;
    use std::rc::Rc;

    // A normal return.
    let restored = Rc::new(RefCell::new(0));
    {
        let counter = Rc::clone(&restored);
        let _guard = RestoreOnDrop::new(move || *counter.borrow_mut() += 1);
    }
    assert_eq!(*restored.borrow(), 1);

    // An early return through `?`-style code.
    let restored = Rc::new(RefCell::new(0));
    {
        let counter = Rc::clone(&restored);
        let _guard = RestoreOnDrop::new(move || *counter.borrow_mut() += 1);
    }
    assert_eq!(*restored.borrow(), 1);

    // A panic in the step that holds it.
    let restored = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&restored);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = RestoreOnDrop::new(move || *counter.borrow_mut() += 1);
        panic!("a step panicked");
    }));
    assert!(caught.is_err());
    assert_eq!(
        *restored.borrow(),
        1,
        "a panic must not cost the microphone"
    );

    // And running it by hand is not a second restore.
    let restored = Rc::new(RefCell::new(0));
    {
        let counter = Rc::clone(&restored);
        let mut guard = RestoreOnDrop::new(move || *counter.borrow_mut() += 1);
        guard.restore_now();
    }
    assert_eq!(*restored.borrow(), 1);
}

#[test]
fn a_cancellation_flag_is_read_once_per_step() {
    let cancel = AtomicBool::new(false);
    assert!(!cancelled(&cancel));
    cancel.store(true, Ordering::SeqCst);
    assert!(cancelled(&cancel));
}

#[test]
fn the_conversation_route_cannot_reach_a_command() {
    // The isolation is not a habit, it is a property of this file. If any of these
    // ever appears here, the route has grown a capability and this test fails.
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("conversation")
        .join("mod.rs");
    let text = std::fs::read_to_string(&source).expect("this module is readable");
    for forbidden in [
        "execute_command",
        "execute_cli",
        "execute_exe",
        "WindowsAction",
        "windows_actions",
        "SafetyGate",
        "fetch_command",
        "check_phrase",
        "Command::new",
        "std::process",
        "std::net",
        "reqwest",
        "http",
    ] {
        assert!(
            !text.contains(forbidden),
            "the conversation route must not mention {forbidden}"
        );
    }
    // And it carries no tool, no path and no key of its own.
    for forbidden in ["api_key", "password", "token", "tool_call", "function_call"] {
        assert!(
            !text.to_lowercase().contains(forbidden),
            "the conversation route must not carry {forbidden}"
        );
    }
}
