use crate::config::McpServerConfig;
use crate::llm::ToolDefinition;
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct McpManager {
    inner: Arc<Mutex<McpState>>,
}

struct McpState {
    servers: BTreeMap<String, ServerConn>,
    errors: BTreeMap<String, String>,
    tools: Vec<(String, String, ToolDefinition)>, // (server_name, remote_tool_name, def)
}

struct ServerConn {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpManager {
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(Mutex::new(McpState {
                servers: BTreeMap::new(),
                errors: BTreeMap::new(),
                tools: Vec::new(),
            })),
        }
    }

    pub async fn connect_all(configs: &BTreeMap<String, McpServerConfig>) -> Self {
        let mgr = Self::empty();
        if configs.is_empty() {
            return mgr;
        }
        let mut state = mgr.inner.lock().await;
        for (name, cfg) in configs {
            if cfg.disabled {
                continue;
            }
            match Self::connect_one(name, cfg).await {
                Ok((conn, defs)) => {
                    for def in defs {
                        let remote = def.name.clone();
                        let safe_server = sanitize_ident(name);
                        let safe_tool = sanitize_ident(&remote);
                        let full_name = format!("mcp__{safe_server}__{safe_tool}");
                        state.tools.push((
                            name.clone(),
                            remote,
                            ToolDefinition {
                                name: full_name,
                                description: format!("[MCP:{name}] {}", def.description),
                                parameters: def.parameters,
                            },
                        ));
                    }
                    state.servers.insert(name.clone(), conn);
                }
                Err(e) => {
                    state.errors.insert(name.clone(), e.to_string());
                }
            }
        }
        drop(state);
        mgr
    }

    async fn connect_one(
        name: &str,
        cfg: &McpServerConfig,
    ) -> Result<(ServerConn, Vec<ToolDefinition>)> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn MCP server '{name}' ({})", cfg.command))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("No stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let mut conn = ServerConn {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };

        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "oxicode", "version": env!("CARGO_PKG_VERSION") }
        });
        tokio::time::timeout(
            Duration::from_secs(10),
            conn.rpc("initialize", init_params),
        )
        .await
        .map_err(|_| anyhow!("MCP '{name}' initialize timed out"))??;

        conn.notify("notifications/initialized", json!({})).await?;

        let list_res = tokio::time::timeout(
            Duration::from_secs(10),
            conn.rpc("tools/list", json!({})),
        )
        .await
        .map_err(|_| anyhow!("MCP '{name}' tools/list timed out"))??;

        let mut defs = Vec::new();
        if let Some(arr) = list_res.get("tools").and_then(Value::as_array) {
            for t in arr {
                let Some(tname) = t.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let desc = t
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let params = t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                defs.push(ToolDefinition {
                    name: tname.to_string(),
                    description: desc,
                    parameters: params,
                });
            }
        }

        Ok((conn, defs))
    }

    pub async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let state = self.inner.lock().await;
        state.tools.iter().map(|(_, _, d)| d.clone()).collect()
    }

    pub async fn is_mcp_tool(&self, name: &str) -> bool {
        let state = self.inner.lock().await;
        state.tools.iter().any(|(_, _, d)| d.name == name)
    }

    pub async fn call_tool(&self, full_name: &str, arguments: &Value) -> Result<String> {
        let mut state = self.inner.lock().await;
        let (server_name, remote_name) = state
            .tools
            .iter()
            .find(|(_, _, d)| d.name == full_name)
            .map(|(s, r, _)| (s.clone(), r.clone()))
            .ok_or_else(|| anyhow!("Unknown MCP tool: {full_name}"))?;

        let conn = state
            .servers
            .get_mut(&server_name)
            .ok_or_else(|| anyhow!("MCP server '{server_name}' is not connected"))?;

        let res = tokio::time::timeout(
            Duration::from_secs(30),
            conn.rpc(
                "tools/call",
                json!({
                    "name": remote_name,
                    "arguments": arguments,
                }),
            ),
        )
        .await
        .map_err(|_| anyhow!("MCP tool '{full_name}' timed out"))??;

        let mut out = String::new();
        if let Some(content) = res.get("content").and_then(Value::as_array) {
            for item in content {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                } else {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&item.to_string());
                }
            }
        }
        if out.is_empty() {
            out = res.to_string();
        }
        Ok(out)
    }

    pub async fn status_snapshot(&self) -> Vec<(String, usize, Option<String>)> {
        let state = self.inner.lock().await;
        let mut out = Vec::new();
        for name in state.servers.keys() {
            let count = state.tools.iter().filter(|(s, _, _)| s == name).count();
            out.push((name.clone(), count, None));
        }
        for (name, err) in &state.errors {
            out.push((name.clone(), 0, Some(err.clone())));
        }
        out
    }
}

impl ServerConn {
    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.stdout.read_line(&mut buf).await?;
            if n == 0 {
                return Err(anyhow!("MCP server closed stdout unexpectedly"));
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                return Err(anyhow!("MCP error: {err}"));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn sanitize_ident(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_manager_has_no_tools() {
        let mgr = McpManager::empty();
        assert!(mgr.tool_definitions().await.is_empty());
        assert!(!mgr.is_mcp_tool("read_file").await);
        assert_eq!(sanitize_ident("my.server/v1"), "my_server_v1");
    }
}
