use super::sse::sse_events;
use super::{ChatMessage, Provider, StreamEvent};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::json;

/// Any provider speaking the OpenAI chat-completions wire format (request body, bearer auth and
/// streamed `choices[0].delta` chunks). Used for OpenAI itself and for Mistral, whose API is
/// compatible — only the endpoint differs.
pub struct OpenAiProvider {
    /// Short name used in error messages, e.g. "openai" or "mistral".
    pub label: &'static str,
    pub api_url: &'static str,
}

pub const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";
pub const MISTRAL_API_URL: &str = "https://api.mistral.ai/v1/chat/completions";

#[async_trait]
impl Provider for OpenAiProvider {
    async fn stream_chat(
        &self,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<BoxStream<'static, StreamEvent>> {
        let turns: Vec<_> = messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect();

        let body = json!({
            "model": model,
            "stream": true,
            "messages": turns,
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(self.api_url)
            .bearer_auth(api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("{} error {status}: {text}", self.label);
        }

        let label = self.label;
        let byte_stream = resp.bytes_stream();
        let event_stream = sse_events(byte_stream).map(move |data| parse_openai_event(&data, label));

        Ok(Box::pin(event_stream))
    }
}

fn parse_openai_event(data: &str, label: &str) -> StreamEvent {
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return StreamEvent::Delta { text: String::new() },
    };

    if let Some(err) = v.get("error") {
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("unknown {label} error"));
        return StreamEvent::Error { message };
    }

    let choice = v.get("choices").and_then(|c| c.get(0));
    let finish_reason = choice.and_then(|c| c.get("finish_reason")).and_then(|f| f.as_str());
    if finish_reason.is_some_and(|r| !r.is_empty()) {
        return StreamEvent::Done;
    }

    let text = choice
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    StreamEvent::Delta { text }
}
