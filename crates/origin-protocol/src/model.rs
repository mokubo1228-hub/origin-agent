#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelInfo {
    pub id: String,
}

impl ModelInfo {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    pub fn is_fallback_candidate(&self) -> bool {
        is_fallback_candidate_model_id(&self.id)
    }
}

pub fn is_fallback_candidate_model_id(model_id: &str) -> bool {
    let normalized = model_id.to_ascii_lowercase();
    !NON_CHAT_FALLBACK_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
}

pub fn select_fallback_model(models: &[ModelInfo]) -> Option<&ModelInfo> {
    models.iter().find(|model| model.is_fallback_candidate())
}

const NON_CHAT_FALLBACK_MARKERS: &[&str] = &["text-embedding", "nomic-embed", "embedding", "embed"];

#[cfg(test)]
mod tests {
    use super::{is_fallback_candidate_model_id, select_fallback_model, ModelInfo};

    #[test]
    fn excludes_embedding_models_from_fallback() {
        assert!(!is_fallback_candidate_model_id("text-embedding-3-large"));
        assert!(!is_fallback_candidate_model_id("nomic-embed-text-v1.5"));
        assert!(!is_fallback_candidate_model_id("local-embedding-model"));
        assert!(!is_fallback_candidate_model_id("vendor/embed-small"));
    }

    #[test]
    fn accepts_normal_chat_model_ids_as_fallback_candidates() {
        assert!(is_fallback_candidate_model_id("qwen2.5-7b-instruct"));
        assert!(is_fallback_candidate_model_id("google/gemma-4-31b"));
        assert!(is_fallback_candidate_model_id("zai-org-glm-5-1"));
    }

    #[test]
    fn selects_first_fallback_candidate() {
        let models = vec![
            ModelInfo::new("text-embedding-nomic-embed-text-v1.5"),
            ModelInfo::new("qwen2.5-7b-instruct"),
            ModelInfo::new("google/gemma-4-31b"),
        ];

        let selected = select_fallback_model(&models).unwrap();

        assert_eq!(selected.id, "qwen2.5-7b-instruct");
    }

    #[test]
    fn returns_none_when_no_fallback_candidate_exists() {
        let models = vec![
            ModelInfo::new("text-embedding-3-large"),
            ModelInfo::new("nomic-embed-text-v1.5"),
        ];

        assert!(select_fallback_model(&models).is_none());
    }
}
