//! The local AI gateway: the only layer that manages `llama-server`.
//!
//! The interface never touches the process or the HTTP API; it asks the gateway
//! for a status and for generations. The gateway owns
//!
//! * configuration validation and resource checking;
//! * the lifecycle of one managed server process (`Stopped` → `Starting` →
//!   `Ready` → `Generating` → `Stopping` → `Stopped`, or `Failed`);
//! * the loopback HTTP client and the streaming events;
//! * generation cancellation.
//!
//! Concurrency rule: the state mutex is only held for short transitions. Probing,
//! waiting for readiness, streaming, and stopping all happen outside the lock, so
//! a slow server can never block a status query, and a worker thread never holds
//! the lock while it reads from a socket.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::client::{
    cancellation_flag, ChatCompletionRequest, ChunkOutcome, CompletionOptions, HealthState,
    LocalAiClient, ServerProbe, StreamTimer,
};
use super::config::LocalAiConfig;
use super::config::ThinkingMode;
use super::model::{
    validate_model, CheckLevel, LocalModelConfig as ModelFileInfo, ValidationIssue,
};
use super::process::{ManagedServer, ProcessRunner, RealProcessRunner};
use super::resources::{estimate_resources, ResourceEstimate};
use crate::ai::{prompt, ChatError, ChatMessage, ChatProvider, ChatRequest, ChatResponse, Persona};

/// How often readiness is probed while starting.
pub const READY_POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Lifecycle state of the managed server.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalAiState {
    #[default]
    Stopped,
    Starting,
    Ready,
    Generating,
    Stopping,
    Failed,
}

impl LocalAiState {
    /// Whether a generation may be started from this state.
    pub fn accepts_generation(&self) -> bool {
        matches!(self, Self::Ready | Self::Generating)
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Starting | Self::Stopping)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Generating => "generating",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
        }
    }
}

/// What the running server actually supports.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LocalAiCapabilities {
    pub endpoint_available: bool,
    pub model_id: Option<String>,
    pub models: Vec<String>,
    pub streaming: bool,
    /// The chat template mentions a thinking switch, so a preference can be sent.
    pub thinking_switch: bool,
    /// A `reasoning_content` field has been observed in a response.
    pub reasoning_field_observed: bool,
    pub build_info: Option<String>,
    pub notes: Vec<String>,
}

impl LocalAiCapabilities {
    /// Whether the requested thinking mode can be honoured right now.
    pub fn thinking_available(&self, mode: ThinkingMode) -> bool {
        match mode {
            ThinkingMode::Auto => true,
            ThinkingMode::Disabled | ThinkingMode::Enabled => self.thinking_switch,
        }
    }
}

/// Full status for the interface.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LocalAiStatus {
    pub state: LocalAiState,
    pub host: String,
    pub port: u16,
    pub pid: Option<u32>,
    pub model_file: Option<String>,
    pub server_file: Option<String>,
    pub profile: Persona,
    pub thinking: ThinkingMode,
    pub capabilities: LocalAiCapabilities,
    /// Content-free error message, when the last operation failed.
    pub last_error: Option<String>,
    pub stderr_tail: Vec<String>,
    pub stderr_truncated: bool,
    pub out_of_memory_hint: bool,
    /// How long the managed server has been running, as a short label.
    pub uptime: Option<String>,
    /// Whether a generation is currently running.
    pub generating: bool,
}

/// A generation request coming from the interface.
///
/// The system prompt is *not* part of this type: the profile selects it inside
/// the gateway, so a chat message can never replace the constraints.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct GenerationRequest {
    pub messages: Vec<ChatMessage>,
    pub profile: Option<Persona>,
    pub thinking: Option<ThinkingMode>,
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

impl GenerationRequest {
    /// A request with a single user message.
    pub fn user_message(content: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage {
                role: crate::ai::ChatRole::User,
                content: content.into(),
            }],
            stream: true,
            ..Self::default()
        }
    }

    /// Whether the request has anything to answer.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Events emitted while a generation runs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GenerationEvent {
    Started {
        id: Uuid,
        model: String,
        profile: Persona,
        thinking: ThinkingMode,
        /// Whether the server accepted the thinking preference.
        thinking_applied: bool,
        stream: bool,
    },
    Token {
        text: String,
    },
    Thinking {
        text: String,
    },
    Completed {
        usage: Option<super::client::GenerationUsage>,
        finish_reason: Option<String>,
        duration_ms: u64,
        cancelled: bool,
    },
    Cancelled {
        /// Whether some text had already been produced.
        partial: bool,
        duration_ms: u64,
    },
    Failed {
        /// Content-free message; never a prompt or a completion.
        error: String,
    },
}

impl GenerationEvent {
    /// Short label for diagnostics, without any content.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Started { .. } => "started",
            Self::Token { .. } => "token",
            Self::Thinking { .. } => "thinking",
            Self::Completed { .. } => "completed",
            Self::Cancelled { .. } => "cancelled",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Handle of a running generation, used to cancel it.
#[derive(Clone, Debug)]
pub struct GenerationHandle {
    id: Uuid,
    cancel: Arc<AtomicBool>,
}

impl GenerationHandle {
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Requests cancellation. Idempotent.
    pub fn cancel(&self) -> bool {
        !self.cancel.swap(true, Ordering::SeqCst)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Sink for generation events.
pub type EventSink = Arc<dyn Fn(GenerationEvent) + Send + Sync>;

/// A generation that finished.
#[derive(Clone, Debug, Default)]
pub struct GenerationOutcome {
    pub text: String,
    pub thinking: String,
    pub cancelled: bool,
    pub usage: Option<super::client::GenerationUsage>,
    pub finish_reason: Option<String>,
    /// Whether the server sent a separate reasoning field.
    pub saw_reasoning_field: bool,
}

struct ServerState {
    server: Option<ManagedServer>,
    state: LocalAiState,
    last_error: Option<String>,
    started_at: Option<Instant>,
    capabilities: LocalAiCapabilities,
    generating: bool,
    cancel: Option<Arc<AtomicBool>>,
    config: LocalAiConfig,
}

/// The managed local AI runtime.
pub struct LocalAiGateway {
    state: Arc<Mutex<ServerState>>,
    runner: Arc<dyn ProcessRunner>,
}

impl Default for LocalAiGateway {
    fn default() -> Self {
        Self::new(LocalAiConfig::default())
    }
}

impl std::fmt::Debug for LocalAiGateway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalAiGateway")
            .field("state", &self.state())
            .field("config", &self.config())
            .finish()
    }
}

impl LocalAiGateway {
    pub fn new(config: LocalAiConfig) -> Self {
        Self::with_runner(config, Arc::new(RealProcessRunner))
    }

    /// Builds a gateway with an explicit process runner, for tests.
    pub fn with_runner(config: LocalAiConfig, runner: Arc<dyn ProcessRunner>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ServerState {
                server: None,
                state: LocalAiState::Stopped,
                last_error: None,
                started_at: None,
                capabilities: LocalAiCapabilities::default(),
                generating: false,
                cancel: None,
                config,
            })),
            runner,
        }
    }

    /// Replaces the configuration. Refused while the server is busy.
    pub fn apply_config(&self, config: LocalAiConfig) -> Result<(), ChatError> {
        let mut state = self.lock();
        if state.state.is_busy() {
            return Err(ChatError::ServerNotReady);
        }
        state.config = config;
        Ok(())
    }

    pub fn config(&self) -> LocalAiConfig {
        self.lock().config.clone()
    }

    /// The state enum on its own, without probing.
    pub fn state(&self) -> LocalAiState {
        self.lock().state
    }

    fn lock(&self) -> MutexGuard<'_, ServerState> {
        // A poisoned mutex means another thread panicked while holding it;
        // recovering keeps the status queryable instead of failing forever.
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn set_failed(&self, error: &ChatError) {
        let mut state = self.lock();
        state.state = LocalAiState::Failed;
        state.last_error = Some(error.to_string());
        state.generating = false;
        state.cancel = None;
    }

    /// Validates files, then estimates whether the machine can run them.
    pub fn validate(&self, config: &LocalAiConfig) -> LocalAiReport {
        let shape_issues = config.validate_shape().err().unwrap_or_default();
        let model_validation = validate_model(config);
        let model_size = model_validation
            .model
            .as_ref()
            .map(|model| model.size_bytes)
            .unwrap_or(0);
        let resources = estimate_resources(config, model_size);

        let mut issues: Vec<ValidationIssue> = shape_issues
            .iter()
            .map(|issue| ValidationIssue::blocked(&issue.field, &issue.message))
            .collect();
        issues.extend(model_validation.issues.clone());
        issues.extend(resources.issues.clone());

        let level = issues
            .iter()
            .map(|issue| issue.level)
            .max()
            .unwrap_or(CheckLevel::Ok);
        LocalAiReport {
            level,
            issues,
            model: model_validation.model,
            server_file: model_validation.server_file,
            resources,
        }
    }

    /// Starts the managed server and waits until it answers.
    ///
    /// Idempotent: calling it while the server is ready, starting, or generating
    /// returns the current status instead of spawning a second process.
    pub fn start(&self) -> Result<LocalAiStatus, ChatError> {
        let (config, already) = {
            let state = self.lock();
            (state.config.clone(), state.state)
        };
        if matches!(
            already,
            LocalAiState::Ready
                | LocalAiState::Generating
                | LocalAiState::Starting
                | LocalAiState::Stopping
        ) {
            return self.status();
        }

        // Fail before spawning anything when the machine cannot run it.
        let report = self.validate(&config);
        if report.is_blocked() {
            let detail = report
                .issues
                .iter()
                .find(|issue| issue.level == CheckLevel::Blocked)
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "the configuration cannot be used".to_string());
            let error = ChatError::InsufficientResources(detail);
            self.set_failed(&error);
            return Err(error);
        }

        let program: PathBuf = config.server_path().ok_or(ChatError::NotConfigured)?;
        let arguments = config.server_arguments()?;

        {
            let mut state = self.lock();
            state.state = LocalAiState::Starting;
            state.last_error = None;
        }

        let mut server = match ManagedServer::start(self.runner.as_ref(), &program, &arguments) {
            Ok(server) => server,
            Err(error) => {
                self.set_failed(&error);
                return Err(error);
            }
        };

        // Readiness is probed outside the state lock, so status stays responsive.
        let client = LocalAiClient::new(config.server.host.trim(), config.server.port)?;
        let timeout = Duration::from_secs(config.server.startup_timeout_seconds);
        let ready =
            super::process::wait_until_ready(&mut server, timeout, READY_POLL_INTERVAL, || {
                match client.health() {
                    Ok(HealthState::Ready) => Ok(true),
                    Ok(_) => Ok(false),
                    Err(ChatError::ServerNotRunning) | Err(ChatError::TimedOut) => Ok(false),
                    Err(other) => Err(other),
                }
            });

        if let Err(error) = ready {
            server.shutdown();
            self.set_failed(&error);
            return Err(error);
        }

        // Capability probe, also outside the lock.
        let probe = client.probe();

        {
            let mut state = self.lock();
            state.server = Some(server);
            state.state = LocalAiState::Ready;
            state.started_at = Some(Instant::now());
            state.capabilities = capabilities_from_probe(&probe);
            state.last_error = None;
        }
        self.status()
    }

    /// Stops the managed server.
    ///
    /// Only the child started by this process is affected: no process is
    /// discovered or killed by name, so a server the user started by hand is left
    /// untouched.
    pub fn stop(&self) -> Result<LocalAiStatus, ChatError> {
        // The idle case must leave the lock before asking for a status: the
        // status query takes the same non-reentrant mutex, so calling it while
        // the guard is still alive would deadlock the calling thread.
        let idle = {
            let mut state = self.lock();
            if state.server.is_none() {
                state.state = LocalAiState::Stopped;
                state.generating = false;
                state.cancel = None;
                state.capabilities = LocalAiCapabilities::default();
                state.started_at = None;
                true
            } else {
                state.state = LocalAiState::Stopping;
                if let Some(cancel) = state.cancel.take() {
                    cancel.store(true, Ordering::SeqCst);
                }
                false
            }
        };
        if idle {
            return self.status();
        }

        // Take the server out of the state, then stop it without holding the lock.
        let mut server = self.lock().server.take();
        if let Some(server) = server.as_mut() {
            let result = server.stop();
            server.shutdown();
            {
                let mut state = self.lock();
                state.state = LocalAiState::Stopped;
                state.generating = false;
                state.cancel = None;
                state.started_at = None;
                state.capabilities = LocalAiCapabilities::default();
            }
            result?;
        }
        self.status()
    }

    /// Restarts the managed server.
    pub fn restart(&self) -> Result<LocalAiStatus, ChatError> {
        let _ = self.stop();
        self.start()
    }

    /// Stops the server if it is running, without reporting a status.
    pub fn shutdown(&self) {
        let _ = self.stop();
    }

    /// Current status, with a short probe when a server is supposed to be running.
    pub fn status(&self) -> Result<LocalAiStatus, ChatError> {
        let (config, alive) = {
            let mut state = self.lock();
            let mut alive = state.server.is_some();
            if let Some(server) = state.server.as_mut() {
                match server.try_wait() {
                    Ok(Some(_exit)) => {
                        alive = false;
                        state.server = None;
                        state.state = LocalAiState::Failed;
                        state.generating = false;
                        state.cancel = None;
                        state.last_error = Some(ChatError::ServerStopped.to_string());
                    }
                    Ok(None) => {}
                    Err(_) => {}
                }
            } else if !matches!(state.state, LocalAiState::Starting) {
                alive = false;
            }

            let (tail, truncated, out_of_memory) = match state.server.as_ref() {
                Some(server) => (
                    server.stderr_tail(),
                    server.stderr_was_truncated(),
                    server.indicates_out_of_memory(),
                ),
                None => (Vec::new(), false, false),
            };
            let pid = state.server.as_ref().and_then(|server| server.pid());
            let status = LocalAiStatus {
                state: state.state,
                host: state.config.server.host.trim().to_string(),
                port: state.config.server.port,
                pid,
                model_file: state
                    .config
                    .model_path()
                    .map(|path| super::model::display_name(&path)),
                server_file: state
                    .config
                    .server_path()
                    .map(|path| super::model::display_name(&path)),
                profile: state.config.profile,
                thinking: state.config.thinking,
                capabilities: state.capabilities.clone(),
                last_error: state.last_error.clone(),
                stderr_tail: tail,
                stderr_truncated: truncated,
                out_of_memory_hint: out_of_memory,
                uptime: state
                    .started_at
                    .map(|started| format!("{}s", started.elapsed().as_secs())),
                generating: state.generating,
            };
            (state.config.clone(), (status, alive))
        };

        let (mut status, alive) = alive;
        if alive {
            if let Ok(client) = LocalAiClient::new(config.server.host.trim(), config.server.port) {
                let probe = client.probe();
                let mut capabilities = capabilities_from_probe(&probe);
                let mut state = self.lock();
                if state.server.is_some() {
                    // A probe describes what the server offers, not what a
                    // response contained, so an observed reasoning field has to
                    // survive the refresh instead of being reset on every poll.
                    capabilities.reasoning_field_observed =
                        state.capabilities.reasoning_field_observed;
                    state.capabilities = capabilities.clone();
                }
                status.capabilities = capabilities;
            }
        }
        Ok(status)
    }

    /// Cancels the running generation, if any.
    pub fn cancel(&self) -> bool {
        let cancel = self.lock().cancel.clone();
        match cancel {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Starts a streaming generation, reporting events through `sink`.
    ///
    /// The work happens on a worker thread, so the caller (and therefore the
    /// window) never blocks, and cancellation is a flag check between tokens.
    pub fn start_generation(
        &self,
        request: GenerationRequest,
        sink: EventSink,
    ) -> Result<GenerationHandle, ChatError> {
        if request.is_empty() {
            return Err(ChatError::InvalidConfiguration(
                "write a message first".to_string(),
            ));
        }

        let config = {
            let mut state = self.lock();
            if state.server.is_none() {
                return Err(ChatError::ServerNotRunning);
            }
            if state.generating {
                // Only one generation per session, by construction.
                return Err(ChatError::GenerationInProgress);
            }
            state.generating = true;
            state.state = LocalAiState::Generating;
            state.config.clone()
        };
        let capabilities = self.lock().capabilities.clone();

        let cancel = Arc::new(cancellation_flag());
        {
            let mut state = self.lock();
            state.cancel = Some(Arc::clone(&cancel));
        }
        let id = Uuid::new_v4();
        let handle = GenerationHandle {
            id,
            cancel: Arc::clone(&cancel),
        };

        let profile = request.profile.unwrap_or(config.profile);
        let thinking = request.thinking.unwrap_or(config.thinking);
        let model = capabilities
            .model_id
            .clone()
            .unwrap_or_else(|| model_label(&config));
        let built = ChatCompletionRequest::build(
            &model,
            profile,
            &request.messages,
            CompletionOptions {
                thinking,
                max_tokens: request.max_tokens.unwrap_or(config.max_tokens),
                temperature: request.temperature.unwrap_or(config.temperature),
                top_p: request.top_p.unwrap_or(config.top_p),
                stream: request.stream,
                thinking_supported: capabilities.thinking_switch,
            },
        );

        sink(GenerationEvent::Started {
            id,
            model,
            profile,
            thinking,
            thinking_applied: built.chat_template_kwargs.is_some(),
            stream: request.stream,
        });

        let stream_mode = request.stream;
        let state_handle = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("jarvis-local-ai".to_string())
            .spawn(move || {
                let timer = StreamTimer::new();
                let result = run_generation(&config, &built, stream_mode, &cancel, &sink);
                let duration_ms = timer.elapsed_ms();

                // Publish the completion before emitting the final event, so a
                // status query right after the event sees the settled state.
                let mut saw_thinking = false;
                {
                    let mut state = state_handle
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    state.generating = false;
                    state.cancel = None;
                    match &result {
                        Ok(outcome) => {
                            saw_thinking = outcome.saw_reasoning_field;
                            if state.server.is_some() {
                                state.state = LocalAiState::Ready;
                            }
                        }
                        Err(error) if *error == ChatError::Cancelled => {
                            if state.server.is_some() {
                                state.state = LocalAiState::Ready;
                            }
                        }
                        Err(error) => {
                            state.last_error = Some(error.to_string());
                            state.state = LocalAiState::Failed;
                        }
                    }
                    if saw_thinking {
                        state.capabilities.reasoning_field_observed = true;
                    }
                }

                match result {
                    Ok(outcome) if outcome.cancelled => sink(GenerationEvent::Cancelled {
                        partial: !outcome.text.is_empty(),
                        duration_ms,
                    }),
                    Ok(_) => {}
                    Err(ChatError::Cancelled) => sink(GenerationEvent::Cancelled {
                        partial: true,
                        duration_ms,
                    }),
                    Err(error) => sink(GenerationEvent::Failed {
                        error: error.to_string(),
                    }),
                }
            })
            .map_err(|_| {
                let mut state = self.lock();
                state.generating = false;
                state.cancel = None;
                ChatError::ProcessUnavailable
            })?;

        Ok(handle)
    }

    /// Runs a non-streaming generation on the calling thread.
    ///
    /// Used by [`ChatProvider`] and by tests; the interface uses the streaming
    /// API so the window never blocks.
    pub fn generate_blocking(
        &self,
        request: GenerationRequest,
        sink: EventSink,
    ) -> Result<GenerationOutcome, ChatError> {
        if request.is_empty() {
            return Err(ChatError::InvalidConfiguration(
                "write a message first".to_string(),
            ));
        }
        let config = {
            let mut state = self.lock();
            if state.server.is_none() {
                return Err(ChatError::ServerNotRunning);
            }
            if state.generating {
                return Err(ChatError::GenerationInProgress);
            }
            state.generating = true;
            state.config.clone()
        };
        let capabilities = self.lock().capabilities.clone();
        let profile = request.profile.unwrap_or(config.profile);
        let thinking = request.thinking.unwrap_or(config.thinking);
        let model = capabilities
            .model_id
            .clone()
            .unwrap_or_else(|| model_label(&config));
        let built = ChatCompletionRequest::build(
            &model,
            profile,
            &request.messages,
            CompletionOptions {
                thinking,
                max_tokens: request.max_tokens.unwrap_or(config.max_tokens),
                temperature: request.temperature.unwrap_or(config.temperature),
                top_p: request.top_p.unwrap_or(config.top_p),
                // A blocking generation cannot be cancelled part-way, so it never
                // asks the server to stream.
                stream: false,
                thinking_supported: capabilities.thinking_switch,
            },
        );

        sink(GenerationEvent::Started {
            id: Uuid::new_v4(),
            model,
            profile,
            thinking,
            thinking_applied: built.chat_template_kwargs.is_some(),
            stream: false,
        });

        let timer = StreamTimer::new();
        let result = LocalAiClient::new(config.server.host.trim(), config.server.port)
            .and_then(|client| client.chat_once(&built));

        let mut state = self.lock();
        state.generating = false;
        state.cancel = None;
        match result {
            Ok(text) => {
                if state.server.is_some() {
                    state.state = LocalAiState::Ready;
                }
                drop(state);
                sink(GenerationEvent::Completed {
                    usage: None,
                    finish_reason: None,
                    duration_ms: timer.elapsed_ms(),
                    cancelled: false,
                });
                Ok(GenerationOutcome {
                    text,
                    ..GenerationOutcome::default()
                })
            }
            Err(error) => {
                state.last_error = Some(error.to_string());
                state.state = LocalAiState::Failed;
                drop(state);
                sink(GenerationEvent::Failed {
                    error: error.to_string(),
                });
                Err(error)
            }
        }
    }
}

/// Runs one streaming generation and reports events.
fn run_generation(
    config: &LocalAiConfig,
    request: &ChatCompletionRequest,
    streaming: bool,
    cancel: &Arc<AtomicBool>,
    sink: &EventSink,
) -> Result<GenerationOutcome, ChatError> {
    let client = LocalAiClient::new(config.server.host.trim(), config.server.port)?;
    let timer = StreamTimer::new();

    if !streaming {
        let text = client.chat_once(request)?;
        sink(GenerationEvent::Completed {
            usage: None,
            finish_reason: None,
            duration_ms: timer.elapsed_ms(),
            cancelled: false,
        });
        return Ok(GenerationOutcome {
            text,
            ..GenerationOutcome::default()
        });
    }

    let mut outcome = GenerationOutcome::default();
    let result = client.stream_chat(request, cancel, &mut |chunk| match chunk {
        ChunkOutcome::Token(text) => {
            outcome.text.push_str(&text);
            sink(GenerationEvent::Token { text });
        }
        ChunkOutcome::Thinking(text) => {
            outcome.thinking.push_str(&text);
            outcome.saw_reasoning_field = true;
            sink(GenerationEvent::Thinking { text });
        }
        ChunkOutcome::TokenAndThinking { text, thinking } => {
            // One delta, two events: the reasoning is reported first, then the
            // answer text, so neither half is lost and the order stays natural.
            outcome.thinking.push_str(&thinking);
            outcome.saw_reasoning_field = true;
            sink(GenerationEvent::Thinking { text: thinking });
            outcome.text.push_str(&text);
            sink(GenerationEvent::Token { text });
        }
        ChunkOutcome::Completed {
            finish_reason,
            usage,
        } => {
            outcome.finish_reason = finish_reason;
            outcome.usage = usage;
            sink(GenerationEvent::Completed {
                usage,
                finish_reason: outcome.finish_reason.clone(),
                duration_ms: timer.elapsed_ms(),
                cancelled: false,
            });
        }
        ChunkOutcome::Empty | ChunkOutcome::Malformed => {}
    });

    match result {
        Ok(()) => Ok(outcome),
        Err(ChatError::Cancelled) => {
            outcome.cancelled = true;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
}

fn capabilities_from_probe(probe: &ServerProbe) -> LocalAiCapabilities {
    LocalAiCapabilities {
        endpoint_available: probe.reachable,
        model_id: probe.model_id(),
        models: probe.model_ids.clone(),
        // llama-server always offers the streaming endpoint this client uses.
        streaming: probe.reachable,
        thinking_switch: probe.thinking_switch_in_template,
        reasoning_field_observed: false,
        build_info: probe.build_info.clone(),
        notes: probe.notes.clone(),
    }
}

fn model_label(config: &LocalAiConfig) -> String {
    config
        .model_path()
        .map(|path| super::model::display_name(&path))
        .unwrap_or_else(|| "local-model".to_string())
}

/// Validation report shown to the user before starting.
#[derive(Clone, Debug, Serialize)]
pub struct LocalAiReport {
    pub level: CheckLevel,
    pub issues: Vec<ValidationIssue>,
    pub model: Option<ModelFileInfo>,
    pub server_file: Option<String>,
    pub resources: ResourceEstimate,
}

impl LocalAiReport {
    pub fn is_blocked(&self) -> bool {
        self.level == CheckLevel::Blocked
    }
}

/// The gateway also implements the provider-neutral chat contract.
///
/// This is the only implementation that talks to a real model. `send_message`
/// blocks the calling thread, so callers in the interface use the streaming API
/// instead; the contract exists for non-UI callers and for tests.
impl ChatProvider for LocalAiGateway {
    fn send_message(
        &self,
        request: ChatRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ChatResponse, ChatError>> + Send + '_>,
    > {
        Box::pin(async move {
            let generation = GenerationRequest {
                messages: request.messages,
                profile: Some(request.persona),
                thinking: None,
                stream: false,
                max_tokens: None,
                temperature: None,
                top_p: None,
            };
            let outcome = self.generate_blocking(generation, Arc::new(|_| {}))?;
            Ok(ChatResponse {
                content: outcome.text,
            })
        })
    }
}

/// The prompt version currently in use, for diagnostics.
pub fn prompt_version() -> &'static str {
    prompt::PROMPT_VERSION
}

/// Whether a profile is one of the built-in ones.
pub fn is_known_profile(persona: Persona) -> bool {
    matches!(persona, Persona::Jarvis | Persona::Altron)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// How long a transition may take before it counts as a deadlock.
    const TRANSITION_TIMEOUT: Duration = Duration::from_secs(10);

    /// Runs `action` on another thread and fails instead of hanging forever.
    ///
    /// The state mutex is not reentrant, so a transition that asks for a status
    /// while it still holds the lock blocks its own thread. A watchdog makes that
    /// failure visible as a test failure rather than as a stuck test run.
    fn without_deadlock<T: Send + 'static>(action: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(action());
        });
        receiver
            .recv_timeout(TRANSITION_TIMEOUT)
            .expect("the transition deadlocked on the state lock")
    }

    #[test]
    fn stopping_a_gateway_that_never_started_returns_immediately() {
        let gateway = LocalAiGateway::default();
        let status = without_deadlock(move || gateway.stop().expect("stopping must not fail"));
        assert_eq!(status.state, LocalAiState::Stopped);
        assert!(!status.generating);
        assert!(status.pid.is_none());
    }

    #[test]
    fn stopping_twice_and_shutting_down_an_idle_gateway_stay_responsive() {
        let gateway = LocalAiGateway::default();
        let gateway = Arc::new(gateway);
        let second = Arc::clone(&gateway);
        let status =
            without_deadlock(move || second.stop().expect("the second stop must not fail"));
        assert_eq!(status.state, LocalAiState::Stopped);

        // `shutdown` is the exit path: it must return even when nothing runs.
        without_deadlock(move || gateway.shutdown());
    }

    #[test]
    fn a_status_query_on_a_stopped_gateway_locks_once() {
        let gateway = LocalAiGateway::default();
        let status = without_deadlock(move || gateway.status().expect("status must not fail"));
        assert_eq!(status.state, LocalAiState::Stopped);
        assert!(!status.capabilities.endpoint_available);
        assert!(status.stderr_tail.is_empty());
        assert!(status.last_error.is_none());
    }

    #[test]
    fn cancelling_without_a_generation_is_a_no_op() {
        let gateway = LocalAiGateway::default();
        assert!(!gateway.cancel());
    }

    #[test]
    fn a_restart_without_a_configuration_reports_the_problem_instead_of_starting() {
        let gateway = LocalAiGateway::default();
        // `restart` stops first and then tries to start; with no paths configured
        // it must report an invalid configuration rather than spawn anything.
        let error = without_deadlock(move || gateway.restart().expect_err("must be refused"));
        assert!(matches!(
            error,
            ChatError::InsufficientResources(_) | ChatError::InvalidConfiguration(_)
        ));
    }
}
