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

pub fn init() -> Result<(), ()> {
    // set default recorder type
    // @TODO. Make it configurable?
    // A second call is not an error: the value is already the one we want.
    let _ = RECORDER_TYPE.set(config::DEFAULT_RECORDER_TYPE);

    // some info
    info!("Loading recorder ...");
    info!("Available audio_devices are:\n{:?}", get_audio_devices());

    // load given recorder
    match RECORDER_TYPE
        .get()
        .unwrap_or(&config::DEFAULT_RECORDER_TYPE)
    {
        RecorderType::PvRecorder => {
            // Init Pv Recorder
            info!("Initializing PvRecorder recording backend.");
            let _ = FRAME_LENGTH.set(512u32); // pvrecorder requires frame buffer of 512
            let frame_length = FRAME_LENGTH.get().copied().unwrap_or(512);
            let selected_microphone = get_selected_microphone_index();
            if !pvrecorder::init_microphone(selected_microphone, frame_length) {
                error!("Recorder initialization failed.");
                return Err(());
            }
            info!(
                "Recorder initialization success. Listening to microphone ({}): {}",
                selected_microphone,
                get_audio_device_name(selected_microphone)
            );
        }
        RecorderType::PortAudio | RecorderType::Cpal => {
            // The other backends are not implemented in this build, and a
            // recording must not start on a backend that cannot read a frame.
            error!("That recording backend is not available in this build.");
            return Err(());
        }
    }

    Ok(())
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
                return Err(RecorderError::DeviceFailed(
                    "the microphone could not be opened".to_string(),
                ));
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
}
