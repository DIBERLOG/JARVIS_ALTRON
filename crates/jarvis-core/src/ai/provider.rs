use super::{ChatError, ChatRequest, ChatResponse};
use std::{future::Future, pin::Pin};

pub trait ChatProvider: Send + Sync {
    fn send_message(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ChatError>> + Send + '_>>;
}

#[derive(Default)]
pub struct DisabledProvider;
impl ChatProvider for DisabledProvider {
    fn send_message(
        &self,
        _request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse, ChatError>> + Send + '_>> {
        Box::pin(async { Err(ChatError::Disabled) })
    }
}
