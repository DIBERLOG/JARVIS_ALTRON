//! Local AI runtime: one gateway in front of a managed `llama-server`.
//!
//! ```text
//! interface  ->  LocalAiGateway  ->  llama-server (127.0.0.1, OpenAI-compatible)
//!                |  config + validation + resources
//!                |  process lifecycle (start, ready, stop, stderr tail)
//!                `  streaming client + cancellation
//! ```
//!
//! The interface never spawns a process or speaks HTTP: it asks the gateway for a
//! status, for a validation report, and for a generation. The gateway has no
//! access to the encrypted storages, no shell, and no tool surface — see
//! `docs/ADR_LOCAL_AI_GATEWAY.md`.

pub mod client;
pub mod config;
pub mod gateway;
pub mod http;
pub mod model;
pub mod process;
pub mod resources;
pub mod managed;

pub use client::{
    ApiMessage, ChatCompletionRequest, ChatTemplateKwargs, ChunkOutcome, CompletionOptions,
    GenerationUsage, HealthState, LocalAiClient, ServerProbe, SseEvent, SseParser, StreamOptions,
    StreamTimer, CHAT_COMPLETIONS_PATH, HEALTH_PATH, MODELS_PATH, PROPS_PATH,
};
pub use config::{
    ConfigIssue, LocalAiConfig, LocalModelConfig, ThinkingMode, CONFIG_SCHEMA_VERSION,
    DEFAULT_CONTEXT_SIZE, DEFAULT_HOST, DEFAULT_MAX_TOKENS, DEFAULT_PORT,
    DEFAULT_STARTUP_TIMEOUT_SECONDS, DEFAULT_TEMPERATURE, DEFAULT_TOP_P, SETTINGS_KEY,
};
pub use gateway::{
    EventSink, GenerationEvent, GenerationHandle, GenerationOutcome, GenerationRequest,
    LocalAiCapabilities, LocalAiGateway, LocalAiReport, LocalAiState, LocalAiStatus,
    READY_POLL_INTERVAL,
};
pub use http::{
    loopback_address, BodyReader, LoopbackEndpoint, ResponseHead, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_READ_TIMEOUT, DEFAULT_STALL_TIMEOUT, MAX_BODY_BYTES,
};
pub use model::{
    display_name, read_gguf_info, validate_files, validate_model, CheckLevel, GgufInfo,
    LocalModelConfig as ModelFileInfo, ModelValidation, ValidationIssue,
};
pub use process::{
    drain_stderr, ManagedServer, ProcessRunner, RealProcessRunner, ServerExit, ServerProcess,
    SpawnedServer, StderrBuffer, EXIT_POLL_INTERVAL, KILL_GRACE, STDERR_LINE_CHARS,
    STDERR_TAIL_LINES,
};
pub use resources::{
    available_memory_bytes, estimate_resources, format_bytes, total_memory_bytes, ResourceEstimate,
    HEADROOM_WARNING_RATIO, KV_BYTES_PER_TOKEN_ESTIMATE, RUNTIME_OVERHEAD_BYTES,
};
pub use managed::{
    download_verified, managed_model_manifest, managed_runtime_manifest, managed_model_directory,
    managed_runtime_directory, DownloadError, DownloadProgress, ManagedArtifact,
    ManagedModelManifest, ManagedRuntimeManifest,
};
