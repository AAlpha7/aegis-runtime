//! OpenAI-compatible chat-completion streaming.
//!
//! Owner Web turns opt in by setting `stream: true` on the same chat
//! completion request the blocking path already uses. This module
//! turns the SSE body into content deltas plus one completion JSON that the
//! existing parser can load. A dropped connection does not yield a completion,
//! so a partial reply is never treated as the persisted answer.
//!
//! Paths that answer with a single JSON body (the provider ignored `stream`)
//! produce a completion and zero deltas. Callers keep that as a blocking turn.

use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Debug)]
pub enum ChatStreamFail {
    /// The socket closed before `finish_reason` or `[DONE]`.
    Incomplete,
    Provider(String),
    Transport(String),
}

impl std::fmt::Display for ChatStreamFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incomplete => write!(f, "chat stream ended before the model finished"),
            Self::Provider(message) | Self::Transport(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ChatStreamFail {}

/// A finished read of one chat-completion HTTP response.
#[derive(Debug)]
pub enum ChatStreamRead {
    /// Full completion JSON. `deltas` were already delivered via the callback
    /// when this was an SSE body; a buffered JSON body emits none.
    Completed(Value),
    /// HTTP 400/422 naming the `stream` flag. No deltas were emitted.
    Rejected { status: u16, body: String },
}

/// Whether an error body is the provider refusing `stream: true` before any
/// tokens. Mid-stream failures are not this case.
pub fn provider_rejected_stream(status: u16, body: &str) -> bool {
    if status != 400 && status != 422 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    if !lower.contains("stream") {
        return false;
    }
    lower.contains("not supported")
        || lower.contains("unsupported")
        || lower.contains("unknown")
        || lower.contains("unrecognized")
        || lower.contains("unexpected")
        || lower.contains("invalid")
        || lower.contains("extra")
}

pub async fn read_openai_chat_response(
    response: reqwest::Response,
    mut on_content_delta: impl FnMut(&str),
) -> Result<ChatStreamRead, ChatStreamFail> {
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let preview = truncate(&body, 400);
        if provider_rejected_stream(status.as_u16(), &body) {
            return Ok(ChatStreamRead::Rejected {
                status: status.as_u16(),
                body: preview,
            });
        }
        return Err(ChatStreamFail::Provider(format!(
            "chat completion returned {}: {preview}",
            status.as_u16()
        )));
    }

    let mut mode = if content_type.contains("text/event-stream") {
        BodyMode::Sse
    } else {
        BodyMode::Unknown
    };
    let mut response = response;
    let mut buf: Vec<u8> = Vec::new();
    let mut assembler = Assembler::default();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ChatStreamFail::Transport(error.to_string()))?
    {
        buf.extend_from_slice(&chunk);
        if mode == BodyMode::Unknown
            && let Some(first) = buf.iter().copied().find(|byte| !byte.is_ascii_whitespace())
        {
            mode = if first == b'{' {
                BodyMode::Json
            } else {
                BodyMode::Sse
            };
        }
        if mode == BodyMode::Sse {
            drain_sse_lines(&mut buf, &mut assembler, &mut on_content_delta)?;
        }
    }

    match mode {
        BodyMode::Json | BodyMode::Unknown => parse_buffered_json(&buf),
        BodyMode::Sse => {
            if !buf.is_empty() {
                let line = std::mem::take(&mut buf);
                feed_sse_line(&line, &mut assembler, &mut on_content_delta)?;
            }
            assembler.finish().map(ChatStreamRead::Completed)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyMode {
    Unknown,
    Json,
    Sse,
}

fn parse_buffered_json(buf: &[u8]) -> Result<ChatStreamRead, ChatStreamFail> {
    let text = String::from_utf8_lossy(buf);
    let value: Value = serde_json::from_str(text.trim()).map_err(|error| {
        ChatStreamFail::Provider(format!("chat completion was not JSON or SSE: {error}"))
    })?;
    if value.get("error").is_some() && value.get("choices").is_none() {
        return Err(ChatStreamFail::Provider(format!(
            "chat completion error: {}",
            truncate(&value.to_string(), 400)
        )));
    }
    Ok(ChatStreamRead::Completed(value))
}

fn drain_sse_lines(
    buf: &mut Vec<u8>,
    assembler: &mut Assembler,
    on_content_delta: &mut impl FnMut(&str),
) -> Result<(), ChatStreamFail> {
    while let Some(pos) = buf.iter().position(|byte| *byte == b'\n') {
        let mut line: Vec<u8> = buf.drain(..=pos).collect();
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        feed_sse_line(&line, assembler, on_content_delta)?;
    }
    Ok(())
}

fn feed_sse_line(
    line: &[u8],
    assembler: &mut Assembler,
    on_content_delta: &mut impl FnMut(&str),
) -> Result<(), ChatStreamFail> {
    let Some(payload) = sse_data_payload(line) else {
        return Ok(());
    };
    if payload == b"[DONE]" {
        assembler.saw_done = true;
        return Ok(());
    }
    let text = std::str::from_utf8(payload).map_err(|error| {
        ChatStreamFail::Provider(format!("chat stream chunk was not UTF-8: {error}"))
    })?;
    let value: Value = serde_json::from_str(text).map_err(|error| {
        ChatStreamFail::Provider(format!("chat stream chunk was not JSON: {error}"))
    })?;
    if value.get("error").is_some() && value.get("choices").is_none() {
        return Err(ChatStreamFail::Provider(format!(
            "chat stream error: {}",
            truncate(&value.to_string(), 400)
        )));
    }
    assembler.push_chunk(&value, on_content_delta);
    Ok(())
}

fn sse_data_payload(line: &[u8]) -> Option<&[u8]> {
    let line = trim_ascii(line);
    let rest = line.strip_prefix(b"data:")?;
    let rest = trim_ascii(rest);
    if rest.is_empty() { None } else { Some(rest) }
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);
    &bytes[start..end]
}

#[derive(Default)]
struct Assembler {
    id: Option<String>,
    model: Option<String>,
    created: Option<u64>,
    content: String,
    tool_calls: BTreeMap<u32, ToolAcc>,
    finish_reason: Option<String>,
    usage: Option<Value>,
    saw_done: bool,
    saw_chunk: bool,
}

struct ToolAcc {
    id: Option<String>,
    name: String,
    arguments: String,
}

impl Assembler {
    fn push_chunk(&mut self, value: &Value, on_content_delta: &mut impl FnMut(&str)) {
        self.saw_chunk = true;
        if self.id.is_none() {
            self.id = value.get("id").and_then(Value::as_str).map(str::to_string);
        }
        if self.model.is_none() {
            self.model = value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if self.created.is_none() {
            self.created = value.get("created").and_then(Value::as_u64);
        }
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(usage.clone());
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };
        if let Some(reason) = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty())
        {
            self.finish_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            return;
        };
        // Reasoning traces are not the owner-visible answer and are not what
        // L1 stores. Only `delta.content` is forwarded.
        if let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            self.content.push_str(content);
            on_content_delta(content);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                let slot = self.tool_calls.entry(index).or_insert_with(|| ToolAcc {
                    id: None,
                    name: String::new(),
                    arguments: String::new(),
                });
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    slot.id = Some(id.to_string());
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        slot.name.push_str(name);
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        slot.arguments.push_str(arguments);
                    }
                }
            }
        }
    }

    fn finish(self) -> Result<Value, ChatStreamFail> {
        if !self.saw_done && self.finish_reason.is_none() {
            return Err(ChatStreamFail::Incomplete);
        }
        if !self.saw_chunk && !self.saw_done {
            return Err(ChatStreamFail::Incomplete);
        }
        Ok(self.completion_json())
    }

    fn completion_json(&self) -> Value {
        let tool_calls: Vec<Value> = self
            .tool_calls
            .iter()
            .map(|(index, call)| {
                json!({
                    "id": call.id.clone().unwrap_or_else(|| format!("call-{index}")),
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": call.arguments,
                    }
                })
            })
            .collect();
        let content = if self.content.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            Value::String(self.content.clone())
        };
        let mut message = json!({
            "role": "assistant",
            "content": content,
        });
        if !tool_calls.is_empty() {
            message["tool_calls"] = json!(tool_calls);
        }
        let finish = self.finish_reason.clone().unwrap_or_else(|| {
            if tool_calls.is_empty() {
                "stop".to_string()
            } else {
                "tool_calls".to_string()
            }
        });
        let mut body = json!({
            "id": self.id.clone().unwrap_or_else(|| "chatcmpl-stream".to_string()),
            "object": "chat.completion",
            "created": self.created.unwrap_or(0),
            "model": self.model.clone().unwrap_or_default(),
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish,
            }],
        });
        if let Some(usage) = &self.usage {
            body["usage"] = usage.clone();
        }
        body
    }
}

fn truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_openai::types::chat::CreateChatCompletionResponse;

    fn feed(body: &str) -> (Vec<String>, Result<ChatStreamRead, ChatStreamFail>) {
        let mut deltas = Vec::new();
        let mut assembler = Assembler::default();
        let mut error = None;
        for line in body.split('\n') {
            let bytes = line.as_bytes();
            if let Err(fail) = feed_sse_line(bytes, &mut assembler, &mut |delta| {
                deltas.push(delta.to_string());
            }) {
                error = Some(fail);
                break;
            }
        }
        let read = if let Some(fail) = error {
            Err(fail)
        } else {
            assembler.finish().map(ChatStreamRead::Completed)
        };
        (deltas, read)
    }

    #[test]
    fn sse_content_deltas_rebuild_one_completion() {
        let body = concat!(
            "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"早\"}}]}\n",
            "\n",
            "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"hidden\"}}]}\n",
            "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"安\"},\"finish_reason\":\"stop\"}]}\n",
            "data: [DONE]\n",
        );
        let (deltas, read) = feed(body);
        assert_eq!(deltas, vec!["早".to_string(), "安".to_string()]);
        let ChatStreamRead::Completed(value) = read.unwrap() else {
            panic!("expected completion");
        };
        let parsed: CreateChatCompletionResponse = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.choices[0].message.content.as_deref(), Some("早安"));
        assert!(parsed.choices[0].message.tool_calls.is_none());
    }

    #[test]
    fn truncated_stream_is_not_a_completion() {
        let body = "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"只写了一半\"}}]}\n";
        let (deltas, read) = feed(body);
        assert_eq!(deltas, vec!["只写了一半".to_string()]);
        assert!(matches!(read, Err(ChatStreamFail::Incomplete)));
    }

    #[test]
    fn tool_call_arguments_are_not_owner_text() {
        let body = concat!(
            "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"我先查一下。\"}}]}\n",
            "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"web_search\",\"arguments\":\"{\\\"q\\\":\"}}]}}]}\n",
            "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"天气\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n",
            "data: [DONE]\n",
        );
        let (deltas, read) = feed(body);
        assert_eq!(deltas, vec!["我先查一下。".to_string()]);
        let ChatStreamRead::Completed(value) = read.unwrap() else {
            panic!("expected completion");
        };
        let parsed: CreateChatCompletionResponse = serde_json::from_value(value).unwrap();
        let calls = parsed.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        let async_openai::types::chat::ChatCompletionMessageToolCalls::Function(call) = &calls[0]
        else {
            panic!("expected a function tool call");
        };
        assert_eq!(call.function.name, "web_search");
        assert_eq!(call.function.arguments, "{\"q\":\"天气\"}");
    }

    #[test]
    fn provider_error_frame_fails_the_read() {
        let body = "data: {\"error\":{\"message\":\"overloaded\"}}\n";
        let (_deltas, read) = feed(body);
        assert!(matches!(read, Err(ChatStreamFail::Provider(_))));
    }

    #[test]
    fn stream_flag_rejection_is_narrow() {
        assert!(provider_rejected_stream(
            400,
            "{\"error\":{\"message\":\"unknown parameter: stream\"}}"
        ));
        assert!(!provider_rejected_stream(500, "stream timeout"));
        assert!(!provider_rejected_stream(400, "model grok-test not found"));
    }

    #[tokio::test]
    async fn http_sse_yields_deltas_before_the_body_ends() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut incoming = [0u8; 1024];
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let _ = sock.read(&mut incoming).await;
            let body = concat!(
                "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"早\"}}]}\n\n",
                "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"安\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            );
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(header.as_bytes()).await.unwrap();
            sock.write_all(body.as_bytes()).await.unwrap();
        });
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/stream"))
            .send()
            .await
            .unwrap();
        let mut deltas = Vec::new();
        let read = read_openai_chat_response(response, |delta| deltas.push(delta.to_string()))
            .await
            .unwrap();
        assert_eq!(deltas, vec!["早".to_string(), "安".to_string()]);
        let ChatStreamRead::Completed(value) = read else {
            panic!("expected completion");
        };
        let parsed: CreateChatCompletionResponse = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.choices[0].message.content.as_deref(), Some("早安"));
    }

    #[tokio::test]
    async fn http_json_body_emits_no_deltas() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut incoming = [0u8; 1024];
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let _ = sock.read(&mut incoming).await;
            let body = "{\"id\":\"c1\",\"object\":\"chat.completion\",\"created\":1,\"model\":\"grok-test\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"整段\"},\"finish_reason\":\"stop\"}]}";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(header.as_bytes()).await.unwrap();
            sock.write_all(body.as_bytes()).await.unwrap();
        });
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap();
        let mut deltas = Vec::new();
        let read = read_openai_chat_response(response, |delta| deltas.push(delta.to_string()))
            .await
            .unwrap();
        assert!(deltas.is_empty());
        let ChatStreamRead::Completed(value) = read else {
            panic!("expected completion");
        };
        assert_eq!(value["choices"][0]["message"]["content"], "整段");
    }
}
