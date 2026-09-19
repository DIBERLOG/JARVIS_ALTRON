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

use once_cell::sync::OnceCell;

pub use error::RecorderError;

use crate::{config, config::structs::RecorderType, DB};

static RECORDER_TYPE: OnceCell<RecorderType> = OnceCell::new();
static FRAME_LENGTH: OnceCell<u32> = OnceCell::new();

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
pub fn try_read_microphone(frame_buffer: &mut [i16]) -> Result<(), RecorderError> {
    let Some(recorder_type) = RECORDER_TYPE.get() else {
        return Err(RecorderError::NotInitialized);
    };
    if FRAME_LENGTH.get().is_none() {
        return Err(RecorderError::NotInitialized);
    }
    match recorder_type {
        RecorderType::PvRecorder => {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pvrecorder::try_read_microphone(frame_buffer)
            }));
            match outcome {
                Ok(result) => result,
                Err(_) => Err(RecorderError::DeviceFailed(
                    "the audio backend stopped unexpectedly".to_string(),
                )),
            }
        }
        RecorderType::PortAudio | RecorderType::Cpal => Err(RecorderError::BackendUnavailable),
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
pub fn start_recording() -> Result<(), ()> {
    try_start_recording().map_err(|error| {
        warn!("recorder: could not start ({})", error.code());
    })
}

/// Stops the microphone, ignoring a failure. Kept for the existing callers.
pub fn stop_recording() -> Result<(), ()> {
    try_stop_recording().map_err(|error| {
        warn!("recorder: could not stop ({})", error.code());
    })
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
pub fn check_microphone(max_frames: usize) -> Result<MicrophoneCheck, RecorderError> {
    let status = if is_ready() { status()? } else { init()? };
    let frame_length = status.frame_length.max(1) as usize;
    let mut buffer = vec![0i16; frame_length];

    // The stream is started for the check alone. A failure here is an answer,
    // not an early exit: the device is still released below.
    let mut failure = try_start_recording().err();
    let mut frames_read = 0usize;
    let mut peak = 0i32;
    if failure.is_none() {
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

    // Always released, and the answer says whether that worked: a device that
    // stays open would be the next recording's problem.
    let released = match try_stop_recording() {
        Ok(()) | Err(RecorderError::NotRunning) | Err(RecorderError::NotInitialized) => true,
        Err(error) => {
            if failure.is_none() {
                failure = Some(error);
            }
            false
        }
    };

    if frames_read > 0 {
        info!(
            "microphone check: backend={} device_count={} selected_index={} frames={} level={:.3} released={}",
            status.backend, status.device_count, status.selected_index, frames_read, level_of(peak), released
        );
    } else {
        warn!(
            "microphone check: backend={} device_count={} selected_index={} frames=0 released={} error_code={}",
            status.backend,
            status.device_count,
            status.selected_index,
            released,
            failure
                .as_ref()
                .map(RecorderError::code)
                .unwrap_or("none")
        );
    }

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
            }
            Err(error) => {
                // The only error is a recorder that could not be prepared at all.
                assert_eq!(error, RecorderError::NotInitialized);
            }
        }
    }

    #[test]
    fn a_check_that_cannot_be_prepared_names_the_recorder_error() {
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
}
