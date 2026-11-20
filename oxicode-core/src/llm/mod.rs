pub mod anthropic;
pub mod gemini;
pub mod openai;

use crate::config::{ApiProtocol, Config};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    ThinkingDelta(String),
    TextDelta(String),
    ToolCall(ToolCall),
    Usage { input_tokens: u32, output_tokens: u32 },
    Done,
}

pub async fn stream_chat(
    client: &reqwest::Client,
    cfg: &Config,
    system: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    tx: mpsc::UnboundedSender<StreamEvent>,
) -> Result<()> {
    const MAX_RETRIES: usize = 2;
    let mut attempt = 0usize;

    loop {
        let (attempt_tx, mut attempt_rx) = mpsc::unbounded_channel::<StreamEvent>();
        let tx_fwd = tx.clone();

        let fwd = tokio::spawn(async move {
            let mut forwarded = false;
            while let Some(ev) = attempt_rx.recv().await {
                forwarded = true;
                if tx_fwd.send(ev).is_err() {
                    break;
                }
            }
            forwarded
        });

        let res = match cfg.active_spec().protocol {
            ApiProtocol::OpenAICompatible => {
                openai::stream_chat(client, cfg, system, messages, tools, attempt_tx).await
            }
            ApiProtocol::Anthropic => {
                anthropic::stream_chat(client, cfg, system, messages, tools, attempt_tx).await
            }
            ApiProtocol::Gemini => {
                gemini::stream_chat(client, cfg, system, messages, tools, attempt_tx).await
            }
        };

        let forwarded = fwd.await.unwrap_or(false);
        match res {
            Ok(()) => return Ok(()),
            Err(err) => {
                if attempt < MAX_RETRIES && !forwarded && let Some(delay) = extract_retry_delay(&err, attempt) {
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(err);
            }
        }
    }
}

pub fn format_api_error(provider: &str, status: reqwest::StatusCode, body: &str) -> String {
    let trimmed = body.trim();
    let parsed_msg = serde_json::from_str::<Value>(trimmed).ok().and_then(|v| {
        let err_obj = v.get("error").or_else(|| v.as_array().and_then(|a| a.first()).and_then(|x| x.get("error")));
        if let Some(obj) = err_obj {
            let msg = obj
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| obj.as_str())
                .unwrap_or(trimmed);
            let retry_s = obj.get("retry_after_seconds").and_then(Value::as_u64);
            Some(match retry_s {
                Some(s) => format!("{msg} [retry_after={s}s]"),
                None => msg.to_string(),
            })
        } else {
            v.get("message").and_then(Value::as_str).map(str::to_string)
        }
    });

    let detail = parsed_msg.unwrap_or_else(|| {
        let clipped: String = trimmed.chars().take(280).collect();
        if clipped.is_empty() {
            status.canonical_reason().unwrap_or("Unknown error").to_string()
        } else {
            clipped
        }
    });
    format!("{provider} API error ({}): {detail}", status.as_u16())
}

fn extract_retry_delay(err: &anyhow::Error, attempt: usize) -> Option<Duration> {
    let msg = err.to_string();
    let is_rate_or_server = msg.contains("(429)")
        || msg.contains("(502)")
        || msg.contains("(503)")
        || msg.contains("(504)")
        || msg.contains("(529)")
        || msg.contains("Failed to connect");
    if !is_rate_or_server {
        return None;
    }
    if let Some(idx) = msg.find("[retry_after=")
        && let Some(end) = msg[idx + 13..].find("s]")
        && let Ok(secs) = msg[idx + 13..idx + 13 + end].parse::<u64>()
    {
        if secs <= 12 {
            return Some(Duration::from_secs(secs.max(1)));
        }
        return None;
    }
    let backoff = 2u64 << attempt; // 2s, 4s
    Some(Duration::from_secs(backoff))
}

pub async fn resolve_model_id(
    client: &reqwest::Client,
    cfg: &Config,
    input: &str,
) -> Option<String> {
    let spec = cfg.active_spec();
    if spec.protocol != ApiProtocol::OpenAICompatible {
        return None;
    }
    let provider = cfg.active();
    let base = crate::config::normalize_base_url(&provider.base_url);
    let url = format!("{base}/models");
    let mut req = client.get(&url);
    if !provider.api_key.is_empty() {
        req = req.bearer_auth(&provider.api_key);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: Value = resp.json().await.ok()?;
    let arr = json.get("data").and_then(Value::as_array)?;
    let needle = input.trim().to_lowercase();
    // Exact ID match first, then display name match, then suffix/substring match
    for m in arr {
        if let Some(id) = m.get("id").and_then(Value::as_str)
            && id.to_lowercase() == needle
        {
            return Some(id.to_string());
        }
    }
    for m in arr {
        if let Some(name) = m.get("name").and_then(Value::as_str)
            && name.to_lowercase() == needle
            && let Some(id) = m.get("id").and_then(Value::as_str)
        {
            return Some(id.to_string());
        }
    }
    let slug = needle.replace(' ', "-");
    for m in arr {
        if let Some(id) = m.get("id").and_then(Value::as_str)
            && (id.to_lowercase().ends_with(&slug) || id.to_lowercase().contains(&slug))
        {
            return Some(id.to_string());
        }
    }
    None
}

pub async fn fetch_models(client: &reqwest::Client, cfg: &Config) -> Result<Vec<String>> {
    let spec = cfg.active_spec();
    let provider = cfg.active();
    let base = crate::config::normalize_base_url(&provider.base_url);

    match spec.protocol {
        ApiProtocol::OpenAICompatible => {
            let url = format!("{base}/models");
            let mut req = client.get(&url);
            if !provider.api_key.is_empty() {
                req = req.bearer_auth(&provider.api_key);
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("Failed to fetch models from {url}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow!("Models endpoint error ({status}): {body}"));
            }
            let json: Value = resp.json().await.context("Invalid models JSON")?;
            let mut models: Vec<String> = json
                .get("data")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            models.sort();
            models.dedup();
            Ok(models)
        }
        ApiProtocol::Gemini => {
            if provider.api_key.is_empty() {
                return Err(anyhow!("GEMINI_API_KEY is not set"));
            }
            let url = format!("{base}/v1beta/models");
            let resp = client
                .get(&url)
                .header("x-goog-api-key", &provider.api_key)
                .send()
                .await
                .with_context(|| format!("Failed to fetch Gemini models from {url}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow!("Gemini models error ({status}): {body}"));
            }
            let json: Value = resp.json().await.context("Invalid Gemini models JSON")?;
            let mut models: Vec<String> = json
                .get("models")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            let name = m.get("name").and_then(Value::as_str)?;
                            Some(name.strip_prefix("models/").unwrap_or(name).to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            models.sort();
            models.dedup();
            Ok(models)
        }
        ApiProtocol::Anthropic => {
            if provider.api_key.is_empty() {
                return Ok(vec![
                    "claude-sonnet-4-20250514".to_string(),
                    "claude-opus-4-20250514".to_string(),
                    "claude-3-7-sonnet-latest".to_string(),
                    "claude-3-5-haiku-latest".to_string(),
                ]);
            }
            let url = format!("{base}/v1/models");
            let resp = client
                .get(&url)
                .header("x-api-key", &provider.api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            if let Ok(r) = resp
                && r.status().is_success()
                && let Ok(json) = r.json::<Value>().await
            {
                let mut models: Vec<String> = json
                    .get("data")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                if !models.is_empty() {
                    models.sort();
                    return Ok(models);
                }
            }
            Ok(vec![
                "claude-sonnet-4-20250514".to_string(),
                "claude-opus-4-20250514".to_string(),
                "claude-3-7-sonnet-latest".to_string(),
                "claude-3-5-haiku-latest".to_string(),
            ])
        }
    }
}

/// Incremental SSE line splitter that preserves multi-byte UTF-8 characters across chunk boundaries.
pub struct SseBuffer {
    buf: Vec<u8>,
}

impl Default for SseBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl SseBuffer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut raw_line: Vec<u8> = self.buf.drain(..=pos).collect();
            raw_line.pop(); // remove '\n'
            if raw_line.last() == Some(&b'\r') {
                raw_line.pop();
            }
            lines.push(String::from_utf8_lossy(&raw_line).into_owned());
        }
        lines
    }

    pub fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let mut raw_line = std::mem::take(&mut self.buf);
        if raw_line.last() == Some(&b'\r') {
            raw_line.pop();
        }
        Some(String::from_utf8_lossy(&raw_line).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_buffer_handles_split_utf8_and_trailing_line() {
        let mut sse = SseBuffer::new();
        let line1 = "data: Привет\n".as_bytes();
        // Split right inside Cyrillic multi-byte UTF-8 sequence
        let mid = 8;
        assert!(sse.push_bytes(&line1[..mid]).is_empty());
        let out = sse.push_bytes(&line1[mid..]);
        assert_eq!(out, vec!["data: Привет"]);

        // Trailing line without newline is flushed by finish()
        assert!(sse.push_bytes(b"data: [DONE]").is_empty());
        assert_eq!(sse.finish(), Some("data: [DONE]".to_string()));
        assert_eq!(sse.finish(), None);
    }

    #[test]
    fn formats_json_api_error_concisely() {
        let body = r#"{"error":{"message":"Rate limit reached","retry_after_seconds":5}}"#;
        let msg = format_api_error("OpenAI", reqwest::StatusCode::TOO_MANY_REQUESTS, body);
        assert_eq!(
            msg,
            "OpenAI API error (429): Rate limit reached [retry_after=5s]"
        );
    }
}
