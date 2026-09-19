//! Local AI runtime, part one: configuration, model checks, and the loopback
//! transport.
//!
//! ```text
//! configuration + GGUF validation  ->  config.rs, model.rs
//! loopback HTTP/1.1 + SSE client   ->  http.rs, client.rs
//! ```
//!
//! The client speaks to one peer only: a `llama-server` on `127.0.0.1` or `::1`.
//! It has no TLS, no redirects, no proxy, and no cookie handling, and it never
//! logs a body, because the bodies contain the user's conversation. The process
//! lifecycle and the gateway that drives it are built on top of these modules.

pub mod client;
pub mod config;
pub mod http;
pub mod model;

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
pub use http::{
    loopback_address, BodyReader, LoopbackEndpoint, ResponseHead, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_READ_TIMEOUT, DEFAULT_STALL_TIMEOUT, MAX_BODY_BYTES,
};
pub use model::{
    display_name, read_gguf_info, validate_files, validate_model, CheckLevel, GgufInfo,
    LocalModelConfig as ModelFileInfo, ModelValidation, ValidationIssue,
};
