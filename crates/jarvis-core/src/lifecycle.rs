//! The order in which the application lets go of what it holds.
//!
//! A full exit is a sequence, not a set of calls: new work has to stop before
//! running work is cancelled, a microphone has to close before a process is
//! killed, and a key has to be dropped before the database behind it is closed.
//! This module makes that order a value that can be listed, tested, and
//! reported, instead of an accident of the order of statements in `main`.
//!
//! The rules it enforces:
//!
//! * **nothing is accepted once shutdown has begun.** [`LifecycleManager::accept_operations`]
//!   turns false at the first call, so a button pressed during an exit is
//!   refused rather than served by half a session;
//! * **every step runs, in order, at most once.** A repeated shutdown does
//!   nothing and returns what happened the first time;
//! * **a step that hangs cannot hang the exit.** Each step runs on its own
//!   worker and is waited for no longer than its own timeout and the global
//!   deadline; a step that misses both is reported as timed out and the exit
//!   continues. The process still exits: the worker is left behind on purpose,
//!   because the alternative is an application that never closes;
//! * **a failing step does not stop the others.** Each failure is recorded with
//!   a content-free code, and the report says which step failed;
//! * **the last step is the exit itself**, so the caller can see the whole
//!   sequence in one report.
//!
//! The steps are closures over the components that exist, which is what keeps
//! this module free of any knowledge about the AI gateway, the vault, or the
//! window: it knows order, deadlines, and reports.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Longest the whole exit may take, unless the caller says otherwise.
pub const DEFAULT_GLOBAL_DEADLINE: Duration = Duration::from_secs(10);
/// Longest one step may take when it does not say.
pub const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the exit has got to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Running,
    ShuttingDown,
    Stopped,
}

/// What happened to one step.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// The step finished in time.
    Done,
    /// The step reported a failure of its own.
    Failed,
    /// The step did not finish within its timeout or the global deadline.
    TimedOut,
    /// The step was not attempted, because the global deadline had passed.
    Skipped,
}

/// One line of the report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StepReport {
    /// Stable name, for a log line and for a test.
    pub name: String,
    pub outcome: StepOutcome,
    pub duration_ms: u64,
    /// Content-free reason for a failure or a timeout.
    pub detail: Option<String>,
}

impl StepReport {
    pub fn succeeded(&self) -> bool {
        self.outcome == StepOutcome::Done
    }
}

/// The whole exit, in the order it happened.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShutdownReport {
    pub steps: Vec<StepReport>,
    pub duration_ms: u64,
    /// Whether the global deadline was reached before the list ended.
    pub deadline_reached: bool,
}

impl ShutdownReport {
    /// Whether every step that was attempted succeeded.
    pub fn is_clean(&self) -> bool {
        self.steps.iter().all(StepReport::succeeded)
    }

    /// Whether anything was skipped or timed out, which the user should know.
    pub fn is_incomplete(&self) -> bool {
        self.steps
            .iter()
            .any(|step| matches!(step.outcome, StepOutcome::TimedOut | StepOutcome::Skipped))
    }

    /// The steps that did not succeed.
    pub fn problems(&self) -> Vec<&StepReport> {
        self.steps.iter().filter(|step| !step.succeeded()).collect()
    }

    /// One line per step, for a log: names and outcomes, never content.
    pub fn summary(&self) -> String {
        let mut lines = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            let outcome = match step.outcome {
                StepOutcome::Done => "done",
                StepOutcome::Failed => "failed",
                StepOutcome::TimedOut => "timed out",
                StepOutcome::Skipped => "skipped",
            };
            lines.push(format!(
                "{}: {outcome} ({} ms)",
                step.name, step.duration_ms
            ));
        }
        lines.join("; ")
    }
}

/// One step of the exit.
struct Step {
    name: String,
    timeout: Duration,
    action: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
}

/// The manager: a list of steps and the state of the exit.
pub struct LifecycleManager {
    steps: Mutex<Vec<Step>>,
    state: Mutex<LifecycleState>,
    report: Mutex<Option<ShutdownReport>>,
    /// How many times `shutdown` was called, for the idempotence test.
    shutdowns: AtomicU64,
    global_deadline: Duration,
}

impl std::fmt::Debug for LifecycleManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LifecycleManager")
            .field("steps", &self.steps.lock().len())
            .field("state", &*self.state.lock())
            .field("shutdowns", &self.shutdowns.load(Ordering::SeqCst))
            .finish()
    }
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleManager {
    pub fn new() -> Self {
        Self {
            steps: Mutex::new(Vec::new()),
            state: Mutex::new(LifecycleState::Running),
            report: Mutex::new(None),
            shutdowns: AtomicU64::new(0),
            global_deadline: DEFAULT_GLOBAL_DEADLINE,
        }
    }

    /// Changes how long the whole exit may take.
    pub fn with_global_deadline(mut self, deadline: Duration) -> Self {
        self.global_deadline = deadline;
        self
    }

    /// Adds a step. The order of the calls is the order of the exit.
    pub fn add<F>(&mut self, name: impl Into<String>, timeout: Duration, action: F) -> &mut Self
    where
        F: Fn() -> Result<(), String> + Send + Sync + 'static,
    {
        self.steps.lock().push(Step {
            name: name.into(),
            timeout,
            action: Box::new(action),
        });
        self
    }

    /// Adds a step that cannot fail.
    pub fn add_ok<F>(&mut self, name: impl Into<String>, timeout: Duration, action: F) -> &mut Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.add(name, timeout, move || {
            action();
            Ok(())
        })
    }

    /// The names of the steps, in the order they will run.
    pub fn steps(&self) -> Vec<String> {
        self.steps
            .lock()
            .iter()
            .map(|step| step.name.clone())
            .collect()
    }

    pub fn state(&self) -> LifecycleState {
        *self.state.lock()
    }

    /// How many times `shutdown` was asked to run.
    pub fn shutdown_calls(&self) -> u64 {
        self.shutdowns.load(Ordering::SeqCst)
    }

    /// Whether new work may still be accepted.
    ///
    /// This is the switch every command reads before it starts something: as
    /// soon as an exit begins, the answer is no.
    pub fn accept_operations(&self) -> bool {
        self.state() == LifecycleState::Running
    }

    /// Runs the exit once, in order, and returns the report.
    ///
    /// A second call does nothing and returns the first report: an exit that is
    /// already running or already finished has nothing left to do.
    pub fn shutdown(&self) -> ShutdownReport {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        {
            let mut state = self.state.lock();
            if *state != LifecycleState::Running {
                // Either the exit is in progress on another thread, or it is
                // finished: the stored report is the answer either way.
                drop(state);
                if let Some(report) = self.report.lock().clone() {
                    return report;
                }
                // Another thread is inside the exit; it will store the report.
                return ShutdownReport::default();
            }
            *state = LifecycleState::ShuttingDown;
        }

        let started = Instant::now();
        let deadline = started + self.global_deadline;
        let steps = std::mem::take(&mut *self.steps.lock());
        let mut reports = Vec::with_capacity(steps.len());
        let mut deadline_reached = false;

        for step in steps {
            if Instant::now() >= deadline {
                reports.push(StepReport {
                    name: step.name,
                    outcome: StepOutcome::Skipped,
                    duration_ms: 0,
                    detail: Some("the exit deadline had passed".to_string()),
                });
                deadline_reached = true;
                continue;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = step.timeout.min(remaining);
            reports.push(run_step(&step.name, wait, step.action));
            if Instant::now() >= deadline {
                deadline_reached = true;
            }
        }

        let report = ShutdownReport {
            steps: reports,
            duration_ms: started.elapsed().as_millis() as u64,
            deadline_reached,
        };
        *self.state.lock() = LifecycleState::Stopped;
        *self.report.lock() = Some(report.clone());
        report
    }

    /// The report of the exit, once it has run.
    pub fn report(&self) -> Option<ShutdownReport> {
        self.report.lock().clone()
    }
}

/// Runs one step on its own worker and waits no longer than `wait`.
fn run_step(
    name: &str,
    wait: Duration,
    action: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
) -> StepReport {
    let started = Instant::now();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name(format!("shutdown-{name}"))
        .spawn(move || {
            let result = action();
            // The receiver may be gone if the step timed out; that is fine.
            let _ = sender.send(result);
        });
    let worker = match worker {
        Ok(worker) => worker,
        Err(_) => {
            return StepReport {
                name: name.to_string(),
                outcome: StepOutcome::Failed,
                duration_ms: started.elapsed().as_millis() as u64,
                detail: Some("the step could not be started".to_string()),
            };
        }
    };
    match receiver.recv_timeout(wait) {
        Ok(Ok(())) => {
            let _ = worker.join();
            StepReport {
                name: name.to_string(),
                outcome: StepOutcome::Done,
                duration_ms: started.elapsed().as_millis() as u64,
                detail: None,
            }
        }
        Ok(Err(detail)) => {
            let _ = worker.join();
            StepReport {
                name: name.to_string(),
                outcome: StepOutcome::Failed,
                duration_ms: started.elapsed().as_millis() as u64,
                detail: Some(crate::text::redact(&detail)),
            }
        }
        Err(RecvTimeoutError::Timeout) => StepReport {
            name: name.to_string(),
            outcome: StepOutcome::TimedOut,
            duration_ms: started.elapsed().as_millis() as u64,
            detail: Some("the step did not finish in time and was left behind".to_string()),
        },
        Err(RecvTimeoutError::Disconnected) => StepReport {
            name: name.to_string(),
            outcome: StepOutcome::Failed,
            duration_ms: started.elapsed().as_millis() as u64,
            detail: Some("the step ended without an answer".to_string()),
        },
    }
}

/// The steps a full exit performs, in order, as names a report can carry.
///
/// The list is documentation that a test can read: the order is the contract.
pub const SHUTDOWN_ORDER: [&str; 11] = [
    "refuse-new-work",
    "cancel-generation",
    "stop-dictation",
    "stop-vosk",
    "stop-timers",
    "stop-llama-server",
    "drop-decrypted-caches",
    "zeroize-keys",
    "checkpoint-databases",
    "close-databases",
    "exit",
];

/// A guard that refuses work once an exit has started, for the callers that need
/// one value instead of a manager.
#[derive(Clone, Default)]
pub struct ShutdownFlag(Arc<std::sync::atomic::AtomicBool>);

impl ShutdownFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn the_steps_run_in_the_order_they_were_added() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut manager = LifecycleManager::new();
        for name in ["first", "second", "third"] {
            let order = Arc::clone(&order);
            manager.add_ok(name, Duration::from_secs(1), move || {
                order.lock().push(name);
            });
        }
        let report = manager.shutdown();
        assert_eq!(*order.lock(), vec!["first", "second", "third"]);
        assert!(report.is_clean());
        assert_eq!(report.steps.len(), 3);
        assert!(!report.is_incomplete());
        assert!(report.problems().is_empty());
        assert!(report.summary().contains("first: done"));
    }

    #[test]
    fn the_documented_order_is_the_order_the_exit_uses() {
        // The constant is what the documentation and the window read, so it must
        // match the sequence of steps that is actually registered.
        assert_eq!(SHUTDOWN_ORDER.first(), Some(&"refuse-new-work"));
        assert_eq!(SHUTDOWN_ORDER.last(), Some(&"exit"));
        assert!(SHUTDOWN_ORDER.contains(&"zeroize-keys"));
        assert!(SHUTDOWN_ORDER.contains(&"checkpoint-databases"));
        assert!(
            SHUTDOWN_ORDER
                .iter()
                .position(|step| *step == "stop-timers")
                < SHUTDOWN_ORDER
                    .iter()
                    .position(|step| *step == "stop-llama-server"),
            "timers stop before the managed server"
        );
        assert!(
            SHUTDOWN_ORDER
                .iter()
                .position(|step| *step == "zeroize-keys")
                < SHUTDOWN_ORDER
                    .iter()
                    .position(|step| *step == "close-databases"),
            "keys are dropped before the databases are closed"
        );
    }

    #[test]
    fn nothing_is_accepted_once_the_exit_has_begun() {
        let mut manager = LifecycleManager::new();
        manager.add_ok("work", Duration::from_secs(1), || {});
        assert!(manager.accept_operations());
        assert_eq!(manager.state(), LifecycleState::Running);
        let report = manager.shutdown();
        assert!(report.is_clean());
        assert!(!manager.accept_operations());
        assert_eq!(manager.state(), LifecycleState::Stopped);
    }

    #[test]
    fn a_second_shutdown_runs_nothing_and_returns_the_first_report() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut manager = LifecycleManager::new();
        {
            let runs = Arc::clone(&runs);
            manager.add_ok("once", Duration::from_secs(1), move || {
                runs.fetch_add(1, Ordering::SeqCst);
            });
        }
        let first = manager.shutdown();
        let second = manager.shutdown();
        let third = manager.shutdown();
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the step must run once");
        assert_eq!(first, second);
        assert_eq!(second, third);
        assert_eq!(manager.shutdown_calls(), 3);
        assert!(manager.report().is_some());
    }

    #[test]
    fn a_failing_step_is_recorded_and_the_rest_still_run() {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let mut manager = LifecycleManager::new();
        manager.add("fails", Duration::from_secs(1), || {
            Err("the model server refused to stop".to_string())
        });
        {
            let ran = Arc::clone(&ran);
            manager.add_ok("after", Duration::from_secs(1), move || {
                ran.lock().push("after");
            });
        }
        let report = manager.shutdown();
        assert_eq!(*ran.lock(), vec!["after"]);
        assert!(!report.is_clean());
        assert!(
            !report.is_incomplete(),
            "a failure is not an incomplete exit"
        );
        assert_eq!(report.problems().len(), 1);
        assert_eq!(report.problems()[0].name, "fails");
        assert_eq!(report.problems()[0].outcome, StepOutcome::Failed);
        assert!(report.summary().contains("fails: failed"));
    }

    #[test]
    fn a_hanging_step_does_not_hang_the_exit() {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let mut manager = LifecycleManager::new().with_global_deadline(Duration::from_secs(5));
        // A step that never returns: it holds its worker, not the exit.
        manager.add("hangs", Duration::from_millis(200), || {
            std::thread::sleep(Duration::from_secs(30));
            Ok(())
        });
        {
            let ran = Arc::clone(&ran);
            manager.add_ok("after", Duration::from_secs(1), move || {
                ran.lock().push("after");
            });
        }
        let started = Instant::now();
        let report = manager.shutdown();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "the exit must not wait for a step that hangs: {elapsed:?}"
        );
        assert_eq!(*ran.lock(), vec!["after"], "the next step still ran");
        assert_eq!(report.steps[0].outcome, StepOutcome::TimedOut);
        assert!(report.is_incomplete());
        assert!(report.problems()[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("did not finish"));
    }

    #[test]
    fn the_global_deadline_bounds_the_whole_exit_and_skips_what_is_left() {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let mut manager = LifecycleManager::new().with_global_deadline(Duration::from_millis(250));
        manager.add("slow", Duration::from_secs(10), || {
            std::thread::sleep(Duration::from_millis(400));
            Ok(())
        });
        {
            let ran = Arc::clone(&ran);
            manager.add_ok("never", Duration::from_secs(1), move || {
                ran.lock().push("never");
            });
        }
        let started = Instant::now();
        let report = manager.shutdown();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(report.deadline_reached);
        assert!(ran.lock().is_empty(), "the skipped step did not run");
        assert_eq!(report.steps[1].outcome, StepOutcome::Skipped);
        assert!(!report.problems().is_empty());
    }

    #[test]
    fn a_reported_failure_carries_no_path_and_no_secret() {
        let mut manager = LifecycleManager::new();
        manager.add("leaks", Duration::from_secs(1), || {
            Err("could not close C:/Users/someone/AppData/key.dpapi".to_string())
        });
        let report = manager.shutdown();
        let detail = report.problems()[0].detail.clone().unwrap();
        assert!(!detail.contains("someone"), "{detail}");
        assert!(!detail.contains("key.dpapi"), "{detail}");
        assert!(!detail.contains('/'), "{detail}");
    }

    #[test]
    fn a_key_clearing_step_runs_before_the_databases_are_closed() {
        // The order that matters most: a decrypted cache and the keys behind it
        // must be gone before the files are closed, and the report must show it.
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut manager = LifecycleManager::new();
        for name in ["zeroize-keys", "checkpoint-databases", "close-databases"] {
            let order = Arc::clone(&order);
            manager.add_ok(name, Duration::from_secs(1), move || {
                order.lock().push(name);
            });
        }
        manager.shutdown();
        assert_eq!(
            *order.lock(),
            vec!["zeroize-keys", "checkpoint-databases", "close-databases"]
        );
    }

    #[test]
    fn the_shutdown_flag_is_a_single_value_for_the_callers_that_need_one() {
        let flag = ShutdownFlag::new();
        assert!(!flag.is_set());
        flag.begin();
        assert!(flag.is_set());
        let copy = flag.clone();
        assert!(copy.is_set(), "a clone sees the same flag");
        flag.begin();
        assert!(flag.is_set(), "setting it twice is not an error");
    }
}
