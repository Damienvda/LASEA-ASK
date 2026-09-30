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
    /// The model's reasoning, streamed as it comes (Claude's summarized thinking, Magistral's
    /// thinking chunks). Shown folded in the UI; never part of the answer text.
    Thinking { text: String },
    /// Emitted by the tool-use loops (see agent.rs) right before an MCP tool call, so the UI can
    /// show it in its timeline while the call is in flight. `id` pairs it with its `ToolEnd`.
    ToolStart { id: String, name: String, args: serde_json::Value },
    /// The call finished: whether it succeeded, how long it took, the size of the result fed to
    /// the model and its first characters.
    ToolEnd { id: String, ok: bool, duration_ms: u64, chars: usize, excerpt: String },
    /// Token usage of one model request. The UI adds up every event of a reply, so a provider may
    /// send the input and output counts in separate events.
    Usage { input_tokens: u64, output_tokens: u64, cache_read: u64, cache_write: u64 },
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
