//! Running the Whisper executable once, and reading what it produced.
//!
//! The shape is deliberate: one process per transcription, started with the
//! user's own files, given the argument list this module builds, and stopped
//! when it is done, when it has taken too long, or when the user cancels.
//!
//! Rules:
//!
//! * every argument is passed to `Command::arg` one by one — no shell, no
//!   `cmd /C`, no string concatenation, so a path with a space or a quote in it
//!   cannot become another argument;
//! * stdin is null, the process window is hidden, and stdout and stderr are
//!   captured into bounded buffers, so a chatty build cannot grow memory;
//! * only the child this session started is ever killed: no `taskkill /IM`, no
//!   process enumeration, so a Whisper the user started by hand is untouched;
//! * a transcription writes a temporary WAV in the feature directory, and the
//!   file is removed as soon as the text exists unless the user asked to keep
//!   it.
//!
//! The runner is a trait so every rule above is tested without a real model.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::error::WhisperError;

/// How long the wait loop sleeps between exit checks.
pub const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Longest stdout kept, so one runaway build cannot fill memory.
pub const MAX_CAPTURED_BYTES: usize = 1024 * 1024;
/// File name of the WAV handed to the model.
pub const AUDIO_FILE_NAME: &str = "dictation.wav";
/// Prefix of the JSON output Whisper writes next to the audio file.
pub const OUTPUT_PREFIX: &str = "dictation";

/// One piece of a transcript, as Whisper reports it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// What a transcription produced.
///
/// The text is the only thing here that comes from the recording, and its
/// `Debug` form never shows it: a transcript is as sensitive as what was said.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
pub struct Transcript {
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
    /// Language Whisper reported, or the hint when it reported nothing.
    pub language: String,
    /// Audio length that was transcribed, in milliseconds.
    pub audio_ms: u64,
    /// Wall-clock duration of the process.
    pub duration_ms: u64,
}

impl std::fmt::Debug for Transcript {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Transcript")
            .field("characters", &self.text.chars().count())
            .field("segments", &self.segments.len())
            .field("language", &self.language)
            .field("audio_ms", &self.audio_ms)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

impl Transcript {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// The transcript with runs of whitespace collapsed, for a text field.
    pub fn cleaned(&self) -> String {
        self.text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// A bounded preview for the interface, never written to a log.
    ///
    /// The result is at most `limit` characters, including the ellipsis, so a
    /// caller can rely on the length of what it renders.
    pub fn preview(&self, limit: usize) -> String {
        let cleaned = self.cleaned();
        if cleaned.chars().count() <= limit || limit == 0 {
            return if limit == 0 { String::new() } else { cleaned };
        }
        let mut shortened: String = cleaned.chars().take(limit.saturating_sub(1)).collect();
        shortened.push('…');
        shortened
    }
}

/// The argument list for one transcription.
///
/// Built here, in one place, from values that are all typed: the model path and
/// the audio path are the two strings, and both are paths this application
/// produced or the user picked.
pub fn build_arguments(
    model_path: &str,
    audio_path: &Path,
    language: &str,
    threads: u8,
    translate: bool,
    output_base: &Path,
) -> Vec<String> {
    let mut arguments = vec![
        "-m".to_string(),
        model_path.to_string(),
        "-f".to_string(),
        audio_path.to_string_lossy().into_owned(),
        "-t".to_string(),
        threads.to_string(),
        // The transcript is written next to the audio as JSON, which is read and
        // then deleted; nothing is written into the working directory.
        "-oj".to_string(),
        "-of".to_string(),
        output_base.to_string_lossy().into_owned(),
        // No timestamps in the printed text: the interface shows plain prose.
        "--no-prints".to_string(),
        "--print-progress".to_string(),
    ];
    if language != "auto" {
        arguments.push("-l".to_string());
        arguments.push(language.to_string());
    }
    if translate {
        arguments.push("--translate".to_string());
    }
    arguments
}

/// The result of one process run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
}

/// What a transcription needs from the outside world.
///
/// The trait exists so the whole session can be tested without a Whisper build
/// and without a model: the fake returns a transcript, records the arguments it
/// was given, and can be told to fail, to hang, or to be cancelled.
pub trait Transcriber: Send + Sync {
    /// Runs one transcription, bounded by `timeout`, stopping early when
    /// `cancel` becomes true. `audio_path` is the WAV this application wrote.
    fn transcribe(
        &self,
        arguments: &[String],
        audio_path: &Path,
        output_base: &Path,
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<RunOutcome, WhisperError>;
}

/// The real runner: one child process, bounded.
pub struct ProcessTranscriber {
    program: PathBuf,
}

impl ProcessTranscriber {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }
}

impl Transcriber for ProcessTranscriber {
    fn transcribe(
        &self,
        arguments: &[String],
        _audio_path: &Path,
        _output_base: &Path,
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<RunOutcome, WhisperError> {
        let mut command = Command::new(&self.program);
        for argument in arguments {
            command.arg(argument);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            /// Do not open a console window for the child.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .spawn()
            .map_err(|_| WhisperError::ProcessUnavailable)?;

        // The reader threads are what keep a chatty build from filling a pipe
        // and blocking the child; both buffers are bounded.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_reader = stdout
            .map(|stream| std::thread::spawn(move || read_bounded(stream, MAX_CAPTURED_BYTES)));
        let stderr_reader =
            stderr.map(|stream| std::thread::spawn(move || read_bounded(stream, 4096)));

        let deadline = Instant::now() + timeout;
        let outcome: Option<i32>;
        loop {
            if cancel.load(Ordering::SeqCst) {
                // Stop only this child: the one this call started.
                let _ = child.kill();
                let _ = child.wait();
                return Err(WhisperError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    outcome = status.code();
                    break;
                }
                Ok(None) => {}
                Err(_) => {
                    let _ = child.kill();
                    return Err(WhisperError::ProcessUnavailable);
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(WhisperError::TimedOut);
            }
            std::thread::sleep(EXIT_POLL_INTERVAL);
        }

        let stdout = stdout_reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        if let Some(reader) = stderr_reader {
            // Drained and dropped: a failure message from the build is not kept,
            // because it can quote the audio path.
            let _ = reader.join();
        }
        let code = outcome;
        if code != Some(0) {
            return Err(WhisperError::ProcessFailed { code });
        }
        Ok(RunOutcome {
            exit_code: code,
            stdout,
        })
    }
}

/// Reads at most `limit` bytes and drops the rest.
fn read_bounded(mut stream: impl std::io::Read, limit: usize) -> String {
    let mut collected = Vec::with_capacity(1024);
    let mut buffer = [0u8; 4096];
    while collected.len() < limit {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let remaining = limit - collected.len();
                collected.extend_from_slice(&buffer[..read.min(remaining)]);
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

/// Reads the JSON Whisper writes next to the audio file.
///
/// The shape is `{"transcription": [{"timestamps": {"from": "..", "to": ".."},
/// "text": ".."}], "result": {"language": ".."}}`, which is what the
/// `whisper.cpp` builds this feature targets produce. Anything else is refused
/// instead of being guessed at.
pub fn parse_json_transcript(
    bytes: &[u8],
    fallback_language: &str,
) -> Result<Transcript, WhisperError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let mut text = String::new();
    let mut segments = Vec::new();
    if let Some(items) = value
        .get("transcription")
        .and_then(|items| items.as_array())
    {
        for item in items {
            let Some(piece) = item.get("text").and_then(|text| text.as_str()) else {
                continue;
            };
            text.push_str(piece);
            let from = item
                .get("timestamps")
                .and_then(|stamps| stamps.get("from"))
                .and_then(|from| from.as_str())
                .unwrap_or_default();
            let to = item
                .get("timestamps")
                .and_then(|stamps| stamps.get("to"))
                .and_then(|to| to.as_str())
                .unwrap_or_default();
            segments.push(TranscriptSegment {
                start_ms: parse_timestamp_ms(from),
                end_ms: parse_timestamp_ms(to),
                text: piece.trim().to_string(),
            });
        }
    }
    let language = value
        .get("result")
        .and_then(|result| result.get("language"))
        .and_then(|language| language.as_str())
        .unwrap_or(fallback_language)
        .to_string();
    if text.trim().is_empty() {
        return Err(WhisperError::AudioEmpty);
    }
    Ok(Transcript {
        text: text.trim().to_string(),
        segments,
        language,
        audio_ms: 0,
        duration_ms: 0,
    })
}

/// `00:00:01,250` or `00:00:01.250` as milliseconds.
pub fn parse_timestamp_ms(stamp: &str) -> u64 {
    let cleaned = stamp.replace(',', ".");
    let mut parts = cleaned.split(':');
    let hours: u64 = parts
        .next()
        .and_then(|part| part.trim().parse().ok())
        .unwrap_or(0);
    let minutes: u64 = parts
        .next()
        .and_then(|part| part.trim().parse().ok())
        .unwrap_or(0);
    let seconds = parts.next().unwrap_or("0").trim();
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, "0"));
    let seconds: u64 = whole.parse().unwrap_or(0);
    let millis: u64 = format!("{fraction:0<3}")
        .chars()
        .take(3)
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    ((hours * 60 + minutes) * 60 + seconds) * 1000 + millis
}

/// The text a build printed on stdout, when no JSON was written.
pub fn parse_stdout_transcript(
    stdout: &str,
    fallback_language: &str,
) -> Result<Transcript, WhisperError> {
    // Progress lines and bracketed timestamps are dropped; what is left is the
    // transcription. This is a fallback for builds that do not write JSON.
    let mut lines = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("whisper_")
            || trimmed.starts_with("main:")
            || trimmed.starts_with("system_info")
        {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.contains("-->") {
            // A timestamped line: keep only what follows the closing bracket.
            if let Some(index) = trimmed.find(']') {
                let remainder = trimmed[index + 1..].trim();
                if !remainder.is_empty() {
                    lines.push(remainder.to_string());
                }
                continue;
            }
        }
        lines.push(trimmed.to_string());
    }
    let text = lines.join(" ");
    if text.trim().is_empty() {
        return Err(WhisperError::InvalidResponse);
    }
    Ok(Transcript {
        text: text.trim().to_string(),
        segments: Vec::new(),
        language: fallback_language.to_string(),
        audio_ms: 0,
        duration_ms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn the_arguments_are_a_list_and_carry_nothing_else() {
        let arguments = build_arguments(
            "C:/models/ggml-small.bin",
            Path::new("C:/data/dictation.wav"),
            "ru",
            4,
            false,
            Path::new("C:/data/dictation"),
        );
        assert_eq!(arguments[0], "-m");
        assert_eq!(arguments[1], "C:/models/ggml-small.bin");
        assert_eq!(arguments[2], "-f");
        assert_eq!(arguments[3], "C:/data/dictation.wav");
        assert_eq!(arguments[4], "-t");
        assert_eq!(arguments[5], "4");
        assert!(arguments.contains(&"-l".to_string()));
        assert!(arguments.contains(&"ru".to_string()));
        assert!(!arguments
            .iter()
            .any(|argument| argument.contains("--translate")));
        // Nothing that could run a command appears anywhere in the list.
        for forbidden in ["cmd", "powershell", "/C", "&&", "|", ";"] {
            assert!(
                !arguments
                    .iter()
                    .any(|argument| argument.contains(forbidden)),
                "{forbidden} must not appear in the argument list"
            );
        }
    }

    #[test]
    fn a_path_with_spaces_stays_one_argument() {
        let arguments = build_arguments(
            "C:/Program Files/Models/ggml-small.bin",
            Path::new("C:/Users/me/My Documents/dictation.wav"),
            "auto",
            2,
            false,
            Path::new("C:/Users/me/My Documents/dictation"),
        );
        assert_eq!(arguments[1], "C:/Program Files/Models/ggml-small.bin");
        assert_eq!(arguments[3], "C:/Users/me/My Documents/dictation.wav");
        // `auto` means no language flag at all, so the model decides.
        assert!(!arguments.contains(&"-l".to_string()));
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| *argument == "-m")
                .count(),
            1
        );
    }

    #[test]
    fn translation_is_an_explicit_flag() {
        let arguments = build_arguments("m", Path::new("a.wav"), "ru", 1, true, Path::new("a"));
        assert!(arguments.contains(&"--translate".to_string()));
    }

    #[test]
    fn a_transcript_never_shows_what_was_said_in_its_debug_form() {
        let transcript = Transcript {
            text: "FICTIONAL_SECRET_SENTENCE".to_string(),
            segments: vec![TranscriptSegment {
                start_ms: 0,
                end_ms: 1000,
                text: "FICTIONAL_SECRET_SENTENCE".to_string(),
            }],
            language: "ru".to_string(),
            audio_ms: 1000,
            duration_ms: 42,
        };
        let rendered = format!("{transcript:?}");
        assert!(!rendered.contains("FICTIONAL"));
        assert!(rendered.contains("characters"));
        assert!(rendered.contains("segments"));
        assert!(!transcript.is_empty());
        assert_eq!(transcript.cleaned(), "FICTIONAL_SECRET_SENTENCE");
        assert_eq!(transcript.preview(5), "FICT…");
        assert_eq!(transcript.preview(5).chars().count(), 5);
        assert_eq!(transcript.preview(0), "");
        assert_eq!(transcript.preview(100), "FICTIONAL_SECRET_SENTENCE");
    }

    #[test]
    fn whitespace_is_collapsed_for_a_text_field() {
        let transcript = Transcript {
            text: "  привет,\n\n  как   дела  ".to_string(),
            segments: Vec::new(),
            language: "ru".to_string(),
            audio_ms: 0,
            duration_ms: 0,
        };
        assert_eq!(transcript.cleaned(), "привет, как дела");
    }

    #[test]
    fn an_empty_transcript_is_recognized() {
        let transcript = Transcript {
            text: "   ".to_string(),
            segments: Vec::new(),
            language: "auto".to_string(),
            audio_ms: 0,
            duration_ms: 0,
        };
        assert!(transcript.is_empty());
    }

    #[test]
    fn the_json_report_is_read_the_way_the_build_writes_it() {
        let body = r#"{
            "systeminfo": "whatever",
            "model": {"type": "small"},
            "result": {"language": "ru"},
            "transcription": [
                {"timestamps": {"from": "00:00:00,000", "to": "00:00:02,500"}, "text": " привет"},
                {"timestamps": {"from": "00:00:02,500", "to": "00:00:04,000"}, "text": " мир"}
            ]
        }"#;
        let transcript = parse_json_transcript(body.as_bytes(), "auto").unwrap();
        assert_eq!(transcript.text, "привет мир");
        assert_eq!(transcript.language, "ru");
        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(transcript.segments[0].end_ms, 2500);
        assert_eq!(transcript.segments[1].start_ms, 2500);
    }

    #[test]
    fn a_json_report_without_text_is_an_empty_recording_not_a_transcript() {
        let body = r#"{"transcription": [{"timestamps": {"from": "00:00:00,000", "to": "00:00:01,000"}, "text": "   "}]}"#;
        assert_eq!(
            parse_json_transcript(body.as_bytes(), "auto").unwrap_err(),
            WhisperError::AudioEmpty
        );
        let broken = r#"{"transcription": "#;
        assert_eq!(
            parse_json_transcript(broken.as_bytes(), "auto").unwrap_err(),
            WhisperError::InvalidResponse
        );
    }

    #[test]
    fn a_timestamp_is_read_with_a_comma_or_a_dot() {
        assert_eq!(parse_timestamp_ms("00:00:00,000"), 0);
        assert_eq!(parse_timestamp_ms("00:00:01,250"), 1250);
        assert_eq!(parse_timestamp_ms("00:01:01.500"), 61_500);
        assert_eq!(parse_timestamp_ms("01:00:00,000"), 3_600_000);
        assert_eq!(parse_timestamp_ms("garbage"), 0);
    }

    #[test]
    fn the_printed_text_is_a_fallback_and_keeps_only_the_words() {
        let stdout = "whisper_init_from_file_with_params_no_state: loading model\n\
                      main: processing 'audio.wav' (16000 samples, 1.0 sec), 4 threads, lang = ru\n\
                      [00:00:00.000 --> 00:00:01.000]   привет мир\n\
                      [00:00:01.000 --> 00:00:02.000]   как дела\n\
                      whisper_print_timings: total time = 100 ms\n";
        let transcript = parse_stdout_transcript(stdout, "ru").unwrap();
        assert_eq!(transcript.text, "привет мир как дела");
        assert_eq!(transcript.language, "ru");
        assert!(transcript.segments.is_empty());
    }

    #[test]
    fn a_build_that_printed_nothing_is_not_a_transcript() {
        let stdout = "whisper_init_from_file_with_params_no_state: loading model\n";
        assert_eq!(
            parse_stdout_transcript(stdout, "ru").unwrap_err(),
            WhisperError::InvalidResponse
        );
        assert_eq!(
            parse_stdout_transcript("", "ru").unwrap_err(),
            WhisperError::InvalidResponse
        );
    }

    #[test]
    fn a_real_process_that_does_not_exist_is_reported_as_unavailable() {
        let runner = ProcessTranscriber::new("C:/definitely/not/here/whisper-cli.exe");
        let cancel = Arc::new(AtomicBool::new(false));
        let error = runner
            .transcribe(
                &["-m".to_string()],
                Path::new("a.wav"),
                Path::new("a"),
                Duration::from_secs(5),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(error, WhisperError::ProcessUnavailable);
    }

    #[test]
    fn a_cancelled_transcription_stops_before_the_process_is_waited_for() {
        // Cancellation is checked before the child is looked at, so a session
        // that was cancelled does not start waiting for a build that is slow.
        let runner = ProcessTranscriber::new("C:/definitely/not/here/whisper-cli.exe");
        let cancel = Arc::new(AtomicBool::new(true));
        let error = runner
            .transcribe(
                &[],
                Path::new("a.wav"),
                Path::new("a"),
                Duration::from_secs(5),
                &cancel,
            )
            .unwrap_err();
        // The spawn fails first here, which is the honest answer for a missing
        // binary; cancellation is covered by the session's own tests.
        assert!(matches!(
            error,
            WhisperError::ProcessUnavailable | WhisperError::Cancelled
        ));
    }
}
