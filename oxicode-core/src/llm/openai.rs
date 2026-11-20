use super::{Message, Role, SseBuffer, StreamEvent, ToolCall, ToolDefinition, format_api_error};
use crate::config::Config;
use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

#[derive(Default)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

pub async fn stream_chat(
    client: &reqwest::Client,
    cfg: &Config,
    system: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    tx: mpsc::UnboundedSender<StreamEvent>,
) -> Result<()> {
    let spec = cfg.active_spec();
    let provider = cfg.active();
    if spec.requires_key && provider.api_key.is_empty() {
        return Err(anyhow!(
            "API key for {} is not set. Run /connect or set {}",
            spec.display_name,
            spec.env_var.unwrap_or("API key")
        ));
    }

    let base = crate::config::normalize_base_url(&provider.base_url);
    let url = format!("{base}/chat/completions");

    let resolved_model = if provider.model.contains(' ') {
        super::resolve_model_id(client, cfg, &provider.model)
            .await
            .unwrap_or_else(|| provider.model.clone())
    } else {
        provider.model.clone()
    };

    let mut api_messages = Vec::with_capacity(messages.len() + 1);
    if !system.is_empty() {
        api_messages.push(json!({
            "role": "system",
            "content": system,
        }));
    }

    for msg in messages {
        match msg.role {
            Role::System => {
                api_messages.push(json!({ "role": "system", "content": msg.content }));
            }
            Role::User => {
                api_messages.push(json!({ "role": "user", "content": msg.content }));
            }
            Role::Assistant => {
                let content_val = if msg.content.is_empty() && !msg.tool_calls.is_empty() {
                    Value::Null
                } else {
                    json!(msg.content)
                };
                let mut obj = json!({
                    "role": "assistant",
                    "content": content_val,
                });
                if !msg.tool_calls.is_empty() {
                    let tcs: Vec<Value> = msg
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    obj["tool_calls"] = json!(tcs);
                }
                api_messages.push(obj);
            }
            Role::Tool => {
                api_messages.push(json!({
                    "role": "tool",
                    "tool_call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "content": msg.content,
                }));
            }
        }
    }

    let mut body = json!({
        "model": resolved_model,
        "messages": api_messages,
        "max_tokens": cfg.max_tokens,
        "stream": true,
    });

    if !tools.is_empty() {
        let api_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = json!(api_tools);
    }

    let mut req = client.post(&url).json(&body);
    if !provider.api_key.is_empty() {
        req = req.bearer_auth(&provider.api_key);
    }
    if cfg.provider == "openrouter" {
        req = req
            .header("HTTP-Referer", "https://github.com/neme23232-rgb/oxicode")
            .header("X-Title", "oxicode");
    }

    let resp = req
        .send()
        .await
        .with_context(|| format!("Failed to connect to {} at {url}", spec.display_name))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{}", format_api_error(spec.display_name, status, &text)));
    }

    let mut stream = resp.bytes_stream();
    let mut sse = SseBuffer::new();
    let mut pending_tools: BTreeMap<usize, PendingToolCall> = BTreeMap::new();
    let mut done_seen = false;

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("Error reading OpenAI-compatible stream")?;
        for line in sse.push_bytes(&bytes) {
            if handle_openai_sse_line(&line, spec.display_name, &tx, &mut pending_tools)? {
                done_seen = true;
                break;
            }
        }
        if done_seen {
            break;
        }
    }

    if !done_seen && let Some(tail) = sse.finish() {
        let _ = handle_openai_sse_line(&tail, spec.display_name, &tx, &mut pending_tools)?;
    }

    for (idx, tc) in pending_tools {
        if tc.name.is_empty() {
            continue;
        }
        let arguments = if tc.arguments.trim().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            match serde_json::from_str::<Value>(&tc.arguments) {
                Ok(v) => v,
                Err(e) => json!({
                    "__oxicode_parse_error": e.to_string(),
                    "__oxicode_raw_args": tc.arguments,
                }),
            }
        };
        let _ = tx.send(StreamEvent::ToolCall(ToolCall {
            id: if tc.id.is_empty() {
                format!("call_{}_{idx}", tc.name)
            } else {
                tc.id
            },
            name: tc.name,
            arguments,
            thought_signature: None,
        }));
    }

    let _ = tx.send(StreamEvent::Done);
    Ok(())
}

fn handle_openai_sse_line(
    line: &str,
    provider_name: &str,
    tx: &mpsc::UnboundedSender<StreamEvent>,
    pending_tools: &mut BTreeMap<usize, PendingToolCall>,
) -> Result<bool> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(false);
    };
    let data = data.trim_start();
    if data == "[DONE]" {
        return Ok(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return Ok(false);
    };

    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_else(|| err.as_str().unwrap_or("Unknown stream error"));
        return Err(anyhow!("{provider_name} stream error: {msg}"));
    }

    if let Some(usage) = v.get("usage")
        && let (Some(in_tok), Some(out_tok)) = (
            usage.get("prompt_tokens").and_then(Value::as_u64),
            usage.get("completion_tokens").and_then(Value::as_u64),
        )
    {
        let _ = tx.send(StreamEvent::Usage {
            input_tokens: in_tok as u32,
            output_tokens: out_tok as u32,
        });
    }

    let Some(choices) = v.get("choices").and_then(Value::as_array) else {
        return Ok(false);
    };
    let Some(delta) = choices.first().and_then(|c| c.get("delta")) else {
        return Ok(false);
    };

    if let Some(reasoning) = delta
        .get("reasoning_content")
        .or_else(|| delta.get("reasoning"))
        .and_then(Value::as_str)
        && !reasoning.is_empty()
    {
        let _ = tx.send(StreamEvent::ThinkingDelta(reasoning.to_string()));
    }

    if let Some(text) = delta.get("content").and_then(Value::as_str)
        && !text.is_empty()
    {
        let _ = tx.send(StreamEvent::TextDelta(text.to_string()));
    }

    if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
        for tc in tcs {
            let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let entry = pending_tools.entry(idx).or_default();
            if let Some(id) = tc.get("id").and_then(Value::as_str)
                && !id.is_empty()
                && entry.id.is_empty()
            {
                entry.id = id.to_string();
            }
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(Value::as_str)
                    && !name.is_empty()
                    && entry.arguments.is_empty()
                    && entry.name != name
                {
                    entry.name.push_str(name);
                }
                if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                    entry.arguments.push_str(args);
                } else if let Some(args_obj) = func.get("arguments").filter(|a| a.is_object()) {
                    entry.arguments = args_obj.to_string();
                }
            }
        }
    }

    Ok(false)
}
