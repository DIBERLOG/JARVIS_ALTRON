//! Summaries and memory candidates, produced by the existing local gateway.
//!
//! There is no second AI client: [`LocalAiSummaryProvider`] calls
//! `LocalAiGateway::generate_blocking`, which is the same managed `llama-server`,
//! the same profile prompt, and the same safety constraints the chat uses. Only the
//! *task* differs, and the task is stated in a user-level message.
//!
//! Three rules are enforced here rather than left to the model:
//!
//! * reasoning is never stored: the gateway returns the visible answer only, and
//!   nothing from the thinking channel is read;
//! * a summary is filtered by the secret module before it is returned, and a
//!   summary that looks like it contains a credential is **refused**;
//! * a candidate must name a user message that was actually sent to the model, so a
//!   sentence the model invented about the user can never become a memory entry.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use uuid::Uuid;

use crate::ai::local::{GenerationRequest, LocalAiGateway, ThinkingMode};
use crate::ai::{ChatMessage, ChatRole, Persona};
use crate::sync::{CryptoProvider, SyncRepository};

use super::error::MemoryError;
use super::model::{
    truncate_chars, FactView, MemoryCategory, MemoryScope, MessageRole, MessageStatus, MessageView,
    MAX_SUMMARY_BYTES,
};
use super::redaction;
use super::store::MemoryStore;

/// How many messages a summarization request may carry at once.
pub const MAX_SUMMARIZED_MESSAGES: usize = 200;
/// Longest single message included in a summarization request.
pub const MAX_SUMMARIZED_MESSAGE_CHARS: usize = 4000;
/// How many candidate suggestions are accepted from one answer.
pub const MAX_CANDIDATES_PER_REQUEST: usize = 8;
/// Longest candidate text accepted from a model.
pub const MAX_CANDIDATE_CHARS: usize = super::model::MAX_FACT_CHARS;

/// Instruction used for summarization.
pub const SUMMARY_INSTRUCTION: &str = "\
Summarize the earlier part of this conversation for your own future reference. \
Write plain text in the language the user writes in, with these five short \
sections and nothing else:
Topic:
Decisions:
Open tasks:
User requirements:
User corrections:
Keep it under 1200 characters. Never include passwords, API keys, tokens, \
recovery codes, or any other secret, and never include your own reasoning.";

/// Instruction used for candidate extraction.
pub const CANDIDATE_INSTRUCTION: &str = "\
Read the conversation and list only durable facts the USER stated about \
themselves: preferences, personal facts, projects, standing instructions, or \
corrections of you. Never list something you said about the user, and never \
invent anything. Answer with a JSON array and nothing else. Each item is \
{\"content\": \"...\", \"category\": \"preference|personal_fact|project|instruction|correction|other\", \
\"confidence\": 0.0-1.0, \"source_message\": \"<id of the user message it came from>\"}. \
If there is nothing, answer []. Never include secrets.";

/// One request for a summary.
pub struct SummaryRequest<'a> {
    pub persona: Persona,
    pub title: &'a str,
    /// Every message the summary must cover, oldest first.
    pub messages: &'a [MessageView],
    /// The summary being replaced, when there is one.
    pub previous: Option<&'a str>,
    /// How many newest messages stay verbatim and must stay out of the summary.
    pub keep_recent: usize,
}

/// One request for memory candidates.
pub struct CandidateRequest<'a> {
    pub persona: Persona,
    /// The messages to read, including the answer that was just produced.
    pub messages: &'a [MessageView],
}

/// A candidate the model proposed. It is not stored by this module.
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateSuggestion {
    pub content: String,
    pub category: MemoryCategory,
    pub scope: MemoryScope,
    pub confidence: f32,
    /// The user message the suggestion is anchored to.
    pub source_message_id: Uuid,
}

/// Produces summaries and candidate suggestions.
///
/// The trait exists so tests can use a deterministic fixture: no test needs a real
/// model, and the storage tests never touch the gateway.
pub trait SummaryProvider: Send + Sync {
    /// Writes a summary of `request.messages`.
    fn summarize(&self, request: &SummaryRequest<'_>) -> Result<String, MemoryError>;

    /// Proposes memory candidates. An empty answer is a valid answer.
    fn suggest_candidates(
        &self,
        request: &CandidateRequest<'_>,
    ) -> Result<Vec<CandidateSuggestion>, MemoryError>;
}

/// The production provider: a local gateway that is already running.
pub struct LocalAiSummaryProvider {
    gateway: Arc<LocalAiGateway>,
}

impl LocalAiSummaryProvider {
    pub fn new(gateway: Arc<LocalAiGateway>) -> Self {
        Self { gateway }
    }

    /// Runs one non-streaming generation and returns the visible answer.
    ///
    /// The generation runs on the calling thread; the command layer keeps it off the
    /// window thread with `spawn_blocking`.
    fn generate(
        &self,
        persona: Persona,
        instruction: &str,
        body: &str,
    ) -> Result<String, MemoryError> {
        let request = GenerationRequest {
            messages: vec![
                ChatMessage {
                    role: ChatRole::User,
                    content: instruction.to_string(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: body.to_string(),
                },
            ],
            profile: Some(persona),
            // Reasoning is never wanted here, and its result is never stored.
            thinking: Some(ThinkingMode::Disabled),
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
        };
        match self.gateway.generate_blocking(request, Arc::new(|_| {})) {
            Ok(outcome) => Ok(outcome.text),
            Err(crate::ai::ChatError::ServerNotRunning)
            | Err(crate::ai::ChatError::ServerNotReady)
            | Err(crate::ai::ChatError::NotConfigured) => Err(MemoryError::SummaryUnavailable),
            Err(error) => {
                log::warn!("memory: summarization failed: {error}");
                Err(MemoryError::SummaryUnavailable)
            }
        }
    }
}

impl SummaryProvider for LocalAiSummaryProvider {
    fn summarize(&self, request: &SummaryRequest<'_>) -> Result<String, MemoryError> {
        let body = render_summary_input(request);
        let answer = self.generate(request.persona, SUMMARY_INSTRUCTION, &body)?;
        let cleaned = clean_model_text(&answer);
        if cleaned.trim().is_empty() {
            return Err(MemoryError::SummarizerOutput);
        }
        if cleaned.len() > MAX_SUMMARY_BYTES {
            return Err(MemoryError::SummarizerOutput);
        }
        // A summary that looks like it carries a credential is refused outright:
        // the messages stay, and the chat continues without a summary.
        let scan = redaction::scan(&cleaned);
        if !scan.is_clean() {
            return Err(MemoryError::SecretDetected(scan.kinds()));
        }
        if super::model::looks_like_instruction(&cleaned) {
            // Not fatal, but the caller should know the summary reads like an order.
            log::debug!("memory: the summary contains instruction-like text");
        }
        Ok(cleaned)
    }

    fn suggest_candidates(
        &self,
        request: &CandidateRequest<'_>,
    ) -> Result<Vec<CandidateSuggestion>, MemoryError> {
        let body = render_candidate_input(request);
        let answer = self.generate(request.persona, CANDIDATE_INSTRUCTION, &body)?;
        parse_candidates(&answer, request)
    }
}

/// Renders the messages a summary must cover.
///
/// The newest `keep_recent` messages are left out: they stay verbatim in the
/// context, so the summary never becomes the only record of what was just said.
pub fn render_summary_input(request: &SummaryRequest<'_>) -> String {
    let total = request.messages.len();
    let keep = request.keep_recent.min(total);
    let cover = &request.messages[..total.saturating_sub(keep)];
    let mut body = String::new();
    body.push_str(&format!("Conversation title: {}\n", request.title));
    if let Some(previous) = request.previous.filter(|text| !text.trim().is_empty()) {
        body.push_str(&format!(
            "Previous summary, to be replaced:\n{}\n",
            truncate_chars(previous, 2000)
        ));
    }
    body.push_str("Messages to summarize:\n");
    for message in cover.iter().take(MAX_SUMMARIZED_MESSAGES) {
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        let content = truncate_chars(
            &message.content.replace('\n', " "),
            MAX_SUMMARIZED_MESSAGE_CHARS,
        );
        body.push_str(&format!("[{role}] {content}\n"));
    }
    body
}

/// Renders the messages candidates may be derived from, with their identifiers.
pub fn render_candidate_input(request: &CandidateRequest<'_>) -> String {
    let mut body = String::new();
    for message in request.messages.iter().take(MAX_SUMMARIZED_MESSAGES) {
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        let content = truncate_chars(
            &message.content.replace('\n', " "),
            MAX_SUMMARIZED_MESSAGE_CHARS,
        );
        body.push_str(&format!("[{role} {}] {content}\n", message.id));
    }
    body
}

/// Removes code fences and surrounding whitespace from a model answer.
pub fn clean_model_text(answer: &str) -> String {
    let trimmed = answer.trim();
    let without_fence = if trimmed.starts_with("```") {
        let body = trimmed.trim_start_matches('`');
        let body = body.strip_prefix("text").unwrap_or(body);
        let body = body.trim_start_matches('\n');
        body.split("```").next().unwrap_or(body).trim()
    } else {
        trimmed
    };
    // A model sometimes prefixes the answer with a label.
    let without_label = match without_fence.split_once(':') {
        Some((head, rest))
            if head.chars().count() <= 24
                && ["summary", "резюме", "підсумок", "итог"]
                    .iter()
                    .any(|word| head.to_lowercase().contains(word)) =>
        {
            rest.trim()
        }
        _ => without_fence,
    };
    without_label.trim().to_string()
}

/// Parses the JSON array a model answered with.
///
/// Anything that does not carry a user message identifier that was actually sent is
/// dropped: the model cannot create memory out of its own statements.
pub fn parse_candidates(
    answer: &str,
    request: &CandidateRequest<'_>,
) -> Result<Vec<CandidateSuggestion>, MemoryError> {
    let allowed: BTreeSet<Uuid> = request
        .messages
        .iter()
        .filter(|message| {
            message.role == MessageRole::User && message.status != MessageStatus::Failed
        })
        .map(|message| message.id)
        .collect();

    let text = answer.trim();
    let start = text.find('[').ok_or(MemoryError::SummarizerOutput)?;
    let end = text.rfind(']').ok_or(MemoryError::SummarizerOutput)?;
    if end <= start {
        return Err(MemoryError::SummarizerOutput);
    }
    let parsed: Vec<RawCandidate> =
        serde_json::from_str(&text[start..=end]).map_err(|_| MemoryError::SummarizerOutput)?;

    let mut suggestions = Vec::new();
    for raw in parsed.into_iter().take(MAX_CANDIDATES_PER_REQUEST) {
        let content = raw.content.trim().to_string();
        if content.is_empty() || content.chars().count() > MAX_CANDIDATE_CHARS {
            continue;
        }
        let Ok(source_message_id) = Uuid::parse_str(raw.source_message.trim()) else {
            continue;
        };
        if !allowed.contains(&source_message_id) {
            // Not anchored to a user message that exists: refused.
            continue;
        }
        // A candidate that looks like a credential is refused, never stored.
        if !redaction::scan(&content).is_clean() {
            continue;
        }
        let category = MemoryCategory::from_storage_name(&raw.category.to_lowercase())
            .unwrap_or(MemoryCategory::Other);
        let confidence = if raw.confidence.is_finite() {
            raw.confidence.clamp(0.0, 1.0)
        } else {
            0.0
        };
        suggestions.push(CandidateSuggestion {
            content,
            category,
            // The scope is decided by the interface when the user approves the
            // candidate; the model never chooses where memory lands.
            scope: MemoryScope::of_profile(request.persona),
            confidence,
            source_message_id,
        });
    }
    Ok(suggestions)
}

#[derive(Deserialize)]
struct RawCandidate {
    #[serde(default)]
    content: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    source_message: String,
}

/// Prevents two summaries of the same conversation from running at once.
///
/// The registry is a plain value type, so the rule can be tested without threads.
#[derive(Default)]
pub struct SummaryJobs {
    running: Mutex<BTreeSet<Uuid>>,
}

impl SummaryJobs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claims a conversation. Returns `false` when a job is already running.
    pub fn try_start(&self, conversation_id: Uuid) -> bool {
        let mut running = self.lock();
        running.insert(conversation_id)
    }

    /// Releases a conversation.
    pub fn finish(&self, conversation_id: Uuid) {
        let mut running = self.lock();
        running.remove(&conversation_id);
    }

    pub fn is_running(&self, conversation_id: Uuid) -> bool {
        self.lock().contains(&conversation_id)
    }

    pub fn running_count(&self) -> usize {
        self.lock().len()
    }

    /// Conversations with a job running right now, for the interface.
    pub fn running(&self) -> Vec<Uuid> {
        self.lock().iter().copied().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeSet<Uuid>> {
        self.running
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl std::fmt::Debug for SummaryJobs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SummaryJobs")
            .field("running", &self.running_count())
            .finish()
    }
}

/// A summary job that releases its claim when it ends, including on failure.
#[derive(Debug)]
pub struct SummaryGuard<'a> {
    jobs: &'a SummaryJobs,
    conversation_id: Uuid,
}

impl<'a> SummaryGuard<'a> {
    /// Claims a conversation, or returns `SummaryInProgress`.
    pub fn claim(jobs: &'a SummaryJobs, conversation_id: Uuid) -> Result<Self, MemoryError> {
        if !jobs.try_start(conversation_id) {
            return Err(MemoryError::SummaryInProgress);
        }
        Ok(Self {
            jobs,
            conversation_id,
        })
    }
}

impl Drop for SummaryGuard<'_> {
    fn drop(&mut self) {
        self.jobs.finish(self.conversation_id);
    }
}

/// Writes a summary for one conversation, if the threshold is reached.
///
/// Returns `None` when the settings or the message count say "not now", and also
/// when the model fails: a failed call is not an error the caller has to handle,
/// because the messages stay stored and the chat continues without a summary.
pub fn summarize_conversation<R: SyncRepository, C: CryptoProvider>(
    store: &mut MemoryStore<R, C>,
    provider: &dyn SummaryProvider,
    settings: &super::config::MemorySettings,
    persona: Persona,
    conversation_id: Uuid,
) -> Result<Option<String>, MemoryError> {
    let Some(conversation) = store.conversation(conversation_id)? else {
        return Ok(None);
    };
    let uncovered = store.uncovered_messages(conversation_id)?;
    if !settings.should_summarize(uncovered.len()) {
        return Ok(None);
    }
    // The newest messages stay verbatim and are never covered by the summary.
    let cover_count = uncovered.len().saturating_sub(settings.summary_keep_recent);
    if cover_count == 0 {
        return Ok(None);
    }
    let covered = &uncovered[..cover_count];
    let Some(boundary) = covered.last().map(|message| message.id) else {
        return Ok(None);
    };
    let previous = store
        .summary(conversation_id)?
        .map(|summary| summary.summary);
    let request = SummaryRequest {
        persona,
        title: &conversation.title,
        messages: covered,
        previous: previous.as_deref(),
        keep_recent: 0,
    };
    let summary = match provider.summarize(&request) {
        Ok(summary) => summary,
        Err(error) => {
            log::warn!("memory: summary not written: {error}");
            return Ok(None);
        }
    };
    store.save_summary(conversation_id, &summary, boundary, covered.len() as u32)?;
    Ok(Some(summary))
}

/// How many facts the context builder may use for this profile.
pub fn usable_fact_count<R: SyncRepository, C: CryptoProvider>(
    store: &mut MemoryStore<R, C>,
    persona: Persona,
) -> Result<usize, MemoryError> {
    Ok(store.usable_facts(persona)?.len())
}

/// How many candidates are waiting for review.
pub fn pending_candidate_count<R: SyncRepository, C: CryptoProvider>(
    store: &mut MemoryStore<R, C>,
) -> Result<usize, MemoryError> {
    Ok(store.list_candidates(None)?.len())
}

/// The scope a candidate should be approved into by default.
pub fn default_candidate_scope(persona: Persona) -> MemoryScope {
    MemoryScope::of_profile(persona)
}

/// A fact rendered for diagnostics, without its text.
pub fn describe_fact(fact: &FactView) -> String {
    format!(
        "{}/{} ({})",
        fact.scope.as_str(),
        fact.category.as_str(),
        fact.source.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::{MessageStatus, MessageView};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn message(role: MessageRole, content: &str, id: u128) -> MessageView {
        MessageView {
            id: Uuid::from_u128(id),
            revision: 1,
            role,
            content: content.to_string(),
            status: MessageStatus::Completed,
            partial: false,
            sequence: id as u64,
            created_at: "2026-09-19T12:00:00+00:00".to_string(),
        }
    }

    /// A deterministic provider: no model, no gateway, no network.
    struct FixtureProvider {
        summary: String,
        candidates: String,
        calls: AtomicUsize,
    }

    impl FixtureProvider {
        fn new(summary: &str, candidates: &str) -> Self {
            Self {
                summary: summary.to_string(),
                candidates: candidates.to_string(),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl SummaryProvider for FixtureProvider {
        fn summarize(&self, _request: &SummaryRequest<'_>) -> Result<String, MemoryError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.summary.clone())
        }

        fn suggest_candidates(
            &self,
            request: &CandidateRequest<'_>,
        ) -> Result<Vec<CandidateSuggestion>, MemoryError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            parse_candidates(&self.candidates, request)
        }
    }

    /// A provider that always fails, standing in for an unavailable model.
    struct FailingProvider;

    impl SummaryProvider for FailingProvider {
        fn summarize(&self, _request: &SummaryRequest<'_>) -> Result<String, MemoryError> {
            Err(MemoryError::SummaryUnavailable)
        }
        fn suggest_candidates(
            &self,
            _request: &CandidateRequest<'_>,
        ) -> Result<Vec<CandidateSuggestion>, MemoryError> {
            Err(MemoryError::SummaryUnavailable)
        }
    }

    #[test]
    fn the_summary_input_leaves_the_newest_messages_out() {
        let messages = vec![
            message(MessageRole::User, "первый", 1),
            message(MessageRole::Assistant, "ответ", 2),
            message(MessageRole::User, "последний", 3),
        ];
        let request = SummaryRequest {
            persona: Persona::Jarvis,
            title: "Тема",
            messages: &messages,
            previous: None,
            keep_recent: 1,
        };
        let body = render_summary_input(&request);
        assert!(body.contains("первый"));
        assert!(!body.contains("последний"));
        assert!(body.contains("Conversation title: Тема"));
    }

    #[test]
    fn the_previous_summary_is_shown_so_it_can_be_replaced() {
        let messages = vec![message(MessageRole::User, "продолжение", 1)];
        let request = SummaryRequest {
            persona: Persona::Jarvis,
            title: "Тема",
            messages: &messages,
            previous: Some("старое резюме"),
            keep_recent: 0,
        };
        let body = render_summary_input(&request);
        assert!(body.contains("Previous summary"));
        assert!(body.contains("старое резюме"));

        let cleaned = clean_model_text("```text\nРезюме: обсуждали проект\n```");
        assert_eq!(cleaned, "обсуждали проект");
        assert_eq!(clean_model_text("  Summary: тема  "), "тема");
        assert_eq!(clean_model_text("обычное резюме"), "обычное резюме");
    }

    #[test]
    fn only_candidates_anchored_to_a_user_message_survive() {
        let user = message(MessageRole::User, "Я люблю Rust", 11);
        let assistant = message(MessageRole::Assistant, "Пользователь любит Rust", 12);
        let messages = vec![user.clone(), assistant.clone()];
        let request = CandidateRequest {
            persona: Persona::Jarvis,
            messages: &messages,
        };
        let answer = format!(
            r#"[{{"content":"Любит Rust","category":"preference","confidence":0.8,"source_message":"{}"}},
                {{"content":"Любит Rust","category":"preference","confidence":0.9,"source_message":"{}"}},
                {{"content":"Придумано моделью","category":"other","confidence":0.5,"source_message":"{}"}}]"#,
            user.id,
            assistant.id,
            Uuid::new_v4()
        );
        let suggestions = parse_candidates(&answer, &request).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].content, "Любит Rust");
        assert_eq!(suggestions[0].source_message_id, user.id);
        assert_eq!(suggestions[0].scope, MemoryScope::Jarvis);
    }

    #[test]
    fn candidates_carrying_a_secret_or_a_bad_shape_are_dropped() {
        let user = message(MessageRole::User, "мой ключ", 21);
        let messages = vec![user.clone()];
        let request = CandidateRequest {
            persona: Persona::Altron,
            messages: &messages,
        };
        let answer = format!(
            r#"[{{"content":"password: FICTIONAL_SECRET_VALUE","category":"other","confidence":0.9,"source_message":"{}"}},
                {{"content":"","category":"other","confidence":0.9,"source_message":"{}"}},
                {{"content":"Живёт в Берлине","category":"personal_fact","confidence":5.0,"source_message":"{}"}}]"#,
            user.id, user.id, user.id
        );
        let suggestions = parse_candidates(&answer, &request).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].content, "Живёт в Берлине");
        // Confidence is clamped, and the scope is the profile's own area.
        assert_eq!(suggestions[0].confidence, 1.0);
        assert_eq!(suggestions[0].scope, MemoryScope::Altron);
    }

    #[test]
    fn a_malformed_answer_is_reported_not_guessed() {
        let messages = vec![message(MessageRole::User, "текст", 31)];
        let request = CandidateRequest {
            persona: Persona::Jarvis,
            messages: &messages,
        };
        assert_eq!(
            parse_candidates("no json here", &request).unwrap_err(),
            MemoryError::SummarizerOutput
        );
        assert_eq!(
            parse_candidates("[not json]", &request).unwrap_err(),
            MemoryError::SummarizerOutput
        );
        // An empty list is a valid answer.
        assert!(parse_candidates("[]", &request).unwrap().is_empty());
        assert!(parse_candidates("nothing to remember\n[]", &request)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_job_registry_allows_one_summary_per_conversation() {
        let jobs = SummaryJobs::new();
        let conversation = Uuid::new_v4();
        let guard = SummaryGuard::claim(&jobs, conversation).unwrap();
        assert!(jobs.is_running(conversation));
        assert_eq!(
            SummaryGuard::claim(&jobs, conversation).unwrap_err(),
            MemoryError::SummaryInProgress
        );
        // Another conversation is independent.
        let other = Uuid::new_v4();
        let second = SummaryGuard::claim(&jobs, other).unwrap();
        assert_eq!(jobs.running_count(), 2);
        drop(guard);
        assert!(!jobs.is_running(conversation));
        drop(second);
        assert_eq!(jobs.running_count(), 0);
        // The claim is free again after the guard is gone.
        assert!(SummaryGuard::claim(&jobs, conversation).is_ok());
    }

    #[test]
    fn the_fixture_provider_is_usable_without_a_model() {
        let provider = FixtureProvider::new("Тема: тест", "[]");
        let messages = vec![message(MessageRole::User, "привет", 41)];
        let summary = provider
            .summarize(&SummaryRequest {
                persona: Persona::Jarvis,
                title: "Тест",
                messages: &messages,
                previous: None,
                keep_recent: 0,
            })
            .unwrap();
        assert_eq!(summary, "Тема: тест");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

        let failing = FailingProvider;
        assert_eq!(
            failing
                .summarize(&SummaryRequest {
                    persona: Persona::Jarvis,
                    title: "Тест",
                    messages: &messages,
                    previous: None,
                    keep_recent: 0,
                })
                .unwrap_err(),
            MemoryError::SummaryUnavailable
        );
    }

    #[test]
    fn helper_queries_do_not_panic_on_an_empty_store() {
        assert_eq!(
            default_candidate_scope(Persona::Altron),
            MemoryScope::Altron
        );
        let fact = crate::memory::model::FactPayload::manual(
            MemoryScope::Global,
            MemoryCategory::Preference,
            "Любит Rust",
        )
        .unwrap();
        let view = FactView::from_payload(Uuid::new_v4(), 1, &fact);
        assert_eq!(describe_fact(&view), "global/preference (manual)");

        // The generic helpers work with any repository and crypto implementation.
        let mut store = crate::memory::tests::fixture_store();
        assert_eq!(usable_fact_count(&mut store, Persona::Jarvis).unwrap(), 0);
        assert_eq!(pending_candidate_count(&mut store).unwrap(), 0);
        // With no conversation the summary helper changes nothing.
        assert_eq!(
            summarize_conversation(
                &mut store,
                &FailingProvider,
                &crate::memory::config::MemorySettings::default(),
                Persona::Jarvis,
                Uuid::new_v4()
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_conversation_over_the_threshold_is_summarized_once() {
        let settings = crate::memory::config::MemorySettings {
            summary_trigger_messages: 4,
            summary_keep_recent: 2,
            ..crate::memory::config::MemorySettings::default()
        };
        let mut store = crate::memory::tests::fixture_store();
        let conversation = store.create_conversation("Тема", Persona::Jarvis).unwrap();
        for index in 0..6u32 {
            store
                .append_user_message(conversation.id, &format!("вопрос {index}"))
                .unwrap();
            store
                .append_assistant_message(
                    conversation.id,
                    &format!("ответ {index}"),
                    MessageStatus::Completed,
                    false,
                )
                .unwrap();
        }
        // 12 messages, 2 stay verbatim, so 10 are covered.
        let provider = FixtureProvider::new("Topic: тест", "[]");
        let summary = summarize_conversation(
            &mut store,
            &provider,
            &settings,
            Persona::Jarvis,
            conversation.id,
        )
        .unwrap()
        .expect("a summary must be written");
        assert_eq!(summary, "Topic: тест");
        let saved = store.summary(conversation.id).unwrap().unwrap();
        assert_eq!(saved.covered_messages, 10);
        assert!(!saved.stale);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

        // Below the threshold nothing happens again.
        let again = summarize_conversation(
            &mut store,
            &provider,
            &settings,
            Persona::Jarvis,
            conversation.id,
        )
        .unwrap();
        assert!(again.is_none());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failing_model_leaves_the_messages_and_returns_no_summary() {
        let settings = crate::memory::config::MemorySettings {
            summary_trigger_messages: 4,
            summary_keep_recent: 1,
            ..crate::memory::config::MemorySettings::default()
        };
        let mut store = crate::memory::tests::fixture_store();
        let conversation = store.create_conversation("Тема", Persona::Jarvis).unwrap();
        for index in 0..4u32 {
            store
                .append_user_message(conversation.id, &format!("вопрос {index}"))
                .unwrap();
        }
        let before = store.list_messages(conversation.id, 0, 100).unwrap().total;
        let result = summarize_conversation(
            &mut store,
            &FailingProvider,
            &settings,
            Persona::Jarvis,
            conversation.id,
        )
        .unwrap();
        assert!(result.is_none());
        assert!(store.summary(conversation.id).unwrap().is_none());
        // Every message is still there: a failed summary never drops history.
        let after = store.list_messages(conversation.id, 0, 100).unwrap().total;
        assert_eq!(after, before);
    }
}
