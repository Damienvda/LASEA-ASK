use futures::stream::BoxStream;
use futures::StreamExt;

/// Turns a raw SSE byte stream into a stream of `data:` payload strings,
/// one per event, skipping keep-alives, comments and the `[DONE]` sentinel.
/// Shared by every provider since they all speak text/event-stream.
pub fn sse_events(
    byte_stream: impl futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
) -> BoxStream<'static, String> {
    let stream = futures::stream::unfold(
        (byte_stream.boxed(), String::new()),
        |(mut stream, mut buf)| async move {
            loop {
                if let Some(pos) = buf.find("\n\n") {
                    let event = buf[..pos].to_string();
                    buf.drain(..pos + 2);
                    let data: String = event
                        .lines()
                        .filter_map(|l| l.strip_prefix("data: ").or_else(|| l.strip_prefix("data:")))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    return Some((data, (stream, buf)));
                }
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        // SSE allows CRLF line endings (e.g. the Python MCP SDK sends "\r\n\r\n"
                        // between events). Normalize to LF so the "\n\n" split above works; JSON
                        // payloads never contain a raw CR, and stripping every CR also handles a
                        // "\r\n" pair split across two chunks.
                        buf.push_str(&String::from_utf8_lossy(&chunk).replace('\r', ""));
                        continue;
                    }
                    Some(Err(_)) | None => return None,
                }
            }
        },
    );
    stream.boxed()
}
