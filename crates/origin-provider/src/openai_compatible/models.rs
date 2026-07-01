use origin_protocol::model::ModelInfo;

use crate::openai_compatible::types::ModelsResponse;
use crate::{ProviderError, ProviderResult};

pub fn parse_models_response(provider: &str, body: &str) -> ProviderResult<Vec<ModelInfo>> {
    let response: ModelsResponse =
        serde_json::from_str(body).map_err(|error| ProviderError::InvalidResponse {
            provider: provider.to_string(),
            message: error.to_string(),
            source: Some(Box::new(error)),
        })?;

    Ok(response
        .data
        .into_iter()
        .map(|model| ModelInfo::new(model.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::parse_models_response;

    #[test]
    fn parses_lmstudio_models_fixture() {
        let fixture = include_str!("../../fixtures/lmstudio-models.json");
        let models = parse_models_response("lmstudio", fixture).expect("fixture should parse");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "qwen2.5-coder-7b-instruct");
        assert_eq!(models[1].id, "deepseek-coder-v2-lite-instruct");
    }
}
