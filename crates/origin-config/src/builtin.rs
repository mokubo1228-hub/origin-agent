use origin_protocol::provider::{
    OpenAiCompatibleApi, ProviderAuth, ProviderId, ProviderKind, ProviderPreset,
};

pub const DEFAULT_PROVIDER_ID: &str = "lmstudio";
pub const LMSTUDIO_DEFAULT_BASE_URL: &str = "http://localhost:1234/v1";

pub fn lmstudio_preset() -> ProviderPreset {
    ProviderPreset {
        id: ProviderId::new(DEFAULT_PROVIDER_ID),
        kind: ProviderKind::OpenAiCompatible,
        base_url: LMSTUDIO_DEFAULT_BASE_URL.to_string(),
        api: OpenAiCompatibleApi::ChatCompletions,
        auth: ProviderAuth::None,
    }
}
