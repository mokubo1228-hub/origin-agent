use origin_protocol::provider::ProviderId;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use crate::builtin;
use crate::file::{load_user_config, UserConfig};

#[derive(Default)]
pub struct CliOverrides {
    pub provider: Option<String>,
    pub base_url: Option<String>,
}

pub struct ResolvedConfig {
    pub provider: ProviderId,
    pub base_url_override: Option<String>,
    pub api_key_env_override: Option<String>,
    pub default_model: Option<String>,
    pub config_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

pub fn resolve_config(overrides: CliOverrides) -> Result<ResolvedConfig, ConfigError> {
    let loaded = load_user_config()?;
    Ok(resolve_config_with_user_config(
        overrides,
        loaded.as_ref().map(|loaded| &loaded.config),
        loaded.as_ref().map(|loaded| loaded.path.clone()),
    ))
}

pub fn resolve_config_with_user_config(
    overrides: CliOverrides,
    user_config: Option<&UserConfig>,
    config_path: Option<PathBuf>,
) -> ResolvedConfig {
    let provider = non_empty(overrides.provider)
        .or_else(|| user_config.and_then(|config| non_empty(config.default_provider.clone())))
        .unwrap_or_else(|| builtin::DEFAULT_PROVIDER_ID.to_string());
    let provider_config = user_config.and_then(|config| config.provider(&provider));
    let base_url_override = non_empty(overrides.base_url)
        .or_else(|| provider_config.and_then(|config| non_empty(config.base_url.clone())));
    let api_key_env_override =
        provider_config.and_then(|config| non_empty(config.api_key_env.clone()));
    let default_model = provider_config
        .and_then(|config| non_empty(config.default_model.clone()))
        .or_else(|| user_config.and_then(|config| non_empty(config.default_model.clone())));

    ResolvedConfig {
        provider: ProviderId::new(provider),
        base_url_override,
        api_key_env_override,
        default_model,
        config_path,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "Could not read config `{}`: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(f, "Could not parse config `{}`: {source}", path.display())
            }
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::file::{ProviderConfig, UserConfig};

    use super::{resolve_config_with_user_config, CliOverrides};

    #[test]
    fn missing_config_uses_builtin_default_provider() {
        let resolved = resolve_config_with_user_config(CliOverrides::default(), None, None);

        assert_eq!(resolved.provider.as_str(), "lmstudio");
        assert_eq!(resolved.default_model, None);
    }

    #[test]
    fn reads_default_provider_and_global_default_model() {
        let config = UserConfig {
            default_provider: Some("openai".to_string()),
            default_model: Some("global-model".to_string()),
            providers: BTreeMap::new(),
        };

        let resolved =
            resolve_config_with_user_config(CliOverrides::default(), Some(&config), None);

        assert_eq!(resolved.provider.as_str(), "openai");
        assert_eq!(resolved.default_model.as_deref(), Some("global-model"));
    }

    #[test]
    fn provider_default_model_beats_global_default_model() {
        let config = user_config_with_provider(
            "lmstudio",
            ProviderConfig {
                default_model: Some("provider-model".to_string()),
                ..ProviderConfig::default()
            },
            Some("global-model"),
        );

        let resolved =
            resolve_config_with_user_config(CliOverrides::default(), Some(&config), None);

        assert_eq!(resolved.default_model.as_deref(), Some("provider-model"));
    }

    #[test]
    fn cli_provider_beats_config_default_provider() {
        let config = UserConfig {
            default_provider: Some("openai".to_string()),
            ..UserConfig::default()
        };
        let overrides = CliOverrides {
            provider: Some("lmstudio".to_string()),
            base_url: None,
        };

        let resolved = resolve_config_with_user_config(overrides, Some(&config), None);

        assert_eq!(resolved.provider.as_str(), "lmstudio");
    }

    #[test]
    fn cli_base_url_beats_config_base_url() {
        let config = user_config_with_provider(
            "lmstudio",
            ProviderConfig {
                base_url: Some("http://config.test/v1".to_string()),
                ..ProviderConfig::default()
            },
            None,
        );
        let overrides = CliOverrides {
            provider: None,
            base_url: Some("http://cli.test/v1".to_string()),
        };

        let resolved = resolve_config_with_user_config(overrides, Some(&config), None);

        assert_eq!(
            resolved.base_url_override.as_deref(),
            Some("http://cli.test/v1")
        );
    }

    #[test]
    fn uses_config_base_url_without_cli_base_url() {
        let config = user_config_with_provider(
            "lmstudio",
            ProviderConfig {
                base_url: Some("http://config.test/v1".to_string()),
                ..ProviderConfig::default()
            },
            None,
        );

        let resolved =
            resolve_config_with_user_config(CliOverrides::default(), Some(&config), None);

        assert_eq!(
            resolved.base_url_override.as_deref(),
            Some("http://config.test/v1")
        );
    }

    #[test]
    fn config_api_key_env_override_is_resolved() {
        let config = user_config_with_provider(
            "openai",
            ProviderConfig {
                api_key_env: Some("CUSTOM_KEY".to_string()),
                ..ProviderConfig::default()
            },
            None,
        );

        let resolved =
            resolve_config_with_user_config(CliOverrides::default(), Some(&config), None);

        assert_eq!(resolved.api_key_env_override.as_deref(), Some("CUSTOM_KEY"));
    }

    fn user_config_with_provider(
        provider: &str,
        provider_config: ProviderConfig,
        global_model: Option<&str>,
    ) -> UserConfig {
        let mut providers = BTreeMap::new();
        providers.insert(provider.to_string(), provider_config);
        UserConfig {
            default_provider: Some(provider.to_string()),
            default_model: global_model.map(str::to_string),
            providers,
        }
    }
}
