use super::sse::sse_events;
use super::{ChatMessage, Provider, StreamEvent};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::json;

pub struct AnthropicProvider;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

#[async_trait]
impl Provider for AnthropicProvider {
    async fn stream_chat(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<BoxStream<'static, StreamEvent>> {
        // Anthropic takes system prompt separately from the turn-taking messages.
        let system: String = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n\n");

        let turns: Vec<_> = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect();

        let mut body = json!({
            "model": model,
            "max_tokens": 4096,
            "stream": true,
            "messages": turns,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }

        let client = reqwest::Client::new();
        let resp = client
            .post(API_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("anthropic error {status}: {text}");
        }

        let byte_stream = resp.bytes_stream();
        let event_stream = sse_events(byte_stream).map(|data| parse_anthropic_event(&data));

        Ok(Box::pin(event_stream))
    }
}

fn parse_anthropic_event(data: &str) -> StreamEvent {
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return StreamEvent::Delta { text: String::new() },
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("content_block_delta") => {
            let text = v
                .get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            StreamEvent::Delta { text }
        }
        Some("message_stop") => StreamEvent::Done,
        Some("error") => {
            let message = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown anthropic error")
                .to_string();
            StreamEvent::Error { message }
        }
        _ => StreamEvent::Delta { text: String::new() },
    }
}
