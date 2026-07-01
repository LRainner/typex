#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderService {
    Asr,
    Llm,
}

impl std::fmt::Display for ProviderService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Asr => f.write_str("ASR"),
            Self::Llm => f.write_str("LLM"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    InvalidConfig,
    MissingCredential,
    InvalidEndpoint,
    Network,
    Timeout,
    Unauthorized,
    Forbidden,
    RateLimited,
    NotFound,
    BadRequest,
    Server,
    InvalidResponse,
    Stream,
    UnsupportedProvider,
    EmptyResponse,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{service} provider {provider}: {message}")]
pub struct ProviderError {
    pub service: ProviderService,
    pub provider: String,
    pub kind: ProviderErrorKind,
    pub message: String,
    pub status: Option<u16>,
}

impl ProviderError {
    pub fn new(
        service: ProviderService,
        provider: impl Into<String>,
        kind: ProviderErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            service,
            provider: provider.into(),
            kind,
            message: message.into(),
            status: None,
        }
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn message_key(&self) -> &'static str {
        self.kind.message_key()
    }
}

impl ProviderErrorKind {
    pub fn message_key(self) -> &'static str {
        match self {
            Self::InvalidConfig => "settings.connection_error_invalid_config",
            Self::MissingCredential => "settings.connection_error_missing_credential",
            Self::InvalidEndpoint => "settings.connection_error_invalid_endpoint",
            Self::Network => "settings.connection_error_network",
            Self::Timeout => "settings.connection_error_timeout",
            Self::Unauthorized => "settings.connection_error_unauthorized",
            Self::Forbidden => "settings.connection_error_forbidden",
            Self::RateLimited => "settings.connection_error_rate_limited",
            Self::NotFound => "settings.connection_error_not_found",
            Self::BadRequest => "settings.connection_error_bad_request",
            Self::Server => "settings.connection_error_server",
            Self::InvalidResponse => "settings.connection_error_invalid_response",
            Self::Stream => "settings.connection_error_stream",
            Self::UnsupportedProvider => "settings.connection_error_unsupported_provider",
            Self::EmptyResponse => "settings.connection_error_empty_response",
            Self::Other => "settings.connection_failed",
        }
    }
}

pub fn kind_from_http_status(status: u16) -> ProviderErrorKind {
    match status {
        400 => ProviderErrorKind::BadRequest,
        401 => ProviderErrorKind::Unauthorized,
        403 => ProviderErrorKind::Forbidden,
        404 => ProviderErrorKind::NotFound,
        408 => ProviderErrorKind::Timeout,
        429 => ProviderErrorKind::RateLimited,
        500..=599 => ProviderErrorKind::Server,
        _ => ProviderErrorKind::Other,
    }
}

pub fn find_provider_error(error: &anyhow::Error) -> Option<&ProviderError> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ProviderError>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_http_status_to_provider_error_kind() {
        assert_eq!(kind_from_http_status(400), ProviderErrorKind::BadRequest);
        assert_eq!(kind_from_http_status(401), ProviderErrorKind::Unauthorized);
        assert_eq!(kind_from_http_status(403), ProviderErrorKind::Forbidden);
        assert_eq!(kind_from_http_status(404), ProviderErrorKind::NotFound);
        assert_eq!(kind_from_http_status(408), ProviderErrorKind::Timeout);
        assert_eq!(kind_from_http_status(429), ProviderErrorKind::RateLimited);
        assert_eq!(kind_from_http_status(500), ProviderErrorKind::Server);
        assert_eq!(kind_from_http_status(503), ProviderErrorKind::Server);
        assert_eq!(kind_from_http_status(418), ProviderErrorKind::Other);
    }

    #[test]
    fn maps_error_kind_to_message_key() {
        let error = ProviderError::new(
            ProviderService::Asr,
            "openai-compatible",
            ProviderErrorKind::Unauthorized,
            "bad credentials",
        );
        assert_eq!(
            error.message_key(),
            "settings.connection_error_unauthorized"
        );
    }

    #[test]
    fn finds_provider_error_in_anyhow_chain() {
        let error = ProviderError::new(
            ProviderService::Llm,
            "openai-compatible",
            ProviderErrorKind::Network,
            "request failed",
        );
        let error = anyhow::Error::new(error).context("outer context");
        assert_eq!(
            find_provider_error(&error).map(|e| e.kind),
            Some(ProviderErrorKind::Network)
        );
    }
}
