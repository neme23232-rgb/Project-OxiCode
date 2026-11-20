use crate::config::Config;
use crate::extensions::ExtensionRegistry;
use crate::llm::{self, Message, Role, StreamEvent, ToolCall};
use crate::mcp::McpManager;
use crate::tools::{self, FileSnapshot};
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Build,
    Plan,
}

impl AgentMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentMode::Build => "BUILD",
            AgentMode::Plan => "PLAN",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            AgentMode::Build => AgentMode::Plan,
            AgentMode::Plan => AgentMode::Build,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    ApproveOnce,
    ApproveAll,
    Reject,
}

#[derive(Debug)]
pub enum AgentEvent {
    ThinkingDelta(String),
    AssistantDelta(String),
    ToolStarted {
        name: String,
        preview: String,
    },
    ApprovalRequest {
        name: String,
        preview: String,
        responder: oneshot::Sender<ApprovalDecision>,
    },
    ToolFinished {
        name: String,
        output: String,
        is_error: bool,
    },
    FileChanged(FileSnapshot),
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    TurnFinished,
    Error(String),
}

pub fn build_system_prompt(
    work_dir: &Path,
    mode: AgentMode,
    extensions: &ExtensionRegistry,
) -> String {
    let os = std::env::consts::OS;
    let mode_note = match mode {
        AgentMode::Build => {
            "Mode: BUILD (full access to inspect files, edit code, and run commands)."
        }
        AgentMode::Plan => {
            "Mode: PLAN (read-only analysis). Only inspect files and propose a concrete plan; do NOT attempt to edit files or run shell commands."
        }
    };

    let mut prompt = format!(
        "You are oxicode, a fast, concise terminal AI coding assistant.\n\
         Workspace: {}\n\
         OS: {os}\n\
         {mode_note}\n\n\
         Use the available tools to inspect the codebase, make surgical edits, and verify changes with tests or builds.\n\
         Be concise, direct, and accurate. Only report file paths, symbols, and command outputs you directly observed.",
        work_dir.display()
    );

    if let Some(cfg_dir) = dirs::config_dir() {
        let global_agents = cfg_dir.join("oxicode").join("AGENTS.md");
        if let Ok(extra) = std::fs::read_to_string(&global_agents) {
            let trimmed = extra.trim();
            if !trimmed.is_empty() {
                prompt.push_str("\n\nGlobal instructions (AGENTS.md):\n");
                prompt.push_str(trimmed);
            }
        }
    }

    for candidate in ["AGENTS.md", ".oxicode.md", "CLAUDE.md", "GEMINI.md"] {
        let p = work_dir.join(candidate);
        if let Ok(extra) = std::fs::read_to_string(&p) {
            let trimmed = extra.trim();
            if !trimmed.is_empty() {
                prompt.push_str("\n\nProject instructions (");
                prompt.push_str(candidate);
                prompt.push_str("):\n");
                prompt.push_str(trimmed);
                break;
            }
        }
    }

    for rule in &extensions.rules {
        prompt.push_str("\n\nRule:\n");
        prompt.push_str(rule);
    }

    if !extensions.skills.is_empty() {
        prompt.push_str("\n\nAvailable skills (call `load_skill` tool with the skill name when relevant):\n");
        for sk in &extensions.skills {
            prompt.push_str(&format!("- {}: {}\n", sk.name, sk.description));
        }
    }

    prompt
}

pub struct TurnParams {
    pub client: reqwest::Client,
    pub cfg: Config,
    pub work_dir: PathBuf,
    pub mode: AgentMode,
    pub mcp: McpManager,
    pub extensions: ExtensionRegistry,
}

const MAX_AGENT_STEPS: usize = 40;

/// Ensures history never has dangling assistant `tool_calls` without matching `tool_result` entries.
pub fn sanitize_history(history: &mut Vec<Message>) {
    let mut cleaned: Vec<Message> = Vec::with_capacity(history.len());
    let mut pending_call_ids: Vec<String> = Vec::new();

    for msg in history.drain(..) {
        match msg.role {
            Role::Assistant => {
                if !pending_call_ids.is_empty() {
                    for missing_id in pending_call_ids.drain(..) {
                        cleaned.push(Message::tool_result(
                            missing_id,
                            "interrupted",
                            "Tool execution was interrupted.",
                        ));
                    }
                }
                if !msg.tool_calls.is_empty() {
                    pending_call_ids = msg.tool_calls.iter().map(|tc| tc.id.clone()).collect();
                }
                cleaned.push(msg);
            }
            Role::Tool => {
                if let Some(ref cid) = msg.tool_call_id {
                    pending_call_ids.retain(|id| id != cid);
                    cleaned.push(msg);
                }
            }
            Role::User => {
                if !pending_call_ids.is_empty() {
                    for missing_id in pending_call_ids.drain(..) {
                        cleaned.push(Message::tool_result(
                            missing_id,
                            "interrupted",
                            "Tool execution was interrupted.",
                        ));
                    }
                }
                cleaned.push(msg);
            }
            Role::System => cleaned.push(msg),
        }
    }

    if !pending_call_ids.is_empty() {
        for missing_id in pending_call_ids.drain(..) {
            cleaned.push(Message::tool_result(
                missing_id,
                "interrupted",
                "Tool execution was interrupted.",
            ));
        }
    }

    *history = cleaned;
}

pub async fn run_turn(
    params: TurnParams,
    mut history: Vec<Message>,
    ui_tx: mpsc::UnboundedSender<AgentEvent>,
) -> (Vec<Message>, bool) {
    sanitize_history(&mut history);
    let TurnParams {
        client,
        mut cfg,
        work_dir,
        mode,
        mcp,
        extensions,
    } = params;
    let system = build_system_prompt(&work_dir, mode, &extensions);
    let mut tool_defs = tools::definitions_for_mode(mode == AgentMode::Plan);
    for td in extensions.tool_definitions() {
        if mode == AgentMode::Build || !tools::requires_approval(&td.name) {
            tool_defs.push(td);
        }
    }
    if mode == AgentMode::Build {
        tool_defs.extend(mcp.tool_definitions().await);
    }
    let mut auto_approve = cfg.auto_approve;

    for step in 0..MAX_AGENT_STEPS {
        let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<StreamEvent>();
        let client_clone = client.clone();
        let cfg_clone = cfg.clone();
        let sys_clone = system.clone();
        let hist_clone = history.clone();
        let defs_clone = tool_defs.clone();

        let stream_handle = tokio::spawn(async move {
            llm::stream_chat(
                &client_clone,
                &cfg_clone,
                &sys_clone,
                &hist_clone,
                &defs_clone,
                stream_tx,
            )
            .await
        });

        let mut assistant_text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut step_usage: Option<(u32, u32)> = None;

        while let Some(ev) = stream_rx.recv().await {
            match ev {
                StreamEvent::ThinkingDelta(delta) => {
                    let _ = ui_tx.send(AgentEvent::ThinkingDelta(delta));
                }
                StreamEvent::TextDelta(delta) => {
                    assistant_text.push_str(&delta);
                    let _ = ui_tx.send(AgentEvent::AssistantDelta(delta));
                }
                StreamEvent::ToolCall(tc) => {
                    tool_calls.push(tc);
                }
                StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    // Track latest cumulative usage snapshot for this single HTTP turn
                    step_usage = Some((input_tokens, output_tokens));
                }
                StreamEvent::Done => break,
            }
        }

        if let Some((input_tokens, output_tokens)) = step_usage {
            let _ = ui_tx.send(AgentEvent::Usage {
                input_tokens,
                output_tokens,
            });
        }

        match stream_handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = ui_tx.send(AgentEvent::Error(e.to_string()));
                let _ = ui_tx.send(AgentEvent::TurnFinished);
                return (history, auto_approve);
            }
            Err(e) => {
                let _ = ui_tx.send(AgentEvent::Error(format!("Task panicked: {e}")));
                let _ = ui_tx.send(AgentEvent::TurnFinished);
                return (history, auto_approve);
            }
        }

        history.push(Message::assistant(assistant_text, tool_calls.clone()));

        if tool_calls.is_empty() {
            break;
        }

        for tc in tool_calls {
            if let Some(parse_err) = tc
                .arguments
                .get("__oxicode_parse_error")
                .and_then(Value::as_str)
            {
                let msg = format!(
                    "Invalid JSON tool arguments for '{}': {parse_err}. Please retry with valid JSON.",
                    tc.name
                );
                let _ = ui_tx.send(AgentEvent::ToolFinished {
                    name: tc.name.clone(),
                    output: msg.clone(),
                    is_error: true,
                });
                history.push(Message::tool_result(tc.id, tc.name, msg));
                continue;
            }

            let smart_preview = tools::smart_diff_preview(&work_dir, &tc.name, &tc.arguments);
            let mut approved = true;

            if tools::requires_approval(&tc.name) && !auto_approve {
                let (resp_tx, resp_rx) = oneshot::channel();
                let _ = ui_tx.send(AgentEvent::ApprovalRequest {
                    name: tc.name.clone(),
                    preview: smart_preview.clone(),
                    responder: resp_tx,
                });

                match resp_rx.await {
                    Ok(ApprovalDecision::ApproveOnce) => approved = true,
                    Ok(ApprovalDecision::ApproveAll) => {
                        approved = true;
                        auto_approve = true;
                        cfg.auto_approve = true;
                    }
                    Ok(ApprovalDecision::Reject) | Err(_) => approved = false,
                }
            }

            if !approved {
                let msg = "Tool execution rejected by user.".to_string();
                let _ = ui_tx.send(AgentEvent::ToolFinished {
                    name: tc.name.clone(),
                    output: msg.clone(),
                    is_error: true,
                });
                history.push(Message::tool_result(tc.id, tc.name, msg));
                continue;
            }

            let snapshot = tools::capture_snapshot(&work_dir, &tc.name, &tc.arguments);

            let _ = ui_tx.send(AgentEvent::ToolStarted {
                name: tc.name.clone(),
                preview: smart_preview,
            });

            let exec_res = if tc.name == "load_skill" {
                let sname = tc
                    .arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match extensions.find_skill(sname) {
                    Some(sk) => Ok(format!(
                        "# Skill: {} ({})\n\n{}",
                        sk.name,
                        sk.path.display(),
                        sk.body
                    )),
                    None => Err(anyhow!("Skill '{sname}' not found")),
                }
            } else if extensions.is_plugin_tool(&tc.name) {
                extensions
                    .execute_plugin_tool(&tc.name, &tc.arguments, &work_dir)
                    .await
            } else if mcp.is_mcp_tool(&tc.name).await {
                mcp.call_tool(&tc.name, &tc.arguments).await
            } else {
                tools::execute(&work_dir, &tc.name, &tc.arguments).await
            };

            match exec_res {
                Ok(out) => {
                    if let Some(snap) = snapshot {
                        let _ = ui_tx.send(AgentEvent::FileChanged(snap));
                    }
                    let _ = ui_tx.send(AgentEvent::ToolFinished {
                        name: tc.name.clone(),
                        output: out.clone(),
                        is_error: false,
                    });
                    history.push(Message::tool_result(tc.id, tc.name, out));
                }
                Err(e) => {
                    let err_msg = format!("Error: {e}");
                    let _ = ui_tx.send(AgentEvent::ToolFinished {
                        name: tc.name.clone(),
                        output: err_msg.clone(),
                        is_error: true,
                    });
                    history.push(Message::tool_result(tc.id, tc.name, err_msg));
                }
            }
        }

        if step + 1 == MAX_AGENT_STEPS {
            let _ = ui_tx.send(AgentEvent::Error(format!(
                "Stopped after {MAX_AGENT_STEPS} tool steps to avoid an infinite loop."
            )));
        }
    }

    let _ = ui_tx.send(AgentEvent::TurnFinished);
    (history, auto_approve)
}

pub async fn compact_history(
    client: &reqwest::Client,
    cfg: &Config,
    history: &[Message],
) -> Result<(Vec<Message>, String)> {
    if history.is_empty() {
        return Err(anyhow!("Nothing to compact."));
    }
    let mut transcript = String::new();
    for msg in history {
        let role = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
            Role::System => "System",
        };
        if !msg.content.trim().is_empty() {
            let snippet: String = msg.content.chars().take(1200).collect();
            transcript.push_str(&format!("[{role}]: {snippet}\n"));
        }
    }
    let prompt = format!(
        "Summarize this coding session concisely (goals, files changed, decisions, current status, and pending tasks) so work can continue without losing context:\n\n{transcript}"
    );
    let (tx, mut rx) = mpsc::unbounded_channel();
    let req_msgs = vec![Message::user(prompt)];
    let client_c = client.clone();
    let cfg_c = cfg.clone();

    let handle = tokio::spawn(async move {
        llm::stream_chat(
            &client_c,
            &cfg_c,
            "You are a concise session summarizer.",
            &req_msgs,
            &[],
            tx,
        )
        .await
    });

    let mut summary = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            StreamEvent::TextDelta(d) => summary.push_str(&d),
            StreamEvent::Done => break,
            _ => {}
        }
    }
    handle.await??;

    if summary.trim().is_empty() {
        return Err(anyhow!("Model returned an empty summary."));
    }
    let compacted = vec![Message::user(format!(
        "Previous conversation summary:\n{}",
        summary.trim()
    ))];
    Ok((compacted, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_history_repairs_dangling_tool_calls() {
        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "a.rs"}),
            thought_signature: None,
        };
        let mut hist = vec![
            Message::user("check a.rs"),
            Message::assistant("", vec![tc]),
            Message::user("continue"),
        ];
        sanitize_history(&mut hist);
        assert_eq!(hist.len(), 4);
        assert_eq!(hist[2].role, Role::Tool);
        assert_eq!(hist[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(hist[3].role, Role::User);
    }
}
