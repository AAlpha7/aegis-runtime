use crate::llm::LlmProviderErrorKind;

#[derive(Debug, thiserror::Error)]
pub enum SkillOptError {
    #[error("{kind:?}: {message}")]
    Routing {
        kind: LlmProviderErrorKind,
        message: String,
    },
    #[error("fail closed: {message}")]
    FailClosed {
        kind: LlmProviderErrorKind,
        message: String,
    },
    #[error("invalid skill optimization input: {message}")]
    Invalid { message: String },
}

impl SkillOptError {
    pub fn provider_kind(&self) -> Option<LlmProviderErrorKind> {
        match self {
            Self::Routing { kind, .. } | Self::FailClosed { kind, .. } => Some(*kind),
            Self::Invalid { .. } => None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }

    pub fn fail_closed(kind: LlmProviderErrorKind, message: impl Into<String>) -> Self {
        Self::FailClosed {
            kind,
            message: message.into(),
        }
    }

    pub fn routing(err: crate::llm::LlmProviderError) -> Self {
        Self::Routing {
            kind: err.kind,
            message: err.message,
        }
    }
}

impl From<crate::llm::LlmProviderError> for SkillOptError {
    fn from(err: crate::llm::LlmProviderError) -> Self {
        Self::routing(err)
    }
}
