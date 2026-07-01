use origin_protocol::provider::{
    OpenAiCompatibleApi, ProviderAuth, ProviderId, ProviderKind, ProviderPreset,
};

pub const LMSTUDIO_PROVIDER_ID: &str = "lmstudio";
pub const LMSTUDIO_DEFAULT_BASE_URL: &str = "http://localhost:1234/v1";

pub fn provider_id() -> ProviderId {
    ProviderId::new(LMSTUDIO_PROVIDER_ID)
}

pub fn preset() -> ProviderPreset {
    ProviderPreset {
        id: provider_id(),
        kind: ProviderKind::OpenAiCompatible,
        base_url: LMSTUDIO_DEFAULT_BASE_URL.to_string(),
        api: OpenAiCompatibleApi::ChatCompletions,
        auth: ProviderAuth::None,
    }
}
