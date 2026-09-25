pub mod anthropic;
pub mod openai;
pub mod sse;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
}

/// A single normalized streaming event, provider-agnostic, sent on to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Delta { text: String },
    /// Emitted by the tool-use agent loop (see agent.rs) right before it calls an MCP tool, so
    /// the UI can show a "using tool X" indicator while the call is in flight.
    ToolCall { name: String },
    Done,
    Error { message: String },
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stream a chat completion. Each item is a normalized StreamEvent.
    async fn stream_chat(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<BoxStream<'static, StreamEvent>>;
}

pub fn provider_by_name(name: &str) -> Option<Box<dyn Provider>> {
    match name {
        "anthropic" => Some(Box::new(anthropic::AnthropicProvider)),
        "openai" => Some(Box::new(openai::OpenAiProvider {
            label: "openai",
            api_url: openai::OPENAI_API_URL,
        })),
        // Mistral's chat API is OpenAI-compatible, so it reuses the same provider.
        "mistral" => Some(Box::new(openai::OpenAiProvider {
            label: "mistral",
            api_url: openai::MISTRAL_API_URL,
        })),
        _ => None,
    }
}
