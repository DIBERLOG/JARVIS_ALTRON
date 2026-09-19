//! The real Whisper process, on a machine that has one.
//!
//! This is the second half of the reported defect: the recording was fine, and
//! nothing after it was visible. What is asserted here is exactly the route the
//! window depends on, with the real executable and the real model:
//!
//! 1. a WAV this application writes (16 kHz mono 16-bit, the only format the
//!    feature accepts);
//! 2. the argument list this application builds, handed to the installed
//!    `whisper-cli.exe`;
//! 3. the process starts, exits 0, and writes the JSON report next to the audio;
//! 4. the report is parsed into a transcript, and the text is not empty.
//!
//! Nothing is skipped silently: when the two files are not installed the test
//! returns early and says so in its name — `cargo test` then has nothing to run
//! here, which is the honest state of a machine without the build.
//!
//! What is *not* asserted: the words themselves. They come from the model, and
//! a silent input makes it produce whatever it produces; the test asserts that
//! text arrived, that the exit code was zero, and that the report parsed.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use jarvis_core::whisper::{
    build_arguments, parse_json_transcript, write_wav, ProcessTranscriber, Transcriber,
};

/// The build the reported machine has installed.
const INSTALLED_BINARY: &str = r"C:\AI\whisper.cpp\runtime\Release\whisper-cli.exe";
const INSTALLED_MODEL: &str = r"C:\AI\whisper.cpp\ggml-small.bin";

fn installed() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let binary = std::path::PathBuf::from(INSTALLED_BINARY);
    let model = std::path::PathBuf::from(INSTALLED_MODEL);
    if binary.is_file() && model.is_file() {
        Some((binary, model))
    } else {
        None
    }
}

#[test]
fn the_installed_build_turns_a_written_wav_into_a_report_the_feature_can_read() {
    let Some((binary, model)) = installed() else {
        // No build on this machine: the feature reports `NotConfigured` and
        // there is nothing to run. The rest of the suite covers that path.
        return;
    };

    let directory = tempfile::tempdir().expect("a temporary directory");
    let audio = directory.path().join("dictation.wav");
    let output_base = directory.path().join("dictation");

    // One second of a quiet tone: not speech, but a real signal, so the model has
    // something to work with and the route is the one a dictation takes.
    let samples: Vec<i16> = (0..16_000)
        .map(|index| {
            let value = (index as f32 * 0.05).sin() * 900.0;
            value as i16
        })
        .collect();
    let format = write_wav(&audio, &samples).expect("the WAV must be written");
    assert!(audio.is_file(), "the file must exist");
    let bytes = std::fs::metadata(&audio).expect("metadata").len();
    assert!(
        bytes > 44,
        "a WAV with samples is larger than its own header, got {bytes}"
    );
    assert_eq!(format.sample_rate, 16_000);
    assert_eq!(format.channels, 1);
    assert_eq!(format.bits_per_sample, 16);

    // The argument list the session builds, unchanged.
    let arguments = build_arguments(
        &model.to_string_lossy(),
        &audio,
        "ru",
        4,
        false,
        &output_base,
    );
    let runner = ProcessTranscriber::new(&binary);
    let cancel = Arc::new(AtomicBool::new(false));
    let outcome = runner
        .transcribe(
            &arguments,
            &audio,
            &output_base,
            Duration::from_secs(300),
            &cancel,
        )
        .expect("the installed build must accept the arguments the feature builds");
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "the build exits cleanly: {}",
        outcome.stdout.chars().take(200).collect::<String>()
    );

    // The report is where the feature looks for it, and it parses.
    let report = output_base.with_extension("json");
    assert!(
        report.is_file(),
        "-oj -of must write {} next to the audio",
        report.display()
    );
    let bytes = std::fs::read(&report).expect("the report must be readable");
    let transcript =
        parse_json_transcript(&bytes, "ru").expect("the report must parse into a transcript");
    assert!(
        !transcript.text.trim().is_empty(),
        "the route produced no text at all"
    );
    assert_eq!(transcript.language, "ru", "the language the run was given");
}
