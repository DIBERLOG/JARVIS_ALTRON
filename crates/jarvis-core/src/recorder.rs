//! Microphone capture.
//!
//! Two rules shape this module, and both of them used to be broken:
//!
//! * **nothing here panics.** The cells that hold the backend and the frame
//!   length are empty until [`init`] runs, and a process that never called it —
//!   the window is one — reached the native read with an empty cell and died
//!   inside the worker thread that was recording. Every path now reports a typed
//!   [`RecorderError`] instead, and the one place a native library can still
//!   abort is wrapped in a panic boundary as the last line of defence;
//! * **every failure names itself.** `not_initialized`, `no_input_device`,
//!   `unsupported_configuration`, `device_failed`, `already_running`,
//!   `not_running`, and `backend_unavailable` are the answers, and each one is a
//!   sentence a person can act on. None of them carries a device name, a path,
//!   or audio.
//!
//! The older infallible functions (`read_microphone`, `start_recording`,
//! `stop_recording`) are kept for the callers that have nowhere to put an error.
//! They log the code and do nothing else: they no longer panic, and they no
//! longer pretend that a read happened when it did not.

mod error;
mod pvrecorder;

// mod cpal;
// mod portaudio;

use once_cell::sync::{Lazy, OnceCell};

pub use error::RecorderError;

use crate::{config, config::structs::RecorderType, DB};

static RECORDER_TYPE: OnceCell<RecorderType> = OnceCell::new();
static FRAME_LENGTH: OnceCell<u32> = OnceCell::new();

/// Who is holding the microphone in this process.
///
/// There is exactly one device, so there is exactly one owner: a check, a
/// dictation, or the voice listener. Keeping it here — rather than in each
/// caller — is what stops a diagnostic from leaving the device closed for the
/// next dictation, and what stops a dictation from being told "no microphone"
/// when the real answer is "the microphone is in use".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MicrophoneOwner {
    /// Nobody is holding it.
    Free,
    /// The "check the microphone" diagnostic, which holds it for a moment.
    Check,
    /// A dictation, from the first frame to the last.
    Dictation,
    /// The wake-word listener.
    Voice,
}

/// The fixed name of an owner, for a log line or a message.
pub fn owner_name(owner: MicrophoneOwner) -> &'static str {
    match owner {
        MicrophoneOwner::Free => "nobody",
        MicrophoneOwner::Check => "a microphone check",
        MicrophoneOwner::Dictation => "a dictation",
        MicrophoneOwner::Voice => "the voice listener",
    }
}

/// The owner cell: one lock, no device access.
static OWNER: Lazy<parking_lot::Mutex<MicrophoneOwner>> =
    Lazy::new(|| parking_lot::Mutex::new(MicrophoneOwner::Free));

/// Who holds the microphone right now.
pub fn current_owner() -> MicrophoneOwner {
    *OWNER.lock()
}

/// Whether a stream is open in this process.
///
/// This is the question the callers actually have: not "is the recorder
/// initialised" but "is somebody recording right now".
pub fn is_streaming() -> bool {
    current_owner() != MicrophoneOwner::Free
}

/// Takes the microphone for `owner`, if nobody else has it.
///
/// This is the whole ownership rule in one place, and it touches no device: it
/// is the only way an owner is set, so a second claim can never look like a
/// device failure.
fn claim(owner: MicrophoneOwner) -> Result<(), RecorderError> {
    let mut current = OWNER.lock();
    match *current {
        MicrophoneOwner::Free => {
            *current = owner;
            Ok(())
        }
        held if held == owner => Err(RecorderError::AlreadyRunning),
        MicrophoneOwner::Voice => Err(RecorderError::VoiceOwnsMicrophone),
        held => Err(RecorderError::Busy {
            held_by: owner_name(held),
        }),
    }
}

/// Gives the microphone back. Only the owner can, and a repeated release is
/// harmless.
fn release(owner: MicrophoneOwner) {
    let mut current = OWNER.lock();
    if *current == owner {
        *current = MicrophoneOwner::Free;
    }
}

/// The microphone, open for exactly one owner.
///
/// The lease is the only way to record: [`MicrophoneLease::acquire`] claims the
/// device, initialises the recorder if it has to, and starts the native stream;
/// dropping it stops the stream and frees the claim. Because the release is in
/// `Drop`, there is no path — a returned error, an early exit, a panic in a
/// native library — that leaves the device held.
///
/// This is what the defect was: the dictation read frames from a stream that had
/// never been started, and the check left nothing behind to reuse. Both now go
/// through the same door.
pub struct MicrophoneLease {
    owner: MicrophoneOwner,
}

impl MicrophoneLease {
    /// Opens the microphone for `owner`.
    ///
    /// The stages are reported separately: a claim that fails because someone
    /// else holds the device is not a device failure, and a start that fails
    /// keeps the reason the native library gave.
    pub fn acquire(owner: MicrophoneOwner) -> Result<Self, RecorderError> {
        if !is_ready() {
            init()?;
        }
        claim(owner)?;
        if let Err(error) = try_start_recording() {
            release(owner);
            warn!(
                "recorder: stage=start error_code={} inner_code={} owner={}",
                RecorderError::StartFailed {
                    inner: error.code()
                }
                .code(),
                error.code(),
                owner_name(owner)
            );
            return Err(start_failure(error));
        }
        info!(
            "recorder: stage=start error_code=none owner={} backend={} device_count={} selected_index={}",
            owner_name(owner),
            status().map(|status| status.backend).unwrap_or("unknown"),
            status().map(|status| status.device_count).unwrap_or(0),
            status().map(|status| status.selected_index).unwrap_or(-1),
        );
        Ok(Self { owner })
    }

    /// Who this lease belongs to.
    pub fn owner(&self) -> MicrophoneOwner {
        self.owner
    }
}

impl Drop for MicrophoneLease {
    fn drop(&mut self) {
        // Stop first, then free the claim: a caller that acquires immediately
        // afterwards must never see a device that is still streaming.
        match try_stop_recording() {
            Ok(()) | Err(RecorderError::NotRunning) | Err(RecorderError::NotInitialized) => {}
            Err(error) => warn!(
                "recorder: stage=stop error_code={} owner={}",
                error.code(),
                owner_name(self.owner)
            ),
        }
        release(self.owner);
        info!(
            "recorder: stage=stop error_code=none owner={} released=true",
            owner_name(self.owner)
        );
    }
}

/// Keeps the reason a start failed when it is one the user can act on, and
/// names the stage when it is not.
fn start_failure(error: RecorderError) -> RecorderError {
    match error {
        // These already say what to do about it.
        RecorderError::PermissionDenied(_)
        | RecorderError::NoInputDevice
        | RecorderError::UnsupportedConfiguration(_)
        | RecorderError::BackendUnavailable
        | RecorderError::NotInitialized => error,
        other => RecorderError::StartFailed {
            inner: other.code(),
        },
    }
}

/// Whether the recorder can be used in this process.
///
/// A caller that records asks this first. It is cheap, it never panics, and it
/// is what a settings page needs to explain why a button is disabled.
pub fn is_ready() -> bool {
    RECORDER_TYPE.get().is_some() && FRAME_LENGTH.get().is_some()
}

/// What the recorder found when it was initialised.
///
/// This is the answer to "did it actually open the device": the backend that was
/// chosen, whether the native recorder object exists, how many input devices were
/// seen, and which index was used. It carries no device name and no audio.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RecorderStatus {
    /// The backend this build selected, as a stable name.
    pub backend: &'static str,
    /// Whether the native recorder object was created.
    pub native_ready: bool,
    /// How many input devices the backend reported.
    pub device_count: usize,
    /// The index that was used, or `-1` for the default device.
    pub selected_index: i32,
    pub frame_length: u32,
}

/// The recorder's state, without touching the device.
pub fn status() -> Result<RecorderStatus, RecorderError> {
    let Some(recorder_type) = RECORDER_TYPE.get() else {
        return Err(RecorderError::NotInitialized);
    };
    let backend = match recorder_type {
        RecorderType::PvRecorder => "pvrecorder",
        RecorderType::PortAudio => "portaudio",
        RecorderType::Cpal => "cpal",
    };
    Ok(RecorderStatus {
        backend,
        native_ready: pvrecorder::is_ready(),
        device_count: get_audio_devices().len(),
        selected_index: get_selected_microphone_index(),
        frame_length: FRAME_LENGTH.get().copied().unwrap_or(0),
    })
}

/// Initialises the recorder and reports exactly what happened.
///
/// The result is a value, never a hidden failure: a missing input device, a
/// backend this build does not implement, and a device that refuses to open are
/// three different answers, and the caller passes the code on to the interface.
pub fn init() -> Result<RecorderStatus, RecorderError> {
    // set default recorder type
    // @TODO. Make it configurable?
    // A second call is not an error: the value is already the one we want.
    let _ = RECORDER_TYPE.set(config::DEFAULT_RECORDER_TYPE);

    // Load the selected backend. Device *names* are never logged: only the
    // count, the index, the backend name, and the error code, which is what a
    // diagnosis needs and all that is safe to keep.
    match RECORDER_TYPE
        .get()
        .unwrap_or(&config::DEFAULT_RECORDER_TYPE)
    {
        RecorderType::PvRecorder => {
            let _ = FRAME_LENGTH.set(512u32); // pvrecorder requires frame buffer of 512
            let frame_length = FRAME_LENGTH.get().copied().unwrap_or(512);
            let device_count = get_audio_devices().len();
            let selected_index = get_selected_microphone_index();
            if device_count == 0 {
                error!(
                    "recorder init: backend=pvrecorder device_count=0 error_code={}",
                    RecorderError::NoInputDevice.code()
                );
                return Err(RecorderError::NoInputDevice);
            }
            if !pvrecorder::init_microphone(selected_index, frame_length) {
                // The native library's own reason is classified, not dropped: a
                // refused access is `permission_denied`, everything else is
                // `device_failed`. The native sentence itself is never logged.
                let error = pvrecorder::classify(&pvrecorder::last_open_error());
                error!(
                    "recorder init: backend=pvrecorder device_count={} selected_index={} native_ready=false error_code={}",
                    device_count,
                    selected_index,
                    error.code()
                );
                return Err(error);
            }
            info!(
                "recorder init: backend=pvrecorder device_count={} selected_index={} native_ready={} frame_length={}",
                device_count,
                selected_index,
                pvrecorder::is_ready(),
                frame_length
            );
        }
        RecorderType::PortAudio | RecorderType::Cpal => {
            let backend = if matches!(RECORDER_TYPE.get(), Some(RecorderType::PortAudio)) {
                "portaudio"
            } else {
                "cpal"
            };
            error!(
                "recorder init: backend={} native_ready=false error_code={}",
                backend,
                RecorderError::BackendUnavailable.code()
            );
            return Err(RecorderError::BackendUnavailable);
        }
    }

    status()
}
/// Reads one frame, reporting why it could not be read.
///
/// This is the function every recording path uses. The two failures that used to
/// panic here — an uninitialised recorder and an unimplemented backend — are
/// values, and a panic from the native library is caught at this boundary so a
/// recording fails instead of taking the worker thread with it. The primary
/// cause is never masked: the checks below name it exactly before any native
/// call is made.
///
/// The first check is the one the defect needed: a read on a stream that was
/// never started is `invalid_state`, not a device failure. Asking the native
/// library to read a closed stream answers with a library error, and reporting
/// that as "the microphone failed" was how a missing `start` looked like broken
/// hardware.
pub fn try_read_microphone(frame_buffer: &mut [i16]) -> Result<(), RecorderError> {
    let Some(recorder_type) = RECORDER_TYPE.get() else {
        return Err(RecorderError::NotInitialized);
    };
    if FRAME_LENGTH.get().is_none() {
        return Err(RecorderError::NotInitialized);
    }
    match recorder_type {
        RecorderType::PvRecorder => {
            if !pvrecorder::is_recording() {
                return Err(RecorderError::InvalidState("the stream is not open"));
            }
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pvrecorder::try_read_microphone(frame_buffer)
            }));
            match outcome {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(read_failure(error)),
                Err(_) => Err(RecorderError::DeviceFailed(
                    "the audio backend stopped unexpectedly".to_string(),
                )),
            }
        }
        RecorderType::PortAudio | RecorderType::Cpal => Err(RecorderError::BackendUnavailable),
    }
}

/// Names the read stage while keeping an actionable reason underneath.
fn read_failure(error: RecorderError) -> RecorderError {
    match error {
        RecorderError::PermissionDenied(_)
        | RecorderError::NoInputDevice
        | RecorderError::BackendUnavailable
        | RecorderError::NotInitialized
        | RecorderError::InvalidState(_) => error,
        other => RecorderError::ReadFailed {
            inner: other.code(),
        },
    }
}

/// Reads one frame, ignoring a failure.
///
/// Kept for the callers that have nowhere to put an error: the voice host loops
/// on this and treats an empty frame as silence. A failure fills the buffer with
/// silence and is logged by code, and nothing panics.
pub fn read_microphone(frame_buffer: &mut [i16]) {
    if let Err(error) = try_read_microphone(frame_buffer) {
        warn!("recorder: a frame could not be read ({})", error.code());
        frame_buffer.fill(0);
    }
}

/// Starts the microphone, reporting why it could not start.
///
/// The native start used to unwrap its own cell, so a machine with no input
/// device panicked there instead of reporting `no_input_device`.
pub fn try_start_recording() -> Result<(), RecorderError> {
    let Some(recorder_type) = RECORDER_TYPE.get() else {
        return Err(RecorderError::NotInitialized);
    };
    let Some(frame_length) = FRAME_LENGTH.get().copied() else {
        return Err(RecorderError::NotInitialized);
    };
    if get_audio_devices().is_empty() {
        return Err(RecorderError::NoInputDevice);
    }
    let device = get_selected_microphone_index();
    match recorder_type {
        RecorderType::PvRecorder => {
            if !pvrecorder::init_microphone(device, frame_length) {
                return Err(pvrecorder::classify(&pvrecorder::last_open_error()));
            }
            pvrecorder::try_start_recording()
        }
        RecorderType::PortAudio | RecorderType::Cpal => Err(RecorderError::BackendUnavailable),
    }
}

/// Stops the microphone, reporting why it could not stop.
///
/// Stopping is always attempted, even when nothing is recording: a caller that
/// releases the device on an error path must not have to know the state first.
pub fn try_stop_recording() -> Result<(), RecorderError> {
    let Some(recorder_type) = RECORDER_TYPE.get() else {
        return Err(RecorderError::NotInitialized);
    };
    match recorder_type {
        RecorderType::PvRecorder => pvrecorder::try_stop_recording(),
        RecorderType::PortAudio | RecorderType::Cpal => Err(RecorderError::BackendUnavailable),
    }
}

/// Starts the microphone, ignoring a failure. Kept for the existing callers.
///
/// The voice host is the only caller left. It is a microphone owner like any
/// other, so the claim is taken here and given back by [`stop_recording`]: a
/// dictation that starts while the listener is running is told
/// `vosk_owns_microphone` instead of being told that the device failed.
pub fn start_recording() -> Result<(), ()> {
    if let Err(error) = claim(MicrophoneOwner::Voice) {
        warn!(
            "recorder: stage=claim error_code={} owner={}",
            error.code(),
            owner_name(MicrophoneOwner::Voice)
        );
        return Err(());
    }
    match try_start_recording() {
        Ok(()) => Ok(()),
        Err(error) => {
            release(MicrophoneOwner::Voice);
            warn!(
                "recorder: stage=start error_code={} owner={}",
                error.code(),
                owner_name(MicrophoneOwner::Voice)
            );
            Err(())
        }
    }
}

/// Stops the microphone, ignoring a failure. Kept for the existing callers.
pub fn stop_recording() -> Result<(), ()> {
    let result = try_stop_recording().map_err(|error| {
        warn!(
            "recorder: stage=stop error_code={} owner={}",
            error.code(),
            owner_name(MicrophoneOwner::Voice)
        );
    });
    release(MicrophoneOwner::Voice);
    result
}

/// The result of the microphone check.
///
/// It answers "does this machine have a microphone this build can open, and does
/// it hear anything" without recording: the device is opened, a few frames are
/// read, and the device is released again. No frame is kept and no device name,
/// path, or audio is part of the value.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MicrophoneCheck {
    /// What the recorder found when it was prepared.
    pub status: RecorderStatus,
    /// How many frames were read; zero when the stream never started.
    pub frames_read: usize,
    /// The loudest sample seen, between 0 and 1. Silence is a valid answer.
    pub level: f32,
    /// Whether the microphone was released again. A check that fails to release
    /// is reported, because a held device would block the next recording.
    pub released: bool,
    /// Why the check failed, when it did. Never flattened: the code is the one
    /// the recorder produced.
    pub error_code: Option<String>,
}

/// Opens the microphone, reads a few frames, and releases it again.
///
/// This is what "Проверить микрофон" does. It never records to a file, never
/// sends anything to whisper, and releases the device on every path — a started
/// stream, a failed read, and a cancelled check all end in the same `stop`.
/// Only a recorder that cannot be prepared at all is returned as an error, since
/// there is then no status to report.
///
/// The check takes the same [`MicrophoneLease`] a dictation takes, so the two
/// cannot disagree about who holds the device: when this function returns, the
/// lease is dropped, the claim is free, and the next dictation starts.
pub fn check_microphone(max_frames: usize) -> Result<MicrophoneCheck, RecorderError> {
    let status = if is_ready() { status()? } else { init()? };
    let frame_length = status.frame_length.max(1) as usize;
    let mut buffer = vec![0i16; frame_length];

    // The lease is the claim and the start in one step; a failure here is an
    // answer, not an early exit, because the device is released by `Drop`.
    let lease = MicrophoneLease::acquire(MicrophoneOwner::Check);
    let mut failure = lease.as_ref().err().cloned();
    let mut frames_read = 0usize;
    let mut peak = 0i32;
    if lease.is_ok() {
        for _ in 0..max_frames {
            match try_read_microphone(&mut buffer) {
                Ok(()) => {
                    frames_read += 1;
                    for sample in &buffer {
                        peak = peak.max(i32::from(*sample).abs());
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
    }

    if frames_read > 0 {
        info!(
            "microphone check: backend={} device_count={} selected_index={} frames={} level={:.3}",
            status.backend,
            status.device_count,
            status.selected_index,
            frames_read,
            level_of(peak)
        );
    } else {
        warn!(
            "microphone check: backend={} device_count={} selected_index={} frames=0 error_code={}",
            status.backend,
            status.device_count,
            status.selected_index,
            failure.as_ref().map(RecorderError::code).unwrap_or("none")
        );
    }

    // Dropping the lease stops the stream and frees the claim, whatever happened
    // above. `released` reports it, because a device that stayed open would be
    // the next recording's problem.
    drop(lease);
    let released = !is_streaming();

    Ok(MicrophoneCheck {
        status,
        frames_read,
        level: level_of(peak),
        released,
        error_code: failure.map(|error| error.code().to_string()),
    })
}

/// A loudness between 0 and 1 from a peak sample value.
fn level_of(peak: i32) -> f32 {
    (peak as f32 / f32::from(i16::MAX)).clamp(0.0, 1.0)
}

pub fn get_selected_microphone_index() -> i32 {
    let idx = match DB.get() {
        Some(manager) => manager.read().microphone,
        // Without settings the device list decides, which is the default device.
        None => -1,
    };

    if idx > 0 {
        // validate that this microphone is actually in the list
        let devices = get_audio_devices();
        if (idx as usize) >= devices.len() {
            warn!(
                "Microphone index {} not found ({} available), falling back to default",
                idx,
                devices.len()
            );
            return -1;
        }
    }

    idx
}

#[cfg(test)]
mod structural_tests {
    #[test]
    fn native_backend_exposes_no_legacy_recording_bypass() {
        let source = include_str!("recorder/pvrecorder.rs");
        for forbidden in [
            "pub fn read_microphone(",
            "pub fn start_recording(",
            "pub fn stop_recording(",
        ] {
            assert!(
                !source.contains(forbidden),
                "native backend must stay behind try_* and MicrophoneLease: {forbidden}"
            );
        }
    }
}

pub fn get_audio_devices() -> Vec<String> {
    match RECORDER_TYPE.get() {
        Some(RecorderType::PvRecorder) => pvrecorder::list_audio_devices(),
        Some(RecorderType::PortAudio) | Some(RecorderType::Cpal) => Vec::new(),
        None => {
            // not initialized yet, default to pvrecorder
            pvrecorder::list_audio_devices()
        }
    }
}

/// The input devices, with the reason when there are none.
///
/// The list is the first thing a recording needs, so an empty list is reported
/// as a failure rather than shown as "no devices" with no explanation.
pub fn try_audio_devices() -> Result<Vec<String>, RecorderError> {
    let devices = get_audio_devices();
    if devices.is_empty() {
        return Err(RecorderError::NoInputDevice);
    }
    Ok(devices)
}

pub fn get_audio_device_name(idx: i32) -> String {
    match RECORDER_TYPE.get() {
        Some(RecorderType::PvRecorder) => pvrecorder::get_audio_device_name(idx),
        Some(RecorderType::PortAudio) | Some(RecorderType::Cpal) => String::new(),
        None => {
            // not initialized yet, default to pvrecorder
            pvrecorder::get_audio_device_name(idx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The microphone is one device per process: the tests that open it take
    /// turns, and the session tests use the same lock.
    pub(crate) fn device_test_lock() -> parking_lot::MutexGuard<'static, ()> {
        static LOCK: Lazy<parking_lot::Mutex<()>> = Lazy::new(|| parking_lot::Mutex::new(()));
        LOCK.lock()
    }

    /// The defect, exactly: a process that never initialised the recorder used
    /// to panic here with `called Option::unwrap() on a None value`.
    #[test]
    fn reading_before_init_is_a_typed_error_and_never_a_panic() {
        let mut buffer = [0i16; 512];
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            try_read_microphone(&mut buffer)
        }));
        assert!(outcome.is_ok(), "reading must never panic");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            read_microphone(&mut buffer)
        }));
        assert!(outcome.is_ok(), "the wrapper must never panic");
        // Without a recorder the answer is a typed error, not a panic.
        if !is_ready() {
            assert_eq!(
                try_read_microphone(&mut buffer),
                Err(RecorderError::NotInitialized)
            );
            assert_eq!(try_start_recording(), Err(RecorderError::NotInitialized));
            assert_eq!(try_stop_recording(), Err(RecorderError::NotInitialized));
        }
    }

    #[test]
    fn readiness_is_reported_without_panicking() {
        let ready = is_ready();
        assert_eq!(ready, is_ready(), "readiness must be stable");
        if !ready {
            assert!(matches!(
                try_read_microphone(&mut [0i16; 512]),
                Err(RecorderError::NotInitialized)
            ));
        }
    }

    #[test]
    fn an_empty_device_list_is_reported_as_no_input_device() {
        let devices = get_audio_devices();
        assert_eq!(try_audio_devices().is_ok(), !devices.is_empty());
        if devices.is_empty() {
            assert_eq!(
                try_audio_devices().unwrap_err(),
                RecorderError::NoInputDevice
            );
        }
    }

    #[test]
    fn the_selected_index_falls_back_instead_of_panicking_without_settings() {
        // This used to unwrap the settings handle; without settings it answers
        // with the default device instead.
        let index = get_selected_microphone_index();
        assert!(
            index >= -1,
            "the index must be a device index or the default"
        );
        let _ = get_audio_device_name(index);
    }

    /// The check is a diagnostic: it must never panic, and it must never leave
    /// the device open. Both are asserted whatever the machine answers.
    #[test]
    fn a_microphone_check_releases_the_device_and_never_panics() {
        let _lock = device_test_lock();
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check_microphone(3)));
        let outcome = outcome.expect("the microphone check must never panic");
        match outcome {
            Ok(check) => {
                assert!(check.released, "the device must be released again");
                assert!(check.frames_read <= 3, "the check must be bounded");
                assert!(
                    (0.0..=1.0).contains(&check.level),
                    "the level is a fraction"
                );
                // A check that read nothing always says why.
                if check.frames_read == 0 {
                    assert!(
                        check.error_code.is_some(),
                        "a silent failure is not allowed"
                    );
                }
                // The claim is free again: this is the state the next dictation
                // depends on.
                assert!(!is_streaming(), "the claim must be free after a check");
            }
            Err(error) => {
                // The only errors are a recorder that cannot be prepared.
                assert!(
                    matches!(
                        error,
                        RecorderError::NotInitialized
                            | RecorderError::NoInputDevice
                            | RecorderError::DeviceFailed(_)
                            | RecorderError::PermissionDenied(_)
                            | RecorderError::BackendUnavailable
                    ),
                    "{error}"
                );
                assert!(!is_streaming(), "a failed check holds nothing");
            }
        }
    }

    #[test]
    fn a_check_that_cannot_be_prepared_names_the_recorder_error() {
        let _lock = device_test_lock();
        // Whatever the machine answers, the code is one of the documented ones.
        let code = match check_microphone(1) {
            Ok(check) => check.error_code.unwrap_or_else(|| "none".to_string()),
            Err(error) => error.code().to_string(),
        };
        assert!(!code.is_empty());
        assert!(
            !code.contains(':') && !code.contains('\\'),
            "a code carries no path"
        );
    }

    /// The ownership rule, without a device: this is the state machine the
    /// defect needed, and none of it touches the native library.
    #[test]
    fn the_microphone_has_exactly_one_owner_at_a_time() {
        // The owner is process-global, so this test takes its turn with the
        // others that touch it, even though it opens no device.
        let _lock = device_test_lock();
        // Whatever the rest of the process is doing, start from free.
        release(MicrophoneOwner::Check);
        release(MicrophoneOwner::Dictation);
        release(MicrophoneOwner::Voice);
        assert_eq!(current_owner(), MicrophoneOwner::Free);

        claim(MicrophoneOwner::Dictation).expect("the first claim must succeed");
        assert_eq!(current_owner(), MicrophoneOwner::Dictation);
        assert!(is_streaming());

        // A second dictation is running, not broken hardware.
        assert_eq!(
            claim(MicrophoneOwner::Dictation),
            Err(RecorderError::AlreadyRunning)
        );
        // A check in the middle of a dictation is told who holds the device.
        assert_eq!(
            claim(MicrophoneOwner::Check),
            Err(RecorderError::Busy {
                held_by: "a dictation"
            })
        );
        // And so is the voice listener.
        assert_eq!(
            claim(MicrophoneOwner::Voice),
            Err(RecorderError::Busy {
                held_by: "a dictation"
            })
        );

        release(MicrophoneOwner::Dictation);
        assert_eq!(current_owner(), MicrophoneOwner::Free);

        // The voice listener is its own answer: stop listening first.
        claim(MicrophoneOwner::Voice).expect("the listener may take it");
        assert_eq!(
            claim(MicrophoneOwner::Dictation),
            Err(RecorderError::VoiceOwnsMicrophone)
        );
        assert_eq!(
            claim(MicrophoneOwner::Check),
            Err(RecorderError::VoiceOwnsMicrophone)
        );
        release(MicrophoneOwner::Voice);
        assert_eq!(current_owner(), MicrophoneOwner::Free);

        // A repeated release by a non-owner changes nothing.
        release(MicrophoneOwner::Check);
        assert_eq!(current_owner(), MicrophoneOwner::Free);
    }

    /// The exact defect: a read on a stream that was never started. The answer
    /// must name the state, not the device.
    #[test]
    fn reading_a_stream_that_was_never_started_is_an_invalid_state() {
        let _lock = device_test_lock();
        // This test cannot work without a device to initialise.
        if try_audio_devices().is_err() {
            return;
        }
        match init() {
            Ok(_) => {}
            Err(_) => return,
        }
        release(MicrophoneOwner::Dictation);
        // The stream is closed on purpose: this is the state the dictation was
        // in when it reported a broken microphone.
        let _ = try_stop_recording();
        let error = try_read_microphone(&mut [0i16; 512]).unwrap_err();
        assert_eq!(
            error,
            RecorderError::InvalidState("the stream is not open"),
            "a read before a start is a state problem, never a device failure"
        );
        assert_eq!(error.stage(), "state");
    }

    /// A check must leave the device exactly as a dictation needs to find it.
    #[test]
    fn a_check_leaves_the_microphone_ready_for_a_dictation() {
        let _lock = device_test_lock();
        if try_audio_devices().is_err() {
            return;
        }
        if init().is_err() {
            return;
        }
        for _ in 0..3 {
            let check = check_microphone(2).expect("the check must be possible");
            assert!(check.released);
            assert!(!is_streaming());
        }
        // The next dictation reaches START: this is the assertion the defect
        // would have failed.
        let lease = MicrophoneLease::acquire(MicrophoneOwner::Dictation)
            .expect("a dictation must be able to start after a check");
        assert!(is_streaming());
        assert_eq!(lease.owner(), MicrophoneOwner::Dictation);
        drop(lease);
        assert!(!is_streaming(), "the lease must release the device");
    }
}
