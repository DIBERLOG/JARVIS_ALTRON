//! Process supervision for the managed `llama-server`.
//!
//! Rules that shape this module:
//!
//! * arguments are passed to `Command::arg` one by one — no shell, no `cmd /C`,
//!   no string concatenation, so a path cannot change the command;
//! * stdout is discarded and stderr is drained by a reader thread into a bounded
//!   ring buffer, so a chatty server cannot grow memory without limit;
//! * only the child this process started is ever killed: no `taskkill /IM`, no
//!   discovery of other processes, so a server the user started by hand is left
//!   alone;
//! * stop is bounded: kill, then wait with a deadline, then kill again.
//!
//! The runner is a trait so supervision can be tested without a real server.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ai::ChatError;

/// How many stderr lines are kept for diagnostics.
pub const STDERR_TAIL_LINES: usize = 200;
/// Longest stderr line kept, so one huge line cannot dominate the buffer.
pub const STDERR_LINE_CHARS: usize = 400;
/// How long `stop` waits for the process to exit after a kill.
pub const KILL_GRACE: Duration = Duration::from_secs(5);
/// Polling interval while waiting for exit.
pub const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Why a managed server exited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerExit {
    pub code: Option<i32>,
    pub success: bool,
}

/// A spawned server process, abstracted for tests.
pub trait ServerProcess: Send {
    /// Process identifier, when the platform provides one.
    fn pid(&self) -> Option<u32>;
    /// Non-blocking check for exit.
    fn try_wait(&mut self) -> Result<Option<ServerExit>, ChatError>;
    /// Terminates the process.
    fn kill(&mut self) -> Result<(), ChatError>;
    /// Blocks until the process exits, bounded by the deadline.
    fn wait_bounded(&mut self, timeout: Duration) -> Result<Option<ServerExit>, ChatError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit) = self.try_wait()? {
                return Ok(Some(exit));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(EXIT_POLL_INTERVAL);
        }
    }
}

/// Spawns server processes.
pub trait ProcessRunner: Send + Sync {
    fn spawn(&self, program: &Path, arguments: &[String]) -> Result<SpawnedServer, ChatError>;
}

/// A spawned process plus its stderr stream, when the runner captured one.
pub struct SpawnedServer {
    pub process: Box<dyn ServerProcess>,
    pub stderr: Option<Box<dyn Read + Send>>,
    /// Short description used in diagnostics, never containing arguments.
    pub program_label: String,
}

/// Bounded ring buffer of the last stderr lines.
#[derive(Debug, Default)]
pub struct StderrBuffer {
    lines: VecDeque<String>,
    total_lines: u64,
    truncated: bool,
}

impl StderrBuffer {
    pub fn push(&mut self, line: &str) {
        self.total_lines += 1;
        let cleaned = redact_and_truncate(line);
        self.lines.push_back(cleaned);
        while self.lines.len() > STDERR_TAIL_LINES {
            self.lines.pop_front();
            self.truncated = true;
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    pub fn total_lines(&self) -> u64 {
        self.total_lines
    }

    pub fn was_truncated(&self) -> bool {
        self.truncated
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.total_lines = 0;
        self.truncated = false;
    }

    /// Whether any line mentions an out-of-memory condition.
    pub fn indicates_out_of_memory(&self) -> bool {
        self.lines.iter().any(|line| {
            let lowered = line.to_lowercase();
            lowered.contains("out of memory")
                || lowered.contains("failed to allocate")
                || lowered.contains("insufficient memory")
        })
    }
}

/// Removes characters that could make a log line unreadable, and truncates it.
fn redact_and_truncate(line: &str) -> String {
    let cleaned: String = line
        .chars()
        .map(|character| {
            // Tabs and control characters are replaced by a space rather than
            // dropped, so a hostile server cannot use them to rewrite a line.
            if character == '\t' || character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if cleaned.chars().count() > STDERR_LINE_CHARS {
        let mut truncated: String = cleaned.chars().take(STDERR_LINE_CHARS).collect();
        truncated.push('…');
        truncated
    } else {
        cleaned
    }
}

/// Collects stderr into a shared bounded buffer until the stream ends.
pub fn drain_stderr(
    stream: Box<dyn Read + Send>,
    buffer: Arc<Mutex<StderrBuffer>>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match line {
                Ok(line) => {
                    if let Ok(mut buffer) = buffer.lock() {
                        buffer.push(&line);
                    }
                }
                Err(_) => break,
            }
        }
    })
}

/// The production runner: `std::process::Command` with separated arguments.
#[derive(Default)]
pub struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn spawn(&self, program: &Path, arguments: &[String]) -> Result<SpawnedServer, ChatError> {
        let mut command = Command::new(program);
        // One argument per element: the model path is never joined into a shell
        // string, so spaces or metacharacters cannot change the command.
        for argument in arguments {
            command.arg(argument);
        }
        command
            .stdin(Stdio::null())
            // stdout is dropped so a full pipe can never block the server.
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            // No console window for the managed child.
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn().map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ChatError::ProcessUnavailable,
            std::io::ErrorKind::PermissionDenied => ChatError::ProcessUnavailable,
            _ => ChatError::ProcessUnavailable,
        })?;
        let stderr = child
            .stderr
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>);
        Ok(SpawnedServer {
            program_label: program
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "llama-server".to_string()),
            process: Box::new(ChildProcess { child }),
            stderr,
        })
    }
}

/// Adapter over `std::process::Child`.
struct ChildProcess {
    child: Child,
}

impl ServerProcess for ChildProcess {
    fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }

    fn try_wait(&mut self) -> Result<Option<ServerExit>, ChatError> {
        match self.child.try_wait() {
            Ok(None) => Ok(None),
            Ok(Some(status)) => Ok(Some(ServerExit {
                code: status.code(),
                success: status.success(),
            })),
            Err(error) => Err(error.into()),
        }
    }

    fn kill(&mut self) -> Result<(), ChatError> {
        match self.child.kill() {
            Ok(()) => Ok(()),
            // Already gone is not a failure.
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// A running managed server with its diagnostics.
pub struct ManagedServer {
    process: Box<dyn ServerProcess>,
    pub program_label: String,
    stderr: Arc<Mutex<StderrBuffer>>,
    stop_reader: Arc<AtomicBool>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for ManagedServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedServer")
            .field("pid", &self.pid())
            .field("program", &self.program_label)
            .finish()
    }
}

impl ManagedServer {
    /// Starts a server, capturing stderr into a bounded buffer.
    pub fn start(
        runner: &dyn ProcessRunner,
        program: &Path,
        arguments: &[String],
    ) -> Result<Self, ChatError> {
        let spawned = runner.spawn(program, arguments)?;
        let stderr = Arc::new(Mutex::new(StderrBuffer::default()));
        let stop_reader = Arc::new(AtomicBool::new(false));
        let reader = spawned
            .stderr
            .map(|stream| drain_stderr(stream, Arc::clone(&stderr), Arc::clone(&stop_reader)));
        Ok(Self {
            process: spawned.process,
            program_label: spawned.program_label,
            stderr,
            stop_reader,
            reader,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.process.pid()
    }

    /// Non-blocking exit check.
    pub fn try_wait(&mut self) -> Result<Option<ServerExit>, ChatError> {
        self.process.try_wait()
    }

    /// Last stderr lines, for diagnostics.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr
            .lock()
            .map(|buffer| buffer.lines())
            .unwrap_or_default()
    }

    pub fn stderr_line_count(&self) -> u64 {
        self.stderr
            .lock()
            .map(|buffer| buffer.total_lines())
            .unwrap_or(0)
    }

    pub fn stderr_was_truncated(&self) -> bool {
        self.stderr
            .lock()
            .map(|buffer| buffer.was_truncated())
            .unwrap_or(false)
    }

    pub fn indicates_out_of_memory(&self) -> bool {
        self.stderr
            .lock()
            .map(|buffer| buffer.indicates_out_of_memory())
            .unwrap_or(false)
    }

    /// Stops the server: kill, wait with a deadline, kill again if needed.
    ///
    /// Bounded by construction, so a stuck process cannot hang the caller.
    pub fn stop(&mut self) -> Result<(), ChatError> {
        // Ask it to stop, then give it a bounded moment to disappear.
        let first = self.process.kill();
        let exited = self.process.wait_bounded(KILL_GRACE)?;
        if exited.is_none() {
            // Second attempt, then give up on the handle without blocking.
            let _ = self.process.kill();
            let _ = self.process.wait_bounded(KILL_GRACE)?;
        }
        first
    }

    /// Stops the process and releases the stderr reader.
    pub fn shutdown(&mut self) {
        let _ = self.stop();
        self.stop_reader.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader.take() {
            // The reader thread ends when the pipe closes; do not block on it.
            drop(reader);
        }
    }
}

impl Drop for ManagedServer {
    fn drop(&mut self) {
        // Never leave a child behind, even on an early return.
        let _ = self.process.kill();
        let _ = self.process.wait_bounded(KILL_GRACE);
        self.stop_reader.store(true, Ordering::Relaxed);
    }
}

/// Waits until `check` succeeds or the timeout elapses.
///
/// Used for startup: the caller passes a readiness probe, and the wait also
/// notices a process that exits early.
pub fn wait_until_ready(
    server: &mut ManagedServer,
    timeout: Duration,
    poll_interval: Duration,
    mut check: impl FnMut() -> Result<bool, ChatError>,
) -> Result<(), ChatError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(_exit) = server.try_wait()? {
            return Err(ChatError::ServerStopped);
        }
        match check() {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            // A refused connection while starting is expected; keep waiting.
            Err(ChatError::ServerNotRunning) | Err(ChatError::TimedOut) => {}
            Err(other) => return Err(other),
        }
        if Instant::now() >= deadline {
            return Err(ChatError::StartupTimedOut);
        }
        std::thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    /// A scripted process, so supervision can be tested without a real server.
    struct FakeProcess {
        pid: Option<u32>,
        exits_after_polls: Option<usize>,
        polls: usize,
        killed: bool,
        exit: ServerExit,
        kill_effect: Option<ServerExit>,
    }

    impl FakeProcess {
        fn running() -> Self {
            Self {
                pid: Some(4242),
                exits_after_polls: None,
                polls: 0,
                killed: false,
                exit: ServerExit {
                    code: Some(0),
                    success: true,
                },
                kill_effect: Some(ServerExit {
                    code: Some(1),
                    success: false,
                }),
            }
        }

        fn crashing_after(polls: usize) -> Self {
            Self {
                exits_after_polls: Some(polls),
                ..Self::running()
            }
        }

        fn ignoring_kill() -> Self {
            Self {
                kill_effect: None,
                ..Self::running()
            }
        }
    }

    impl ServerProcess for FakeProcess {
        fn pid(&self) -> Option<u32> {
            self.pid
        }

        fn try_wait(&mut self) -> Result<Option<ServerExit>, ChatError> {
            self.polls += 1;
            if self.killed {
                return Ok(self.kill_effect);
            }
            match self.exits_after_polls {
                Some(limit) if self.polls > limit => Ok(Some(self.exit)),
                _ => Ok(None),
            }
        }

        fn kill(&mut self) -> Result<(), ChatError> {
            self.killed = true;
            Ok(())
        }
    }

    /// A scripted spawn, so a test can describe what each `start` returns.
    type SpawnScript = Box<dyn FnMut() -> Box<dyn ServerProcess> + Send>;

    struct FakeRunner {
        script: Mutex<Vec<SpawnScript>>,
        spawns: AtomicUsize,
        fail: AtomicBool,
        last_arguments: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        fn new(scripts: Vec<SpawnScript>) -> Self {
            Self {
                script: Mutex::new(scripts),
                spawns: AtomicUsize::new(0),
                fail: AtomicBool::new(false),
                last_arguments: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProcessRunner for FakeRunner {
        fn spawn(&self, _program: &Path, arguments: &[String]) -> Result<SpawnedServer, ChatError> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(ChatError::ProcessUnavailable);
            }
            *self.last_arguments.lock().unwrap() = arguments.to_vec();
            let mut script = self.script.lock().unwrap();
            let process = if script.is_empty() {
                Box::new(FakeProcess::running()) as Box<dyn ServerProcess>
            } else {
                (script.remove(0))()
            };
            Ok(SpawnedServer {
                process,
                stderr: None,
                program_label: "fake-llama-server".to_string(),
            })
        }
    }

    #[test]
    fn start_captures_the_program_and_passes_arguments_separately() {
        let runner = FakeRunner::new(Vec::new());
        let server = ManagedServer::start(
            &runner,
            &PathBuf::from("C:/tools/llama-server.exe"),
            &["--model".to_string(), "C:/my models/model.gguf".to_string()],
        )
        .unwrap();
        assert_eq!(server.pid(), Some(4242));
        assert_eq!(server.program_label, "fake-llama-server");
        let arguments = runner.last_arguments.lock().unwrap().clone();
        assert_eq!(arguments.len(), 2);
        assert_eq!(arguments[1], "C:/my models/model.gguf");
        assert_eq!(runner.spawns.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failed_spawn_is_reported_as_a_controlled_error() {
        let runner = FakeRunner::new(Vec::new());
        runner.fail.store(true, Ordering::SeqCst);
        let error = ManagedServer::start(&runner, &PathBuf::from("absent.exe"), &[])
            .err()
            .unwrap();
        assert_eq!(error, ChatError::ProcessUnavailable);
    }

    #[test]
    fn wait_until_ready_returns_when_the_probe_succeeds() {
        let runner = FakeRunner::new(Vec::new());
        let mut server = ManagedServer::start(&runner, &PathBuf::from("server.exe"), &[]).unwrap();
        let mut attempts = 0;
        wait_until_ready(
            &mut server,
            Duration::from_millis(500),
            Duration::from_millis(1),
            || {
                attempts += 1;
                Ok(attempts >= 3)
            },
        )
        .unwrap();
        assert!(attempts >= 3);
    }

    #[test]
    fn wait_until_ready_times_out_and_reports_it() {
        let runner = FakeRunner::new(Vec::new());
        let mut server = ManagedServer::start(&runner, &PathBuf::from("server.exe"), &[]).unwrap();
        let error = wait_until_ready(
            &mut server,
            Duration::from_millis(20),
            Duration::from_millis(1),
            || Ok(false),
        )
        .unwrap_err();
        assert_eq!(error, ChatError::StartupTimedOut);
    }

    #[test]
    fn wait_until_ready_detects_a_process_that_died() {
        let runner = FakeRunner::new(vec![Box::new(|| {
            Box::new(FakeProcess::crashing_after(0)) as Box<dyn ServerProcess>
        })]);
        let mut server = ManagedServer::start(&runner, &PathBuf::from("server.exe"), &[]).unwrap();
        let error = wait_until_ready(
            &mut server,
            Duration::from_millis(100),
            Duration::from_millis(1),
            || Ok(false),
        )
        .unwrap_err();
        assert_eq!(error, ChatError::ServerStopped);
    }

    #[test]
    fn a_refused_connection_during_startup_is_not_fatal() {
        let runner = FakeRunner::new(Vec::new());
        let mut server = ManagedServer::start(&runner, &PathBuf::from("server.exe"), &[]).unwrap();
        let mut attempts = 0;
        wait_until_ready(
            &mut server,
            Duration::from_millis(200),
            Duration::from_millis(1),
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(ChatError::ServerNotRunning)
                } else {
                    Ok(true)
                }
            },
        )
        .unwrap();
        assert_eq!(attempts, 3);
    }

    #[test]
    fn stop_is_bounded_even_when_the_process_ignores_a_kill() {
        let runner = FakeRunner::new(vec![Box::new(|| {
            Box::new(FakeProcess::ignoring_kill()) as Box<dyn ServerProcess>
        })]);
        let mut server = ManagedServer::start(&runner, &PathBuf::from("server.exe"), &[]).unwrap();
        let started = Instant::now();
        // Returns instead of hanging forever; the fake never exits.
        let _ = server.stop();
        assert!(
            started.elapsed() < KILL_GRACE * 3,
            "stop must not wait indefinitely"
        );
    }

    #[test]
    fn the_stderr_buffer_is_bounded_and_keeps_the_tail() {
        let mut buffer = StderrBuffer::default();
        for index in 0..(STDERR_TAIL_LINES + 50) {
            buffer.push(&format!("line {index}"));
        }
        assert_eq!(buffer.lines().len(), STDERR_TAIL_LINES);
        assert!(buffer.was_truncated());
        assert_eq!(buffer.total_lines(), (STDERR_TAIL_LINES + 50) as u64);
        assert!(buffer.lines().last().unwrap().contains("line"));
        // The oldest lines are gone.
        assert!(!buffer.lines()[0].contains("line 0"));
    }

    #[test]
    fn the_stderr_buffer_truncates_huge_lines_and_strips_control_characters() {
        let mut buffer = StderrBuffer::default();
        let long = "x".repeat(STDERR_LINE_CHARS * 3);
        buffer.push(&long);
        let line = &buffer.lines()[0];
        assert!(line.chars().count() <= STDERR_LINE_CHARS + 1);
        buffer.push("tab\there\u{7}bell");
        let cleaned = buffer.lines().last().unwrap().clone();
        assert!(!cleaned.contains('\t'));
        assert!(!cleaned.contains('\u{7}'));
    }

    #[test]
    fn the_stderr_buffer_recognises_out_of_memory_failures() {
        let mut buffer = StderrBuffer::default();
        buffer.push("llama_model_load: error loading model");
        assert!(!buffer.indicates_out_of_memory());
        buffer.push("ggml_backend_alloc: failed to allocate buffer of size 1024 MiB");
        assert!(buffer.indicates_out_of_memory());
    }

    #[test]
    fn draining_stderr_fills_the_shared_buffer() {
        let buffer = Arc::new(Mutex::new(StderrBuffer::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let stream = Box::new(Cursor::new(b"first line\nsecond line\n".to_vec()));
        let handle = drain_stderr(stream, Arc::clone(&buffer), stop);
        handle.join().unwrap();
        let lines = buffer.lock().unwrap().lines();
        assert_eq!(
            lines,
            vec!["first line".to_string(), "second line".to_string()]
        );
    }

    #[test]
    fn dropping_a_managed_server_kills_only_that_process() {
        let runner = FakeRunner::new(Vec::new());
        {
            let _server = ManagedServer::start(&runner, &PathBuf::from("server.exe"), &[]).unwrap();
        }
        // Nothing global happened: no taskkill, no process enumeration. The fake
        // runner recorded exactly one spawn.
        assert_eq!(runner.spawns.load(Ordering::SeqCst), 1);
    }
}
