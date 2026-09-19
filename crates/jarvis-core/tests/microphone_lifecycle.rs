//! The microphone lifecycle, against the real device.
//!
//! This is the test the reported defect needed. On the machine where dictation
//! failed, `recorder init` succeeded, the microphone check succeeded, and then
//! the dictation reported `recorder_unavailable` before the native start was
//! ever called: the session read frames from a stream that nothing had opened.
//! A test that only mocks the frame source cannot see that, because the mock
//! does not have a stream to open.
//!
//! The rules here:
//!
//! * the device belongs to whoever claims it, so the tests in this file take
//!   turns through one lock;
//! * a machine without an input device is not a failure of the test: the typed
//!   error is asserted instead, so the suite still means something on a build
//!   agent with no sound card;
//! * nothing is written, nothing is transcribed, and no frame is kept.

use std::sync::MutexGuard;

use jarvis_core::recorder::{self, MicrophoneLease, MicrophoneOwner, RecorderError};
use jarvis_core::whisper::{FrameSource, RecorderFrames, WhisperError};

/// The microphone is one device per process: the tests take turns.
static DEVICE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn take_turns() -> MutexGuard<'static, ()> {
    // A test that panicked while holding the lock must not hide the others.
    DEVICE.lock().unwrap_or_else(|error| error.into_inner())
}

/// Whether this machine has a device this build can open.
fn device_is_available() -> bool {
    match recorder::try_audio_devices() {
        Ok(devices) if !devices.is_empty() => recorder::init().is_ok(),
        _ => false,
    }
}

/// The answer a machine without a microphone must give: a typed code, no panic.
fn assert_a_missing_device_is_typed() {
    let error = recorder::check_microphone(1)
        .err()
        .or_else(|| MicrophoneLease::acquire(MicrophoneOwner::Dictation).err())
        .expect("without a device the check must report an error");
    assert!(
        matches!(
            error,
            RecorderError::NoInputDevice
                | RecorderError::NotInitialized
                | RecorderError::PermissionDenied(_)
                | RecorderError::DeviceFailed(_)
                | RecorderError::BackendUnavailable
        ),
        "{error}"
    );
    assert!(!recorder::is_streaming(), "nothing may be left claimed");
}

/// Opens the microphone the way a dictation does, and gives it back.
fn a_dictation_reaches_start() {
    let mut source = RecorderFrames::new();
    source
        .start()
        .expect("a dictation must reach the native start");
    assert!(source.holds_microphone(), "the source holds the lease");
    assert_eq!(recorder::current_owner(), MicrophoneOwner::Dictation);
    assert!(recorder::is_streaming(), "the socket is open");

    // Frames arrive after the start. Before the fix this was the call that
    // answered with a native library error, which is why the log showed a
    // microphone failure and no START line.
    let mut buffer = [0i16; 512];
    for _ in 0..3 {
        source
            .read_frame(&mut buffer)
            .expect("a frame must be readable once the stream is open");
    }

    source.stop();
    assert!(!source.holds_microphone(), "the lease is given back");
    assert!(!recorder::is_streaming(), "the claim is free");
    assert_eq!(recorder::current_owner(), MicrophoneOwner::Free);
}

/// The whole reported sequence: init → check → released → dictate → START →
/// frames → stop, and then a second dictation that must reach START as well.
#[test]
fn a_check_releases_the_microphone_and_the_next_dictation_starts() {
    let _turns = take_turns();
    if !device_is_available() {
        assert_a_missing_device_is_typed();
        return;
    }

    for round in 0..3 {
        let check = recorder::check_microphone(2).expect("the check must be possible");
        assert!(check.released, "round {round}: the check must release");
        assert_eq!(
            recorder::current_owner(),
            MicrophoneOwner::Free,
            "round {round}: the claim must be free after a check"
        );
        assert!(
            !recorder::is_streaming(),
            "round {round}: nothing may stay open"
        );

        // The dictation comes straight after the check, with nothing in
        // between: this is the sequence that failed on the real machine.
        a_dictation_reaches_start();
    }

    // Several checks in a row, then one more dictation.
    for _ in 0..3 {
        let check = recorder::check_microphone(1).expect("the check must be possible");
        assert!(check.released);
    }
    a_dictation_reaches_start();
}

/// A dictation right after the recorder is prepared, with no check at all.
#[test]
fn a_dictation_without_a_check_starts_the_stream() {
    let _turns = take_turns();
    if !device_is_available() {
        assert_a_missing_device_is_typed();
        return;
    }
    // The claim is made clean by the check above in the start-up route; if a
    // previous test in this file left something behind, this is the assertion
    // that would catch it.
    assert_eq!(recorder::current_owner(), MicrophoneOwner::Free);
    a_dictation_reaches_start();
}

/// A second dictation while the first is running is told who holds the device.
#[test]
fn a_second_dictation_is_busy_and_not_a_device_failure() {
    let _turns = take_turns();
    if !device_is_available() {
        return;
    }
    let lease = MicrophoneLease::acquire(MicrophoneOwner::Dictation)
        .expect("the first dictation must be able to start");
    assert!(recorder::is_streaming());

    let mut second = RecorderFrames::new();
    match second.start() {
        Err(WhisperError::RecorderUnavailable { stage, code }) => {
            assert_eq!(code, "already_running");
            assert_eq!(stage, "claim", "the claim is where it failed");
        }
        other => panic!("a second dictation must be refused, got {other:?}"),
    }
    // A frame from the second source is refused for the same reason: it never
    // opened a stream, so the frames are not its to take.
    let mut buffer = [0i16; 512];
    match second.read_frame(&mut buffer) {
        Err(WhisperError::RecorderUnavailable { stage, code }) => {
            assert_eq!(code, "invalid_state");
            assert_eq!(stage, "read");
        }
        other => panic!("a source that never started must not read, got {other:?}"),
    }

    drop(lease);
    assert!(!recorder::is_streaming());
    // With the first one gone, the second one works.
    a_dictation_reaches_start();
}

/// The voice listener is its own answer: the code says who holds the device.
#[test]
fn the_voice_listener_reports_its_own_code() {
    let _turns = take_turns();
    if !device_is_available() {
        return;
    }
    recorder::start_recording().expect("the listener must be able to start");
    assert_eq!(recorder::current_owner(), MicrophoneOwner::Voice);

    let mut source = RecorderFrames::new();
    match source.start() {
        Err(WhisperError::RecorderUnavailable { stage, code }) => {
            assert_eq!(code, "vosk_owns_microphone");
            assert_eq!(stage, "claim");
        }
        other => panic!("the listener owns the device, got {other:?}"),
    }

    let _ = recorder::stop_recording();
    assert_eq!(recorder::current_owner(), MicrophoneOwner::Free);
    a_dictation_reaches_start();
}
