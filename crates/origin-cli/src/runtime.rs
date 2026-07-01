use std::env;
use std::path::PathBuf;

use origin_config::{resolve_config, resolve_config_path, CliOverrides};
use origin_protocol::model::select_fallback_model;
use origin_protocol::provider::ProviderAuth;
use origin_provider::openai_compatible::OpenAiCompatibleProvider;
use origin_provider::{builtin_registry, ModelProvider};
use origin_session::default_sessions_root;
use origin_tools::{discover_git_repo, git_dirty_status};

const ORIGIN_MODEL_ENV: &str = "ORIGIN_MODEL";

#[derive(Clone)]
pub(crate) struct RuntimeState {
    pub(crate) provider: OpenAiCompatibleProvider,
    pub(crate) provider_id: String,
    pub(crate) provider_kind: String,
    pub(crate) base_url: String,
    pub(crate) auth_label: String,
    pub(crate) model_id: Option<String>,
    pub(crate) model_source: String,
    pub(crate) model_error: Option<String>,
    pub(crate) config_path: String,
    pub(crate) sessions_root: String,
    pub(crate) cwd: String,
    pub(crate) git_state: String,
}

pub(crate) struct RuntimeModel {
    pub(crate) id: Option<String>,
    pub(crate) source: String,
    pub(crate) error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ProviderSwitchResult {
    pub(crate) provider_id: String,
    pub(crate) model_id: Option<String>,
    pub(crate) model_source: String,
}

impl RuntimeState {
    pub(crate) fn initialize() -> Result<Self, String> {
        let config = resolve_config(CliOverrides::default()).map_err(|error| error.to_string())?;
        let registry = builtin_registry();
        let preset = registry
            .get_preset(config.provider.as_str())
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        let provider = registry
            .openai_compatible_provider(
                config.provider.as_str(),
                config.base_url_override,
                config.api_key_env_override.clone(),
            )
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        let resolved_model = resolve_runtime_model(config.default_model, &provider);
        let sessions_root = default_sessions_root()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("unknown ({error})"));
        let cwd = env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("unknown ({error})"));

        Ok(Self {
            provider_id: provider.id().as_str().to_string(),
            provider_kind: provider.kind().as_str().to_string(),
            base_url: provider.base_url().to_string(),
            auth_label: auth_label(&preset.auth, config.api_key_env_override.as_deref()),
            provider,
            model_id: resolved_model.id,
            model_source: resolved_model.source,
            model_error: resolved_model.error,
            config_path: display_config_path(config.config_path.as_ref()),
            sessions_root,
            cwd,
            git_state: git_state_label(),
        })
    }

    pub(crate) fn initialize_without_model_fallback() -> Result<Self, String> {
        let config = resolve_config(CliOverrides::default()).map_err(|error| error.to_string())?;
        let registry = builtin_registry();
        let preset = registry
            .get_preset(config.provider.as_str())
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        let provider = registry
            .openai_compatible_provider(
                config.provider.as_str(),
                config.base_url_override,
                config.api_key_env_override.clone(),
            )
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        let resolved_model = resolve_runtime_model_without_provider_call(config.default_model);
        let sessions_root = default_sessions_root()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("unknown ({error})"));
        let cwd = env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("unknown ({error})"));

        Ok(Self {
            provider_id: provider.id().as_str().to_string(),
            provider_kind: provider.kind().as_str().to_string(),
            base_url: provider.base_url().to_string(),
            auth_label: auth_label(&preset.auth, config.api_key_env_override.as_deref()),
            provider,
            model_id: resolved_model.id,
            model_source: resolved_model.source,
            model_error: resolved_model.error,
            config_path: display_config_path(config.config_path.as_ref()),
            sessions_root,
            cwd,
            git_state: git_state_label(),
        })
    }

    pub(crate) fn model_for_chat(&self) -> Result<&str, String> {
        self.model_id.as_deref().ok_or_else(|| {
            format!(
                "No model is resolved for this shell session: {}",
                self.model_error.as_deref().unwrap_or("unknown error")
            )
        })
    }

    pub(crate) fn ensure_model_for_chat(&mut self) -> Result<&str, String> {
        if self.model_id.is_none() {
            let resolved_model = resolve_runtime_model_without_env(None, &self.provider);
            self.model_id = resolved_model.id;
            self.model_source = resolved_model.source;
            self.model_error = resolved_model.error;
        }

        self.model_for_chat()
    }

    pub(crate) fn set_model(&mut self, model_id: &str) -> Result<(), String> {
        self.set_model_with_source(model_id, "shell")
    }

    pub(crate) fn set_model_with_source(
        &mut self,
        model_id: &str,
        source: &str,
    ) -> Result<(), String> {
        let Some(model_id) = non_empty(model_id.to_string()) else {
            return Err("Model id cannot be empty.\nUsage: /model set <model-id>".to_string());
        };
        self.model_id = Some(model_id.clone());
        self.model_source = source.to_string();
        self.model_error = None;

        Ok(())
    }

    pub(crate) fn set_provider(
        &mut self,
        provider_id: &str,
    ) -> Result<ProviderSwitchResult, String> {
        let Some(provider_id) = non_empty(provider_id.to_string()) else {
            return Err("Provider id cannot be empty.".to_string());
        };
        let config = resolve_config(CliOverrides {
            provider: Some(provider_id.clone()),
            base_url: None,
        })
        .map_err(|error| error.to_string())?;
        let registry = builtin_registry();
        let preset = registry
            .get_preset(config.provider.as_str())
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        ensure_auth_available(
            config.provider.as_str(),
            &preset.auth,
            config.api_key_env_override.as_deref(),
        )?;
        let provider = registry
            .openai_compatible_provider(
                config.provider.as_str(),
                config.base_url_override,
                config.api_key_env_override.clone(),
            )
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        let resolved_model = resolve_runtime_model_without_env(config.default_model, &provider);
        Ok(self.apply_provider(
            provider,
            auth_label(&preset.auth, config.api_key_env_override.as_deref()),
            resolved_model,
        ))
    }

    pub(crate) fn set_model_route(
        &mut self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderSwitchResult, String> {
        let Some(provider_id) = non_empty(provider_id.to_string()) else {
            return Err(
                "Provider id cannot be empty.\nUsage: /model set <provider>/<model-id>".to_string(),
            );
        };
        let Some(model_id) = non_empty(model_id.to_string()) else {
            return Err(
                "Model id cannot be empty.\nUsage: /model set <provider>/<model-id>".to_string(),
            );
        };
        let config = resolve_config(CliOverrides {
            provider: Some(provider_id.clone()),
            base_url: None,
        })
        .map_err(|error| error.to_string())?;
        let registry = builtin_registry();
        let preset = registry
            .get_preset(config.provider.as_str())
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        ensure_auth_available(
            config.provider.as_str(),
            &preset.auth,
            config.api_key_env_override.as_deref(),
        )?;
        let provider = registry
            .openai_compatible_provider(
                config.provider.as_str(),
                config.base_url_override,
                config.api_key_env_override.clone(),
            )
            .ok_or_else(|| format!("Unknown provider `{}`.", config.provider.as_str()))?;
        let resolved_model = RuntimeModel {
            id: Some(model_id),
            source: "session-route".to_string(),
            error: None,
        };
        Ok(self.apply_provider(
            provider,
            auth_label(&preset.auth, config.api_key_env_override.as_deref()),
            resolved_model,
        ))
    }

    pub(crate) fn apply_provider(
        &mut self,
        provider: OpenAiCompatibleProvider,
        auth_label: String,
        resolved_model: RuntimeModel,
    ) -> ProviderSwitchResult {
        let result = ProviderSwitchResult {
            provider_id: provider.id().as_str().to_string(),
            model_id: resolved_model.id.clone(),
            model_source: resolved_model.source.clone(),
        };

        self.provider_id = provider.id().as_str().to_string();
        self.provider_kind = provider.kind().as_str().to_string();
        self.base_url = provider.base_url().to_string();
        self.auth_label = auth_label;
        self.provider = provider;
        self.model_id = resolved_model.id;
        self.model_source = resolved_model.source;
        self.model_error = resolved_model.error;

        result
    }
}

pub(crate) fn resolve_runtime_model(
    config_model: Option<String>,
    provider: &OpenAiCompatibleProvider,
) -> RuntimeModel {
    if let Some(id) = env_model() {
        return RuntimeModel {
            id: Some(id),
            source: "env".to_string(),
            error: None,
        };
    }
    resolve_runtime_model_without_env(config_model, provider)
}

pub(crate) fn resolve_runtime_model_without_provider_call(
    config_model: Option<String>,
) -> RuntimeModel {
    if let Some(id) = env_model() {
        return RuntimeModel {
            id: Some(id),
            source: "env".to_string(),
            error: None,
        };
    }
    if let Some(id) = config_model.and_then(non_empty) {
        return RuntimeModel {
            id: Some(id),
            source: "config".to_string(),
            error: None,
        };
    }

    RuntimeModel {
        id: None,
        source: "unresolved".to_string(),
        error: Some("model fallback is not resolved in TUI skeleton".to_string()),
    }
}

pub(crate) fn resolve_runtime_model_without_env(
    config_model: Option<String>,
    provider: &OpenAiCompatibleProvider,
) -> RuntimeModel {
    if let Some(id) = config_model.and_then(non_empty) {
        return RuntimeModel {
            id: Some(id),
            source: "config".to_string(),
            error: None,
        };
    }

    match provider.list_models() {
        Ok(models) => match select_fallback_model(&models) {
            Some(model) => RuntimeModel {
                id: Some(model.id.clone()),
                source: "fallback".to_string(),
                error: None,
            },
            None => RuntimeModel {
                id: None,
                source: "unresolved".to_string(),
                error: Some("provider returned no fallback candidate models".to_string()),
            },
        },
        Err(error) => RuntimeModel {
            id: None,
            source: "unresolved".to_string(),
            error: Some(first_error_line(&error.user_message())),
        },
    }
}

pub(crate) fn auth_label(auth: &ProviderAuth, api_key_env_override: Option<&str>) -> String {
    match (auth, api_key_env_override) {
        (ProviderAuth::None, None) => "none".to_string(),
        (ProviderAuth::None, Some(_)) | (ProviderAuth::BearerEnv { .. }, _) => "env".to_string(),
    }
}

pub(crate) fn ensure_auth_available(
    provider_id: &str,
    auth: &ProviderAuth,
    api_key_env_override: Option<&str>,
) -> Result<(), String> {
    let api_key_env = match (auth, api_key_env_override) {
        (ProviderAuth::None, None) => return Ok(()),
        (ProviderAuth::None, Some(api_key_env)) => api_key_env,
        (ProviderAuth::BearerEnv { api_key_env }, None) => api_key_env,
        (ProviderAuth::BearerEnv { .. }, Some(api_key_env)) => api_key_env,
    };
    if env::var(api_key_env).ok().and_then(non_empty).is_some() {
        return Ok(());
    }

    Err(format!(
        "Provider `{provider_id}` requires an API key.\nSet {api_key_env} and try again.\nDo not print the API key value."
    ))
}

pub(crate) fn first_error_line(error: &str) -> String {
    error.lines().next().unwrap_or(error).to_string()
}

fn env_model() -> Option<String> {
    env::var(ORIGIN_MODEL_ENV).ok().and_then(non_empty)
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn display_config_path(config_path: Option<&PathBuf>) -> String {
    config_path
        .cloned()
        .or_else(resolve_config_path)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_state_label() -> String {
    match discover_git_repo() {
        Ok(repo) => match git_dirty_status(&repo) {
            Ok(status) if status.is_dirty => "dirty".to_string(),
            Ok(_) => "clean".to_string(),
            Err(error) => format!("unknown ({error})"),
        },
        Err(_) => "not a git repository".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use origin_protocol::provider::{ProviderAuth, ProviderId, ProviderKind};
    use origin_provider::openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleProvider};

    use super::{
        auth_label, ensure_auth_available, resolve_runtime_model, RuntimeModel, RuntimeState,
    };

    #[test]
    fn runtime_state_holds_provider_model_config_session_cwd_and_git_state() {
        let state = test_runtime_state();

        assert_eq!(state.provider_id, "lmstudio");
        assert_eq!(state.provider_kind, "openai-compatible");
        assert_eq!(state.base_url, "http://localhost:1234/v1");
        assert_eq!(state.auth_label, "none");
        assert_eq!(state.model_id.as_deref(), Some("runtime-model"));
        assert_eq!(state.model_source, "config");
        assert_eq!(state.model_error, None);
        assert_eq!(state.config_path, "/home/user/.origin/config.toml");
        assert_eq!(state.sessions_root, "/home/user/.origin/sessions");
        assert_eq!(state.cwd, "/repo");
        assert_eq!(state.git_state, "clean");
        assert_eq!(state.model_for_chat().unwrap(), "runtime-model");
    }

    #[test]
    fn set_model_marks_model_source_as_shell() {
        let mut state = test_runtime_state();

        state.set_model("google/gemma-4-31b").unwrap();

        assert_eq!(state.model_id.as_deref(), Some("google/gemma-4-31b"));
        assert_eq!(state.model_source, "shell");
        assert_eq!(state.model_error, None);
    }

    #[test]
    fn apply_provider_updates_provider_base_url_auth_and_model() {
        let mut state = test_runtime_state();
        let result = state.apply_provider(
            test_provider_with_id("openai", "https://api.openai.com/v1"),
            "env".to_string(),
            RuntimeModel {
                id: Some("zai-org-glm-5-1".to_string()),
                source: "config".to_string(),
                error: None,
            },
        );

        assert_eq!(result.provider_id, "openai");
        assert_eq!(result.model_id.as_deref(), Some("zai-org-glm-5-1"));
        assert_eq!(state.provider_id, "openai");
        assert_eq!(state.base_url, "https://api.openai.com/v1");
        assert_eq!(state.auth_label, "env");
        assert_eq!(state.model_for_chat().unwrap(), "zai-org-glm-5-1");
    }

    #[test]
    fn runtime_model_uses_config_before_provider_fallback() {
        let provider = test_provider();
        let model = resolve_runtime_model(Some("config-model".to_string()), &provider);

        assert_eq!(model.id.as_deref(), Some("config-model"));
        assert_eq!(model.source, "config");
    }

    #[test]
    fn auth_label_hides_auth_details() {
        assert_eq!(auth_label(&ProviderAuth::None, None), "none");
        assert_eq!(
            auth_label(
                &ProviderAuth::BearerEnv {
                    api_key_env: "OPENAI_API_KEY".to_string()
                },
                None
            ),
            "env"
        );
    }

    #[test]
    fn auth_check_reports_missing_key_without_secret_terms() {
        let auth = ProviderAuth::BearerEnv {
            api_key_env: "ORIGIN_TEST_MISSING_KEY_RUNTIME".to_string(),
        };
        let err = ensure_auth_available("openai", &auth, None).unwrap_err();

        assert!(err.contains("Provider `openai` requires an API key."));
        assert!(err.contains("ORIGIN_TEST_MISSING_KEY_RUNTIME"));
        assert!(err.contains("Do not print the API key value."));
        assert!(!err.contains("Authorization"));
        assert!(!err.contains("Bearer"));
    }

    fn test_runtime_state() -> RuntimeState {
        RuntimeState {
            provider: test_provider(),
            provider_id: "lmstudio".to_string(),
            provider_kind: ProviderKind::OpenAiCompatible.as_str().to_string(),
            base_url: "http://localhost:1234/v1".to_string(),
            auth_label: "none".to_string(),
            model_id: Some("runtime-model".to_string()),
            model_source: "config".to_string(),
            model_error: None,
            config_path: "/home/user/.origin/config.toml".to_string(),
            sessions_root: "/home/user/.origin/sessions".to_string(),
            cwd: "/repo".to_string(),
            git_state: "clean".to_string(),
        }
    }

    fn test_provider() -> OpenAiCompatibleProvider {
        test_provider_with_id("lmstudio", "http://localhost:1234/v1")
    }

    fn test_provider_with_id(id: &str, base_url: &str) -> OpenAiCompatibleProvider {
        OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
            id: ProviderId::new(id),
            base_url: base_url.to_string(),
            auth: ProviderAuth::None,
        })
    }
}
