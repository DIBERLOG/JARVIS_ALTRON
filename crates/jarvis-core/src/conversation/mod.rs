//! Talking to Jarvis about anything.
//!
//! This module is deliberately small and deliberately closed. A conversation is a
//! question and an answer, and it is **not** a way to make the application do
//! something: nothing here can reach a command, a Windows action, the safety gate,
//! a matcher or a process. The isolation is the point, so it is structural — this
//! file does not import any of those modules, and
//! `the_conversation_route_cannot_reach_a_command` reads this source and refuses it
//! if one of them ever appears.
//!
//! The route an answer takes, and the stages it is reported in:
//!
//! ```text
//! Recording  →  Transcribing  →  Thinking  →  Answering  →  (Speaking)  →  Finished
//! ```
//!
//! Every stage can be cancelled, every stage has a deadline, and the listener is
//! restored by the caller on every exit — success, error, cancellation or panic —
//! through [`RestoreOnDrop`].
//!
//! The model itself is behind [`ChatProvider`]. This stage ships [`Disabled`]: the
//! route works, the isolation is proven, and the answer is a typed "not configured"
//! until a provider is chosen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Longest question this route accepts, in characters.
pub const MAX_QUESTION_CHARS: usize = 2000;

/// Longest answer this route keeps, in characters.
pub const MAX_ANSWER_CHARS: usize = 8000;

/// How long one recording may last.
pub const RECORD_TIMEOUT: Duration = Duration::from_secs(30);

/// How long transcription may take.
pub const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long an answer may take.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(180);

/// The system prompt used when no profile has been chosen.
///
/// Profiles (JARVIS and ULTRON) replace this text; nothing else about the route
/// changes with them, which is what keeps a profile from being a permission.
pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are an assistant. Answer briefly, then add detail if it helps. \
Say when you are not sure. Never claim to have run something on the user's computer.";

/// What a spoken control phrase asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationIntent {
    /// Begin a conversation, or begin one more turn of it.
    Start,
    /// Ask one more question in the conversation that is already open.
    Continue,
    /// End the conversation.
    Stop,
    /// Abandon the current question.
    Cancel,
}

impl ConversationIntent {
    /// The stable name, for logs and for the interface.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start_conversation",
            Self::Continue => "continue_conversation",
            Self::Stop => "stop_conversation",
            Self::Cancel => "cancel_conversation",
        }
    }

    /// Whether this intent opens the microphone rather than closing something.
    pub fn opens_the_microphone(self) -> bool {
        matches!(self, Self::Start | Self::Continue)
    }
}

/// One control phrase: the words that select an intent, per language.
///
/// A phrase is matched by containment, because the listener has already removed the
/// wake word and the filler words a person puts in front of a sentence: what
/// arrives here is "поговорим", not "Джарвис, давай поговорим".
struct ControlPhrase {
    intent: ConversationIntent,
    phrases: &'static [&'static str],
}

const CONTROL_PHRASES: &[ControlPhrase] = &[
    // Cancel first: "отмени разговор" also contains "разговор", and abandoning a
    // question is the narrower wish.
    ControlPhrase {
        intent: ConversationIntent::Cancel,
        phrases: &[
            "отмени разговор",
            "отмена разговора",
            "не надо разговор",
            "cancel the conversation",
            "cancel conversation",
            "never mind that question",
            "скасуй розмову",
            "не треба розмови",
        ],
    },
    ControlPhrase {
        intent: ConversationIntent::Stop,
        phrases: &[
            "закончи разговор",
            "заверши разговор",
            "заверши диалог",
            "конец разговора",
            "хватит разговоров",
            "закрой разговор",
            "end the conversation",
            "stop the conversation",
            "that is enough talking",
            "закінчи розмову",
            "заверши діалог",
            "досить розмов",
        ],
    },
    ControlPhrase {
        intent: ConversationIntent::Start,
        phrases: &[
            "давай поговорим",
            "поговорим",
            "хочу поговорить",
            "хочу поговорити",
            "у меня вопрос",
            "у мене питання",
            "режим диалога",
            "режим діалогу",
            "давай обсудим",
            "начнем разговор",
            "почнемо розмову",
            "let us talk",
            "lets talk",
            "i want to talk",
            "i have a question",
            "conversation mode",
            "start a conversation",
            "start conversation",
        ],
    },
    ControlPhrase {
        intent: ConversationIntent::Continue,
        phrases: &[
            "продолжим",
            "продолжай",
            "продолжи разговор",
            "продовжимо",
            "продовжуй",
            "continue the conversation",
            "continue",
            "go on",
        ],
    },
];

/// Folds a phrase the way the control table is written: lower case, `ё` written as
/// `е`, an apostrophe removed (`let's talk` is `lets talk`), everything else that is
/// not a letter or a digit turned into a space, and the spaces collapsed.
///
/// It is deliberately local: the conversation route does not import the command
/// matcher's normalizer, because importing it would be the first thread of a rope
/// between a conversation and a command.
fn fold(phrase: &str) -> String {
    let lowered = phrase.trim().to_lowercase().replace('ё', "е");
    let mut cleaned = String::with_capacity(lowered.len());
    for character in lowered.chars() {
        match character {
            '\'' | '’' | '`' => {}
            c if c.is_alphanumeric() => cleaned.push(c),
            _ => cleaned.push(' '),
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a phrase asks for a conversation, and which part of it.
///
/// The phrases are the ones a person says; the wake word and the filler words have
/// already been removed by the listener, and both are handled by matching on
/// containment rather than on equality.
pub fn intent_of(phrase: &str) -> Option<ConversationIntent> {
    let phrase = fold(phrase);
    if phrase.is_empty() {
        return None;
    }
    for control in CONTROL_PHRASES {
        if control
            .phrases
            .iter()
            .any(|candidate| phrase.contains(&fold(candidate)))
        {
            return Some(control.intent);
        }
    }
    None
}

/// Where a conversation is right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationStage {
    Idle,
    /// The microphone is open and the question is being recorded: "слушаю".
    Recording,
    /// Whisper is turning the recording into text: "распознаю".
    Transcribing,
    /// The provider is answering: "думаю".
    Thinking,
    /// The answer has arrived and is being shown: "отвечаю".
    Answering,
    /// The answer is being read out: "озвучиваю".
    Speaking,
    /// One turn is over; the microphone belongs to the listener again.
    Finished,
    Cancelled,
    Failed,
}

impl ConversationStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Thinking => "thinking",
            Self::Answering => "answering",
            Self::Speaking => "speaking",
            Self::Finished => "finished",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// Whether a request is in flight: the microphone belongs to this route.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Recording
                | Self::Transcribing
                | Self::Thinking
                | Self::Answering
                | Self::Speaking
        )
    }
}

/// Why a conversation cannot continue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationError {
    /// One conversation at a time.
    Busy,
    /// The asked-for transition does not belong to the current stage.
    WrongStage {
        from: &'static str,
        to: &'static str,
    },
    /// The question is empty, or longer than [`MAX_QUESTION_CHARS`].
    QuestionTooLong,
    /// No provider is configured.
    NotConfigured,
    /// The stage ran out of time.
    Timeout { code: &'static str },
    /// The person abandoned it.
    Cancelled,
    /// The provider failed; the code is safe to show and to log.
    Failed { code: String },
}

impl ConversationError {
    /// The stable code, for the interface and for the log.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Busy => "conversation_busy",
            Self::WrongStage { .. } => "conversation_wrong_stage",
            Self::QuestionTooLong => "question_too_long",
            Self::NotConfigured => "provider_not_configured",
            Self::Timeout { code } => code,
            Self::Cancelled => "cancelled",
            Self::Failed { .. } => "provider_failed",
        }
    }
}

/// One conversation, one turn at a time.
///
/// The state machine is here rather than in the host so that "one request at a
/// time", "cancel from anywhere" and "every stage has a deadline" are testable
/// without a microphone, a model or a window.
#[derive(Clone, Debug)]
pub struct ConversationSession {
    stage: ConversationStage,
    stage_started: Instant,
    turns: u32,
    last_code: Option<String>,
}

impl Default for ConversationSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationSession {
    pub fn new() -> Self {
        Self {
            stage: ConversationStage::Idle,
            stage_started: Instant::now(),
            turns: 0,
            last_code: None,
        }
    }

    pub fn stage(&self) -> ConversationStage {
        self.stage
    }

    /// How many answered turns this session has had.
    pub fn turns(&self) -> u32 {
        self.turns
    }

    /// The last code, for the interface: a refusal, a timeout or a failure.
    pub fn last_code(&self) -> Option<&str> {
        self.last_code.as_deref()
    }

    /// Opens the microphone for one question.
    ///
    /// Refused while a stage is active: one conversation, one request, whatever the
    /// trigger was.
    pub fn begin(&mut self, now: Instant) -> Result<(), ConversationError> {
        if self.stage.is_active() {
            return Err(ConversationError::Busy);
        }
        self.stage = ConversationStage::Recording;
        self.stage_started = now;
        self.last_code = None;
        Ok(())
    }

    /// Moves to the next stage, refusing a transition that does not belong.
    pub fn enter(
        &mut self,
        stage: ConversationStage,
        now: Instant,
    ) -> Result<(), ConversationError> {
        let allowed = matches!(
            (self.stage, stage),
            (
                ConversationStage::Recording,
                ConversationStage::Transcribing
            ) | (ConversationStage::Transcribing, ConversationStage::Thinking)
                | (ConversationStage::Thinking, ConversationStage::Answering)
                | (ConversationStage::Answering, ConversationStage::Speaking)
                | (ConversationStage::Answering, ConversationStage::Finished)
                | (ConversationStage::Speaking, ConversationStage::Finished)
        );
        if !allowed {
            return Err(ConversationError::WrongStage {
                from: self.stage.as_str(),
                to: stage.as_str(),
            });
        }
        self.stage = stage;
        self.stage_started = now;
        if stage == ConversationStage::Finished {
            self.turns += 1;
        }
        Ok(())
    }

    /// Abandons whatever is happening. True when something was abandoned.
    ///
    /// Cancelling is always allowed, from any stage, and it is the only way out of a
    /// stage that is waiting for something that will not arrive.
    pub fn cancel(&mut self, now: Instant) -> bool {
        if !self.stage.is_active() {
            return false;
        }
        self.stage = ConversationStage::Cancelled;
        self.stage_started = now;
        self.last_code = Some("cancelled".to_string());
        true
    }

    /// Ends the conversation: the microphone goes back to the listener.
    pub fn finish(&mut self, now: Instant) {
        self.stage = ConversationStage::Finished;
        self.stage_started = now;
    }

    /// Records a failure with a safe code, and ends the turn.
    pub fn fail(&mut self, code: impl Into<String>, now: Instant) {
        self.stage = ConversationStage::Failed;
        self.stage_started = now;
        self.last_code = Some(code.into());
    }

    /// The deadline of the current stage, if it is overdrawn.
    ///
    /// The check is a question, not a thread: the host asks after every blocking
    /// step, and the answer is the code to report.
    pub fn timeout(&self, now: Instant) -> Option<&'static str> {
        let elapsed = now.saturating_duration_since(self.stage_started);
        match self.stage {
            ConversationStage::Recording if elapsed > RECORD_TIMEOUT => Some("record_timeout"),
            ConversationStage::Transcribing if elapsed > TRANSCRIBE_TIMEOUT => {
                Some("transcribe_timeout")
            }
            ConversationStage::Thinking | ConversationStage::Answering
                if elapsed > ANSWER_TIMEOUT =>
            {
                Some("answer_timeout")
            }
            _ => None,
        }
    }
}

/// Checks a question before it is sent anywhere.
pub fn check_question(question: &str) -> Result<&str, ConversationError> {
    let question = question.trim();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(ConversationError::QuestionTooLong);
    }
    Ok(question)
}

/// Cuts an answer to what this route keeps, on a character boundary.
pub fn cut_answer(answer: &str) -> String {
    let answer = answer.trim();
    if answer.chars().count() <= MAX_ANSWER_CHARS {
        return answer.to_string();
    }
    answer.chars().take(MAX_ANSWER_CHARS).collect()
}

/// The model that answers a question.
///
/// There is exactly one provider interface in this build, and it is not declared
/// here: [`crate::ai::ChatProvider`], which the local `llama-server` gateway already
/// implements and which a cloud provider implements behind the same shape. A second
/// trait would be a second place to decide what a model may see, so this route uses
/// that one and adds nothing to it.
///
/// The interface carries no tools, no permissions and no system access: a provider
/// receives text and returns text, and the profile it is asked under is part of the
/// request rather than something this route chooses. The provider that exists when
/// nothing is configured is [`crate::ai::DisabledProvider`], and it answers with a
/// typed refusal — the route never invents an answer for it.
pub use crate::ai::ChatProvider;

/// Runs what has to run when a conversation ends, however it ends.
///
/// The listener must get the microphone back on every path — success, error,
/// cancellation or a panic in a step that was not written by this route. A guard
/// cannot be forgotten the way a line of cleanup can.
pub struct RestoreOnDrop<F: FnMut()> {
    restore: Option<F>,
}

impl<F: FnMut()> RestoreOnDrop<F> {
    pub fn new(restore: F) -> Self {
        Self {
            restore: Some(restore),
        }
    }

    /// Runs the restore now, so a later drop does nothing.
    pub fn restore_now(&mut self) {
        if let Some(mut restore) = self.restore.take() {
            restore();
        }
    }
}

impl<F: FnMut()> Drop for RestoreOnDrop<F> {
    fn drop(&mut self) {
        if let Some(mut restore) = self.restore.take() {
            restore();
        }
    }
}

/// Whether a cancellation has been asked for.
pub fn cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests;
