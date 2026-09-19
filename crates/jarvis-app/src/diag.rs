//! Where a spoken phrase went, in short lines that never contain the phrase.
//!
//! The problem this answers: the listener either ran something or said "команда не
//! найдена", and there was no way to tell which of the seven stops a phrase
//! reached — whether the microphone heard it, whether the wake word was found,
//! what the matcher was given, whether the matcher answered, and whether the
//! executor ran. Every line carries a stage name from
//! [`jarvis_core::commands::CommandStage`], the *length* of the text rather than
//! the text, and a reason code. A transcript is a person's speech; it is not
//! written here and not sent to the window.

use jarvis_core::commands::CommandStage;
use jarvis_core::ipc::{self, IpcEvent};

/// One stage, one line. `code` is a fixed key, never user text.
pub struct Note<'a> {
    pub length: usize,
    pub code: Option<&'a str>,
    pub command_id: Option<&'a str>,
    pub success: Option<bool>,
}

impl<'a> Note<'a> {
    pub fn length(length: usize) -> Self {
        Self {
            length,
            code: None,
            command_id: None,
            success: None,
        }
    }

    pub fn code(mut self, code: &'a str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn command(mut self, command_id: &'a str) -> Self {
        self.command_id = Some(command_id);
        self
    }

    pub fn success(mut self, success: bool) -> Self {
        self.success = Some(success);
        self
    }
}

/// Writes one stage: to the log, and to every connected window.
pub fn stage(stage: CommandStage, note: Note<'_>) {
    info!(
        "voice-diag stage={} length={} code={} command={} success={}",
        stage.as_str(),
        note.length,
        note.code.unwrap_or("-"),
        note.command_id.unwrap_or("-"),
        match note.success {
            Some(true) => "ok",
            Some(false) => "error",
            None => "-",
        }
    );

    ipc::send(IpcEvent::CommandDiagnostic {
        stage: stage.as_str().to_string(),
        length: note.length,
        code: note.code.map(|code| code.to_string()),
        command_id: note.command_id.map(|id| id.to_string()),
        success: note.success,
    });
}

/// The transcript arrived. Only its length is recorded.
pub fn received(length: usize) {
    stage(CommandStage::ListenerReceivedPhrase, Note::length(length));
}

/// The wake word was looked for, and how much was left after taking it out.
pub fn wake_word(detected: bool, remaining: usize) {
    stage(
        CommandStage::WakeWordDetected,
        Note::length(remaining).code(if detected { "found" } else { "absent" }),
    );
}

/// The length of the normalized phrase, which is what the matcher is given.
pub fn normalized(length: usize) {
    stage(CommandStage::NormalizedLength, Note::length(length));
}

/// The matcher answered with a command.
pub fn matched(command_id: &str, length: usize) {
    stage(
        CommandStage::CommandMatch,
        Note::length(length).command(command_id),
    );
}

/// Nothing was accepted, and why, as a code.
pub fn rejection(code: &str, length: usize) {
    stage(CommandStage::RejectionCode, Note::length(length).code(code));
}

/// The command was handed to the executor.
pub fn started(command_id: &str, length: usize) {
    stage(
        CommandStage::ExecutionStarted,
        Note::length(length).command(command_id),
    );
}

/// The executor answered.
pub fn finished(command_id: &str, success: bool, code: Option<&str>, length: usize) {
    let note = Note::length(length).command(command_id).success(success);
    let note = match code {
        Some(code) => note.code(code),
        None => note,
    };
    stage(CommandStage::ExecutionResult, note);
}

/// A typed native action reached the safe action pipeline, or was refused before it
/// could run. The action's name is a fixed key of the pack schema, never a path.
pub fn native_executed(action: &str, success: bool) {
    stage(
        CommandStage::ExecutionResult,
        Note::length(0).code(action).success(success),
    );
}

/// A typed event of the application was handled. No process was started.
pub fn internal_event(event: &str) {
    stage(
        CommandStage::ExecutionResult,
        Note::length(0).code("internal").command(event),
    );
}
