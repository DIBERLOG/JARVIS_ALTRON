//! OpenAI-compatible client for the local `llama-server` endpoint.
//!
//! The client speaks only to loopback (enforced in [`super::http`]), keeps no
//! prompt or completion in any error, and turns a Server-Sent Events stream into
//! typed [`GenerationEvent`] values.
//!
//! Reasoning output: a separate `reasoning_content` delta field is reported as
//! [`GenerationEvent::Thinking`] **only** when the server actually sends that
//! field. Text that merely looks like reasoning is never split out with pattern
//! matching, so the interface cannot claim a distinction the backend did not
//! make. Whether the server can even accept a thinking preference depends on the
//! chat template of the loaded model and on the llama.cpp build; the capability
//! probe reports that instead of assuming it.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::config::ThinkingMode;
use super::http::{LoopbackEndpoint, ResponseHead};
use crate::ai::{ChatError, ChatMessage, ChatRole, Persona};

/// Paths of the llama-server HTTP API.
pub const HEALTH_PATH: &str = "/health";
pub const PROPS_PATH: &str = "/props";
pub const MODELS_PATH: &str = "/v1/models";
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

/// Timeout for the short capability probes.
const PROBE_STALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Token accounting reported by the server, when it reports any.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GenerationUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

/// One SSE event after parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Incremental Server-Sent Events parser.
///
/// Comment lines are ignored, multiple `data:` lines are joined with newlines,
/// and an event is dispatched on the blank line that terminates it.
#[derive(Default)]
pub struct SseParser {
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one line; returns an event when the line completes one.
    pub fn push_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            if self.data.is_empty() && self.event.is_none() {
                return None;
            }
            let event = SseEvent {
                event: self.event.take(),
                data: self.data.join("\n"),
            };
            self.data.clear();
            // An event with no data at all is not useful to the caller.
            if event.data.is_empty() && event.event.is_none() {
                return None;
            }
            return Some(event);
        }
        if let Some(rest) = line.strip_prefix(':') {
            let _ = rest; // comment, for example a keep-alive
            return None;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            self.event = Some(rest.trim().to_string());
            return None;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            // A single leading space after the colon is part of the format.
            let value = rest.strip_prefix(' ').unwrap_or(rest);
            self.data.push(value.to_string());
            return None;
        }
        // Unknown field names are ignored, as the specification requires.
        None
    }

    /// Whether a partial event is buffered.
    pub fn has_pending(&self) -> bool {
        !self.data.is_empty() || self.event.is_some()
    }
}

/// A chat completion request in the OpenAI shape.
#[derive(Clone, Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ApiMessage>,
    pub stream: bool,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    /// Forwarded to the chat template when the server supports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<ChatTemplateKwargs>,
    /// Ask for the token accounting on the final chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ChatTemplateKwargs {
    pub enable_thinking: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApiMessage {
    pub role: &'static str,
    pub content: String,
}

impl ApiMessage {
    pub fn from_chat(message: &ChatMessage) -> Self {
        Self {
            role: match message.role {
                ChatRole::System => "system",
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
            },
            content: message.content.clone(),
        }
    }
}

/// Sampling, streaming, and reasoning options for one completion.
///
/// Grouped into one value because they always travel together: the gateway reads
/// them from the configuration and the request only overrides what the caller
/// asked for explicitly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompletionOptions {
    pub thinking: ThinkingMode,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub stream: bool,
    /// Whether the server's chat template accepts a thinking switch at all.
    pub thinking_supported: bool,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            thinking: ThinkingMode::default(),
            max_tokens: super::config::DEFAULT_MAX_TOKENS,
            temperature: super::config::DEFAULT_TEMPERATURE,
            top_p: super::config::DEFAULT_TOP_P,
            stream: false,
            thinking_supported: false,
        }
    }
}

impl ChatCompletionRequest {
    /// Builds a request for one profile, including its system prompt.
    pub fn build(
        model: &str,
        persona: Persona,
        messages: &[ChatMessage],
        options: CompletionOptions,
    ) -> Self {
        let mut api_messages = Vec::with_capacity(messages.len() + 1);
        api_messages.push(ApiMessage {
            role: "system",
            content: crate::ai::system_prompt(persona),
        });
        api_messages.extend(messages.iter().map(ApiMessage::from_chat));

        // A preference is only sent when the loaded template accepts one.
        let chat_template_kwargs = match (options.thinking, options.thinking_supported) {
            (ThinkingMode::Disabled, true) => Some(ChatTemplateKwargs {
                enable_thinking: false,
            }),
            (ThinkingMode::Enabled, true) => Some(ChatTemplateKwargs {
                enable_thinking: true,
            }),
            _ => None,
        };

        Self {
            model: model.to_string(),
            messages: api_messages,
            stream: options.stream,
            max_tokens: options.max_tokens,
            temperature: options.temperature,
            top_p: options.top_p,
            chat_template_kwargs,
            stream_options: options.stream.then_some(StreamOptions {
                include_usage: true,
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ChunkResponse {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<GenerationUsage>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<ChunkDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    /// Present only on builds and templates that separate reasoning.
    #[serde(default)]
    reasoning_content: Option<String>,
}

/// What one parsed chunk means to the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChunkOutcome {
    /// Ordinary answer text.
    Token(String),
    /// Reasoning text, reported only when the server sent a separate field.
    Thinking(String),
    /// One delta carried answer text and reasoning at once.
    ///
    /// Both are reported: dropping the answer here would silently lose the first
    /// tokens of a reply whenever a build closes its reasoning and opens the
    /// answer in the same chunk.
    TokenAndThinking { text: String, thinking: String },
    /// The stream finished.
    Completed {
        finish_reason: Option<String>,
        usage: Option<GenerationUsage>,
    },
    /// Nothing useful in this chunk.
    Empty,
    /// The chunk could not be parsed.
    Malformed,
}

/// Interprets one `data:` payload of the stream.
pub fn interpret_chunk(data: &str) -> ChunkOutcome {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return ChunkOutcome::Empty;
    }
    if trimmed == "[DONE]" {
        return ChunkOutcome::Completed {
            finish_reason: None,
            usage: None,
        };
    }
    match serde_json::from_str::<ChunkResponse>(trimmed) {
        Err(_) => ChunkOutcome::Malformed,
        Ok(response) => {
            let mut text = String::new();
            let mut thinking = String::new();
            let mut finish_reason = None;
            for choice in &response.choices {
                if let Some(delta) = &choice.delta {
                    if let Some(content) = &delta.content {
                        text.push_str(content);
                    }
                    if let Some(reasoning) = &delta.reasoning_content {
                        thinking.push_str(reasoning);
                    }
                }
                if choice.finish_reason.is_some() {
                    finish_reason = choice.finish_reason.clone();
                }
            }
            if !thinking.is_empty() && !text.is_empty() {
                return ChunkOutcome::TokenAndThinking { text, thinking };
            }
            if !thinking.is_empty() {
                return ChunkOutcome::Thinking(thinking);
            }
            if !text.is_empty() {
                return ChunkOutcome::Token(text);
            }
            if finish_reason.is_some() || response.usage.is_some() {
                return ChunkOutcome::Completed {
                    finish_reason,
                    usage: response.usage,
                };
            }
            ChunkOutcome::Empty
        }
    }
}

/// Whether the `/health` endpoint reports a usable server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthState {
    Loading,
    Ready,
    Unknown,
}

/// Result of probing the server.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServerProbe {
    pub reachable: bool,
    pub health: Option<HealthState>,
    pub model_ids: Vec<String>,
    /// `chat_template` of the loaded model, when the server reports it.
    pub chat_template_present: bool,
    /// The template mentions a thinking switch.
    pub thinking_switch_in_template: bool,
    pub build_info: Option<String>,
    pub notes: Vec<String>,
}

impl ServerProbe {
    /// First usable model identifier, if the server reports any.
    pub fn model_id(&self) -> Option<String> {
        self.model_ids.first().cloned()
    }
}

/// Client for the local endpoint.
#[derive(Clone, Debug)]
pub struct LocalAiClient {
    endpoint: LoopbackEndpoint,
}

impl LocalAiClient {
    pub fn new(host: &str, port: u16) -> Result<Self, ChatError> {
        Ok(Self {
            endpoint: LoopbackEndpoint::new(host, port)?,
        })
    }

    pub fn endpoint(&self) -> &LoopbackEndpoint {
        &self.endpoint
    }

    /// `GET /health`: 200 means ready, 503 means still loading.
    pub fn health(&self) -> Result<HealthState, ChatError> {
        let response = self
            .endpoint
            .request("GET", HEALTH_PATH, "application/json", None)?;
        let state = match response.head.status {
            200 => HealthState::Ready,
            503 => HealthState::Loading,
            _ => HealthState::Unknown,
        };
        // The body is intentionally not read and never logged.
        Ok(state)
    }

    /// Probes the server: health, model list, and chat-template capability.
    ///
    /// Every failure is reported as "not reachable" rather than as an error,
    /// because this runs while the server is still starting.
    pub fn probe(&self) -> ServerProbe {
        let mut probe = ServerProbe::default();

        match self
            .loopback_endpoint()
            .request("GET", HEALTH_PATH, "application/json", None)
        {
            Err(_) => {
                probe
                    .notes
                    .push("the server is not reachable yet".to_string());
                return probe;
            }
            Ok(response) => {
                probe.reachable = true;
                probe.health = Some(match response.head.status {
                    200 => HealthState::Ready,
                    503 => HealthState::Loading,
                    _ => HealthState::Unknown,
                });
            }
        }

        if let Ok((status, body)) = self.loopback_endpoint().get_json(MODELS_PATH) {
            if status == 200 {
                if let Ok(parsed) = serde_json::from_slice::<ModelsResponse>(&body) {
                    probe.model_ids = parsed
                        .data
                        .into_iter()
                        .map(|model| model.id)
                        .filter(|id| !id.is_empty())
                        .collect();
                }
            }
        }
        if probe.model_ids.is_empty() {
            probe
                .notes
                .push("the server did not report a model identifier".to_string());
        }

        if let Ok((status, body)) = self.loopback_endpoint().get_json(PROPS_PATH) {
            if status == 200 {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) {
                    if let Some(template) = value.get("chat_template").and_then(|v| v.as_str()) {
                        probe.chat_template_present = true;
                        let lowered = template.to_lowercase();
                        // Heuristic: the switch is only usable when the template
                        // references it. This depends on the llama.cpp build.
                        probe.thinking_switch_in_template =
                            lowered.contains("enable_thinking") || lowered.contains("thinking");
                    }
                    if let Some(build) = value.get("build_info").and_then(|v| v.as_str()) {
                        probe.build_info = Some(build.to_string());
                    }
                }
            }
        }
        if !probe.chat_template_present {
            probe.notes.push(
                "the chat template could not be read, so the thinking switch cannot be confirmed"
                    .to_string(),
            );
        }
        probe
    }

    fn loopback_endpoint(&self) -> LoopbackEndpoint {
        self.endpoint.clone().with_timeouts(
            super::http::DEFAULT_CONNECT_TIMEOUT,
            super::http::DEFAULT_READ_TIMEOUT,
            PROBE_STALL_TIMEOUT,
        )
    }

    /// Starts a streaming completion and reports chunks through `on_chunk`.
    ///
    /// Returns `Ok(())` when the stream completed, and `ChatError::Cancelled`
    /// when `cancel` was set.
    pub fn stream_chat(
        &self,
        request: &ChatCompletionRequest,
        cancel: &AtomicBool,
        on_chunk: &mut dyn FnMut(ChunkOutcome),
    ) -> Result<(), ChatError> {
        let body = serde_json::to_string(request).map_err(|_| {
            ChatError::InvalidConfiguration("request cannot be encoded".to_string())
        })?;
        let response = self.endpoint.request(
            "POST",
            CHAT_COMPLETIONS_PATH,
            "text/event-stream",
            Some(&body),
        )?;
        let status = response.head.status;
        if !response.head.is_success() {
            // The body may echo the prompt, so it is discarded, not reported.
            return Err(ChatError::HttpStatus(status));
        }
        let mut response = response;

        let mut parser = SseParser::new();
        let mut malformed = 0usize;
        let mut produced = 0usize;
        let body = response.body_mut();
        while let Some(line) = body.next_line(cancel)? {
            let Some(event) = parser.push_line(&line) else {
                continue;
            };
            match interpret_chunk(&event.data) {
                outcome @ (ChunkOutcome::Token(_)
                | ChunkOutcome::Thinking(_)
                | ChunkOutcome::TokenAndThinking { .. }) => {
                    produced += 1;
                    on_chunk(outcome);
                }
                ChunkOutcome::Completed {
                    finish_reason,
                    usage,
                } => {
                    on_chunk(ChunkOutcome::Completed {
                        finish_reason,
                        usage,
                    });
                    return Ok(());
                }
                ChunkOutcome::Empty => {}
                ChunkOutcome::Malformed => {
                    malformed += 1;
                }
            }
        }
        if parser.has_pending() {
            malformed += 1;
        }
        // A stream that produced nothing and contained unparsable data is a
        // failure, not an empty answer.
        if produced == 0 && malformed > 0 {
            return Err(ChatError::InvalidStream);
        }
        Ok(())
    }

    /// Non-streaming completion, used as a fallback and by `ChatProvider`.
    pub fn chat_once(&self, request: &ChatCompletionRequest) -> Result<String, ChatError> {
        let mut request = request.clone();
        request.stream = false;
        request.stream_options = None;
        let body = serde_json::to_string(&request).map_err(|_| {
            ChatError::InvalidConfiguration("request cannot be encoded".to_string())
        })?;
        let (status, bytes) = self.endpoint.post_json(CHAT_COMPLETIONS_PATH, &body)?;
        if status != 200 {
            return Err(ChatError::HttpStatus(status));
        }
        let parsed: CompletionResponse =
            serde_json::from_slice(&bytes).map_err(|_| ChatError::InvalidResponse)?;
        let content = parsed
            .choices
            .into_iter()
            .filter_map(|choice| choice.message)
            .map(|message| message.content)
            .collect::<Vec<_>>()
            .join("");
        if content.is_empty() {
            return Err(ChatError::InvalidResponse);
        }
        Ok(content)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct CompletionResponse {
    #[serde(default)]
    choices: Vec<CompletionChoice>,
}

#[derive(Clone, Debug, Deserialize)]
struct CompletionChoice {
    #[serde(default)]
    message: Option<CompletionMessage>,
}

#[derive(Clone, Debug, Deserialize)]
struct CompletionMessage {
    #[serde(default)]
    content: String,
}

/// Tracks a stream wall-clock duration for the completion event.
pub struct StreamTimer {
    started: Instant,
}

impl Default for StreamTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamTimer {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

/// Marks a cancellation flag and reports whether it was already set.
pub fn request_cancel(flag: &AtomicBool) -> bool {
    flag.swap(true, Ordering::SeqCst)
}

/// Creates a cancellation flag.
pub fn cancellation_flag() -> AtomicBool {
    AtomicBool::new(false)
}

/// Reports the head of a response without keeping the body.
pub fn describe_status(head: &ResponseHead) -> String {
    format!("HTTP {}", head.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sse_parser_dispatches_on_blank_lines() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push_line(": keep-alive"), None);
        assert_eq!(parser.push_line("data: {\"a\":1}"), None);
        let event = parser.push_line("").unwrap();
        assert_eq!(event.data, "{\"a\":1}");
        assert_eq!(event.event, None);
        assert!(!parser.has_pending());
    }

    #[test]
    fn the_sse_parser_joins_multiple_data_lines_and_keeps_event_names() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push_line("event: message"), None);
        assert_eq!(parser.push_line("data: first"), None);
        assert_eq!(parser.push_line("data: second"), None);
        let event = parser.push_line("").unwrap();
        assert_eq!(event.event.as_deref(), Some("message"));
        assert_eq!(event.data, "first\nsecond");
    }

    #[test]
    fn the_sse_parser_keeps_leading_space_rules_and_ignores_unknown_fields() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push_line("id: 42"), None);
        assert_eq!(parser.push_line("retry: 100"), None);
        assert_eq!(parser.push_line("data:no-space"), None);
        let event = parser.push_line("").unwrap();
        assert_eq!(event.data, "no-space");
        assert!(!parser.has_pending());

        // A blank line with nothing buffered produces nothing.
        assert_eq!(parser.push_line(""), None);
    }

    #[test]
    fn interpreting_chunks_reports_tokens_thinking_and_completion() {
        let token = interpret_chunk(r#"{"choices":[{"delta":{"content":"Hello"}}]}"#);
        assert_eq!(token, ChunkOutcome::Token("Hello".to_string()));

        let thinking =
            interpret_chunk(r#"{"choices":[{"delta":{"reasoning_content":"step by step"}}]}"#);
        assert_eq!(thinking, ChunkOutcome::Thinking("step by step".to_string()));

        let done = interpret_chunk("[DONE]");
        assert_eq!(
            done,
            ChunkOutcome::Completed {
                finish_reason: None,
                usage: None
            }
        );

        let finish = interpret_chunk(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":7,"total_tokens":12}}"#,
        );
        assert_eq!(
            finish,
            ChunkOutcome::Completed {
                finish_reason: Some("stop".to_string()),
                usage: Some(GenerationUsage {
                    prompt_tokens: 5,
                    completion_tokens: 7,
                    total_tokens: 12
                })
            }
        );

        assert_eq!(interpret_chunk("   "), ChunkOutcome::Empty);
        assert_eq!(interpret_chunk(r#"{"choices":[]}"#), ChunkOutcome::Empty);
        assert_eq!(interpret_chunk("{not json"), ChunkOutcome::Malformed);
    }

    #[test]
    fn a_chunk_with_both_fields_keeps_both() {
        let outcome = interpret_chunk(
            r#"{"choices":[{"delta":{"content":"answer","reasoning_content":"thought"}}]}"#,
        );
        // Neither half may be dropped: the answer is what the user reads.
        assert_eq!(
            outcome,
            ChunkOutcome::TokenAndThinking {
                text: "answer".to_string(),
                thinking: "thought".to_string()
            }
        );
    }

    #[test]
    fn the_request_includes_the_profile_prompt_and_constraints() {
        let messages = vec![ChatMessage {
            role: ChatRole::User,
            content: "FICTIONAL_USER_QUESTION".to_string(),
        }];
        let request = ChatCompletionRequest::build(
            "qwen3-8b",
            Persona::Altron,
            &messages,
            CompletionOptions {
                thinking: ThinkingMode::Disabled,
                max_tokens: 512,
                temperature: 0.5,
                top_p: 0.9,
                stream: true,
                thinking_supported: true,
            },
        );
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, "system");
        assert!(request.messages[0].content.contains("ALTRON"));
        assert!(request.messages[0].content.contains("no shell"));
        assert_eq!(request.messages[1].role, "user");
        assert_eq!(request.messages[1].content, "FICTIONAL_USER_QUESTION");
        assert_eq!(
            request
                .chat_template_kwargs
                .map(|kwargs| kwargs.enable_thinking),
            Some(false)
        );
        assert!(request.stream);
        assert!(request
            .stream_options
            .map(|options| options.include_usage)
            .unwrap_or(false));

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"max_tokens\":512"));
        assert!(json.contains("\"model\":\"qwen3-8b\""));
    }

    #[test]
    fn thinking_preference_is_only_sent_when_the_template_supports_it() {
        let messages = vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".to_string(),
        }];
        for mode in ThinkingMode::all() {
            let supported = ChatCompletionRequest::build(
                "m",
                Persona::Jarvis,
                &messages,
                CompletionOptions {
                    thinking: mode,
                    max_tokens: 16,
                    temperature: 0.7,
                    top_p: 0.95,
                    stream: true,
                    thinking_supported: true,
                },
            );
            let unsupported = ChatCompletionRequest::build(
                "m",
                Persona::Jarvis,
                &messages,
                CompletionOptions {
                    thinking: mode,
                    max_tokens: 16,
                    temperature: 0.7,
                    top_p: 0.95,
                    stream: true,
                    thinking_supported: false,
                },
            );
            match mode {
                ThinkingMode::Auto => {
                    assert!(supported.chat_template_kwargs.is_none());
                    assert!(unsupported.chat_template_kwargs.is_none());
                }
                ThinkingMode::Disabled => {
                    assert_eq!(
                        supported.chat_template_kwargs.map(|k| k.enable_thinking),
                        Some(false)
                    );
                    // Without template support the preference cannot be honoured.
                    assert!(unsupported.chat_template_kwargs.is_none());
                }
                ThinkingMode::Enabled => {
                    assert_eq!(
                        supported.chat_template_kwargs.map(|k| k.enable_thinking),
                        Some(true)
                    );
                    assert!(unsupported.chat_template_kwargs.is_none());
                }
            }
        }
    }

    #[test]
    fn the_request_never_carries_an_api_key_or_secret_field() {
        let request = ChatCompletionRequest::build(
            "m",
            Persona::Jarvis,
            &[ChatMessage {
                role: ChatRole::User,
                content: "FICTIONAL_USER_QUESTION".to_string(),
            }],
            CompletionOptions {
                thinking: ThinkingMode::Auto,
                max_tokens: 16,
                temperature: 0.7,
                top_p: 0.95,
                stream: false,
                thinking_supported: true,
            },
        );
        let value = serde_json::to_value(&request).unwrap();
        let object = value.as_object().unwrap();

        // The request carries exactly the OpenAI fields it needs, and nothing
        // that could hold a credential or unrelated data.
        let allowed = [
            "model",
            "messages",
            "stream",
            "max_tokens",
            "temperature",
            "top_p",
            "chat_template_kwargs",
            "stream_options",
        ];
        for key in object.keys() {
            assert!(
                allowed.contains(&key.as_str()),
                "unexpected request field: {key}"
            );
        }
        for forbidden in [
            "api_key",
            "apikey",
            "authorization",
            "password",
            "master_key",
            "notes",
            "storage",
            "secret",
        ] {
            assert!(
                !object.contains_key(forbidden),
                "a chat request must not carry a {forbidden} field"
            );
        }

        // Every message carries only a role and content.
        for message in object["messages"].as_array().unwrap() {
            let message = message.as_object().unwrap();
            assert_eq!(message.len(), 2);
            assert!(message.contains_key("role"));
            assert!(message.contains_key("content"));
        }
        // The user's own text is the only content that came from the interface.
        let rendered = value.to_string();
        assert!(rendered.contains("FICTIONAL_USER_QUESTION"));
        assert!(!rendered.contains("FICTIONAL_STORED_DATA"));
    }

    #[test]
    fn a_non_streaming_request_omits_stream_options() {
        let mut request = ChatCompletionRequest::build(
            "m",
            Persona::Jarvis,
            &[],
            CompletionOptions {
                thinking: ThinkingMode::Auto,
                max_tokens: 16,
                temperature: 0.7,
                top_p: 0.95,
                stream: true,
                thinking_supported: true,
            },
        );
        request.stream = false;
        request.stream_options = None;
        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains("stream_options"));
    }

    #[test]
    fn cancellation_helpers_are_idempotent() {
        let flag = cancellation_flag();
        assert!(!flag.load(Ordering::SeqCst));
        assert!(!request_cancel(&flag));
        assert!(request_cancel(&flag));
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn the_stream_timer_reports_milliseconds() {
        let timer = StreamTimer::new();
        assert!(timer.elapsed_ms() < 5_000);
    }

    #[test]
    fn the_client_refuses_a_non_loopback_host() {
        assert!(LocalAiClient::new("127.0.0.1", 8080).is_ok());
        assert!(LocalAiClient::new("0.0.0.0", 8080).is_err());
        assert!(LocalAiClient::new("192.168.0.10", 8080).is_err());
    }

    #[test]
    fn a_status_description_never_includes_a_body() {
        let head = ResponseHead {
            status: 500,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
        };
        assert_eq!(describe_status(&head), "HTTP 500");
    }
}
