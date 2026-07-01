use origin_protocol::chat::{ChatRequest, ChatResponse};

use crate::openai_compatible::types::{
    ChatCompletionsMessage, ChatCompletionsRequest, ChatCompletionsResponse,
};
use crate::{ProviderError, ProviderResult};

pub fn serialize_chat_request(provider: &str, request: &ChatRequest) -> ProviderResult<String> {
    let body = ChatCompletionsRequest {
        model: &request.model,
        messages: request
            .messages
            .iter()
            .map(|message| ChatCompletionsMessage {
                role: &message.role,
                content: &message.content,
            })
            .collect(),
        temperature: 0.7,
        stream: false,
        max_tokens: request.max_tokens,
    };

    serde_json::to_string(&body).map_err(|error| ProviderError::InvalidResponse {
        provider: provider.to_string(),
        message: error.to_string(),
        source: Some(Box::new(error)),
    })
}

pub fn parse_chat_response(provider: &str, body: &str) -> ProviderResult<ChatResponse> {
    let response: ChatCompletionsResponse =
        serde_json::from_str(body).map_err(|error| ProviderError::InvalidResponse {
            provider: provider.to_string(),
            message: error.to_string(),
            source: Some(Box::new(error)),
        })?;

    for choice in response.choices {
        if let Some(content) = choice.message.content {
            return parse_assistant_content(
                provider,
                content,
                choice.message.reasoning_content,
                choice.finish_reason.as_deref(),
            );
        }
    }

    Err(ProviderError::InvalidResponse {
        provider: provider.to_string(),
        message: "chat completions response did not include assistant content".to_string(),
        source: None,
    })
}

fn parse_assistant_content(
    provider: &str,
    content: String,
    reasoning_content: Option<String>,
    finish_reason: Option<&str>,
) -> ProviderResult<ChatResponse> {
    if !content.is_empty() {
        return Ok(ChatResponse::new(content));
    }

    let has_reasoning_content = reasoning_content
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if has_reasoning_content {
        let mut message = "chat completions response returned reasoning content but no assistant content. Reasoning text was not displayed or saved.".to_string();
        if finish_reason == Some("length") {
            message.push_str(
                " finish_reason=length suggests the model may have exhausted output tokens before producing final content.",
            );
        }
        message.push_str(
            " Try disabling thinking/reasoning mode, increasing output/token budget, or using a non-thinking/chat-tuned model.",
        );
        return Err(ProviderError::InvalidResponse {
            provider: provider.to_string(),
            message,
            source: None,
        });
    }

    Err(ProviderError::InvalidResponse {
        provider: provider.to_string(),
        message: "chat completions response included empty assistant content".to_string(),
        source: None,
    })
}

#[cfg(test)]
mod tests {
    use origin_protocol::chat::{ChatMessage, ChatRequest};

    use super::{parse_chat_response, serialize_chat_request};

    #[test]
    fn parses_lmstudio_chat_completions_fixture() {
        let fixture = include_str!("../../fixtures/lmstudio-chat-completions.json");
        let response = parse_chat_response("lmstudio", fixture).expect("fixture should parse");

        assert_eq!(response.content, "Hello from LM Studio.");
    }

    #[test]
    fn parses_openai_response_with_provider_specific_fields() {
        let fixture = r#"{
          "id": "chatcmpl-test",
          "object": "chat.completion",
          "model": "zai-org-glm-5-1",
          "vendor_parameters": {"include_vendor_system_prompt": false},
          "choices": [
            {
              "index": 0,
              "message": {
                "role": "assistant",
                "content": "Hello from the provider.",
                "reasoning_content": "internal reasoning should be ignored",
                "tool_calls": []
              },
              "finish_reason": "stop"
            }
          ]
        }"#;
        let response = parse_chat_response("openai", fixture).expect("fixture should parse");

        assert_eq!(response.content, "Hello from the provider.");
        assert!(!response.content.contains("internal reasoning"));
    }

    #[test]
    fn rejects_reasoning_only_response_without_exposing_reasoning_text() {
        let fixture = r#"{
          "id": "chatcmpl-test",
          "object": "chat.completion",
          "model": "qwen3.6-27b-obliterated",
          "choices": [
            {
              "index": 0,
              "message": {
                "role": "assistant",
                "content": "",
                "reasoning_content": "private hidden chain of thought"
              },
              "finish_reason": "length"
            }
          ]
        }"#;
        let error = parse_chat_response("lmstudio", fixture).unwrap_err();
        let message = error.user_message();

        assert!(message.contains("reasoning content but no assistant content"));
        assert!(message.contains("Reasoning text was not displayed or saved"));
        assert!(message.contains("finish_reason=length"));
        assert!(message.contains("exhausted output tokens"));
        assert!(message.contains("disabling thinking/reasoning mode"));
        assert!(!message.contains("private hidden chain of thought"));
    }

    #[test]
    fn rejects_empty_assistant_content_without_reasoning_content() {
        let fixture = r#"{
          "id": "chatcmpl-test",
          "object": "chat.completion",
          "model": "qwen3.6-27b-obliterated",
          "choices": [
            {
              "index": 0,
              "message": {
                "role": "assistant",
                "content": ""
              },
              "finish_reason": "stop"
            }
          ]
        }"#;
        let error = parse_chat_response("lmstudio", fixture).unwrap_err();
        let message = error.user_message();

        assert!(message.contains("empty assistant content"));
        assert!(!message.contains("reasoning content but no assistant content"));
    }

    #[test]
    fn rejects_missing_assistant_content_with_existing_message() {
        let fixture = r#"{
          "id": "chatcmpl-test",
          "object": "chat.completion",
          "model": "qwen3.6-27b-obliterated",
          "choices": [
            {
              "index": 0,
              "message": {
                "role": "assistant"
              },
              "finish_reason": "stop"
            }
          ]
        }"#;
        let error = parse_chat_response("lmstudio", fixture).unwrap_err();
        let message = error.user_message();

        assert!(message.contains("did not include assistant content"));
    }

    #[test]
    fn serializes_non_streaming_chat_request() {
        let request = ChatRequest {
            model: "qwen2.5-coder".to_string(),
            messages: vec![ChatMessage::user("hello")],
            max_tokens: None,
        };
        let body = serialize_chat_request("lmstudio", &request).expect("request should serialize");

        assert!(body.contains("\"model\":\"qwen2.5-coder\""));
        assert!(body.contains("\"role\":\"user\""));
        assert!(body.contains("\"content\":\"hello\""));
        assert!(body.contains("\"temperature\":0.7"));
        assert!(body.contains("\"stream\":false"));
        assert!(!body.contains("\"max_tokens\""));
    }

    #[test]
    fn serializes_max_tokens_when_set() {
        let request = ChatRequest {
            model: "qwen2.5-coder".to_string(),
            messages: vec![ChatMessage::user("hello")],
            max_tokens: Some(4096),
        };
        let body = serialize_chat_request("lmstudio", &request).expect("request should serialize");

        assert!(body.contains("\"max_tokens\":4096"));
    }
}
