use oxicode_core::HttpClient;
use oxicode_core::agent::{self, AgentEvent, AgentMode, ApprovalDecision, TurnParams};
use oxicode_core::commands;
use oxicode_core::config::{self, Config, PROVIDERS};
use oxicode_core::extensions::{self, ExtensionRegistry};
use oxicode_core::highlight;
use oxicode_core::llm::{self, Message};
use oxicode_core::mcp::McpManager;
use oxicode_core::session::{self, SavedSession};
use oxicode_core::theme::{ThemeCss, ThemeKind};
use oxicode_core::tools::{self, FileSnapshot};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

static APPROVAL_SEQ: AtomicU64 = AtomicU64::new(1);

struct InnerState {
    cfg: Config,
    work_dir: PathBuf,
    git_branch: Option<String>,
    session_id: String,
    client: HttpClient,
    mcp: McpManager,
    extensions: ExtensionRegistry,
    history: Vec<Message>,
    undo_stack: Vec<FileSnapshot>,
    redo_stack: Vec<FileSnapshot>,
    changed_files: BTreeMap<String, (usize, usize)>,
    pending_approvals: HashMap<String, oneshot::Sender<ApprovalDecision>>,
    active_task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct AppState(Arc<Mutex<InnerState>>);

#[derive(Serialize)]
struct ProviderInfo {
    id: &'static str,
    display_name: &'static str,
    base_url: String,
    model: String,
    requires_key: bool,
    has_key: bool,
    masked_key: String,
}

#[derive(Serialize)]
struct SkillInfo {
    name: String,
    description: String,
    source: String,
}

#[derive(Serialize)]
struct PluginInfo {
    id: String,
    name: String,
    version: String,
    description: String,
    enabled: bool,
    tool_count: usize,
    skill_count: usize,
    command_count: usize,
    mcp_count: usize,
}

#[derive(Serialize)]
struct CustomCmdInfo {
    slash: String,
    description: String,
}

#[derive(Serialize)]
struct McpInfo {
    name: String,
    tool_count: usize,
    error: Option<String>,
}

#[derive(Serialize)]
struct ChangedFileInfo {
    path: String,
    added: usize,
    removed: usize,
    icon_badge: &'static str,
    icon_color: String,
}

#[derive(Serialize)]
struct FileListItem {
    path: String,
    icon_badge: &'static str,
    icon_color: String,
}

#[derive(Serialize)]
struct BootstrapPayload {
    provider_id: String,
    provider_name: String,
    model: String,
    has_key: bool,
    auto_approve: bool,
    default_plan_mode: bool,
    show_tool_details: bool,
    show_fox_mascot: bool,
    max_tokens: u32,
    config_path: String,
    context_window: u32,
    work_dir: String,
    git_branch: Option<String>,
    workspace_files: Vec<FileListItem>,
    theme: ThemeCss,
    themes: Vec<ThemeCss>,
    providers: Vec<ProviderInfo>,
    sessions: Vec<SavedSession>,
    skills: Vec<SkillInfo>,
    plugins: Vec<PluginInfo>,
    custom_commands: Vec<CustomCmdInfo>,
    mcp_servers: Vec<McpInfo>,
    changed_files: Vec<ChangedFileInfo>,
    undo_count: usize,
    redo_count: usize,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentStreamPayload {
    ThinkingDelta { text: String },
    AssistantDelta { text: String },
    ToolStarted {
        name: String,
        preview: String,
        icon_badge: String,
        icon_color: String,
    },
    ApprovalRequest { id: String, name: String, preview: String },
    ToolFinished { name: String, output: String, is_error: bool },
    FileChanged {
        path: String,
        added: usize,
        removed: usize,
        icon_badge: String,
        icon_color: String,
        undo_count: usize,
        redo_count: usize,
    },
    Usage { input_tokens: u32, output_tokens: u32 },
    TurnFinished,
    Error { message: String },
}

fn mask_key(k: &str) -> String {
    if k.is_empty() {
        return String::new();
    }
    let len = k.chars().count();
    if len <= 6 {
        return "*".repeat(len);
    }
    let tail: String = k.chars().rev().take(4).collect::<String>().chars().rev().collect();
    format!("{}...{tail}", "*".repeat((len - 4).min(12)))
}

async fn build_bootstrap(st: &InnerState) -> BootstrapPayload {
    let pal = st.cfg.theme.palette();
    let workspace_files = tools::list_workspace_files(&st.work_dir, 400)
        .into_iter()
        .map(|p| {
            let fi = highlight::file_icon(&p, pal);
            FileListItem {
                path: p,
                icon_badge: fi.badge,
                icon_color: fi.color.hex(),
            }
        })
        .collect();

    let providers = PROVIDERS
        .iter()
        .map(|spec| {
            let entry = st.cfg.providers.get(spec.id);
            let key = entry.map(|p| p.api_key.as_str()).unwrap_or("");
            ProviderInfo {
                id: spec.id,
                display_name: spec.display_name,
                base_url: entry
                    .map(|p| p.base_url.clone())
                    .unwrap_or_else(|| spec.default_base_url.to_string()),
                model: entry
                    .map(|p| p.model.clone())
                    .unwrap_or_else(|| spec.default_model.to_string()),
                requires_key: spec.requires_key,
                has_key: st.cfg.has_key(spec.id),
                masked_key: mask_key(key),
            }
        })
        .collect();

    let mcp_servers = st
        .mcp
        .status_snapshot()
        .await
        .into_iter()
        .map(|(name, tool_count, error)| McpInfo {
            name,
            tool_count,
            error,
        })
        .collect();

    let changed_files = st
        .changed_files
        .iter()
        .map(|(p, (added, removed))| {
            let fi = highlight::file_icon(p, pal);
            ChangedFileInfo {
                path: p.clone(),
                added: *added,
                removed: *removed,
                icon_badge: fi.badge,
                icon_color: fi.color.hex(),
            }
        })
        .collect();

    BootstrapPayload {
        provider_id: st.cfg.provider.clone(),
        provider_name: st.cfg.active_spec().display_name.to_string(),
        model: st.cfg.active().model.clone(),
        has_key: st.cfg.has_key(&st.cfg.provider),
        auto_approve: st.cfg.auto_approve,
        default_plan_mode: st.cfg.default_plan_mode,
        show_tool_details: st.cfg.show_tool_details,
        show_fox_mascot: st.cfg.show_fox_mascot,
        max_tokens: st.cfg.max_tokens,
        config_path: Config::global_config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        context_window: st.cfg.context_window_tokens(),
        work_dir: st.work_dir.display().to_string(),
        git_branch: st.git_branch.clone(),
        workspace_files,
        theme: st.cfg.theme.to_css(),
        themes: ThemeKind::all().iter().map(ThemeKind::to_css).collect(),
        providers,
        sessions: session::list_sessions(30),
        skills: st
            .extensions
            .skills
            .iter()
            .map(|s| SkillInfo {
                name: s.name.clone(),
                description: s.description.clone(),
                source: s.source.clone(),
            })
            .collect(),
        plugins: st
            .extensions
            .plugins
            .iter()
            .map(|p| PluginInfo {
                id: p.id.clone(),
                name: p.name.clone(),
                version: p.version.clone(),
                description: p.description.clone(),
                enabled: p.enabled,
                tool_count: p.tool_count,
                skill_count: p.skill_count,
                command_count: p.command_count,
                mcp_count: p.mcp_count,
            })
            .collect(),
        custom_commands: st
            .extensions
            .custom_commands
            .iter()
            .map(|c| CustomCmdInfo {
                slash: c.slash.clone(),
                description: c.description.clone(),
            })
            .collect(),
        mcp_servers,
        changed_files,
        undo_count: st.undo_stack.len(),
        redo_count: st.redo_stack.len(),
    }
}

#[tauri::command]
async fn get_bootstrap(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    let guard = state.0.lock().await;
    Ok(build_bootstrap(&guard).await)
}

fn expand_prompt(work_dir: &Path, text: &str, ext: &ExtensionRegistry) -> String {
    let mut prompt = text.to_string();
    if let Some(first) = text.split_whitespace().next()
        && first.starts_with('/')
        && let Some(cc) = ext
            .custom_commands
            .iter()
            .find(|c| c.slash == first.to_lowercase())
    {
        let rest = text[first.len()..].trim();
        prompt = cc.expand(rest);
    }

    let mut attachments = Vec::new();
    for word in prompt.split_whitespace() {
        let Some(rel) = word.strip_prefix('@') else {
            continue;
        };
        let clean = rel.trim_end_matches([',', '.', ';', ':', ')']);
        if clean.is_empty() {
            continue;
        }
        let path = work_dir.join(clean);
        if path.is_file()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let snippet = if content.len() > 12_000 {
                let mut end = 12_000;
                while !content.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}\n... [truncated]", &content[..end])
            } else {
                content
            };
            attachments.push(format!("--- File: {clean} ---\n{snippet}"));
        }
    }
    if attachments.is_empty() {
        prompt
    } else {
        format!("{prompt}\n\n{}", attachments.join("\n\n"))
    }
}

#[tauri::command]
async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    mode: String,
) -> Result<(), String> {
    let trimmed = prompt.trim().to_string();
    if trimmed.is_empty() {
        return Ok(());
    }

    let (client, cfg, work_dir, mcp, extensions, history, pal) = {
        let mut guard = state.0.lock().await;
        if guard.active_task.is_some() {
            return Err("Generation is already in progress.".to_string());
        }
        let expanded = expand_prompt(&guard.work_dir, &trimmed, &guard.extensions);
        guard.history.push(Message::user(expanded));
        (
            guard.client.clone(),
            guard.cfg.clone(),
            guard.work_dir.clone(),
            guard.mcp.clone(),
            guard.extensions.clone(),
            guard.history.clone(),
            guard.cfg.theme.palette(),
        )
    };

    let agent_mode = if mode.eq_ignore_ascii_case("plan") {
        AgentMode::Plan
    } else {
        AgentMode::Build
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
    let state_arc = state.0.clone();
    let app_clone = app.clone();

    let fwd_task = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::ThinkingDelta(text) => {
                    let _ = app_clone.emit("agent-event", AgentStreamPayload::ThinkingDelta { text });
                }
                AgentEvent::AssistantDelta(text) => {
                    let _ = app_clone.emit("agent-event", AgentStreamPayload::AssistantDelta { text });
                }
                AgentEvent::ToolStarted { name, preview } => {
                    let first = preview.lines().next().unwrap_or("");
                    let file_arg = first.split_whitespace().nth(1).unwrap_or(first);
                    let fi = highlight::file_icon(file_arg, pal);
                    let _ = app_clone.emit(
                        "agent-event",
                        AgentStreamPayload::ToolStarted {
                            name,
                            preview,
                            icon_badge: fi.badge.to_string(),
                            icon_color: fi.color.hex(),
                        },
                    );
                }
                AgentEvent::ApprovalRequest {
                    name,
                    preview,
                    responder,
                } => {
                    let id = format!("apr_{}", APPROVAL_SEQ.fetch_add(1, Ordering::Relaxed));
                    {
                        let mut guard = state_arc.lock().await;
                        guard.pending_approvals.insert(id.clone(), responder);
                    }
                    let _ = app_clone.emit(
                        "agent-event",
                        AgentStreamPayload::ApprovalRequest { id, name, preview },
                    );
                }
                AgentEvent::FileChanged(snap) => {
                    let (undo_count, redo_count, fi) = {
                        let mut guard = state_arc.lock().await;
                        let e = guard
                            .changed_files
                            .entry(snap.rel_path.clone())
                            .or_insert((0, 0));
                        e.0 += snap.added;
                        e.1 += snap.removed;
                        let fi = highlight::file_icon(&snap.rel_path, pal);
                        let rel = snap.rel_path.clone();
                        let added = snap.added;
                        let removed = snap.removed;
                        guard.undo_stack.push(snap);
                        guard.redo_stack.clear();
                        let u = guard.undo_stack.len();
                        let r = guard.redo_stack.len();
                        let _ = app_clone.emit(
                            "agent-event",
                            AgentStreamPayload::FileChanged {
                                path: rel,
                                added,
                                removed,
                                icon_badge: fi.badge.to_string(),
                                icon_color: fi.color.hex(),
                                undo_count: u,
                                redo_count: r,
                            },
                        );
                        (u, r, fi)
                    };
                    let _ = (undo_count, redo_count, fi);
                }
                AgentEvent::ToolFinished {
                    name,
                    output,
                    is_error,
                } => {
                    let _ = app_clone.emit(
                        "agent-event",
                        AgentStreamPayload::ToolFinished {
                            name,
                            output,
                            is_error,
                        },
                    );
                }
                AgentEvent::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    let _ = app_clone.emit(
                        "agent-event",
                        AgentStreamPayload::Usage {
                            input_tokens,
                            output_tokens,
                        },
                    );
                }
                AgentEvent::Error(message) => {
                    let _ = app_clone.emit("agent-event", AgentStreamPayload::Error { message });
                }
                AgentEvent::TurnFinished => {
                    let _ = app_clone.emit("agent-event", AgentStreamPayload::TurnFinished);
                }
            }
        }
    });

    let state_arc_turn = state.0.clone();
    let handle = tokio::spawn(async move {
        let (new_history, auto_approve) = agent::run_turn(
            TurnParams {
                client,
                cfg,
                work_dir,
                mode: agent_mode,
                mcp,
                extensions,
            },
            history,
            tx,
        )
        .await;

        let mut guard = state_arc_turn.lock().await;
        guard.history = new_history;
        guard.cfg.auto_approve = auto_approve;
        guard.active_task = None;
        let _ = session::save_session(
            &guard.session_id,
            &guard.work_dir,
            &guard.cfg.provider,
            &guard.cfg.active().model,
            &guard.history,
        );
        drop(guard);
        let _ = fwd_task.await;
    });

    state.0.lock().await.active_task = Some(handle);
    Ok(())
}

#[tauri::command]
async fn respond_approval(
    state: State<'_, AppState>,
    id: String,
    decision: String,
) -> Result<(), String> {
    let mut guard = state.0.lock().await;
    if let Some(tx) = guard.pending_approvals.remove(&id) {
        let dec = match decision.as_str() {
            "approve_all" => {
                guard.cfg.auto_approve = true;
                ApprovalDecision::ApproveAll
            }
            "approve_once" => ApprovalDecision::ApproveOnce,
            _ => ApprovalDecision::Reject,
        };
        let _ = tx.send(dec);
    }
    Ok(())
}

#[tauri::command]
async fn stop_generation(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().await;
    if let Some(handle) = guard.active_task.take() {
        handle.abort();
    }
    guard.pending_approvals.clear();
    let _ = app.emit("agent-event", AgentStreamPayload::TurnFinished);
    Ok(())
}

#[tauri::command]
async fn run_inline_shell(state: State<'_, AppState>, command: String) -> Result<String, String> {
    let work_dir = state.0.lock().await.work_dir.clone();
    tools::execute(&work_dir, "bash", &json!({ "command": command }))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn undo_change(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    let snap = guard.undo_stack.pop().ok_or("Nothing to undo.")?;
    tools::restore_snapshot(&snap).map_err(|e| e.to_string())?;
    guard.redo_stack.push(snap);
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn redo_change(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    let snap = guard.redo_stack.pop().ok_or("Nothing to redo.")?;
    tools::reapply_snapshot(&snap).map_err(|e| e.to_string())?;
    guard.undo_stack.push(snap);
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn compact_chat(state: State<'_, AppState>) -> Result<String, String> {
    let (client, cfg, history) = {
        let guard = state.0.lock().await;
        (guard.client.clone(), guard.cfg.clone(), guard.history.clone())
    };
    let (compacted, summary) = agent::compact_history(&client, &cfg, &history)
        .await
        .map_err(|e| e.to_string())?;
    let mut guard = state.0.lock().await;
    guard.history = compacted;
    Ok(summary)
}

#[tauri::command]
async fn export_chat(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state.0.lock().await;
    let mut md = format!(
        "# oxicode session\n\n- **Provider**: {} ({})\n- **Workspace**: `{}`\n\n---\n\n",
        guard.cfg.active_spec().display_name,
        guard.cfg.active().model,
        guard.work_dir.display()
    );
    for msg in &guard.history {
        match msg.role {
            llm::Role::User => md.push_str(&format!("## User\n\n{}\n\n", msg.content)),
            llm::Role::Assistant => {
                if !msg.content.is_empty() {
                    md.push_str(&format!("## Assistant\n\n{}\n\n", msg.content));
                }
                for tc in &msg.tool_calls {
                    md.push_str(&format!("### Tool `{}`\n\n```json\n{}\n```\n\n", tc.name, tc.arguments));
                }
            }
            llm::Role::Tool => {}
            llm::Role::System => {}
        }
    }
    let path = commands::export_markdown(&guard.work_dir, None, &md).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
async fn set_theme(state: State<'_, AppState>, name: String) -> Result<ThemeCss, String> {
    let kind = ThemeKind::parse(&name).ok_or_else(|| format!("Unknown theme: {name}"))?;
    let mut guard = state.0.lock().await;
    guard.cfg.theme = kind;
    let _ = guard.cfg.save_global();
    Ok(kind.to_css())
}

#[tauri::command]
async fn set_auto_approve(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut guard = state.0.lock().await;
    guard.cfg.auto_approve = enabled;
    let _ = guard.cfg.save_global();
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn save_settings(
    state: State<'_, AppState>,
    theme: String,
    auto_approve: bool,
    default_plan_mode: bool,
    show_tool_details: bool,
    show_fox_mascot: bool,
    max_tokens: u32,
) -> Result<BootstrapPayload, String> {
    let kind = ThemeKind::parse(&theme).ok_or_else(|| format!("Unknown theme: {theme}"))?;
    let mut guard = state.0.lock().await;
    guard.cfg.theme = kind;
    guard.cfg.auto_approve = auto_approve;
    guard.cfg.default_plan_mode = default_plan_mode;
    guard.cfg.show_tool_details = show_tool_details;
    guard.cfg.show_fox_mascot = show_fox_mascot;
    if max_tokens >= 512 {
        guard.cfg.max_tokens = max_tokens;
    }
    guard.cfg.save_global().map_err(|e| e.to_string())?;
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn save_provider(
    state: State<'_, AppState>,
    provider_id: String,
    api_key: String,
    base_url: String,
    model: String,
) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    guard
        .cfg
        .set_provider(&provider_id)
        .ok_or_else(|| format!("Unknown provider: {provider_id}"))?;
    let entry = guard.cfg.active_mut();
    if !api_key.trim().is_empty() {
        entry.api_key = api_key.trim().to_string();
    }
    if !base_url.trim().is_empty() {
        entry.base_url = config::normalize_base_url(&base_url);
    }
    if !model.trim().is_empty() {
        entry.model = model.trim().to_string();
    }
    guard.cfg.save_global().map_err(|e| e.to_string())?;
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn fetch_provider_models(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let (client, cfg) = {
        let guard = state.0.lock().await;
        (guard.client.clone(), guard.cfg.clone())
    };
    llm::fetch_models(&client, &cfg)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_model(state: State<'_, AppState>, model: String) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    guard.cfg.active_mut().model = model.trim().to_string();
    guard.cfg.save_global().map_err(|e| e.to_string())?;
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn set_work_dir(state: State<'_, AppState>, path: String) -> Result<BootstrapPayload, String> {
    let candidate = PathBuf::from(path.trim().trim_matches('"'));
    if !candidate.is_dir() {
        return Err(format!("Directory does not exist: {}", candidate.display()));
    }
    let canonical = candidate
        .canonicalize()
        .map(|p| session::strip_unc_prefix(&p))
        .unwrap_or_else(|_| session::strip_unc_prefix(&candidate));

    let mut guard = state.0.lock().await;
    guard.work_dir = canonical.clone();
    guard.git_branch = session::read_git_branch(&canonical);
    guard.extensions = ExtensionRegistry::load(&canonical, &guard.cfg.disabled_plugins);
    let mut mcp_cfg = guard.cfg.mcp_servers.clone();
    mcp_cfg.extend(guard.extensions.extra_mcp.clone());
    guard.mcp = McpManager::connect_all(&mcp_cfg).await;
    Ok(build_bootstrap(&guard).await)
}

#[derive(Serialize)]
struct DirBrowsePayload {
    current: String,
    parent: Option<String>,
    dirs: Vec<String>,
}

#[tauri::command]
async fn browse_dirs(path: Option<String>) -> Result<DirBrowsePayload, String> {
    let target = match path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    let clean = session::strip_unc_prefix(&target);
    let mut dirs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&clean) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir()
                && let Some(name) = p.file_name().and_then(|s| s.to_str())
                && !name.starts_with('.')
            {
                dirs.push(session::strip_unc_prefix(&p).display().to_string());
            }
        }
    }
    dirs.sort();
    Ok(DirBrowsePayload {
        current: clean.display().to_string(),
        parent: clean.parent().map(|p| p.display().to_string()),
        dirs,
    })
}

#[tauri::command]
async fn new_session(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    guard.history.clear();
    guard.undo_stack.clear();
    guard.redo_stack.clear();
    guard.changed_files.clear();
    guard.session_id = session::new_session_id();
    Ok(build_bootstrap(&guard).await)
}

#[derive(Serialize)]
struct LoadedSessionPayload {
    session: SavedSession,
    bootstrap: BootstrapPayload,
}

#[tauri::command]
async fn load_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<LoadedSessionPayload, String> {
    let found = session::list_sessions(50)
        .into_iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    let mut guard = state.0.lock().await;
    guard.session_id = found.id.clone();
    guard.history = found.messages.clone();
    let bootstrap = build_bootstrap(&guard).await;
    Ok(LoadedSessionPayload {
        session: found,
        bootstrap,
    })
}

#[tauri::command]
async fn toggle_plugin(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    if guard.cfg.disabled_plugins.contains(&plugin_id) {
        guard.cfg.disabled_plugins.retain(|d| d != &plugin_id);
    } else {
        guard.cfg.disabled_plugins.push(plugin_id);
    }
    let _ = guard.cfg.save_global();
    guard.extensions = ExtensionRegistry::load(&guard.work_dir, &guard.cfg.disabled_plugins);
    let mut mcp_cfg = guard.cfg.mcp_servers.clone();
    mcp_cfg.extend(guard.extensions.extra_mcp.clone());
    guard.mcp = McpManager::connect_all(&mcp_cfg).await;
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn create_starter_skill(
    state: State<'_, AppState>,
    name: String,
) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    extensions::scaffold_starter_skill(&guard.work_dir, &name).map_err(|e| e.to_string())?;
    guard.extensions = ExtensionRegistry::load(&guard.work_dir, &guard.cfg.disabled_plugins);
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn create_starter_plugin(
    state: State<'_, AppState>,
    name: String,
) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    extensions::scaffold_starter_plugin(&guard.work_dir, &name).map_err(|e| e.to_string())?;
    guard.extensions = ExtensionRegistry::load(&guard.work_dir, &guard.cfg.disabled_plugins);
    Ok(build_bootstrap(&guard).await)
}

#[tauri::command]
async fn reload_mcp(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    let mut guard = state.0.lock().await;
    guard.extensions = ExtensionRegistry::load(&guard.work_dir, &guard.cfg.disabled_plugins);
    let mut mcp_cfg = guard.cfg.mcp_servers.clone();
    mcp_cfg.extend(guard.extensions.extra_mcp.clone());
    guard.mcp = McpManager::connect_all(&mcp_cfg).await;
    Ok(build_bootstrap(&guard).await)
}

pub fn run() {
    let raw_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let work_dir = raw_dir
        .canonicalize()
        .map(|p| session::strip_unc_prefix(&p))
        .unwrap_or(raw_dir);
    let cfg = Config::load(&work_dir, None, None, None).unwrap_or_default();
    let extensions = ExtensionRegistry::load(&work_dir, &cfg.disabled_plugins);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut mcp_cfg = cfg.mcp_servers.clone();
    mcp_cfg.extend(extensions.extra_mcp.clone());
    let mcp = rt.block_on(McpManager::connect_all(&mcp_cfg));
    drop(rt);

    let inner = InnerState {
        git_branch: session::read_git_branch(&work_dir),
        cfg,
        work_dir,
        session_id: session::new_session_id(),
        client: HttpClient::new(),
        mcp,
        extensions,
        history: Vec::new(),
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        changed_files: BTreeMap::new(),
        pending_approvals: HashMap::new(),
        active_task: None,
    };

    tauri::Builder::default()
        .manage(AppState(Arc::new(Mutex::new(inner))))
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            send_message,
            respond_approval,
            stop_generation,
            run_inline_shell,
            undo_change,
            redo_change,
            compact_chat,
            export_chat,
            set_theme,
            set_auto_approve,
            save_settings,
            save_provider,
            fetch_provider_models,
            set_model,
            set_work_dir,
            browse_dirs,
            new_session,
            load_session,
            toggle_plugin,
            create_starter_skill,
            create_starter_plugin,
            reload_mcp,
        ])
        .run(tauri::generate_context!())
        .expect("error while running oxicode desktop");
}
