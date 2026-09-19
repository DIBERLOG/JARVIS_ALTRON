use once_cell::sync::OnceCell;
use pv_recorder::{PvRecorder, PvRecorderBuilder};
use std::sync::atomic::{AtomicBool, Ordering};

use super::RecorderError;

static RECORDER: OnceCell<PvRecorder> = OnceCell::new();
static IS_RECORDING: AtomicBool = AtomicBool::new(false);

pub fn init_microphone(device_index: i32, frame_length: u32) -> bool {
    if RECORDER.get().is_some() {
        return true; // already initialized
    }

    // initialize
    let pv_recorder = PvRecorderBuilder::new(frame_length as i32)
        .device_index(device_index)
        // .frame_length(frame_length as i32)
        .init();

    match pv_recorder {
        Ok(pv) => {
            // store
            let _ = RECORDER.set(pv);

            // success
            true
        }
        Err(msg) => {
            error!("Failed to initialize pvrecorder.\nError details: {:?}", msg);

            // fail
            false
        }
    }
}

/// Reads one frame, reporting why it could not be read.
///
/// The original body silently did nothing when the microphone was not open,
/// which is what made the failure appear one level up as a panic. It is now a
/// reason, and a short frame leaves the rest of the buffer silent instead of
/// repeating the previous frame's audio.
pub fn try_read_microphone(frame_buffer: &mut [i16]) -> Result<(), RecorderError> {
    let Some(recorder) = RECORDER.get() else {
        return Err(RecorderError::NotInitialized);
    };
    match recorder.read() {
        Ok(frame) => {
            let samples = frame.as_slice();
            let usable = samples.len().min(frame_buffer.len());
            frame_buffer[..usable].copy_from_slice(&samples[..usable]);
            frame_buffer[usable..].fill(0);
            Ok(())
        }
        Err(message) => Err(RecorderError::DeviceFailed(shorten(&message.to_string()))),
    }
}

pub fn read_microphone(frame_buffer: &mut [i16]) {
    if try_read_microphone(frame_buffer).is_err() {
        frame_buffer.fill(0);
    }
}

#[allow(dead_code)]
fn read_microphone_original(frame_buffer: &mut [i16]) {
    // ensure microphone is initialized
    if RECORDER.get().is_some() {
        // read to frame buffer

        let frame = RECORDER.get().unwrap().read();

        match frame {
            Ok(f) => {
                frame_buffer.copy_from_slice(f.as_slice());
            }
            Err(msg) => {
                // @TODO: Fix? PvRecorder always wait for PCM buffer size of 512.
                error!("Failed to read audio frame. {:?}", msg);
            }
        }
    }
}

pub fn start_recording(device_index: i32, frame_length: u32) -> Result<(), ()> {
    // ensure microphone is initialized
    init_microphone(device_index, frame_length);

    // start recording
    let Some(recorder) = RECORDER.get() else {
        // The microphone could not be opened, so there is nothing to start.
        return Err(());
    };
    if IS_RECORDING.load(Ordering::SeqCst) {
        return Err(());
    }
    match recorder.start() {
        Ok(_) => {
            info!("START recording from microphone ...");

            // change recording state
            IS_RECORDING.store(true, Ordering::SeqCst);

            // success
            Ok(())
        }
        Err(msg) => {
            error!("Failed to START audio recording: {}", msg);

            // fail
            Err(())
        }
    }
}

/// Starts the microphone, reporting why it could not start.
///
/// The unwrap that used to be here panicked on a machine whose microphone could
/// not be opened; the failure is now a value.
pub fn try_start_recording() -> Result<(), RecorderError> {
    if RECORDER.get().is_none() {
        return Err(RecorderError::DeviceFailed(
            "the microphone could not be opened".to_string(),
        ));
    }
    if IS_RECORDING.load(Ordering::SeqCst) {
        return Err(RecorderError::AlreadyRunning);
    }
    let Some(recorder) = RECORDER.get() else {
        return Err(RecorderError::NotInitialized);
    };
    match recorder.start() {
        Ok(_) => {
            info!("START recording from microphone ...");
            IS_RECORDING.store(true, Ordering::SeqCst);
            Ok(())
        }
        Err(message) => Err(RecorderError::DeviceFailed(shorten(&message.to_string()))),
    }
}

/// Stops the microphone, reporting why it could not stop.
pub fn try_stop_recording() -> Result<(), RecorderError> {
    let Some(recorder) = RECORDER.get() else {
        return Err(RecorderError::NotInitialized);
    };
    if !IS_RECORDING.load(Ordering::SeqCst) {
        return Err(RecorderError::NotRunning);
    }
    match recorder.stop() {
        Ok(()) => {
            info!("STOP recording from microphone ...");
            IS_RECORDING.store(false, Ordering::SeqCst);
            Ok(())
        }
        Err(message) => Err(RecorderError::DeviceFailed(shorten(&message.to_string()))),
    }
}

/// A bounded, content-free reason from the native library.
fn shorten(message: &str) -> String {
    crate::text::shorten(message, 120)
}

pub fn stop_recording() -> Result<(), ()> {
    // ensure microphone is initialized & recording is in process
    if RECORDER.get().is_some() && IS_RECORDING.load(Ordering::SeqCst) {
        // stop recording
        match RECORDER.get().unwrap().stop() {
            Ok(_) => {
                info!("STOP recording from microphone ...");

                // change recording state
                IS_RECORDING.store(false, Ordering::SeqCst);

                // success
                return Ok(());
            }
            Err(msg) => {
                error!("Failed to STOP audio recording: {}", msg);

                // fail
                return Err(());
            }
        }
    }

    Ok(()) // if already stopped or not yet initialized
}

pub fn list_audio_devices() -> Vec<String> {
    let audio_devices = PvRecorderBuilder::default().get_available_devices();
    match audio_devices {
        Ok(audio_devices) => audio_devices,
        Err(err) => {
            error!("Failed to get audio devices: {}", err);
            Vec::new()
        }
    }
}

pub fn get_audio_device_name(idx: i32) -> String {
    if idx == -1 {
        return String::from("System Default");
    }

    let audio_devices = list_audio_devices();
    let mut first_device: String = String::new();

    for (_idx, device) in audio_devices.iter().enumerate() {
        if idx as usize == _idx {
            return device.to_string();
        }

        if _idx == 0 {
            first_device = device.to_string()
        }
    }

    // return first device as default, if none were matched
    first_device
}
