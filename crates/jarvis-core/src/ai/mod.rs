//! AI contracts and the local runtime.
//!
//! Two layers live here:
//!
//! * the provider-neutral chat contracts (`ChatProvider`, `Persona`,
//!   `ChatMessage`), which describe what an assistant is without knowing how it
//!   is implemented;
//! * [`local`], the Windows runtime that manages `llama-server` and streams
//!   completions from it.
//!
//! There is no tool surface and no path from here to the encrypted storages:
//! chat providers never receive an encrypted-storage handle, a note store, a Lua
//! sandbox, or a process-execution capability. Windows actions stay behind
//! structured commands and the `SafetyGate` of the voice path.

mod conversation;
mod error;
pub mod local;
mod prompt;
mod provider;

pub use conversation::{ChatMessage, ChatRequest, ChatResponse, ChatRole, Persona};
pub use error::ChatError;
pub use prompt::{persona_instructions, system_prompt, PROMPT_VERSION, SAFETY_CONSTRAINTS};
pub use provider::{ChatProvider, DisabledProvider};

/// Name used for the built-in profiles.
///
/// `Persona` is that type; this alias exists so settings and documentation can use
/// the profile wording without introducing a second, parallel enum.
pub type AiProfile = Persona;
