pub mod chat_completions;
mod http;
pub mod models;
pub mod types;

use origin_protocol::chat::{ChatRequest, ChatResponse};
use origin_protocol::model::ModelInfo;
use origin_protocol::provider::{ProviderAuth, ProviderId, ProviderKind, ProviderPreset};
use std::env;
use std::time::Duration;

use crate::provider::ModelProvider;
use crate::{ProviderError, ProviderResult};

pub const DEFAULT_CHAT_TIMEOUT: Duration = Duration::from_secs(120);
pub const PROPOSE_CHAT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct OpenAiCompatibleConfig {
    pub id: ProviderId,
    pub base_url: String,
    pub auth: ProviderAuth,
}

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    config: OpenAiCompatibleConfig,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: OpenAiCompatibleConfig) -> Self {
        Self { config }
    }

    pub fn from_preset(
        preset: &ProviderPreset,
        base_url_override: Option<String>,
        api_key_env_override: Option<String>,
    ) -> Self {
        Self::new(OpenAiCompatibleConfig {
            id: preset.id.clone(),
            base_url: base_url_override.unwrap_or_else(|| preset.base_url.clone()),
            auth: api_key_env_override
                .map(|api_key_env| ProviderAuth::BearerEnv { api_key_env })
                .unwrap_or_else(|| preset.auth.clone()),
        })
    }

    pub fn requires_api_key_env(&self, api_key_env: &str) -> bool {
        matches!(
            &self.config.auth,
            ProviderAuth::BearerEnv { api_key_env: env } if env == api_key_env
        )
    }

    pub fn complete_chat_with_timeout(
        &self,
        request: ChatRequest,
        timeout: Duration,
    ) -> ProviderResult<ChatResponse> {
        let provider = self.id().as_str();
        let base_url = self.base_url();
        let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let auth_header = resolve_auth_header(provider, &self.config.auth)?;
        let body = chat_completions::serialize_chat_request(provider, &request)?;
        let response = http::post_json_with_timeout(
            provider,
            base_url,
            &endpoint,
            &body,
            auth_header.as_deref(),
            "chat_completions",
            timeout,
        )?;

        match response.status {
            200 => chat_completions::parse_chat_response(provider, &response.body),
            404 => Err(ProviderError::ModelEndpointNotFound {
                provider: provider.to_string(),
                url: endpoint,
            }),
            status if status >= 500 => Err(ProviderError::ServerError {
                provider: provider.to_string(),
                status,
                body: response.body,
            }),
            status => Err(ProviderError::InvalidResponse {
                provider: provider.to_string(),
                message: format!("unexpected HTTP status {status}: {}", response.body),
                source: None,
            }),
        }
    }
}

impl ModelProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn base_url(&self) -> &str {
        &self.config.base_url
    }

    fn list_models(&self) -> ProviderResult<Vec<ModelInfo>> {
        let provider = self.id().as_str();
        let base_url = self.base_url();
        let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
        let auth_header = resolve_auth_header(provider, &self.config.auth)?;
        let response = http::get(
            provider,
            base_url,
            &endpoint,
            auth_header.as_deref(),
            "list_models",
        )?;

        match response.status {
            200 => models::parse_models_response(provider, &response.body),
            404 => Err(ProviderError::ModelEndpointNotFound {
                provider: provider.to_string(),
                url: endpoint,
            }),
            status if status >= 500 => Err(ProviderError::ServerError {
                provider: provider.to_string(),
                status,
                body: response.body,
            }),
            status => Err(ProviderError::InvalidResponse {
                provider: provider.to_string(),
                message: format!("unexpected HTTP status {status}: {}", response.body),
                source: None,
            }),
        }
    }

    fn complete_chat(&self, request: ChatRequest) -> ProviderResult<ChatResponse> {
        self.complete_chat_with_timeout(request, DEFAULT_CHAT_TIMEOUT)
    }
}

fn resolve_auth_header(provider: &str, auth: &ProviderAuth) -> ProviderResult<Option<String>> {
    resolve_auth_header_with_env(provider, auth, |key| env::var(key).ok())
}

fn resolve_auth_header_with_env<F>(
    provider: &str,
    auth: &ProviderAuth,
    read_env: F,
) -> ProviderResult<Option<String>>
where
    F: FnOnce(&str) -> Option<String>,
{
    match auth {
        ProviderAuth::None => Ok(None),
        ProviderAuth::BearerEnv { api_key_env } => {
            let api_key = read_env(api_key_env)
                .and_then(non_empty_secret)
                .ok_or_else(|| ProviderError::MissingApiKey {
                    provider: provider.to_string(),
                    api_key_env: api_key_env.clone(),
                })?;

            Ok(Some(format!("Bearer {api_key}")))
        }
    }
}

fn non_empty_secret(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use origin_protocol::provider::ProviderAuth;

    use super::{resolve_auth_header_with_env, DEFAULT_CHAT_TIMEOUT, PROPOSE_CHAT_TIMEOUT};

    #[test]
    fn propose_chat_timeout_is_longer_than_default_chat_timeout() {
        assert!(PROPOSE_CHAT_TIMEOUT > DEFAULT_CHAT_TIMEOUT);
    }

    #[test]
    fn resolves_no_auth_for_lmstudio_style_provider() {
        let auth = resolve_auth_header_with_env("lmstudio", &ProviderAuth::None, |_| {
            panic!("env should not be read for auth none")
        })
        .unwrap();

        assert_eq!(auth, None);
    }

    #[test]
    fn resolves_bearer_auth_header_from_env() {
        let auth = ProviderAuth::BearerEnv {
            api_key_env: "OPENAI_API_KEY".to_string(),
        };
        let header =
            resolve_auth_header_with_env("openai", &auth, |_| Some(" test-key ".to_string()))
                .unwrap();

        assert_eq!(header.as_deref(), Some("Bearer test-key"));
    }

    #[test]
    fn errors_when_required_api_key_is_missing() {
        let auth = ProviderAuth::BearerEnv {
            api_key_env: "OPENAI_API_KEY".to_string(),
        };
        let error = resolve_auth_header_with_env("openai", &auth, |_| None).unwrap_err();

        assert_eq!(
            error.user_message(),
            "OpenAI API key is missing.\nSet OPENAI_API_KEY and try again."
        );
    }

    #[test]
    fn errors_when_required_api_key_is_blank() {
        let auth = ProviderAuth::BearerEnv {
            api_key_env: "OPENAI_API_KEY".to_string(),
        };
        let error = resolve_auth_header_with_env("openai", &auth, |_| Some(" \n\t ".to_string()))
            .unwrap_err();

        assert_eq!(
            error.user_message(),
            "OpenAI API key is missing.\nSet OPENAI_API_KEY and try again."
        );
    }
}
