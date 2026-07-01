use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::precedence::ConfigError;

pub const ORIGIN_CONFIG_ENV: &str = "ORIGIN_CONFIG";

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct UserConfig {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct ProviderConfig {
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: UserConfig,
}

impl UserConfig {
    pub fn provider(&self, provider_id: &str) -> Option<&ProviderConfig> {
        self.providers.get(provider_id)
    }
}

pub fn load_user_config() -> Result<Option<LoadedConfig>, ConfigError> {
    let Some(path) = resolve_config_path() else {
        return Ok(None);
    };
    load_user_config_from_path(&path)
}

pub fn load_user_config_from_path(path: &Path) -> Result<Option<LoadedConfig>, ConfigError> {
    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let config = toml::from_str::<UserConfig>(&contents).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(Some(LoadedConfig {
        path: path.to_path_buf(),
        config,
    }))
}

pub fn resolve_config_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os(ORIGIN_CONFIG_ENV)
        .and_then(|value| non_empty_os_string(value.to_string_lossy().to_string()))
        .map(PathBuf::from)
    {
        return Some(path);
    }

    env::var_os("HOME")
        .map(PathBuf::from)
        .map(default_config_path_from_home)
}

pub fn default_config_path_from_home(home: PathBuf) -> PathBuf {
    home.join(".origin").join("config.toml")
}

fn non_empty_os_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{default_config_path_from_home, load_user_config_from_path, UserConfig};

    #[test]
    fn missing_config_file_returns_none() {
        let path = temp_dir("missing").join("config.toml");

        let config = load_user_config_from_path(&path).unwrap();

        assert_eq!(config, None);
    }

    #[test]
    fn default_config_path_uses_origin_directory() {
        let path = default_config_path_from_home(PathBuf::from("/home/user"));

        assert_eq!(path, PathBuf::from("/home/user/.origin/config.toml"));
    }

    #[test]
    fn reads_global_and_provider_config() {
        let root = temp_dir("read");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        fs::write(
            &path,
            r#"
default_provider = "openai"
default_model = "global-model"

[providers.openai]
base_url = "https://example.test/api/v1"
api_key_env = "CUSTOM_OPENAI_KEY"
default_model = "provider-model"
"#,
        )
        .unwrap();

        let loaded = load_user_config_from_path(&path).unwrap().unwrap();
        let openai = loaded.config.provider("openai").unwrap();

        assert_eq!(loaded.config.default_provider.as_deref(), Some("openai"));
        assert_eq!(loaded.config.default_model.as_deref(), Some("global-model"));
        assert_eq!(
            openai.base_url.as_deref(),
            Some("https://example.test/api/v1")
        );
        assert_eq!(openai.api_key_env.as_deref(), Some("CUSTOM_OPENAI_KEY"));
        assert_eq!(openai.default_model.as_deref(), Some("provider-model"));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let config = toml::from_str::<UserConfig>(
            r#"
default_provider = "lmstudio"
unknown = "ignored"

[providers.lmstudio]
base_url = "http://localhost:1234/v1"
unknown = "ignored"
"#,
        )
        .unwrap();

        assert_eq!(config.default_provider.as_deref(), Some("lmstudio"));
        assert_eq!(
            config.provider("lmstudio").unwrap().base_url.as_deref(),
            Some("http://localhost:1234/v1")
        );
    }

    #[test]
    fn invalid_toml_returns_error() {
        let root = temp_dir("invalid");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        fs::write(&path, "default_provider = [").unwrap();

        let err = load_user_config_from_path(&path).unwrap_err();

        assert!(err.to_string().contains("Could not parse config"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("origin-config-{name}-{nanos}"))
    }
}
