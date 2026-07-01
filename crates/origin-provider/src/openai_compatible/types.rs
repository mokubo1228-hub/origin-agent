use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ChatCompletionsRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<ChatCompletionsMessage<'a>>,
    pub temperature: f32,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionsMessage<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionsResponse {
    pub choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: ChatChoiceMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoiceMessage {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<ModelObject>,
}

#[derive(Debug, Deserialize)]
pub struct ModelObject {
    pub id: String,
}
