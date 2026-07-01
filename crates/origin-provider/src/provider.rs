use origin_protocol::chat::{ChatRequest, ChatResponse};
use origin_protocol::model::ModelInfo;
use origin_protocol::provider::{ProviderId, ProviderKind};

use crate::ProviderResult;

pub trait ModelProvider {
    fn id(&self) -> &ProviderId;
    fn kind(&self) -> ProviderKind;
    fn base_url(&self) -> &str;
    fn list_models(&self) -> ProviderResult<Vec<ModelInfo>>;
    fn complete_chat(&self, request: ChatRequest) -> ProviderResult<ChatResponse>;
}
