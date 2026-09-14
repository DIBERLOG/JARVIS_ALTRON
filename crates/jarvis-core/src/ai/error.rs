use std::fmt;
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatError { Disabled, MissingApiKey, NetworkUnavailable, Unauthorized, RateLimited, TimedOut, InvalidResponse, Provider(String) }
impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disabled => "AI chat is disabled", Self::MissingApiKey => "AI API key is not configured",
            Self::NetworkUnavailable => "AI service is unavailable; check your internet connection",
            Self::Unauthorized => "AI API key was rejected", Self::RateLimited => "AI service limit reached; try again later",
            Self::TimedOut => "AI request timed out", Self::InvalidResponse => "AI service returned an invalid response",
            Self::Provider(message) => message,
        })
    }
}
impl std::error::Error for ChatError {}
