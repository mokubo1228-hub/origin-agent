use origin_protocol::provider::{
    OpenAiCompatibleApi, ProviderAuth, ProviderId, ProviderKind, ProviderPreset,
};

pub const OPENAI_PROVIDER_ID: &str = "openai";
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";

pub fn provider_id() -> ProviderId {
    ProviderId::new(OPENAI_PROVIDER_ID)
}

pub fn preset() -> ProviderPreset {
    ProviderPreset {
        id: provider_id(),
        kind: ProviderKind::OpenAiCompatible,
        base_url: OPENAI_DEFAULT_BASE_URL.to_string(),
        api: OpenAiCompatibleApi::ChatCompletions,
        auth: ProviderAuth::BearerEnv {
            api_key_env: OPENAI_API_KEY_ENV.to_string(),
        },
    }
}
