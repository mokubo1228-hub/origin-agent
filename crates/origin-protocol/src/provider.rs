use std::fmt;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderKind {
    OpenAiCompatible,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenAiCompatibleApi {
    ChatCompletions,
    Responses,
}

impl OpenAiCompatibleApi {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat-completions",
            Self::Responses => "responses",
        }
    }
}

impl fmt::Display for OpenAiCompatibleApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderPreset {
    pub id: ProviderId,
    pub kind: ProviderKind,
    pub base_url: String,
    pub api: OpenAiCompatibleApi,
    pub auth: ProviderAuth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderAuth {
    None,
    BearerEnv { api_key_env: String },
}
