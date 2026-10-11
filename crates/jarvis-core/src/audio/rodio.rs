/*
    Abandoned temporary.
    Problems with blocking behaviour.
    Possible fixes are running rodio in a separate thread or smthng.
*/

use once_cell::sync::OnceCell;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

// use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use rodio::{Decoder, Sink};

// static STREAM: OnceCell<OutputStream> = OnceCell::new();
static SINK: OnceCell<Sink> = OnceCell::new();
static INIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn stop() { if let Some(sink) = SINK.get() { sink.clear(); } }
pub fn is_playing() -> bool { SINK.get().is_some_and(|sink| !sink.empty()) }

pub fn init() -> Result<(), ()> {
    let _guard = INIT_LOCK.lock().map_err(|_| ())?;
    if SINK.get().is_some() {
        return Ok(());
    } // already initialized

    // get output stream handle to the default physical sound device
    let (ready, result) = std::sync::mpsc::sync_channel(1);
    // CPAL's macOS OutputStream is !Send: create and retain it on its owner thread.
    std::thread::Builder::new().name("jarvis-rodio".into()).spawn(move || {
    match rodio::OutputStreamBuilder::open_default_stream() {
        Ok(stream_handle) => {
            // create sink
            let sink = Sink::connect_new(&stream_handle.mixer());
            info!("Sink initialized.");

            // store
            // STREAM.set(_stream).unwrap();
            if ready.send(Ok(sink)).is_ok() {
                loop { std::thread::park(); }
            }
        }
        Err(msg) => {
            error!("Failed to initialize audio stream.\nError details: {}", msg);

            // failed
            let _ = ready.send(Err(()));
        }
    }
    }).map_err(|error| { error!("Cannot start audio owner thread: {error}"); })?;
    let sink = result.recv().map_err(|_| ())??;
    SINK.set(sink).map_err(|_| ())
}

pub fn play_sound(filename: &PathBuf, sleep: bool) {
    // Load a sound from a file, using a path relative to Cargo.toml
    // let filepath = format!("{PUBLIC_PATH}/sound/{filename}.wav");
    let file = BufReader::new(File::open(&filename).unwrap());

    // Decode that sound file into a source
    let source = Decoder::new(file).unwrap();

    // Play the sound directly on the device
    // STREAM_HANDLE.get().unwrap().play_raw(source.convert_samples());
    SINK.get().unwrap().append(source);

    if sleep {
        // The sound plays in a separate thread. This call will block the current thread until the sink
        // has finished playing all its queued sounds.
        SINK.get().unwrap().sleep_until_end();
    }
}
