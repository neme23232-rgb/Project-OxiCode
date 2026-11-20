use super::{Message, Role, SseBuffer, StreamEvent, ToolCall, ToolDefinition, format_api_error};
use crate::config::Config;
use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

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
            "GEMINI_API_KEY is not set. Run /connect or set GEMINI_API_KEY"
        ));
    }

    let model_id = provider
        .model
        .strip_prefix("models/")
        .unwrap_or(&provider.model);
    let url = format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
        provider.base_url.trim_end_matches('/'),
        model_id
    );

    let mut contents: Vec<Value> = Vec::new();
    for msg in messages {
        match msg.role {
            Role::System => {}
            Role::User => {
                contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": msg.content }],
                }));
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if !msg.content.is_empty() {
                    parts.push(json!({ "text": msg.content }));
                }
                for tc in &msg.tool_calls {
                    let mut fc_part = json!({
                        "functionCall": {
                            "name": tc.name,
                            "args": tc.arguments,
                        }
                    });
                    if let Some(sig) = &tc.thought_signature {
                        fc_part["thoughtSignature"] = json!(sig);
                    }
                    parts.push(fc_part);
                }
                if parts.is_empty() {
                    parts.push(json!({ "text": " " }));
                }
                contents.push(json!({
                    "role": "model",
                    "parts": parts,
                }));
            }
            Role::Tool => {
                let part = json!({
                    "functionResponse": {
                        "name": msg.tool_name.as_deref().unwrap_or("tool"),
                        "response": { "result": msg.content },
                    }
                });
                if let Some(last) = contents.last_mut()
                    && last.get("role").and_then(Value::as_str) == Some("user")
                    && let Some(arr) = last.get_mut("parts").and_then(Value::as_array_mut)
                    && arr.iter().any(|p| p.get("functionResponse").is_some())
                {
                    arr.push(part);
                } else {
                    contents.push(json!({
                        "role": "user",
                        "parts": [part],
                    }));
                }
            }
        }
    }

    let mut body = json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": cfg.max_tokens,
        }
    });

    if !system.is_empty() {
        body["systemInstruction"] = json!({
            "parts": [{ "text": system }]
        });
    }

    if !tools.is_empty() {
        let decls: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": sanitize_gemini_schema(t.parameters.clone()),
                })
            })
            .collect();
        body["tools"] = json!([{ "functionDeclarations": decls }]);
    }

    let resp = client
        .post(&url)
        .header("x-goog-api-key", &provider.api_key)
        .json(&body)
        .send()
        .await
        .context("Failed to connect to Gemini API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{}", format_api_error("Gemini", status, &text)));
    }

    let mut stream = resp.bytes_stream();
    let mut sse = SseBuffer::new();
    let mut call_counter: usize = 0;

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("Error reading Gemini stream")?;
        for line in sse.push_bytes(&bytes) {
            handle_gemini_sse_line(&line, &tx, &mut call_counter);
        }
    }
    if let Some(tail) = sse.finish() {
        handle_gemini_sse_line(&tail, &tx, &mut call_counter);
    }

    let _ = tx.send(StreamEvent::Done);
    Ok(())
}

fn handle_gemini_sse_line(
    line: &str,
    tx: &mpsc::UnboundedSender<StreamEvent>,
    call_counter: &mut usize,
) {
    let Some(data) = line.strip_prefix("data:") else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(data.trim_start()) else {
        return;
    };

    if let Some(usage) = v.get("usageMetadata") {
        let in_tok = usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let out_tok = usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        if in_tok > 0 || out_tok > 0 {
            let _ = tx.send(StreamEvent::Usage {
                input_tokens: in_tok,
                output_tokens: out_tok,
            });
        }
    }

    let Some(parts) = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    else {
        return;
    };

    for part in parts {
        let is_thought = part.get("thought").and_then(Value::as_bool).unwrap_or(false);
        if let Some(text) = part.get("text").and_then(Value::as_str)
            && !text.is_empty()
        {
            if is_thought {
                let _ = tx.send(StreamEvent::ThinkingDelta(text.to_string()));
            } else {
                let _ = tx.send(StreamEvent::TextDelta(text.to_string()));
            }
        }
        if let Some(fc) = part.get("functionCall")
            && let Some(name) = fc.get("name").and_then(Value::as_str)
        {
            *call_counter += 1;
            let args = fc
                .get("args")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            let sig = part
                .get("thoughtSignature")
                .and_then(Value::as_str)
                .map(str::to_string);
            let _ = tx.send(StreamEvent::ToolCall(ToolCall {
                id: format!("gemini_call_{}", *call_counter),
                name: name.to_string(),
                arguments: args,
                thought_signature: sig,
            }));
        }
    }
}

/// Remove JSON Schema keywords rejected by Gemini's functionDeclarations validator.
fn sanitize_gemini_schema(mut v: Value) -> Value {
    match &mut v {
        Value::Object(map) => {
            for bad in ["$schema", "$id", "additionalProperties", "default"] {
                map.remove(bad);
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if let Some(child) = map.remove(&k) {
                    map.insert(k, sanitize_gemini_schema(child));
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                let cloned = item.clone();
                *item = sanitize_gemini_schema(cloned);
            }
        }
        _ => {}
    }
    v
}
