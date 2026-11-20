use super::{Message, Role, SseBuffer, StreamEvent, ToolCall, ToolDefinition, format_api_error};
use crate::config::Config;
use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

#[derive(Default)]
struct PendingBlock {
    id: String,
    name: String,
    input_json: String,
}

pub async fn stream_chat(
    client: &reqwest::Client,
    cfg: &Config,
    system: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    tx: mpsc::UnboundedSender<StreamEvent>,
) -> Result<()> {
    let provider = cfg.active();
    if provider.api_key.is_empty() {
        return Err(anyhow!(
            "ANTHROPIC_API_KEY is not set. Run /connect or set ANTHROPIC_API_KEY"
        ));
    }

    let url = format!("{}/v1/messages", provider.base_url.trim_end_matches('/'));
    let mut api_messages: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {}
            Role::User => {
                api_messages.push(json!({
                    "role": "user",
                    "content": msg.content,
                }));
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !msg.content.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": msg.content,
                    }));
                }
                for tc in &msg.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": " " }));
                }
                api_messages.push(json!({
                    "role": "assistant",
                    "content": blocks,
                }));
            }
            Role::Tool => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "content": msg.content,
                });
                if let Some(last) = api_messages.last_mut()
                    && last.get("role").and_then(Value::as_str) == Some("user")
                    && let Some(arr) = last.get_mut("content").and_then(Value::as_array_mut)
                {
                    arr.push(block);
                } else {
                    api_messages.push(json!({
                        "role": "user",
                        "content": [block],
                    }));
                }
            }
        }
    }

    let mut body = json!({
        "model": provider.model,
        "max_tokens": cfg.max_tokens,
        "messages": api_messages,
        "stream": true,
    });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        let api_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        body["tools"] = json!(api_tools);
    }

    let resp = client
        .post(&url)
        .header("x-api-key", &provider.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .with_context(|| format!("Failed to connect to Anthropic API at {url}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{}", format_api_error("Anthropic", status, &text)));
    }

    let mut stream = resp.bytes_stream();
    let mut sse = SseBuffer::new();
    let mut pending_tools: BTreeMap<usize, PendingBlock> = BTreeMap::new();
    let mut input_tokens: u32 = 0;

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("Error reading Anthropic stream")?;
        for line in sse.push_bytes(&bytes) {
            handle_anthropic_sse_line(&line, &tx, &mut pending_tools, &mut input_tokens)?;
        }
    }
    if let Some(tail) = sse.finish() {
        handle_anthropic_sse_line(&tail, &tx, &mut pending_tools, &mut input_tokens)?;
    }

    let _ = tx.send(StreamEvent::Done);
    Ok(())
}

fn handle_anthropic_sse_line(
    line: &str,
    tx: &mpsc::UnboundedSender<StreamEvent>,
    pending_tools: &mut BTreeMap<usize, PendingBlock>,
    input_tokens: &mut u32,
) -> Result<()> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(());
    };
    let Ok(v) = serde_json::from_str::<Value>(data.trim_start()) else {
        return Ok(());
    };
    let event_type = v.get("type").and_then(Value::as_str).unwrap_or("");

    match event_type {
        "error" => {
            let msg = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Anthropic stream error");
            return Err(anyhow!("Anthropic error: {msg}"));
        }
        "message_start" => {
            if let Some(in_tok) = v
                .pointer("/message/usage/input_tokens")
                .and_then(Value::as_u64)
            {
                *input_tokens = in_tok as u32;
            }
        }
        "content_block_start" => {
            let idx = v.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(cb) = v.get("content_block")
                && cb.get("type").and_then(Value::as_str) == Some("tool_use")
            {
                pending_tools.insert(
                    idx,
                    PendingBlock {
                        id: cb
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: cb
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        input_json: String::new(),
                    },
                );
            }
        }
        "content_block_delta" => {
            let idx = v.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(delta) = v.get("delta") else {
                return Ok(());
            };
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        let _ = tx.send(StreamEvent::TextDelta(text.to_string()));
                    }
                }
                Some("thinking_delta") => {
                    if let Some(thinking) = delta.get("thinking").and_then(Value::as_str) {
                        let _ = tx.send(StreamEvent::ThinkingDelta(thinking.to_string()));
                    }
                }
                Some("input_json_delta") => {
                    if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                        && let Some(entry) = pending_tools.get_mut(&idx)
                    {
                        entry.input_json.push_str(partial);
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let idx = v.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(block) = pending_tools.remove(&idx) {
                let arguments = if block.input_json.trim().is_empty() {
                    Value::Object(serde_json::Map::new())
                } else {
                    match serde_json::from_str::<Value>(&block.input_json) {
                        Ok(v) => v,
                        Err(e) => json!({
                            "__oxicode_parse_error": e.to_string(),
                            "__oxicode_raw_args": block.input_json,
                        }),
                    }
                };
                let _ = tx.send(StreamEvent::ToolCall(ToolCall {
                    id: block.id,
                    name: block.name,
                    arguments,
                    thought_signature: None,
                }));
            }
        }
        "message_delta" => {
            if let Some(out_tok) = v.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                let _ = tx.send(StreamEvent::Usage {
                    input_tokens: *input_tokens,
                    output_tokens: out_tok as u32,
                });
            }
        }
        _ => {}
    }
    Ok(())
}
