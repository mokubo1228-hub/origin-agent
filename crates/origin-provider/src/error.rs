use std::error::Error;
use std::fmt;
type ErrorSource = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug)]
pub enum ProviderError {
    ConnectionFailed {
        provider: String,
        base_url: String,
        operation: &'static str,
        source: ErrorSource,
    },
    Timeout {
        provider: String,
        base_url: String,
        operation: &'static str,
        source: ErrorSource,
    },
    InvalidResponse {
        provider: String,
        message: String,
        source: Option<ErrorSource>,
    },
    ModelEndpointNotFound {
        provider: String,
        url: String,
    },
    ServerError {
        provider: String,
        status: u16,
        body: String,
    },
    MissingApiKey {
        provider: String,
        api_key_env: String,
    },
    UnsupportedScheme {
        provider: String,
        base_url: String,
    },
}

pub type ProviderResult<T> = Result<T, ProviderError>;

impl ProviderError {
    pub fn user_message(&self) -> String {
        match self {
            Self::ConnectionFailed {
                provider,
                base_url,
                operation,
                ..
            } if provider == "lmstudio" => {
                let endpoint = match *operation {
                    "chat_completions" => format!("POST {base_url}/chat/completions"),
                    _ => format!("GET {base_url}/models"),
                };
                let command = match *operation {
                    "chat_completions" => "origin chat --base-url <url> --model <model> \"hello\"",
                    _ => "origin models --base-url <url>",
                };
                format!(
                    "Could not connect to LM Studio at {base_url}.\n\n\
                     Make sure LM Studio is running and the local server is enabled.\n\
                     Expected endpoint: {endpoint}\n\n\
                     You can override the endpoint with:\n  {command}"
                )
            }
            Self::ConnectionFailed {
                provider,
                base_url,
                operation,
                ..
            } => format!(
                "Could not connect to provider `{provider}` at {base_url} while running {operation}."
            ),
            Self::Timeout {
                provider,
                base_url,
                operation,
                ..
            } => format!(
                "Timed out waiting for provider `{provider}` at {base_url} while running {operation}.\n\
                 The provider was not reached or did not respond in time; for local providers, model generation may still be running."
            ),
            Self::InvalidResponse {
                provider, message, ..
            } => {
                format!("Provider `{provider}` returned an invalid response: {message}")
            }
            Self::ModelEndpointNotFound { provider, url } => {
                format!("Provider `{provider}` does not expose a models endpoint at {url}.")
            }
            Self::ServerError {
                provider,
                status,
                body,
            } => format!("Provider `{provider}` returned HTTP {status}: {body}"),
            Self::MissingApiKey {
                provider,
                api_key_env,
            } if provider == "openai" => format!(
                "OpenAI API key is missing.\n\
                 Set {api_key_env} and try again."
            ),
            Self::MissingApiKey {
                provider,
                api_key_env,
            } => format!(
                "Provider `{provider}` requires an API key.\n\
                 Set {api_key_env} and try again."
            ),
            Self::UnsupportedScheme { provider, base_url } => format!(
                "Provider `{provider}` uses unsupported base_url `{base_url}`. Phase 1 supports plain http:// endpoints only."
            ),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl Error for ProviderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ConnectionFailed { source, .. } => Some(&**source),
            Self::Timeout { source, .. } => Some(&**source),
            Self::InvalidResponse { source, .. } => source
                .as_ref()
                .map(|source| &**source as &(dyn Error + 'static)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderError;

    #[test]
    fn timeout_message_describes_waiting_for_provider_response() {
        let error = ProviderError::Timeout {
            provider: "lmstudio".to_string(),
            base_url: "http://localhost:1234/v1".to_string(),
            operation: "chat_completions",
            source: Box::new(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout")),
        };
        let message = error.user_message();

        assert!(message.contains("Timed out waiting for provider `lmstudio`"));
        assert!(message.contains("while running chat_completions"));
        assert!(message.contains("did not respond in time"));
        assert!(message.contains("model generation may still be running"));
        assert!(!message.contains("Timed out connecting"));
    }
}
