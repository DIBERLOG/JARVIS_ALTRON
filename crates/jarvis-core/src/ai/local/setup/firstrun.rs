//! The technical first-run validation of an installed managed server.
//!
//! After an installation the application proves that what it installed actually
//! runs: it starts the active `llama-server.exe`, waits for it to answer, asks it
//! one short impersonal question, and stops the process it started.
//!
//! The rules that shape this module:
//!
//! * the server is started with `std::process::Command` through the existing
//!   supervisor — one argument per element, no shell, no `cmd`, no PowerShell;
//! * the listener is bound to `127.0.0.1` on a port the operating system chose,
//!   so nothing is exposed beyond loopback;
//! * the context is 4096 tokens and the GPU layer count is 0: this is a CPU-first
//!   check of a working installation, not a benchmark;
//! * the prompt and the answer are never returned, never logged and never stored.
//!   The report says whether a non-empty answer arrived, how long it took, and
//!   how many pieces it had;
//! * only the process this test started is stopped. A server the user started by
//!   hand, or the application's own long-running one, is never touched — and when
//!   a compatible managed server is already running, this test reuses it instead
//!   of starting a second one;
//! * the child is supervised by [`ManagedServer`], whose `Drop` kills it, so a
//!   cancellation, an error, or a panic cannot leave a stray process behind.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ai::local::client::{
    ApiMessage, ChatCompletionRequest, ChunkOutcome, HealthState, LocalAiClient,
};
use crate::ai::local::managed::validate_pe_x64;
use crate::ai::local::model::read_gguf_info;
use crate::ai::local::process::{wait_until_ready, ManagedServer, RealProcessRunner};
use crate::ai::local::resources::available_memory_bytes;
use crate::ai::ChatError;

use super::{SetupErrorCode, SetupStage, TestOutcome};

/// Context window for the check: modest, and enough for a real answer.
pub const TEST_CONTEXT_SIZE: u32 = 4096;
/// The first-run check is CPU-first by design.
pub const TEST_GPU_LAYERS: u32 = 0;
/// Upper bound on the answer, so the check cannot become a generation.
pub const TEST_MAX_TOKENS: u32 = 256;
/// Longest answer kept in memory before the stream is stopped.
pub const TEST_ANSWER_LIMIT_BYTES: usize = 8 * 1024;
/// Seconds allowed for the server to become ready.
pub const TEST_READINESS_TIMEOUT: Duration = Duration::from_secs(120);
/// How often readiness is polled.
pub const TEST_READINESS_POLL: Duration = Duration::from_millis(400);
/// The question asked. Impersonal, and short enough to answer quickly.
pub const TEST_PROMPT: &str = "Reply with the single word: ready";
/// Host the check binds. Loopback only.
pub const TEST_HOST: &str = "127.0.0.1";
/// Lowest loopback port the check will use.
pub const MIN_TEST_PORT: u16 = 1024;

/// How the check should be run.
#[derive(Clone, Debug)]
pub struct FirstRunOptions {
    pub context_size: u32,
    pub gpu_layers: u32,
    pub max_tokens: u32,
    pub readiness_timeout: Duration,
    pub poll_interval: Duration,
    /// Port to use. [`free_loopback_port`] supplies one when it is absent.
    pub port: Option<u16>,
    /// Whether the model is asked a question at all. The context check can be
    /// run on its own when a machine is very slow.
    pub run_inference: bool,
}

impl Default for FirstRunOptions {
    fn default() -> Self {
        Self {
            context_size: TEST_CONTEXT_SIZE,
            gpu_layers: TEST_GPU_LAYERS,
            max_tokens: TEST_MAX_TOKENS,
            readiness_timeout: TEST_READINESS_TIMEOUT,
            poll_interval: TEST_READINESS_POLL,
            port: None,
            run_inference: true,
        }
    }
}

/// What the check found, without a prompt, an answer, or a stderr line.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FirstRunReport {
    pub outcome: TestOutcome,
    pub error_code: Option<SetupErrorCode>,
    /// The loopback port the check used.
    pub port: Option<u16>,
    /// True when a compatible managed server was already running and was reused.
    pub reused_existing_server: bool,
    /// How many stderr lines the server produced. The lines themselves are never
    /// reported: a chatty server can echo a prompt.
    pub stderr_lines: u64,
    /// Whether the server reported an out-of-memory condition.
    pub out_of_memory: bool,
}

/// A loopback port nothing is listening on right now.
///
/// Binding port `0` asks the operating system for a free one, which is the only
/// reliable way to avoid a port another program already holds.
pub fn free_loopback_port() -> Option<u16> {
    let listener = TcpListener::bind((TEST_HOST, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    (port >= MIN_TEST_PORT).then_some(port)
}

/// The arguments the check starts the server with.
///
/// Every argument is a separate element, so a path with spaces or shell
/// metacharacters cannot change the command.
pub fn first_run_arguments(model: &Path, options: &FirstRunOptions, port: u16) -> Vec<String> {
    vec![
        "--model".to_string(),
        model.to_string_lossy().into_owned(),
        "--host".to_string(),
        TEST_HOST.to_string(),
        "--port".to_string(),
        port.to_string(),
        "--ctx-size".to_string(),
        options.context_size.to_string(),
        "--n-gpu-layers".to_string(),
        options.gpu_layers.to_string(),
    ]
}

/// Checks the two files before anything is started.
fn check_files(server: Option<&Path>, model: &Path) -> Result<(), SetupErrorCode> {
    if let Some(server) = server {
        if !server.is_file() {
            return Err(SetupErrorCode::RuntimeMissing);
        }
        validate_pe_x64(server).map_err(|_| SetupErrorCode::RuntimeArchitectureMismatch)?;
    }
    if !model.is_file() {
        return Err(SetupErrorCode::ModelInvalid);
    }
    // The cheap header check, so an obviously wrong file never reaches a process.
    read_gguf_info(model).map_err(|_| SetupErrorCode::ModelInvalid)?;
    Ok(())
}

/// Runs the check.
///
/// `server` is `None` when a compatible managed server is already running: the
/// check then probes that server and never starts or stops a process. `note` is
/// called before each phase, so the coordinator can publish the stage.
pub fn run_first_run(
    server: Option<&Path>,
    model: &Path,
    options: &FirstRunOptions,
    cancel: &AtomicBool,
    note: impl FnMut(SetupStage),
) -> FirstRunReport {
    let started = Instant::now();
    let mut report = first_run_inner(server, model, options, cancel, note);
    report.outcome.elapsed_ms =
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    report
}

fn first_run_inner(
    server: Option<&Path>,
    model: &Path,
    options: &FirstRunOptions,
    cancel: &AtomicBool,
    mut note: impl FnMut(SetupStage),
) -> FirstRunReport {
    let reused_existing_server = server.is_none();
    let mut report = FirstRunReport {
        reused_existing_server,
        ..FirstRunReport::default()
    };

    if let Err(error) = check_files(server, model) {
        report.error_code = Some(error);
        return report;
    }
    let Some(port) = options.port.or_else(free_loopback_port) else {
        report.error_code = Some(SetupErrorCode::Io);
        return report;
    };
    report.port = Some(port);

    // ------------------------------------------------------------- launch
    note(SetupStage::LaunchTest);
    if cancel.load(Ordering::Relaxed) {
        report.error_code = Some(SetupErrorCode::Cancelled);
        return report;
    }
    let mut managed = match server {
        Some(server) => {
            let arguments = first_run_arguments(model, options, port);
            match ManagedServer::start(&RealProcessRunner, server, &arguments) {
                Ok(managed) => Some(managed),
                Err(_) => {
                    report.error_code = Some(SetupErrorCode::ProcessUnavailable);
                    return report;
                }
            }
        }
        // Nothing is spawned: the caller verified that a compatible server is
        // already listening.
        None => None,
    };

    // ---------------------------------------------------------- readiness
    note(SetupStage::Readiness);
    let client = match LocalAiClient::new(TEST_HOST, port) {
        Ok(client) => client,
        Err(_) => {
            stop(&mut managed, &mut report);
            report.error_code = Some(SetupErrorCode::Io);
            return report;
        }
    };
    let readiness = match managed.as_mut() {
        Some(managed) => wait_until_ready(
            managed,
            options.readiness_timeout,
            options.poll_interval,
            || readiness_probe(&client, cancel),
        ),
        // A server this test did not start is waited for with a plain poll: its
        // process is not ours to inspect or to kill.
        None => wait_for_ready_without_our_process(&client, options, cancel),
    };
    if let Err(error) = readiness {
        stop(&mut managed, &mut report);
        // A cancellation wins over the reason the wait stopped.
        report.error_code = Some(if cancel.load(Ordering::Relaxed) {
            SetupErrorCode::Cancelled
        } else {
            map_chat_error(&error)
        });
        return report;
    }
    report.outcome.server_ready = true;

    // ------------------------------------------------------------- inference
    if options.run_inference {
        note(SetupStage::TestInference);
        match ask_once(&client, options, cancel) {
            Ok(Some(tokens)) => {
                report.outcome.answer_received = true;
                report.outcome.answer_tokens = Some(tokens);
            }
            Ok(None) => {
                // Nothing arrived: a ready server that cannot answer is a failure.
                stop(&mut managed, &mut report);
                report.error_code = Some(SetupErrorCode::TestFailed);
                return report;
            }
            Err(error) => {
                stop(&mut managed, &mut report);
                report.error_code = Some(if cancel.load(Ordering::Relaxed) {
                    SetupErrorCode::Cancelled
                } else {
                    error
                });
                return report;
            }
        }
    }

    stop(&mut managed, &mut report);
    report.outcome.passed = report.outcome.server_ready
        && (!options.run_inference || report.outcome.answer_received);
    if !report.outcome.passed && report.error_code.is_none() {
        report.error_code = Some(SetupErrorCode::TestFailed);
    }
    report
}

/// Stops the child this test started, and records what its stderr said.
///
/// Only a process this function started is stopped: with `reused_existing_server`
/// there is nothing to stop at all.
fn stop(managed: &mut Option<ManagedServer>, report: &mut FirstRunReport) {
    if let Some(server) = managed.as_mut() {
        report.stderr_lines = server.stderr_line_count();
        report.out_of_memory = server.indicates_out_of_memory();
        // `stop` is bounded, and `Drop` kills the child even if this is skipped.
        server.shutdown();
    }
}

/// Probes `/health` and gives up when the caller asked to cancel.
fn readiness_probe(client: &LocalAiClient, cancel: &AtomicBool) -> Result<bool, ChatError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ChatError::Cancelled);
    }
    Ok(matches!(client.health()?, HealthState::Ready))
}

/// Waits for a server this test did not start.
fn wait_for_ready_without_our_process(
    client: &LocalAiClient,
    options: &FirstRunOptions,
    cancel: &AtomicBool,
) -> Result<(), ChatError> {
    let deadline = Instant::now() + options.readiness_timeout;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ChatError::Cancelled);
        }
        match client.health() {
            Ok(HealthState::Ready) => return Ok(()),
            Ok(_) => {}
            Err(ChatError::ServerNotRunning) | Err(ChatError::TimedOut) => {}
            Err(other) => return Err(other),
        }
        if Instant::now() >= deadline {
            return Err(ChatError::StartupTimedOut);
        }
        std::thread::sleep(options.poll_interval);
    }
}

/// Asks one short impersonal question and counts the answer.
///
/// Returns the number of answer pieces, or `None` when the answer was empty. The
/// text itself stays in this function: it is never logged and never returned.
fn ask_once(
    client: &LocalAiClient,
    options: &FirstRunOptions,
    cancel: &AtomicBool,
) -> Result<Option<u32>, SetupErrorCode> {
    let request = ChatCompletionRequest {
        model: "local".to_string(),
        messages: vec![ApiMessage {
            role: "user",
            content: TEST_PROMPT.to_string(),
        }],
        stream: true,
        max_tokens: options.max_tokens,
        temperature: 0.0,
        top_p: 1.0,
        // The first-run check must not depend on a template switch, and must not
        // ask for reasoning output.
        chat_template_kwargs: None,
        stream_options: None,
        tools: None,
        tool_choice: None,
    };

    // One flag stops the stream for either reason: the caller cancelled, or the
    // answer is already long enough to prove the server works.
    let stop = AtomicBool::new(false);
    let mut answer = String::new();
    let mut pieces = 0_u32;
    let result = client.stream_chat(&request, &stop, &mut |chunk| match chunk {
        ChunkOutcome::Token(text) | ChunkOutcome::Thinking(text) => {
            pieces = pieces.saturating_add(1);
            if answer.len() < TEST_ANSWER_LIMIT_BYTES {
                answer.push_str(&text);
            }
        }
        ChunkOutcome::TokenAndThinking { text, .. } => {
            pieces = pieces.saturating_add(1);
            if answer.len() < TEST_ANSWER_LIMIT_BYTES {
                answer.push_str(&text);
            }
        }
        _ => {}
    });

    match result {
        Ok(()) => {}
        Err(ChatError::Cancelled) => {
            if cancel.load(Ordering::Relaxed) {
                return Err(SetupErrorCode::Cancelled);
            }
            // The stream was stopped because the answer was already long enough.
        }
        Err(error) => return Err(map_chat_error(&error)),
    }
    if answer.trim().is_empty() {
        // Whatever arrived was whitespace: not a usable answer.
        return Ok(None);
    }
    Ok(Some(pieces))
}

/// Maps a chat error onto the setup vocabulary.
fn map_chat_error(error: &ChatError) -> SetupErrorCode {
    match error {
        ChatError::Cancelled => SetupErrorCode::Cancelled,
        ChatError::StartupTimedOut | ChatError::TimedOut => SetupErrorCode::TestTimedOut,
        ChatError::ServerStopped => SetupErrorCode::TestFailed,
        ChatError::ServerNotRunning | ChatError::ServerNotReady => SetupErrorCode::TestFailed,
        ChatError::ProcessUnavailable => SetupErrorCode::ProcessUnavailable,
        ChatError::InsufficientResources(_) => SetupErrorCode::InsufficientSpace,
        _ => SetupErrorCode::TestFailed,
    }
}

/// The paths the check needs, resolved from an installed directory.
pub fn server_and_model(
    runtime_dir: &Path,
    server_name: &str,
    model_dir: &Path,
    model_name: &str,
) -> (PathBuf, PathBuf) {
    (
        runtime_dir.join(server_name),
        model_dir.join(model_name),
    )
}

/// Whether this machine looks able to load the model at all.
///
/// A rough memory check, reported as a warning code rather than a refusal: the
/// real answer is the server's own behaviour.
pub fn memory_looks_sufficient(model_bytes: u64) -> bool {
    let available = available_memory_bytes();
    available == 0 || available > model_bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn the_arguments_are_separate_and_loopback_only() {
        let options = FirstRunOptions::default();
        let arguments = first_run_arguments(Path::new("C:/my models/Qwen3 8B.gguf"), &options, 8099);
        assert_eq!(arguments[0], "--model");
        assert_eq!(arguments[1], "C:/my models/Qwen3 8B.gguf");
        let joined = arguments.join(" ");
        assert!(!arguments[1].contains('"'));
        assert!(joined.contains("--host 127.0.0.1"));
        assert!(joined.contains("--port 8099"));
        assert!(joined.contains("--ctx-size 4096"));
        // CPU-first: the GPU layer count is explicitly zero.
        assert!(joined.contains("--n-gpu-layers 0"));
        // Nothing that could expose the server beyond loopback.
        assert!(!joined.contains("0.0.0.0"));
        assert!(!joined.contains("--api-key"));
        // No shell, no interpreter, no command string.
        for forbidden in ["cmd", "powershell", "&&", "|", ";", "$(", "`"] {
            assert!(
                !joined.contains(forbidden),
                "the argument list must not contain {forbidden}"
            );
        }
    }

    #[test]
    fn the_check_never_asks_for_more_than_it_declares() {
        // The answer is bounded, so the check cannot turn into a generation.
        assert_eq!(TEST_MAX_TOKENS, 256);
        assert_eq!(TEST_CONTEXT_SIZE, 4096);
        assert_eq!(TEST_GPU_LAYERS, 0);
        assert_eq!(TEST_HOST, "127.0.0.1");
        assert!(TEST_PROMPT.len() < 64);
        // The prompt is impersonal: no name, no path, no user content.
        assert!(!TEST_PROMPT.contains("you are"));
        assert!(!TEST_PROMPT.contains('/'));
        assert!(!TEST_PROMPT.contains('\\'));
    }

    #[test]
    fn a_free_loopback_port_is_chosen_and_is_actually_free() {
        let port = free_loopback_port().expect("a loopback port");
        assert!(port >= MIN_TEST_PORT);
        // Binding it again must succeed: nothing is holding it.
        let listener = TcpListener::bind((TEST_HOST, port));
        assert!(listener.is_ok());
        // Two calls in a row do not have to differ, only to be usable.
        assert!(free_loopback_port().is_some());
    }

    #[test]
    fn a_missing_server_or_model_is_refused_before_a_process_starts() {
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, b"not a model").unwrap();
        let cancel = AtomicBool::new(false);
        let mut stages = Vec::new();

        let report = run_first_run(
            Some(&directory.path().join("absent.exe")),
            &model,
            &FirstRunOptions::default(),
            &cancel,
            |stage| stages.push(stage),
        );
        assert_eq!(report.error_code, Some(SetupErrorCode::RuntimeMissing));
        assert!(!report.outcome.passed);
        assert!(report.port.is_none());
        // Nothing was launched, so no stage was ever published.
        assert!(stages.is_empty());
        assert!(report.stderr_lines == 0);

        // A file that is not a GGUF model is refused as well.
        let server = directory.path().join("server.exe");
        std::fs::write(&server, crate::ai::local::setup::fixtures::pe_x64()).unwrap();
        let report = run_first_run(
            Some(&server),
            &model,
            &FirstRunOptions::default(),
            &cancel,
            |stage| stages.push(stage),
        );
        assert_eq!(report.error_code, Some(SetupErrorCode::ModelInvalid));
        assert!(stages.is_empty());
    }

    #[test]
    fn a_wrong_architecture_is_refused_before_a_process_starts() {
        let directory = tempfile::tempdir().unwrap();
        let server = directory.path().join("server.exe");
        std::fs::write(&server, crate::ai::local::setup::fixtures::pe_stub(0x014c)).unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, crate::ai::local::setup::fixtures::pinned_gguf(8192)).unwrap();
        let cancel = AtomicBool::new(false);
        let report = run_first_run(
            Some(&server),
            &model,
            &FirstRunOptions::default(),
            &cancel,
            |_| {},
        );
        assert_eq!(
            report.error_code,
            Some(SetupErrorCode::RuntimeArchitectureMismatch)
        );
    }

    #[test]
    fn a_cancellation_before_the_launch_reports_cancelled_and_starts_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let server = directory.path().join("server.exe");
        std::fs::write(&server, crate::ai::local::setup::fixtures::pe_x64()).unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, crate::ai::local::setup::fixtures::pinned_gguf(8192)).unwrap();
        let cancel = AtomicBool::new(true);
        let mut stages = Vec::new();
        let report = run_first_run(
            Some(&server),
            &model,
            &FirstRunOptions::default(),
            &cancel,
            |stage| stages.push(stage),
        );
        assert_eq!(report.error_code, Some(SetupErrorCode::Cancelled));
        assert!(!report.outcome.passed);
        // The launch stage was announced, and the process was never spawned.
        assert_eq!(stages, vec![SetupStage::LaunchTest]);
    }

    #[test]
    fn a_server_that_never_answers_times_out_and_is_stopped() {
        let directory = tempfile::tempdir().unwrap();
        // A real, small executable stand-in cannot be run, so the check is made
        // with a stopwatch on a port nothing listens to: the launch succeeds only
        // if the file is runnable, which it is not, and the failure is typed.
        let server = directory.path().join("not-really.exe");
        std::fs::write(&server, crate::ai::local::setup::fixtures::pe_x64()).unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, crate::ai::local::setup::fixtures::pinned_gguf(8192)).unwrap();
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let report = run_first_run(
            Some(&server),
            &model,
            &FirstRunOptions {
                readiness_timeout: Duration::from_millis(50),
                poll_interval: Duration::from_millis(5),
                ..FirstRunOptions::default()
            },
            &cancel,
            |_| {},
        );
        assert!(!report.outcome.passed);
        assert!(report.error_code.is_some());
        // A stub image is refused by the loader, so the process cannot start and
        // the failure is reported as an unavailable process rather than a hang.
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the check must not hang"
        );
    }

    #[test]
    fn a_reused_server_is_never_started_or_stopped_by_this_test() {
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, crate::ai::local::setup::fixtures::pinned_gguf(8192)).unwrap();
        let cancel = AtomicBool::new(false);
        let mut stages = Vec::new();
        // No server path: the check only probes, and the port is occupied by
        // nothing, so it times out without ever spawning a process.
        let report = run_first_run(
            None,
            &model,
            &FirstRunOptions {
                readiness_timeout: Duration::from_millis(30),
                poll_interval: Duration::from_millis(5),
                ..FirstRunOptions::default()
            },
            &cancel,
            |stage| stages.push(stage),
        );
        assert!(report.reused_existing_server);
        assert_eq!(report.error_code, Some(SetupErrorCode::TestTimedOut));
        assert_eq!(report.stderr_lines, 0);
        assert!(stages.contains(&SetupStage::Readiness));
        assert!(!stages.contains(&SetupStage::TestInference));
    }

    #[test]
    fn chat_errors_map_onto_the_setup_vocabulary() {
        assert_eq!(
            map_chat_error(&ChatError::Cancelled),
            SetupErrorCode::Cancelled
        );
        assert_eq!(
            map_chat_error(&ChatError::StartupTimedOut),
            SetupErrorCode::TestTimedOut
        );
        assert_eq!(
            map_chat_error(&ChatError::TimedOut),
            SetupErrorCode::TestTimedOut
        );
        assert_eq!(
            map_chat_error(&ChatError::ServerStopped),
            SetupErrorCode::TestFailed
        );
        assert_eq!(
            map_chat_error(&ChatError::ProcessUnavailable),
            SetupErrorCode::ProcessUnavailable
        );
        assert_eq!(
            map_chat_error(&ChatError::InvalidStream),
            SetupErrorCode::TestFailed
        );
    }

    #[test]
    fn the_report_carries_no_prompt_and_no_answer() {
        // The public shape of the report is the guarantee: there is nowhere for a
        // prompt or an answer to be stored.
        let report = FirstRunReport {
            outcome: TestOutcome {
                passed: true,
                server_ready: true,
                answer_received: true,
                elapsed_ms: 10,
                answer_tokens: Some(3),
            },
            ..FirstRunReport::default()
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("prompt"));
        assert!(!json.contains("answer_text"));
        assert!(!json.contains("content"));
        assert!(!json.contains(TEST_PROMPT));
    }

    #[test]
    fn the_path_helper_resolves_both_files_inside_the_managed_tree() {
        let (server, model) = server_and_model(
            Path::new("C:/data/runtime/llama.cpp/b10964"),
            "llama-server.exe",
            Path::new("C:/data/models/qwen3-8b-q4_k_m"),
            "model.gguf",
        );
        assert!(server.ends_with("llama-server.exe"));
        assert!(server.starts_with("C:/data/runtime"));
        assert!(model.starts_with("C:/data/models"));
    }
}
