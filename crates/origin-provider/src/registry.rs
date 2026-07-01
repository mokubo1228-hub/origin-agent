use origin_protocol::provider::{ProviderId, ProviderPreset};

use crate::openai_compatible::OpenAiCompatibleProvider;
use crate::presets;

pub struct ProviderRegistry {
    presets: Vec<ProviderPreset>,
}

impl ProviderRegistry {
    pub fn new(presets: Vec<ProviderPreset>) -> Self {
        Self { presets }
    }

    pub fn presets(&self) -> &[ProviderPreset] {
        &self.presets
    }

    pub fn get_preset(&self, id: &str) -> Option<&ProviderPreset> {
        self.presets.iter().find(|preset| preset.id.as_str() == id)
    }

    pub fn openai_compatible_provider(
        &self,
        id: &str,
        base_url_override: Option<String>,
        api_key_env_override: Option<String>,
    ) -> Option<OpenAiCompatibleProvider> {
        let preset = self.get_preset(id)?;
        Some(OpenAiCompatibleProvider::from_preset(
            preset,
            base_url_override,
            api_key_env_override,
        ))
    }
}

pub fn builtin_provider_presets() -> Vec<ProviderPreset> {
    vec![presets::lmstudio::preset(), presets::openai::preset()]
}

pub fn builtin_registry() -> ProviderRegistry {
    ProviderRegistry::new(builtin_provider_presets())
}

pub fn default_provider_id() -> ProviderId {
    presets::lmstudio::provider_id()
}

#[cfg(test)]
mod tests {
    use origin_protocol::provider::ProviderAuth;

    use crate::provider::ModelProvider;

    use super::{builtin_provider_presets, builtin_registry, default_provider_id};

    #[test]
    fn builtin_registry_contains_lmstudio_and_openai() {
        let presets = builtin_provider_presets();
        let ids = presets
            .iter()
            .map(|preset| preset.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["lmstudio", "openai"]);
    }

    #[test]
    fn default_provider_remains_lmstudio() {
        assert_eq!(default_provider_id().as_str(), "lmstudio");
    }

    #[test]
    fn openai_preset_uses_expected_base_url_and_api_key_env() {
        let registry = builtin_registry();
        let preset = registry.get_preset("openai").unwrap();

        assert_eq!(preset.base_url, "https://api.openai.com/v1");
        assert_eq!(
            preset.auth,
            ProviderAuth::BearerEnv {
                api_key_env: "OPENAI_API_KEY".to_string()
            }
        );
    }

    #[test]
    fn base_url_override_preserves_provider_auth() {
        let registry = builtin_registry();
        let provider = registry
            .openai_compatible_provider(
                "openai",
                Some("https://example.test/openai-compatible".to_string()),
                None,
            )
            .unwrap();

        assert_eq!(
            provider.base_url(),
            "https://example.test/openai-compatible"
        );
        assert!(provider.requires_api_key_env("OPENAI_API_KEY"));
    }

    #[test]
    fn api_key_env_override_replaces_preset_auth_env() {
        let registry = builtin_registry();
        let provider = registry
            .openai_compatible_provider("openai", None, Some("CUSTOM_KEY".to_string()))
            .unwrap();

        assert!(provider.requires_api_key_env("CUSTOM_KEY"));
    }
}
