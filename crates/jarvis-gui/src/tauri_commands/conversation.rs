//! The conversation route, in the process that owns the microphone and the model.
//!
//! One question at a time, and never a command:
//!
//! ```text
//! voice host (Vosk)  →  ConversationRequested  →  record  →  Whisper  →  provider
//!                    →  the answer in the dialog  →  (TTS)  →  listener restored
//! ```
//!
//! The question is recorded with the *dictation* engine the global voice input
//! already uses — the same microphone handover, the same Whisper, the same
//! guarantee that the listener gets the device back on every path — with one
//! difference: `keep_text` is set, so the text comes back to be asked instead of
//! being typed or copied.
//!
//! The model is [`jarvis_core::ai::ChatProvider`], the one provider interface this
//! build has: `LocalAiGateway` implements it for the `llama-server` the settings
//! already manage, and `DisabledProvider` answers when nothing is configured. This
//! module holds no command list, no Windows action session and no safety gate, and
//! the text it carries goes to a provider and to the dialog — nowhere else.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use jarvis_core::ai::{
    ChatError, ChatMessage, ChatProvider, ChatRequest, ChatRole, DisabledProvider, Persona,
};
use jarvis_core::conversation::{
    self, ConversationError, ConversationIntent, ConversationSession, ConversationStage,
};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::local_ai::LocalAiHandle;
use super::voice_input::VoiceInputHandle;
use crate::AppState;

/// The event a stage change arrives on.
pub const STAGE_EVENT: &str = "conversation-stage";

/// One answered turn, as the dialog shows it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ConversationTurnView {
    /// The length of the question, never the question.
    pub question_characters: usize,
    /// The answer, as the provider returned it.
    pub answer: String,
    /// The length of the answer, for a line that does not repeat the text.
    pub answer_characters: usize,
    /// The provider that produced it.
    pub provider: String,
    /// Whether the answer was produced off this machine.
    pub cloud: bool,
}

/// What the panel needs, in one answer.
#[derive(Clone, Debug, Serialize)]
pub struct ConversationView {
    pub stage: String,
    pub active: bool,
    pub turns: u32,
    pub provider: String,
    pub provider_available: bool,
    pub cloud: bool,
    pub last_code: Option<String>,
    pub turn: Option<ConversationTurnView>,
    /// The profile whose instructions the answer was asked under.
    pub profile: String,
}

/// The one conversation this process is having.
pub struct ConversationRuntime {
    session: Mutex<ConversationSession>,
    cancel: Arc<AtomicBool>,
    turn: Mutex<Option<ConversationTurnView>>,
    provider: Mutex<Arc<dyn ChatProvider>>,
    persona: Mutex<Persona>,
    voice_input: VoiceInputHandle,
    local_ai: LocalAiHandle,
    app: Mutex<Option<AppHandle>>,
    running: AtomicBool,
}

impl ConversationRuntime {
    pub fn new(voice_input: VoiceInputHandle, local_ai: LocalAiHandle) -> Self {
        Self {
            session: Mutex::new(ConversationSession::new()),
            cancel: Arc::new(AtomicBool::new(false)),
            turn: Mutex::new(None),
            // Nothing is configured until the local server is; the route answers with
            // a typed reason instead of inventing one.
            provider: Mutex::new(Arc::new(DisabledProvider)),
            persona: Mutex::new(Persona::Jarvis),
            voice_input,
            local_ai,
            app: Mutex::new(None),
            running: AtomicBool::new(false),
        }
    }

    /// Uses the local model the settings already manage.
    ///
    /// It is the same gateway the text chat uses, so there is one `llama-server`,
    /// one readiness check and one stream in this process — not a second one for the
    /// microphone.
    pub fn use_local_ai(&self) {
        *self.provider.lock() = self.local_ai.shared();
    }

    /// Uses the profile whose instructions the answers are asked under.
    ///
    /// A profile is text in a request. It changes no permission, and this method
    /// cannot: there is nothing here that a profile could reach even if it tried.
    pub fn use_persona(&self, persona: Persona) {
        *self.persona.lock() = persona;
    }

    pub fn attach(&self, app: AppHandle) {
        *self.app.lock() = Some(app);
    }

    pub fn view(&self) -> ConversationView {
        let session = self.session.lock().clone();
        let turn = self.turn.lock().clone();
        let profile = match *self.persona.lock() {
            Persona::Jarvis => "jarvis",
            Persona::Altron => "altron",
        };
        ConversationView {
            stage: session.stage().as_str().to_string(),
            active: session.stage().is_active(),
            turns: session.turns(),
            provider: "local_llama".to_string(),
            provider_available: self.local_ai.gateway().status().is_ok(),
            // The only provider this route can use is the local one; a cloud provider
            // would set this, and the interface would mark it.
            cloud: false,
            last_code: session.last_code().map(|code| code.to_string()),
            turn,
            profile: profile.to_string(),
        }
    }

    /// Whether a question is being asked right now.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Asks one question: record, transcribe, ask the model, and show the answer.
    ///
    /// The listener is restored by the dictation engine's own guard whatever happens
    /// here. A failure leaves the last answer on screen: losing what was already
    /// said because the next question failed is worse than showing the error.
    pub fn ask(&self) -> Result<ConversationView, String> {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(ConversationError::Busy.code().to_string());
        }
        if let Err(code) = self.ask_inner() {
            log::warn!("conversation: the question failed (error_code={code})");
        }
        self.running.store(false, Ordering::SeqCst);
        // A failure is a state of the dialog, not the end of it.
        Ok(self.view())
    }

    fn ask_inner(&self) -> Result<(), String> {
        let now = Instant::now();
        self.cancel.store(false, Ordering::SeqCst);
        self.session
            .lock()
            .begin(now)
            .map_err(|error| error.code().to_string())?;
        self.emit_stage();

        // 1. the question: the same engine as the dictation, with the text kept.
        let question = self.fail_or(|| self.voice_input.ask())?;
        if let Err(error) = conversation::check_question(&question) {
            return self.fail(error.code());
        }
        if conversation::cancelled(&self.cancel) {
            return self.cancelled();
        }
        self.enter_or_fail(ConversationStage::Transcribing)?;

        // 2. the answer. Only the length of the question is written down.
        let persona = *self.persona.lock();
        {
            let mut turn = self.turn.lock();
            *turn = Some(ConversationTurnView {
                question_characters: question.chars().count(),
                provider: "local_llama".to_string(),
                ..ConversationTurnView::default()
            });
        }
        self.enter_or_fail(ConversationStage::Thinking)?;

        let provider = Arc::clone(&self.provider.lock());
        let request = ChatRequest {
            persona,
            model: model_label(&self.local_ai),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: question.clone(),
            }],
        };
        // The provider interface is asynchronous and this command is synchronous,
        // because recording is: the two are joined here, once.
        let answer = tauri::async_runtime::block_on(provider.send_message(request));
        if conversation::cancelled(&self.cancel) {
            return self.cancelled();
        }
        let answer = match answer {
            Ok(response) => conversation::cut_answer(&response.content),
            Err(error) => return self.fail(&chat_code(&error)),
        };
        {
            let mut turn = self.turn.lock();
            if let Some(turn) = turn.as_mut() {
                turn.answer = answer;
                turn.answer_characters = turn.answer.chars().count();
            }
        }
        {
            let mut session = self.session.lock();
            session
                .enter(ConversationStage::Answering, Instant::now())
                .ok();
            session.finish(Instant::now());
        }
        self.emit_stage();
        log::info!(
            "conversation: answered (profile={} question_characters={} answer_characters={})",
            profile_name(persona),
            question.chars().count(),
            self.turn
                .lock()
                .as_ref()
                .map(|turn| turn.answer_characters)
                .unwrap_or(0)
        );
        Ok(())
    }

    /// Moves to the next stage, or fails the turn with the typed code.
    fn enter_or_fail(&self, stage: ConversationStage) -> Result<(), String> {
        match self.session.lock().enter(stage, Instant::now()) {
            Ok(()) => {
                self.emit_stage();
                Ok(())
            }
            Err(error) => self.fail(error.code()),
        }
    }

    /// Fails the turn with a code and reports it.
    fn fail(&self, code: &str) -> Result<(), String> {
        self.session.lock().fail(code.to_string(), Instant::now());
        self.emit_stage();
        Err(code.to_string())
    }

    fn cancelled(&self) -> Result<(), String> {
        self.session.lock().cancel(Instant::now());
        self.emit_stage();
        Err(ConversationError::Cancelled.code().to_string())
    }

    /// Runs a step, and fails the turn when it fails.
    fn fail_or<T>(&self, step: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        match step() {
            Ok(value) => Ok(value),
            Err(code) => {
                self.fail(&code)?;
                unreachable!("`fail` always returns an error")
            }
        }
    }

    /// Abandons the question in flight, wherever it is.
    pub fn cancel(&self) -> bool {
        let cancelled = self.session.lock().cancel(Instant::now());
        self.cancel.store(true, Ordering::SeqCst);
        self.voice_input.cancel();
        if cancelled {
            self.emit_stage();
        }
        cancelled
    }

    /// Ends the conversation. Whatever has been asked and answered stays on screen.
    pub fn stop(&self) -> bool {
        let cancelled = self.cancel();
        if !cancelled {
            self.session.lock().finish(Instant::now());
            self.emit_stage();
        }
        true
    }

    /// Forgets the last answer.
    pub fn clear(&self) -> bool {
        let cleared = self.turn.lock().take().is_some();
        self.session.lock().finish(Instant::now());
        self.emit_stage();
        cleared
    }

    fn emit_stage(&self) {
        if let Some(app) = self.app.lock().as_ref() {
            let _ = app.emit(STAGE_EVENT, self.view());
        }
    }
}

/// The label of the configured model, for the request.
fn model_label(local_ai: &LocalAiHandle) -> String {
    let path = local_ai.config().server.model_path;
    std::path::Path::new(&path)
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// The stable name of a profile, for logs and for the dialog.
fn profile_name(persona: Persona) -> &'static str {
    match persona {
        Persona::Jarvis => "jarvis",
        Persona::Altron => "altron",
    }
}

/// A stable code for a provider failure, safe to show and to log.
fn chat_code(error: &ChatError) -> String {
    match error {
        ChatError::Disabled => "provider_disabled".to_string(),
        ChatError::MissingApiKey => "provider_missing_key".to_string(),
        ChatError::NetworkUnavailable => "network_unavailable".to_string(),
        ChatError::Unauthorized => "provider_unauthorized".to_string(),
        ChatError::RateLimited => "provider_rate_limited".to_string(),
        ChatError::TimedOut => "provider_timeout".to_string(),
        ChatError::InvalidResponse => "provider_invalid_response".to_string(),
        ChatError::ServerNotRunning => "server_not_running".to_string(),
        ChatError::ServerNotReady => "server_not_ready".to_string(),
        ChatError::ServerStopped => "server_stopped".to_string(),
        ChatError::NotConfigured => "provider_not_configured".to_string(),
        ChatError::GenerationInProgress => "provider_busy".to_string(),
        _ => "provider_failed".to_string(),
    }
}

// ------------------------------------------------------------------- commands

/// What the chat panel needs.
#[tauri::command]
pub async fn conversation_status(state: State<'_, AppState>) -> Result<ConversationView, String> {
    Ok(state.conversation.view())
}

/// Asks one question: the microphone, Whisper, the model, and the answer.
///
/// It is refused while another question is in flight: one conversation, one request,
/// whatever the trigger was.
#[tauri::command]
pub async fn conversation_ask(state: State<'_, AppState>) -> Result<ConversationView, String> {
    state.conversation.ask()
}

/// Abandons the question in flight, at whatever stage it is.
#[tauri::command]
pub async fn conversation_cancel(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.conversation.cancel())
}

/// Ends the conversation; what has already been said stays on screen.
#[tauri::command]
pub async fn conversation_stop(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.conversation.stop())
}

/// Forgets the last answer.
#[tauri::command]
pub async fn conversation_clear(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.conversation.clear())
}

/// Uses the profile whose instructions the answers are asked under. It changes the
/// prompt of the next question and nothing else.
#[tauri::command]
pub async fn conversation_set_profile(
    state: State<'_, AppState>,
    profile: String,
) -> Result<ConversationView, String> {
    let persona = match profile.trim().to_lowercase().as_str() {
        "altron" | "ultron" => Persona::Altron,
        _ => Persona::Jarvis,
    };
    state.conversation.use_persona(persona);
    Ok(state.conversation.view())
}
