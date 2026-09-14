//! Provider-neutral chat contracts. Local command routing is the only route to
//! system actions; chat providers never receive process-execution capability.
mod conversation;
mod error;
mod prompt;
mod provider;

pub use conversation::{ChatMessage, ChatRequest, ChatResponse, ChatRole, Persona};
pub use error::ChatError;
pub use prompt::system_prompt;
pub use provider::{ChatProvider, DisabledProvider};
