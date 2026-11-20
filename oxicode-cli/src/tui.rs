use crate::tui_theme::{self as highlight, Palette, palette_for};
use oxicode_core::HttpClient;
use oxicode_core::agent::{self, AgentEvent, AgentMode, ApprovalDecision, TurnParams};
use oxicode_core::commands::{self, CustomCommand};
use oxicode_core::config::{self, Config, PROVIDERS, UiStyle};
use oxicode_core::extensions::{self, ExtensionRegistry};
use oxicode_core::fox3d::{self, FoxState};
use oxicode_core::llm::{self, Message, Role};
use oxicode_core::mcp::McpManager;
use oxicode_core::session::{self, SavedSession};
use oxicode_core::theme::ThemeKind;
use oxicode_core::tools::{self, FileSnapshot};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use serde_json::json;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

const RICH_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LEGACY_SPINNER: &[&str] = &["|", "/", "-", "\\"];

#[derive(Debug, Clone, Copy)]
struct Glyphs {
    legacy_console: bool,
    spinner: &'static [&'static str],
    dot_ok: &'static str,
    dot_err: &'static str,
    assistant_mark: &'static str,
    user_bar: &'static str,
    changed_mark: &'static str,
    check_ok: &'static str,
    check_err: &'static str,
    list_cursor: &'static str,
    bullet: &'static str,
    ellipsis: &'static str,
    border: BorderType,
}

impl Glyphs {
    const fn rich() -> Self {
        Self {
            legacy_console: false,
            spinner: RICH_SPINNER,
            dot_ok: "●",
            dot_err: "●",
            assistant_mark: "  ◆ ",
            user_bar: " ▌ ",
            changed_mark: "↻",
            check_ok: "✓",
            check_err: "✗",
            list_cursor: " › ",
            bullet: "• ",
            ellipsis: "…",
            border: BorderType::Rounded,
        }
    }

    const fn legacy() -> Self {
        Self {
            legacy_console: true,
            spinner: LEGACY_SPINNER,
            dot_ok: "+",
            dot_err: "!",
            assistant_mark: "  * ",
            user_bar: " | ",
            changed_mark: "*",
            check_ok: "+",
            check_err: "x",
            list_cursor: " > ",
            bullet: "- ",
            ellipsis: "...",
            border: BorderType::Plain,
        }
    }

    fn detect() -> Self {
        if is_legacy_windows_console() {
            Self::legacy()
        } else {
            Self::rich()
        }
    }

    fn spin(self, idx: usize) -> &'static str {
        self.spinner[idx % self.spinner.len()]
    }
}

fn is_legacy_windows_console() -> bool {
    if !cfg!(windows) || cfg!(test) {
        return false;
    }
    let modern_vars = [
        "WT_SESSION",
        "TERM_PROGRAM",
        "ConEmuPID",
        "ALACRITTY_SOCKET",
        "WEZTERM_EXECUTABLE",
        "ANSICON",
    ];
    !modern_vars
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

fn sanitize_tabs(s: &str) -> String {
    if !s.contains('\t') && !s.contains('\r') {
        return s.to_string();
    }
    s.replace('\r', "").replace('\t', "  ")
}

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/connect", "Connect or configure an API provider"),
    ("/models", "Browse and switch available models"),
    ("/themes", "Switch TUI color theme (15 themes)"),
    ("/style", "Switch UI layout style (OxiCode / Antigravity / Logo)"),
    ("/mouse", "Toggle mouse capture (OFF = native text select/copy)"),
    ("/sessions", "Resume a saved conversation session"),
    ("/dir", "Change workspace / project directory"),
    ("/undo", "Revert last file edit/write (Ctrl+X u)"),
    ("/redo", "Reapply last reverted file edit (Ctrl+X r)"),
    ("/diff", "Show changed files summary card"),
    ("/compact", "Summarize & compact conversation context"),
    ("/skills", "Browse, create, or inject skills"),
    ("/plugins", "Manage plugins, rules, and custom tools"),
    ("/mcp", "View or reload MCP servers"),
    ("/export", "Export conversation to Markdown"),
    ("/copy", "Copy last assistant response to clipboard"),
    ("/mode", "Toggle BUILD / PLAN mode"),
    ("/auto", "Toggle auto-approve for edits & shell"),
    ("/settings", "Open unified settings screen (Ctrl+X o)"),
    ("/new", "Start a new session (clear chat)"),
    ("/clear", "Clear chat history"),
    ("/help", "Show keybindings and commands"),
    ("/exit", "Quit oxicode"),
];

enum ChatEntry {
    User(String),
    Thinking {
        text: String,
        started_at: Instant,
        duration_secs: Option<f32>,
    },
    Assistant(String),
    Tool {
        name: String,
        preview: String,
        output: Option<String>,
        is_error: bool,
    },
    System(String),
}

struct PendingApproval {
    name: String,
    preview: String,
    scroll: u16,
    responder: oneshot::Sender<ApprovalDecision>,
}

#[derive(Clone)]
enum Overlay {
    CommandPalette {
        filter: String,
        selected: usize,
    },
    ConnectProvider {
        selected: usize,
    },
    ConnectForm {
        provider_id: String,
        base_url: String,
        api_key: String,
        model: String,
        focus: usize,
    },
    ModelPicker {
        filter: String,
        selected: usize,
        models: Vec<String>,
        loading: bool,
        error: Option<String>,
    },
    ThemePicker {
        selected: usize,
        original: ThemeKind,
    },
    StylePicker {
        selected: usize,
        original: UiStyle,
        original_fox: bool,
    },
    DirPicker {
        path_input: String,
        error: Option<String>,
    },
    SessionPicker {
        sessions: Vec<SavedSession>,
        selected: usize,
    },
    McpStatus {
        servers: Vec<(String, usize, Option<String>)>,
        loading: bool,
    },
    Skills {
        selected: usize,
        status: Option<String>,
    },
    Plugins {
        selected: usize,
        status: Option<String>,
    },
    Settings {
        selected: usize,
    },
    Help,
}

enum UiAsyncEvent {
    ModelsLoaded(Result<Vec<String>, String>),
    InlineShellDone {
        cmd: String,
        output: String,
        is_error: bool,
    },
    CompactDone(Result<(Vec<Message>, String), String>),
    McpStatusLoaded(Vec<(String, usize, Option<String>)>),
    McpReloaded(McpManager, Vec<(String, usize, Option<String>)>),
}

struct AppState {
    cfg: Config,
    work_dir: PathBuf,
    git_branch: Option<String>,
    session_id: String,
    client: HttpClient,
    mcp: McpManager,
    extensions: ExtensionRegistry,
    custom_commands: Vec<CustomCommand>,
    mode: AgentMode,
    history: Vec<Message>,
    entries: Vec<ChatEntry>,
    undo_stack: Vec<FileSnapshot>,
    redo_stack: Vec<FileSnapshot>,
    changed_files: BTreeMap<String, (usize, usize)>,
    input: String,
    cursor: usize,
    prompt_history: Vec<String>,
    prompt_hist_idx: Option<usize>,
    scroll_from_bottom: u16,
    is_busy: bool,
    spinner_idx: usize,
    show_tool_details: bool,
    leader_pending: bool,
    last_context_tokens: u32,
    total_in_tokens: u32,
    total_out_tokens: u32,
    pending_approval: Option<PendingApproval>,
    overlay: Option<Overlay>,
    overlay_back: Vec<Overlay>,
    workspace_files: Vec<String>,
    ac_selected: usize,
    ac_dismissed: bool,
    active_task: Option<JoinHandle<()>>,
    fox: FoxState,
    fox_center: std::cell::Cell<(f32, f32)>,
    glyphs: Glyphs,
    term_size: (u16, u16),
    should_quit: bool,
}

impl AppState {
    fn new(
        cfg: Config,
        work_dir: PathBuf,
        mcp: McpManager,
        extensions: ExtensionRegistry,
        resume: Option<SavedSession>,
    ) -> Self {
        let workspace_files = tools::list_workspace_files(&work_dir, 500);
        let git_branch = session::read_git_branch(&work_dir);
        let custom_commands = extensions.custom_commands.clone();
        let initial_tool_details = cfg.show_tool_details;
        let mut state = Self {
            cfg,
            work_dir,
            git_branch,
            session_id: session::new_session_id(),
            client: oxicode_core::build_http_client(),
            mcp,
            extensions,
            custom_commands,
            mode: AgentMode::Build,
            history: Vec::new(),
            entries: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            changed_files: BTreeMap::new(),
            input: String::new(),
            cursor: 0,
            prompt_history: Vec::new(),
            prompt_hist_idx: None,
            scroll_from_bottom: 0,
            is_busy: false,
            spinner_idx: 0,
            show_tool_details: initial_tool_details,
            leader_pending: false,
            last_context_tokens: 0,
            total_in_tokens: 0,
            total_out_tokens: 0,
            pending_approval: None,
            overlay: None,
            overlay_back: Vec::new(),
            workspace_files,
            ac_selected: 0,
            ac_dismissed: false,
            active_task: None,
            fox: FoxState::default(),
            fox_center: std::cell::Cell::new((55.0, 10.0)),
            glyphs: Glyphs::detect(),
            term_size: (110, 32),
            should_quit: false,
        };
        if let Some(s) = resume {
            state.load_session(s);
        }
        state
    }

    fn load_session(&mut self, s: SavedSession) {
        self.session_id = s.id;
        self.history = s.messages.clone();
        self.entries.clear();
        for msg in &s.messages {
            match msg.role {
                Role::User => self.entries.push(ChatEntry::User(msg.content.clone())),
                Role::Assistant => {
                    if !msg.content.is_empty() {
                        self.entries.push(ChatEntry::Assistant(msg.content.clone()));
                    }
                    for tc in &msg.tool_calls {
                        self.entries.push(ChatEntry::Tool {
                            name: tc.name.clone(),
                            preview: tools::preview_tool(&tc.name, &tc.arguments),
                            output: None,
                            is_error: false,
                        });
                    }
                }
                Role::Tool => {
                    if let Some(ChatEntry::Tool { output, .. }) = self.entries.last_mut() {
                        *output = Some(msg.content.clone());
                    }
                }
                Role::System => {}
            }
        }
        self.scroll_from_bottom = 0;
        self.last_context_tokens = self.estimated_context_tokens();
        self.entries.push(ChatEntry::System(format!(
            "Resumed session: {}",
            s.title
        )));
    }

    fn estimated_context_tokens(&self) -> u32 {
        if self.last_context_tokens > 0 {
            return self.last_context_tokens;
        }
        let chars: usize = self.history.iter().map(|m| m.content.len() + 32).sum();
        (chars / 4) as u32
    }

    fn palette(&self) -> Palette {
        palette_for(self.cfg.theme)
    }

    fn save_cfg_note(&mut self) {
        if let Err(e) = self.cfg.save_global() {
            self.entries.push(ChatEntry::System(format!(
                "Warning: could not save config: {e}"
            )));
        }
    }

    fn set_work_dir(&mut self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim().trim_matches('"');
        if trimmed.is_empty() {
            return Err("Path cannot be empty".to_string());
        }
        let candidate = PathBuf::from(trimmed);
        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            self.work_dir.join(candidate)
        };
        if !resolved.exists() {
            return Err(format!("Directory does not exist: {}", resolved.display()));
        }
        if !resolved.is_dir() {
            return Err(format!("Not a directory: {}", resolved.display()));
        }
        let canonical = resolved
            .canonicalize()
            .map(|p| session::strip_unc_prefix(&p))
            .unwrap_or_else(|_| session::strip_unc_prefix(&resolved));
        self.work_dir = canonical.clone();
        self.git_branch = session::read_git_branch(&self.work_dir);
        self.workspace_files = tools::list_workspace_files(&self.work_dir, 500);
        self.reload_extensions();
        Ok(canonical)
    }

    fn reload_extensions(&mut self) {
        self.extensions = ExtensionRegistry::load(&self.work_dir, &self.cfg.disabled_plugins);
        self.custom_commands = self.extensions.custom_commands.clone();
    }

    fn open_model_picker(&mut self, ui_async_tx: &mpsc::UnboundedSender<UiAsyncEvent>) {
        self.overlay = Some(Overlay::ModelPicker {
            filter: String::new(),
            selected: 0,
            models: Vec::new(),
            loading: true,
            error: None,
        });
        let client = self.client.clone();
        let cfg = self.cfg.clone();
        let tx = ui_async_tx.clone();
        tokio::spawn(async move {
            let res = llm::fetch_models(&client, &cfg)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(UiAsyncEvent::ModelsLoaded(res));
        });
    }

    fn open_mcp_status(&mut self, ui_async_tx: &mpsc::UnboundedSender<UiAsyncEvent>) {
        self.overlay = Some(Overlay::McpStatus {
            servers: Vec::new(),
            loading: true,
        });
        let mcp = self.mcp.clone();
        let tx = ui_async_tx.clone();
        tokio::spawn(async move {
            let snap = mcp.status_snapshot().await;
            let _ = tx.send(UiAsyncEvent::McpStatusLoaded(snap));
        });
    }

    fn reload_mcp(&mut self, ui_async_tx: &mpsc::UnboundedSender<UiAsyncEvent>) {
        self.reload_extensions();
        self.overlay = Some(Overlay::McpStatus {
            servers: Vec::new(),
            loading: true,
        });
        let mut cfg = self.cfg.mcp_servers.clone();
        cfg.extend(self.extensions.extra_mcp.clone());
        let tx = ui_async_tx.clone();
        tokio::spawn(async move {
            let mgr = McpManager::connect_all(&cfg).await;
            let snap = mgr.status_snapshot().await;
            let _ = tx.send(UiAsyncEvent::McpReloaded(mgr, snap));
        });
    }

    fn open_connect(&mut self) {
        let idx = PROVIDERS
            .iter()
            .position(|p| p.id == self.cfg.provider)
            .unwrap_or(0);
        self.overlay = Some(Overlay::ConnectProvider { selected: idx });
    }

    fn open_themes(&mut self) {
        let idx = ThemeKind::all()
            .iter()
            .position(|t| *t == self.cfg.theme)
            .unwrap_or(0);
        self.overlay = Some(Overlay::ThemePicker {
            selected: idx,
            original: self.cfg.theme,
        });
    }

    fn open_style_picker(&mut self) {
        let idx = UiStyle::all()
            .iter()
            .position(|s| *s == self.cfg.ui_style)
            .unwrap_or(0);
        self.overlay = Some(Overlay::StylePicker {
            selected: idx,
            original: self.cfg.ui_style,
            original_fox: self.cfg.show_fox_mascot,
        });
    }

    fn apply_ui_style(&mut self, style: UiStyle) {
        self.cfg.ui_style = style;
        self.cfg.show_fox_mascot = !matches!(style, UiStyle::Logo);
        if !matches!(style, UiStyle::Oxicode) {
            // Static emblem / logo never tracks cursor or typing
            self.fox = FoxState::default();
        }
    }

    fn finish_sub_overlay(&mut self, chosen_model: Option<&str>) {
        if let Some(mut parent) = self.overlay_back.pop() {
            if let (Some(mdl), Overlay::ConnectForm { model, .. }) = (chosen_model, &mut parent) {
                *model = mdl.to_string();
            }
            self.overlay = Some(parent);
        } else {
            self.overlay = None;
        }
    }

    fn open_dir_picker(&mut self) {
        self.overlay = Some(Overlay::DirPicker {
            path_input: self.work_dir.display().to_string(),
            error: None,
        });
    }

    fn open_sessions(&mut self) {
        let sessions = session::list_sessions(25);
        self.overlay = Some(Overlay::SessionPicker {
            sessions,
            selected: 0,
        });
    }

    fn perform_undo(&mut self) {
        let Some(snap) = self.undo_stack.pop() else {
            self.entries
                .push(ChatEntry::System("Nothing to undo.".to_string()));
            return;
        };
        match tools::restore_snapshot(&snap) {
            Ok(msg) => {
                self.redo_stack.push(snap);
                self.workspace_files = tools::list_workspace_files(&self.work_dir, 500);
                self.entries.push(ChatEntry::System(msg));
            }
            Err(e) => {
                self.entries
                    .push(ChatEntry::System(format!("Undo error: {e}")));
            }
        }
    }

    fn perform_redo(&mut self) {
        let Some(snap) = self.redo_stack.pop() else {
            self.entries
                .push(ChatEntry::System("Nothing to redo.".to_string()));
            return;
        };
        match tools::reapply_snapshot(&snap) {
            Ok(msg) => {
                self.undo_stack.push(snap);
                self.workspace_files = tools::list_workspace_files(&self.work_dir, 500);
                self.entries.push(ChatEntry::System(msg));
            }
            Err(e) => {
                self.entries
                    .push(ChatEntry::System(format!("Redo error: {e}")));
            }
        }
    }

    fn start_compact(&mut self, ui_async_tx: &mpsc::UnboundedSender<UiAsyncEvent>) {
        if self.history.is_empty() {
            self.entries
                .push(ChatEntry::System("Nothing to compact.".to_string()));
            return;
        }
        self.is_busy = true;
        let client = self.client.clone();
        let cfg = self.cfg.clone();
        let history = self.history.clone();
        let tx = ui_async_tx.clone();
        tokio::spawn(async move {
            let res = agent::compact_history(&client, &cfg, &history)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(UiAsyncEvent::CompactDone(res));
        });
    }

    fn export_session(&mut self, arg: &str) {
        let mut md = format!(
            "# oxicode session\n\n- **Provider**: {} ({})\n- **Workspace**: `{}`\n\n---\n\n",
            self.cfg.active_spec().display_name,
            self.cfg.active().model,
            self.work_dir.display()
        );
        for entry in &self.entries {
            match entry {
                ChatEntry::User(t) => md.push_str(&format!("## User\n\n{t}\n\n")),
                ChatEntry::Assistant(t) => md.push_str(&format!("## Assistant\n\n{t}\n\n")),
                ChatEntry::Tool {
                    name,
                    preview,
                    output,
                    ..
                } => {
                    md.push_str(&format!("### Tool `{name}`\n\n```\n{preview}\n```\n\n"));
                    if let Some(o) = output {
                        md.push_str(&format!("```\n{o}\n```\n\n"));
                    }
                }
                _ => {}
            }
        }
        if !self.changed_files.is_empty() {
            md.push_str("## Changed Files\n\n");
            for (file, (a, r)) in &self.changed_files {
                md.push_str(&format!("- `{file}` (+{a} -{r})\n"));
            }
        }

        let target = if arg.trim().is_empty() {
            None
        } else {
            Some(arg)
        };
        match commands::export_markdown(&self.work_dir, target, &md) {
            Ok(path) => {
                self.entries.push(ChatEntry::System(format!(
                    "Exported conversation to {}",
                    path.display()
                )));
            }
            Err(e) => {
                self.entries
                    .push(ChatEntry::System(format!("Export failed: {e}")));
            }
        }
    }

    fn copy_last_response(&mut self) {
        let last = self
            .entries
            .iter()
            .rev()
            .find_map(|e| match e {
                ChatEntry::Assistant(t) if !t.trim().is_empty() => Some(t.as_str()),
                _ => None,
            })
            .map(str::to_string);

        match last {
            Some(text) => {
                if write_clipboard_text(&text) {
                    self.entries.push(ChatEntry::System(format!(
                        "Copied {} chars to clipboard.",
                        text.chars().count()
                    )));
                } else {
                    self.entries
                        .push(ChatEntry::System("Failed to write to clipboard.".to_string()));
                }
            }
            None => {
                self.entries
                    .push(ChatEntry::System("No assistant response to copy.".to_string()));
            }
        }
    }

    fn aim_fox_at_input(&mut self) {
        if !matches!(self.cfg.ui_style, UiStyle::Oxicode)
            || !self.entries.is_empty()
            || self.overlay.is_some()
        {
            return;
        }
        if self.input.is_empty() {
            self.fox.look_at(0.0, 0.0);
            return;
        }
        let hero_w = self.term_size.0.clamp(40, 86);
        let inner_w = hero_w.saturating_sub(2).max(20) as usize;
        let mut col = 0usize;
        for ch in self.input[..self.cursor].chars() {
            if ch == '\n' || col >= inner_w {
                col = if ch == '\n' { 0 } else { 1 };
            } else {
                col += 1;
            }
        }
        let half = inner_w as f32 * 0.5;
        let dx = (col as f32 - half) / (half * 0.85);
        self.fox.look_at(dx, 0.12);
    }

    fn active_at_query(&self) -> Option<(usize, String)> {
        let before = &self.input[..self.cursor];
        let at_pos = before.rfind('@')?;
        if at_pos > 0 && !before[..at_pos].ends_with(char::is_whitespace) {
            return None;
        }
        let query = &before[at_pos + 1..];
        if query.contains(char::is_whitespace) {
            return None;
        }
        Some((at_pos, query.to_string()))
    }

    fn filtered_at_files(&self) -> Vec<String> {
        if self.ac_dismissed {
            return Vec::new();
        }
        let Some((_, query)) = self.active_at_query() else {
            return Vec::new();
        };
        let q = query.to_lowercase();
        self.workspace_files
            .iter()
            .filter(|f| q.is_empty() || f.to_lowercase().contains(&q))
            .take(8)
            .cloned()
            .collect()
    }

    fn all_slash_entries(&self) -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = SLASH_COMMANDS
            .iter()
            .map(|(c, d)| (c.to_string(), d.to_string()))
            .collect();
        for cc in &self.custom_commands {
            all.push((cc.slash.clone(), format!("[custom] {}", cc.description)));
        }
        all
    }

    fn filtered_slash_cmds(&self) -> Vec<(String, String)> {
        if self.ac_dismissed || !self.input.starts_with('/') || self.input.contains(' ') {
            return Vec::new();
        }
        let q = self.input.to_lowercase();
        self.all_slash_entries()
            .into_iter()
            .filter(|(cmd, _)| cmd.starts_with(&q))
            .collect()
    }

    fn insert_text_at_cursor(&mut self, text: &str) {
        let clean = text.replace("\r\n", "\n").replace('\r', "\n");
        self.input.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        self.ac_selected = 0;
        self.ac_dismissed = false;
    }

    fn delete_char_forward(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
            self.ac_selected = 0;
            self.ac_dismissed = false;
        }
    }

    fn delete_prev_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let new_cur = prev_word_boundary(&self.input, self.cursor);
        self.input.drain(new_cur..self.cursor);
        self.cursor = new_cur;
        self.ac_selected = 0;
        self.ac_dismissed = false;
    }

    fn move_word_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = prev_word_boundary(&self.input, self.cursor);
        }
    }

    fn move_word_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = next_word_boundary(&self.input, self.cursor);
        }
    }
}

fn prev_word_boundary(s: &str, cursor: usize) -> usize {
    let mut idx = cursor.min(s.len());
    while idx > 0 {
        let Some((pos, ch)) = s[..idx].char_indices().last() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        idx = pos;
    }
    while idx > 0 {
        let Some((pos, ch)) = s[..idx].char_indices().last() else {
            break;
        };
        if ch.is_whitespace() {
            break;
        }
        idx = pos;
    }
    idx
}

fn next_word_boundary(s: &str, cursor: usize) -> usize {
    let mut idx = cursor.min(s.len());
    while idx < s.len() {
        let Some(ch) = s[idx..].chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            break;
        }
        idx += ch.len_utf8();
    }
    while idx < s.len() {
        let Some(ch) = s[idx..].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        idx += ch.len_utf8();
    }
    idx
}

fn read_clipboard_text() -> Option<String> {
    for attempt in 0..3 {
        if let Ok(mut cb) = arboard::Clipboard::new()
            && let Ok(text) = cb.get_text()
        {
            if !text.is_empty() {
                return Some(text);
            }
            return None;
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[cfg(windows)]
    {
        if let Ok(out) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Get-Clipboard -Raw"])
            .output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn write_clipboard_text(text: &str) -> bool {
    if let Ok(mut cb) = arboard::Clipboard::new()
        && cb.set_text(text.to_string()).is_ok()
    {
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    cfg: Config,
    work_dir: PathBuf,
    mcp: McpManager,
    extensions: ExtensionRegistry,
    resume: Option<SavedSession>,
    initial_mode: AgentMode,
    initial_prompt: Option<String>,
    open_settings: bool,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if cfg.mouse_capture {
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
    } else {
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_loop(
        &mut terminal,
        cfg,
        work_dir,
        mcp,
        extensions,
        resume,
        initial_mode,
        initial_prompt,
        open_settings,
    )
    .await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    res
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cfg: Config,
    work_dir: PathBuf,
    mcp: McpManager,
    extensions: ExtensionRegistry,
    resume: Option<SavedSession>,
    initial_mode: AgentMode,
    initial_prompt: Option<String>,
    open_settings: bool,
) -> Result<()> {
    let mut state = AppState::new(cfg, work_dir, mcp, extensions, resume);
    state.mode = initial_mode;
    if open_settings {
        state.overlay = Some(Overlay::Settings { selected: 0 });
    }
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<(Vec<Message>, bool)>();
    let (async_tx, mut async_rx) = mpsc::unbounded_channel::<UiAsyncEvent>();
    if let Some(prompt) = initial_prompt
        && !prompt.trim().is_empty()
    {
        let clean = prompt.trim().to_string();
        state.prompt_history.push(clean.clone());
        if clean.starts_with('/') {
            execute_slash_command(&mut state, &clean, &agent_tx, &done_tx, &async_tx);
        } else {
            let expanded = expand_at_references(&state.work_dir, &clean);
            spawn_agent_turn(&mut state, clean, expanded, &agent_tx, &done_tx);
        }
    }
    let mut term_events = event::EventStream::new();
    let mut spinner_tick = tokio::time::interval(Duration::from_millis(120));
    let mut fox_tick = tokio::time::interval(Duration::from_millis(33));
    if let Ok(sz) = terminal.size() {
        state.term_size = (sz.width, sz.height);
    }
    let mut needs_draw = true;
    let mut mouse_capture_applied = state.cfg.mouse_capture;

    loop {
        if state.cfg.mouse_capture != mouse_capture_applied {
            if state.cfg.mouse_capture {
                let _ = execute!(terminal.backend_mut(), EnableMouseCapture);
            } else {
                let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
            }
            mouse_capture_applied = state.cfg.mouse_capture;
        }
        if needs_draw {
            terminal.draw(|f| draw_ui(f, &state))?;
            needs_draw = false;
        }

        if state.should_quit {
            if let Some(handle) = state.active_task.take() {
                handle.abort();
            }
            break;
        }

        tokio::select! {
            _ = spinner_tick.tick(), if state.is_busy => {
                state.spinner_idx = (state.spinner_idx + 1) % state.glyphs.spinner.len();
                needs_draw = true;
            }
            _ = fox_tick.tick(), if state.entries.is_empty()
                && state.overlay.is_none()
                && matches!(state.cfg.ui_style, UiStyle::Oxicode) => {
                if state.fox.step(0.033, state.is_busy) {
                    needs_draw = true;
                }
            }
            Some(Ok(ev)) = term_events.next() => {
                if let Event::Resize(w, h) = ev {
                    state.term_size = (w, h);
                }
                let is_move = matches!(ev, Event::Mouse(m) if matches!(m.kind, MouseEventKind::Moved));
                handle_term_event(&mut state, ev, &agent_tx, &done_tx, &async_tx);
                if !is_move {
                    needs_draw = true;
                }
            }
            Some(ev) = agent_rx.recv() => {
                handle_agent_event(&mut state, ev);
                needs_draw = true;
            }
            Some((new_history, auto_approve)) = done_rx.recv() => {
                state.history = new_history;
                state.cfg.auto_approve = auto_approve;
                state.is_busy = false;
                state.active_task = None;
                let _ = session::save_session(
                    &state.session_id,
                    &state.work_dir,
                    &state.cfg.provider,
                    &state.cfg.active().model,
                    &state.history,
                );
                needs_draw = true;
            }
            Some(uiev) = async_rx.recv() => {
                handle_ui_async(&mut state, uiev);
                needs_draw = true;
            }
        }
    }

    Ok(())
}

fn handle_ui_async(state: &mut AppState, ev: UiAsyncEvent) {
    match ev {
        UiAsyncEvent::ModelsLoaded(res) => {
            if let Some(Overlay::ModelPicker {
                models,
                loading,
                error,
                selected,
                ..
            }) = state.overlay.as_mut()
            {
                *loading = false;
                match res {
                    Ok(list) => {
                        *models = list;
                        *selected = 0;
                    }
                    Err(e) => {
                        *error = Some(e);
                    }
                }
            }
        }
        UiAsyncEvent::InlineShellDone {
            cmd,
            output,
            is_error,
        } => {
            state.entries.push(ChatEntry::Tool {
                name: "shell".to_string(),
                preview: format!("$ {cmd}"),
                output: Some(output),
                is_error,
            });
            state.is_busy = false;
        }
        UiAsyncEvent::CompactDone(res) => {
            state.is_busy = false;
            match res {
                Ok((new_history, summary)) => {
                    state.history = new_history;
                    state.last_context_tokens = 0;
                    state.last_context_tokens = state.estimated_context_tokens();
                    state.entries.clear();
                    state.entries.push(ChatEntry::System(format!(
                        "Conversation compacted (~{} context tokens remaining).",
                        state.last_context_tokens
                    )));
                    state.entries.push(ChatEntry::Assistant(summary));
                }
                Err(e) => {
                    state
                        .entries
                        .push(ChatEntry::System(format!("Compact failed: {e}")));
                }
            }
        }
        UiAsyncEvent::McpStatusLoaded(servers) => {
            if let Some(Overlay::McpStatus {
                servers: list,
                loading,
            }) = state.overlay.as_mut()
            {
                *list = servers;
                *loading = false;
            }
        }
        UiAsyncEvent::McpReloaded(mgr, servers) => {
            state.mcp = mgr;
            if let Some(Overlay::McpStatus {
                servers: list,
                loading,
            }) = state.overlay.as_mut()
            {
                *list = servers;
                *loading = false;
            }
        }
    }
}

fn paste_into_active_target(state: &mut AppState, pasted: &str) {
    let single_line = pasted.replace(['\r', '\n'], "").trim().to_string();
    if let Some(overlay) = state.overlay.as_mut() {
        match overlay {
            Overlay::CommandPalette { filter, selected } => {
                filter.push_str(&single_line);
                *selected = 0;
            }
            Overlay::ConnectForm {
                base_url,
                api_key,
                model,
                focus,
                ..
            } => {
                let target = match *focus {
                    0 => api_key,
                    1 => base_url,
                    _ => model,
                };
                target.push_str(&single_line);
            }
            Overlay::ModelPicker {
                filter, selected, ..
            } => {
                filter.push_str(&single_line);
                *selected = 0;
            }
            Overlay::DirPicker {
                path_input, error, ..
            } => {
                path_input.push_str(&single_line);
                *error = None;
            }
            _ => {}
        }
        return;
    }

    if !state.is_busy && state.pending_approval.is_none() {
        state.insert_text_at_cursor(pasted);
        state.aim_fox_at_input();
    }
}

fn is_ctrl_char(key: &KeyEvent, latin: char, cyrillic: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && match key.code {
            KeyCode::Char(c) => {
                c.eq_ignore_ascii_case(&latin) || c.to_lowercase().next() == Some(cyrillic)
            }
            _ => false,
        }
}

fn is_paste_shortcut(key: &KeyEvent) -> bool {
    is_ctrl_char(key, 'v', 'м')
        || (key.modifiers.contains(KeyModifiers::SHIFT) && key.code == KeyCode::Insert)
        || matches!(key.code, KeyCode::Char('\x16'))
}

fn handle_term_event(
    state: &mut AppState,
    ev: Event,
    agent_tx: &mpsc::UnboundedSender<AgentEvent>,
    done_tx: &mpsc::UnboundedSender<(Vec<Message>, bool)>,
    async_tx: &mpsc::UnboundedSender<UiAsyncEvent>,
) {
    match ev {
        Event::Paste(text) => {
            paste_into_active_target(state, &text);
        }
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollUp => {
                if let Some(pending) = state.pending_approval.as_mut() {
                    pending.scroll = pending.scroll.saturating_sub(2);
                } else {
                    state.scroll_from_bottom = state.scroll_from_bottom.saturating_add(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(pending) = state.pending_approval.as_mut() {
                    pending.scroll = pending.scroll.saturating_add(2);
                } else {
                    state.scroll_from_bottom = state.scroll_from_bottom.saturating_sub(3);
                }
            }
            MouseEventKind::Moved => {
                if matches!(state.cfg.ui_style, UiStyle::Oxicode) {
                    let (tw, th) = state.term_size;
                    let (fcx, fcy) = state.fox_center.get();
                    let dx = (m.column as f32 - fcx) / (tw.max(1) as f32 * 0.40);
                    let dy = (m.row as f32 - fcy) / (th.max(1) as f32 * 0.45);
                    state.fox.look_at(dx, dy);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(clip) = read_clipboard_text() {
                    paste_into_active_target(state, &clip);
                }
            }
            _ => {}
        },
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if is_ctrl_char(&key, 'c', 'с') || is_ctrl_char(&key, 'd', 'в') {
                state.should_quit = true;
                return;
            }

            if is_paste_shortcut(&key) {
                if let Some(clip) = read_clipboard_text() {
                    paste_into_active_target(state, &clip);
                }
                return;
            }

            // Leader key (Ctrl+X)
            if state.leader_pending {
                state.leader_pending = false;
                match key.code {
                    KeyCode::Char('c') | KeyCode::Char('с') => {
                        state.overlay_back.clear();
                        state.open_connect();
                    }
                    KeyCode::Char('m') | KeyCode::Char('ь') => {
                        state.overlay_back.clear();
                        state.open_model_picker(async_tx);
                    }
                    KeyCode::Char('t') | KeyCode::Char('е') => {
                        state.overlay_back.clear();
                        state.open_themes();
                    }
                    KeyCode::Char('w') | KeyCode::Char('ц') => {
                        state.overlay_back.clear();
                        state.open_dir_picker();
                    }
                    KeyCode::Char('s') | KeyCode::Char('ы') => {
                        state.overlay_back.clear();
                        state.open_sessions();
                    }
                    KeyCode::Char('u') | KeyCode::Char('г') => state.perform_undo(),
                    KeyCode::Char('r') | KeyCode::Char('к') => state.perform_redo(),
                    KeyCode::Char('k') | KeyCode::Char('л') => state.start_compact(async_tx),
                    KeyCode::Char('e') | KeyCode::Char('у') => state.export_session(""),
                    KeyCode::Char('y') | KeyCode::Char('н') => state.copy_last_response(),
                    KeyCode::Char('p') | KeyCode::Char('з') => {
                        state.overlay_back.clear();
                        state.open_mcp_status(async_tx);
                    }
                    KeyCode::Char('n') | KeyCode::Char('т') => {
                        state.history.clear();
                        state.entries.clear();
                        state.undo_stack.clear();
                        state.redo_stack.clear();
                        state.changed_files.clear();
                        state.last_context_tokens = 0;
                        state.session_id = session::new_session_id();
                        state.scroll_from_bottom = 0;
                    }
                    KeyCode::Char('a') | KeyCode::Char('ф') => {
                        state.cfg.auto_approve = !state.cfg.auto_approve;
                        state.save_cfg_note();
                    }
                    KeyCode::Char('d') | KeyCode::Char('в') => {
                        state.show_tool_details = !state.show_tool_details;
                        state.cfg.show_tool_details = state.show_tool_details;
                        let _ = state.cfg.save_global();
                    }
                    KeyCode::Char('o') | KeyCode::Char('щ') => {
                        state.overlay_back.clear();
                        state.overlay = Some(Overlay::Settings { selected: 0 });
                    }
                    KeyCode::Char('h') | KeyCode::Char('р') => {
                        state.overlay_back.clear();
                        state.overlay = Some(Overlay::Help);
                    }
                    KeyCode::Char('q') | KeyCode::Char('й') => state.should_quit = true,
                    _ => {}
                }
                return;
            }

            if is_ctrl_char(&key, 'x', 'ч') {
                state.leader_pending = true;
                return;
            }

            if is_ctrl_char(&key, 'p', 'з') {
                state.overlay_back.clear();
                state.overlay = Some(Overlay::CommandPalette {
                    filter: String::new(),
                    selected: 0,
                });
                return;
            }

            if is_ctrl_char(&key, 'o', 'щ') {
                state.show_tool_details = !state.show_tool_details;
                state.cfg.show_tool_details = state.show_tool_details;
                let _ = state.cfg.save_global();
                return;
            }

            if state.overlay.is_some() {
                handle_overlay_key(state, key, agent_tx, done_tx, async_tx);
                return;
            }

            // Smart scrollable tool approval
            if let Some(mut pending) = state.pending_approval.take() {
                match key.code {
                    KeyCode::Up | KeyCode::PageUp => {
                        pending.scroll = pending.scroll.saturating_sub(2);
                        state.pending_approval = Some(pending);
                        return;
                    }
                    KeyCode::Down | KeyCode::PageDown => {
                        pending.scroll = pending.scroll.saturating_add(2);
                        state.pending_approval = Some(pending);
                        return;
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        let _ = pending.responder.send(ApprovalDecision::ApproveOnce);
                        return;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        let _ = pending.responder.send(ApprovalDecision::ApproveAll);
                        state.cfg.auto_approve = true;
                        return;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        let _ = pending.responder.send(ApprovalDecision::Reject);
                        return;
                    }
                    _ => {
                        state.pending_approval = Some(pending);
                        return;
                    }
                }
            }

            if key.code == KeyCode::Esc && state.is_busy {
                if let Some(handle) = state.active_task.take() {
                    handle.abort();
                }
                if let Some(ChatEntry::Assistant(partial)) = state.entries.last()
                    && !partial.trim().is_empty()
                {
                    state.history.push(Message::assistant(partial.clone(), Vec::new()));
                }
                agent::sanitize_history(&mut state.history);
                state.is_busy = false;
                state
                    .entries
                    .push(ChatEntry::System("Generation interrupted.".to_string()));
                return;
            }

            let at_files = state.filtered_at_files();
            let slash_cmds = state.filtered_slash_cmds();
            let ac_len = if !at_files.is_empty() {
                at_files.len()
            } else {
                slash_cmds.len()
            };

            if ac_len > 0 {
                match key.code {
                    KeyCode::Esc => {
                        state.ac_dismissed = true;
                        return;
                    }
                    KeyCode::Tab => {
                        let idx = state.ac_selected.min(ac_len - 1);
                        if !at_files.is_empty() {
                            if let Some((at_pos, _)) = state.active_at_query() {
                                let chosen = &at_files[idx];
                                state.input.replace_range(
                                    at_pos + 1..state.cursor,
                                    &format!("{chosen} "),
                                );
                                state.cursor = at_pos + 1 + chosen.len() + 1;
                                state.ac_selected = 0;
                            }
                        } else if !slash_cmds.is_empty() {
                            state.input = format!("{} ", slash_cmds[idx].0);
                            state.cursor = state.input.len();
                            state.ac_selected = 0;
                        }
                        return;
                    }
                    KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                        let idx = state.ac_selected.min(ac_len - 1);
                        if !at_files.is_empty() {
                            if let Some((at_pos, _)) = state.active_at_query() {
                                let chosen = &at_files[idx];
                                state.input.replace_range(
                                    at_pos + 1..state.cursor,
                                    &format!("{chosen} "),
                                );
                                state.cursor = at_pos + 1 + chosen.len() + 1;
                                state.ac_selected = 0;
                            }
                            return;
                        }
                        if !slash_cmds.is_empty() {
                            let chosen_cmd = slash_cmds[idx].0.clone();
                            state.input.clear();
                            state.cursor = 0;
                            state.ac_selected = 0;
                            execute_slash_command(state, &chosen_cmd, agent_tx, done_tx, async_tx);
                            return;
                        }
                    }
                    KeyCode::Up => {
                        state.ac_selected = state.ac_selected.saturating_sub(1);
                        return;
                    }
                    KeyCode::Down => {
                        state.ac_selected = (state.ac_selected + 1).min(ac_len - 1);
                        return;
                    }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Tab => {
                    state.mode = state.mode.toggle();
                }
                KeyCode::BackTab => {
                    state.cfg.auto_approve = !state.cfg.auto_approve;
                    state.save_cfg_note();
                }
                KeyCode::PageUp => {
                    state.scroll_from_bottom = state.scroll_from_bottom.saturating_add(8);
                }
                KeyCode::PageDown => {
                    state.scroll_from_bottom = state.scroll_from_bottom.saturating_sub(8);
                }
                KeyCode::Up if key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) => {
                    state.scroll_from_bottom = state.scroll_from_bottom.saturating_add(3);
                }
                KeyCode::Down if key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) => {
                    state.scroll_from_bottom = state.scroll_from_bottom.saturating_sub(3);
                }
                KeyCode::Up => {
                    if !state.prompt_history.is_empty() {
                        let next_idx = match state.prompt_hist_idx {
                            None => state.prompt_history.len() - 1,
                            Some(i) => i.saturating_sub(1),
                        };
                        state.prompt_hist_idx = Some(next_idx);
                        state.input = state.prompt_history[next_idx].clone();
                        state.cursor = state.input.len();
                        state.ac_dismissed = false;
                    }
                }
                KeyCode::Down => {
                    if let Some(i) = state.prompt_hist_idx {
                        if i + 1 < state.prompt_history.len() {
                            state.prompt_hist_idx = Some(i + 1);
                            state.input = state.prompt_history[i + 1].clone();
                        } else {
                            state.prompt_hist_idx = None;
                            state.input.clear();
                        }
                        state.cursor = state.input.len();
                        state.ac_dismissed = false;
                    }
                }
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    state.insert_text_at_cursor("\n");
                }
                KeyCode::Enter => {
                    submit_input(state, agent_tx, done_tx, async_tx);
                }
                KeyCode::Backspace => {
                    if state.cursor > 0 {
                        let prev = state.input[..state.cursor]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        state.input.remove(prev);
                        state.cursor = prev;
                        state.ac_selected = 0;
                        state.ac_dismissed = false;
                    }
                }
                KeyCode::Delete => {
                    state.delete_char_forward();
                }
                KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.move_word_left();
                }
                KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.move_word_right();
                }
                KeyCode::Left => {
                    if state.cursor > 0 {
                        state.cursor = state.input[..state.cursor]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                    }
                }
                KeyCode::Right => {
                    if state.cursor < state.input.len()
                        && let Some(ch) = state.input[state.cursor..].chars().next()
                    {
                        state.cursor += ch.len_utf8();
                    }
                }
                KeyCode::Home => state.cursor = 0,
                KeyCode::End => state.cursor = state.input.len(),
                KeyCode::Char(_) if is_ctrl_char(&key, 'w', 'ц') => {
                    state.delete_prev_word();
                }
                KeyCode::Char(_) if is_ctrl_char(&key, 'u', 'г') => {
                    state.input.clear();
                    state.cursor = 0;
                    state.ac_selected = 0;
                    state.ac_dismissed = false;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.input.insert(state.cursor, c);
                    state.cursor += c.len_utf8();
                    state.ac_selected = 0;
                    state.ac_dismissed = false;
                }
                _ => {}
            }
            state.aim_fox_at_input();
        }
        _ => {}
    }
}

fn handle_overlay_key(
    state: &mut AppState,
    key: KeyEvent,
    agent_tx: &mpsc::UnboundedSender<AgentEvent>,
    done_tx: &mpsc::UnboundedSender<(Vec<Message>, bool)>,
    async_tx: &mpsc::UnboundedSender<UiAsyncEvent>,
) {
    if key.code == KeyCode::Esc {
        match state.overlay.take() {
            Some(Overlay::ThemePicker { original, .. }) => {
                state.cfg.theme = original;
            }
            Some(Overlay::StylePicker {
                original,
                original_fox,
                ..
            }) => {
                state.cfg.ui_style = original;
                state.cfg.show_fox_mascot = original_fox;
            }
            _ => {}
        }
        state.overlay = state.overlay_back.pop();
        return;
    }

    let Some(mut overlay) = state.overlay.take() else {
        return;
    };

    match &mut overlay {
        Overlay::Help => {
            if matches!(key.code, KeyCode::Enter | KeyCode::Char('q')) {
                return;
            }
            state.overlay = Some(overlay);
        }
        Overlay::McpStatus { .. } => {
            if matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R')) {
                state.reload_mcp(async_tx);
                return;
            }
            state.overlay = Some(overlay);
        }
        Overlay::CommandPalette { filter, selected } => {
            let matches: Vec<String> = state
                .all_slash_entries()
                .into_iter()
                .map(|(c, _)| c)
                .filter(|c| filter.is_empty() || c.contains(&filter.to_lowercase()))
                .collect();

            match key.code {
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                    state.overlay = Some(overlay);
                }
                KeyCode::Down => {
                    if !matches.is_empty() {
                        *selected = (*selected + 1).min(matches.len() - 1);
                    }
                    state.overlay = Some(overlay);
                }
                KeyCode::Enter => {
                    if !matches.is_empty() {
                        let idx = (*selected).min(matches.len() - 1);
                        let cmd = matches[idx].clone();
                        execute_slash_command(state, &cmd, agent_tx, done_tx, async_tx);
                    }
                }
                KeyCode::Backspace => {
                    filter.pop();
                    *selected = 0;
                    state.overlay = Some(overlay);
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    filter.push(c);
                    *selected = 0;
                    state.overlay = Some(overlay);
                }
                _ => state.overlay = Some(overlay),
            }
        }
        Overlay::ConnectProvider { selected } => match key.code {
            KeyCode::Up => {
                *selected = selected.saturating_sub(1);
                state.overlay = Some(overlay);
            }
            KeyCode::Down => {
                *selected = (*selected + 1).min(PROVIDERS.len() - 1);
                state.overlay = Some(overlay);
            }
            KeyCode::Enter => {
                let spec = PROVIDERS[*selected];
                let existing = state.cfg.providers.get(spec.id);
                state.overlay_back.push(Overlay::ConnectProvider {
                    selected: *selected,
                });
                state.overlay = Some(Overlay::ConnectForm {
                    provider_id: spec.id.to_string(),
                    base_url: existing
                        .map(|p| p.base_url.clone())
                        .unwrap_or_else(|| spec.default_base_url.to_string()),
                    api_key: existing.map(|p| p.api_key.clone()).unwrap_or_default(),
                    model: existing
                        .map(|p| p.model.clone())
                        .unwrap_or_else(|| spec.default_model.to_string()),
                    focus: if spec.requires_key { 0 } else { 1 },
                });
            }
            _ => state.overlay = Some(overlay),
        },
        Overlay::ConnectForm {
            provider_id,
            base_url,
            api_key,
            model,
            focus,
        } => match key.code {
            KeyCode::Tab | KeyCode::Down => {
                *focus = (*focus + 1) % 3;
                state.overlay = Some(overlay);
            }
            KeyCode::BackTab | KeyCode::Up => {
                *focus = (*focus + 2) % 3;
                state.overlay = Some(overlay);
            }
            KeyCode::Char(_) if is_ctrl_char(&key, 'u', 'г') => {
                let target = match *focus {
                    0 => api_key,
                    1 => base_url,
                    _ => model,
                };
                target.clear();
                state.overlay = Some(overlay);
            }
            KeyCode::Char(_) if is_ctrl_char(&key, 'm', 'ь') => {
                state.cfg.set_provider(provider_id);
                let entry = state.cfg.active_mut();
                entry.api_key = api_key.trim().to_string();
                if !base_url.trim().is_empty() {
                    entry.base_url = config::normalize_base_url(base_url);
                }
                state.save_cfg_note();
                state.overlay_back.push(overlay.clone());
                state.open_model_picker(async_tx);
            }
            KeyCode::Backspace => {
                let target = match *focus {
                    0 => api_key,
                    1 => base_url,
                    _ => model,
                };
                target.pop();
                state.overlay = Some(overlay);
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let target = match *focus {
                    0 => api_key,
                    1 => base_url,
                    _ => model,
                };
                target.push(c);
                state.overlay = Some(overlay);
            }
            KeyCode::Enter => {
                state.cfg.set_provider(provider_id);
                let entry = state.cfg.active_mut();
                entry.api_key = api_key.trim().to_string();
                if !base_url.trim().is_empty() {
                    entry.base_url = config::normalize_base_url(base_url);
                }
                if !model.trim().is_empty() {
                    entry.model = model.trim().to_string();
                }
                state.save_cfg_note();
                let disp = state.cfg.active_spec().display_name;
                let mdl = state.cfg.active().model.clone();
                state
                    .entries
                    .push(ChatEntry::System(format!("Connected {disp} ({mdl}).")));
                if model.trim().is_empty() {
                    state.overlay_back.push(overlay.clone());
                    state.open_model_picker(async_tx);
                } else {
                    while matches!(state.overlay_back.last(), Some(Overlay::ConnectProvider { .. }))
                    {
                        state.overlay_back.pop();
                    }
                    state.finish_sub_overlay(None);
                }
            }
            _ => state.overlay = Some(overlay),
        },
        Overlay::ModelPicker {
            filter,
            selected,
            models,
            loading,
            ..
        } => {
            let matches: Vec<String> = models
                .iter()
                .filter(|m| filter.is_empty() || m.to_lowercase().contains(&filter.to_lowercase()))
                .cloned()
                .collect();

            match key.code {
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                    state.overlay = Some(overlay);
                }
                KeyCode::Down => {
                    if !matches.is_empty() {
                        *selected = (*selected + 1).min(matches.len() - 1);
                    }
                    state.overlay = Some(overlay);
                }
                KeyCode::Enter => {
                    if !*loading
                        && let Some(chosen) =
                            matches.get((*selected).min(matches.len().saturating_sub(1)))
                    {
                        state.cfg.active_mut().model = chosen.clone();
                        state.save_cfg_note();
                        state
                            .entries
                            .push(ChatEntry::System(format!("Switched model to {chosen}")));
                        state.finish_sub_overlay(Some(chosen));
                    } else if !filter.trim().is_empty() {
                        let chosen = filter.trim().to_string();
                        state.cfg.active_mut().model = chosen.clone();
                        state.save_cfg_note();
                        state
                            .entries
                            .push(ChatEntry::System(format!("Switched model to {chosen}")));
                        state.finish_sub_overlay(Some(&chosen));
                    } else {
                        state.overlay = Some(overlay);
                    }
                }
                KeyCode::Backspace => {
                    filter.pop();
                    *selected = 0;
                    state.overlay = Some(overlay);
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    filter.push(c);
                    *selected = 0;
                    state.overlay = Some(overlay);
                }
                _ => state.overlay = Some(overlay),
            }
        }
        Overlay::ThemePicker { selected, original: _ } => {
            let themes = ThemeKind::all();
            match key.code {
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                    state.cfg.theme = themes[*selected];
                    state.overlay = Some(overlay);
                }
                KeyCode::Down => {
                    *selected = (*selected + 1).min(themes.len() - 1);
                    state.cfg.theme = themes[*selected];
                    state.overlay = Some(overlay);
                }
                KeyCode::Enter => {
                    state.cfg.theme = themes[*selected];
                    state.save_cfg_note();
                    state.finish_sub_overlay(None);
                }
                _ => state.overlay = Some(overlay),
            }
        }
        Overlay::StylePicker {
            selected,
            original: _,
            original_fox: _,
        } => {
            let styles = UiStyle::all();
            match key.code {
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                    state.apply_ui_style(styles[*selected]);
                    state.overlay = Some(overlay);
                }
                KeyCode::Down => {
                    *selected = (*selected + 1).min(styles.len() - 1);
                    state.apply_ui_style(styles[*selected]);
                    state.overlay = Some(overlay);
                }
                KeyCode::Enter => {
                    state.apply_ui_style(styles[*selected]);
                    state.save_cfg_note();
                    state.finish_sub_overlay(None);
                }
                _ => state.overlay = Some(overlay),
            }
        }
        Overlay::DirPicker { path_input, error } => match key.code {
            KeyCode::Char(_) if is_ctrl_char(&key, 'u', 'г') => {
                path_input.clear();
                *error = None;
                state.overlay = Some(overlay);
            }
            KeyCode::Backspace => {
                path_input.pop();
                *error = None;
                state.overlay = Some(overlay);
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                path_input.push(c);
                *error = None;
                state.overlay = Some(overlay);
            }
            KeyCode::Enter => {
                let target = path_input.clone();
                match state.set_work_dir(&target) {
                    Ok(dir) => {
                        state.entries.push(ChatEntry::System(format!(
                            "Workspace switched to {} ({} files, {} custom cmds)",
                            dir.display(),
                            state.workspace_files.len(),
                            state.custom_commands.len()
                        )));
                        state.finish_sub_overlay(None);
                    }
                    Err(e) => {
                        *error = Some(e);
                        state.overlay = Some(overlay);
                    }
                }
            }
            _ => state.overlay = Some(overlay),
        },
        Overlay::SessionPicker { sessions, selected } => match key.code {
            KeyCode::Up => {
                *selected = selected.saturating_sub(1);
                state.overlay = Some(overlay);
            }
            KeyCode::Down => {
                if !sessions.is_empty() {
                    *selected = (*selected + 1).min(sessions.len() - 1);
                }
                state.overlay = Some(overlay);
            }
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                if *selected < sessions.len() {
                    let removed = sessions.remove(*selected);
                    let _ = session::delete_session(&removed.id);
                    if !sessions.is_empty() {
                        *selected = (*selected).min(sessions.len() - 1);
                    } else {
                        *selected = 0;
                    }
                }
                state.overlay = Some(overlay);
            }
            KeyCode::Enter => {
                if let Some(chosen) = sessions.get(*selected).cloned() {
                    state.load_session(chosen);
                }
            }
            _ => state.overlay = Some(overlay),
        },
        Overlay::Skills { selected, status } => match key.code {
            KeyCode::Up => {
                *selected = selected.saturating_sub(1);
                state.overlay = Some(overlay);
            }
            KeyCode::Down => {
                if !state.extensions.skills.is_empty() {
                    *selected = (*selected + 1).min(state.extensions.skills.len() - 1);
                }
                state.overlay = Some(overlay);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                match extensions::scaffold_starter_skill(&state.work_dir, "custom-skill") {
                    Ok(p) => {
                        state.reload_extensions();
                        *status = Some(format!("Created {}", p.display()));
                    }
                    Err(e) => *status = Some(format!("Error: {e}")),
                }
                state.overlay = Some(overlay);
            }
            KeyCode::Enter => {
                if let Some(sk) = state.extensions.skills.get(*selected).cloned() {
                    let note = format!("Use skill `{}`: ", sk.name);
                    state.insert_text_at_cursor(&note);
                }
            }
            _ => state.overlay = Some(overlay),
        },
        Overlay::Plugins { selected, status } => match key.code {
            KeyCode::Up => {
                *selected = selected.saturating_sub(1);
                state.overlay = Some(overlay);
            }
            KeyCode::Down => {
                if !state.extensions.plugins.is_empty() {
                    *selected = (*selected + 1).min(state.extensions.plugins.len() - 1);
                }
                state.overlay = Some(overlay);
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(pl) = state.extensions.plugins.get(*selected).cloned() {
                    if pl.enabled {
                        if !state.cfg.disabled_plugins.contains(&pl.id) {
                            state.cfg.disabled_plugins.push(pl.id.clone());
                        }
                    } else {
                        state.cfg.disabled_plugins.retain(|d| d != &pl.id);
                    }
                    state.save_cfg_note();
                    state.reload_extensions();
                    *status = Some(format!(
                        "{} {}",
                        pl.name,
                        if pl.enabled { "disabled" } else { "enabled" }
                    ));
                }
                state.overlay = Some(overlay);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                match extensions::scaffold_starter_plugin(&state.work_dir, "custom-plugin") {
                    Ok(p) => {
                        state.reload_extensions();
                        *status = Some(format!("Created {}", p.display()));
                    }
                    Err(e) => *status = Some(format!("Error: {e}")),
                }
                state.overlay = Some(overlay);
            }
            _ => state.overlay = Some(overlay),
        },
        Overlay::Settings { selected } => {
            const SETTINGS_COUNT: usize = 12;
            match key.code {
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                    state.overlay = Some(overlay);
                }
                KeyCode::Down => {
                    *selected = (*selected + 1).min(SETTINGS_COUNT - 1);
                    state.overlay = Some(overlay);
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => {
                    let step_back = key.code == KeyCode::Left;
                    match *selected {
                        0 if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                            state.overlay_back.push(Overlay::Settings {
                                selected: *selected,
                            });
                            state.open_connect();
                        }
                        1 if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                            state.overlay_back.push(Overlay::Settings {
                                selected: *selected,
                            });
                            state.open_model_picker(async_tx);
                        }
                        2 if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                            state.overlay_back.push(Overlay::Settings {
                                selected: *selected,
                            });
                            state.open_themes();
                        }
                        3 => {
                            if key.code == KeyCode::Enter {
                                state.overlay_back.push(Overlay::Settings {
                                    selected: *selected,
                                });
                                state.open_style_picker();
                            } else {
                                let styles = UiStyle::all();
                                let cur = styles
                                    .iter()
                                    .position(|s| *s == state.cfg.ui_style)
                                    .unwrap_or(0);
                                let next = if step_back {
                                    (cur + styles.len() - 1) % styles.len()
                                } else {
                                    (cur + 1) % styles.len()
                                };
                                state.apply_ui_style(styles[next]);
                                let _ = state.cfg.save_global();
                                state.overlay = Some(overlay);
                            }
                        }
                        4 => {
                            state.cfg.default_plan_mode = !state.cfg.default_plan_mode;
                            state.mode = if state.cfg.default_plan_mode {
                                AgentMode::Plan
                            } else {
                                AgentMode::Build
                            };
                            let _ = state.cfg.save_global();
                            state.overlay = Some(overlay);
                        }
                        5 => {
                            state.cfg.auto_approve = !state.cfg.auto_approve;
                            let _ = state.cfg.save_global();
                            state.overlay = Some(overlay);
                        }
                        6 => {
                            let presets = config::MAX_TOKEN_PRESETS;
                            let cur_idx = presets
                                .iter()
                                .position(|&v| v == state.cfg.max_tokens)
                                .unwrap_or(2);
                            let next_idx = if step_back {
                                (cur_idx + presets.len() - 1) % presets.len()
                            } else {
                                (cur_idx + 1) % presets.len()
                            };
                            state.cfg.max_tokens = presets[next_idx];
                            let _ = state.cfg.save_global();
                            state.overlay = Some(overlay);
                        }
                        7 => {
                            state.cfg.show_tool_details = !state.cfg.show_tool_details;
                            state.show_tool_details = state.cfg.show_tool_details;
                            let _ = state.cfg.save_global();
                            state.overlay = Some(overlay);
                        }
                        8 => {
                            state.cfg.mouse_capture = !state.cfg.mouse_capture;
                            let _ = state.cfg.save_global();
                            state.overlay = Some(overlay);
                        }
                        9 if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                            state.overlay_back.push(Overlay::Settings {
                                selected: *selected,
                            });
                            state.open_dir_picker();
                        }
                        10 if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                            state.overlay_back.push(Overlay::Settings {
                                selected: *selected,
                            });
                            state.reload_extensions();
                            state.overlay = Some(Overlay::Plugins {
                                selected: 0,
                                status: None,
                            });
                        }
                        11 if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                            state.overlay_back.push(Overlay::Settings {
                                selected: *selected,
                            });
                            state.open_mcp_status(async_tx);
                        }
                        _ => state.overlay = Some(overlay),
                    }
                }
                _ => state.overlay = Some(overlay),
            }
        }
    }
}

fn spawn_agent_turn(
    state: &mut AppState,
    prompt_display: String,
    prompt_for_llm: String,
    agent_tx: &mpsc::UnboundedSender<AgentEvent>,
    done_tx: &mpsc::UnboundedSender<(Vec<Message>, bool)>,
) {
    let expanded = expand_at_references(&state.work_dir, &prompt_for_llm);
    state.entries.push(ChatEntry::User(prompt_display));
    state.history.push(Message::user(expanded));
    state.is_busy = true;

    let client = state.client.clone();
    let cfg = state.cfg.clone();
    let work_dir = state.work_dir.clone();
    let mode = state.mode;
    let mcp = state.mcp.clone();
    let ext = state.extensions.clone();
    let history = state.history.clone();
    let tx = agent_tx.clone();
    let d_tx = done_tx.clone();

    let handle = tokio::spawn(async move {
        let result = agent::run_turn(
            TurnParams {
                client,
                cfg,
                work_dir,
                mode,
                mcp,
                extensions: ext,
            },
            history,
            tx,
        )
        .await;
        let _ = d_tx.send(result);
    });
    state.active_task = Some(handle);
}

fn submit_input(
    state: &mut AppState,
    agent_tx: &mpsc::UnboundedSender<AgentEvent>,
    done_tx: &mpsc::UnboundedSender<(Vec<Message>, bool)>,
    async_tx: &mpsc::UnboundedSender<UiAsyncEvent>,
) {
    if state.is_busy {
        return;
    }
    let text = state.input.trim().to_string();
    if text.is_empty() {
        return;
    }
    state.input.clear();
    state.cursor = 0;
    state.prompt_hist_idx = None;
    state.scroll_from_bottom = 0;
    state.ac_selected = 0;
    if state.prompt_history.last().map(String::as_str) != Some(&text) {
        state.prompt_history.push(text.clone());
    }

    if text.starts_with('/') {
        execute_slash_command(state, &text, agent_tx, done_tx, async_tx);
        return;
    }

    if let Some(shell_cmd) = text.strip_prefix('!') {
        let cmd = shell_cmd.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        state.is_busy = true;
        let work_dir = state.work_dir.clone();
        let tx = async_tx.clone();
        tokio::spawn(async move {
            let (output, is_error) =
                match tools::execute(&work_dir, "bash", &json!({ "command": cmd })).await {
                    Ok(o) => (o, false),
                    Err(e) => (e.to_string(), true),
                };
            let _ = tx.send(UiAsyncEvent::InlineShellDone {
                cmd,
                output,
                is_error,
            });
        });
        return;
    }

    spawn_agent_turn(state, text.clone(), text, agent_tx, done_tx);
}

fn expand_at_references(work_dir: &std::path::Path, text: &str) -> String {
    let mut attachments = Vec::new();
    for word in text.split_whitespace() {
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
        text.to_string()
    } else {
        format!("{text}\n\n{}", attachments.join("\n\n"))
    }
}

fn execute_slash_command(
    state: &mut AppState,
    raw: &str,
    agent_tx: &mpsc::UnboundedSender<AgentEvent>,
    done_tx: &mpsc::UnboundedSender<(Vec<Message>, bool)>,
    async_tx: &mpsc::UnboundedSender<UiAsyncEvent>,
) {
    let parts: Vec<&str> = raw.splitn(2, ' ').collect();
    let name = parts[0];
    let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");
    state.overlay_back.clear();

    match name {
        "/help" => state.overlay = Some(Overlay::Help),
        "/settings" | "/config" => state.overlay = Some(Overlay::Settings { selected: 0 }),
        "/connect" => state.open_connect(),
        "/models" => state.open_model_picker(async_tx),
        "/sessions" => state.open_sessions(),
        "/undo" => state.perform_undo(),
        "/redo" => state.perform_redo(),
        "/compact" => state.start_compact(async_tx),
        "/skills" => {
            state.reload_extensions();
            state.overlay = Some(Overlay::Skills {
                selected: 0,
                status: None,
            });
        }
        "/plugins" => {
            state.reload_extensions();
            state.overlay = Some(Overlay::Plugins {
                selected: 0,
                status: None,
            });
        }
        "/mcp" => state.open_mcp_status(async_tx),
        "/export" => state.export_session(arg),
        "/copy" => state.copy_last_response(),
        "/diff" => {
            if state.changed_files.is_empty() {
                state
                    .entries
                    .push(ChatEntry::System("No files changed in this session.".to_string()));
            } else {
                state.entries.push(ChatEntry::System(format!(
                    "{} file(s) changed in this session.",
                    state.changed_files.len()
                )));
            }
        }
        "/themes" | "/theme" => {
            if arg.is_empty() {
                state.open_themes();
            } else if let Some(kind) = ThemeKind::parse(arg) {
                state.cfg.theme = kind;
                state.save_cfg_note();
                state
                    .entries
                    .push(ChatEntry::System(format!("Switched theme to {}", kind.name())));
            } else {
                state.entries.push(ChatEntry::System(
                    "Unknown theme. Use /themes to pick one.".to_string(),
                ));
            }
        }
        "/style" | "/styles" => {
            if arg.is_empty() {
                state.open_style_picker();
            } else if let Some(style) = UiStyle::parse(arg) {
                state.apply_ui_style(style);
                state.save_cfg_note();
                state.entries.push(ChatEntry::System(format!(
                    "Switched UI style to {}",
                    style.label()
                )));
            } else {
                state.entries.push(ChatEntry::System(
                    "Unknown style. Use /style to pick one.".to_string(),
                ));
            }
        }
        "/dir" | "/cd" => {
            if arg.is_empty() {
                state.open_dir_picker();
            } else {
                match state.set_work_dir(arg) {
                    Ok(dir) => {
                        state.entries.push(ChatEntry::System(format!(
                            "Workspace switched to {} ({} files indexed)",
                            dir.display(),
                            state.workspace_files.len()
                        )));
                    }
                    Err(e) => state.entries.push(ChatEntry::System(e)),
                }
            }
        }
        "/mode" => {
            state.mode = state.mode.toggle();
        }
        "/provider" => {
            if arg.is_empty() {
                state.open_connect();
            } else if let Some(spec) = state.cfg.set_provider(arg) {
                state.save_cfg_note();
                let mdl = state.cfg.active().model.clone();
                state.entries.push(ChatEntry::System(format!(
                    "Switched provider to {} ({mdl})",
                    spec.display_name
                )));
            } else {
                let ids: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
                state.entries.push(ChatEntry::System(format!(
                    "Unknown provider '{arg}'. Available: {}",
                    ids.join(", ")
                )));
            }
        }
        "/model" => {
            if arg.is_empty() {
                state.open_model_picker(async_tx);
            } else {
                state.cfg.active_mut().model = arg.to_string();
                state.save_cfg_note();
                state
                    .entries
                    .push(ChatEntry::System(format!("Switched model to {arg}")));
            }
        }
        "/url" => {
            if arg.is_empty() {
                state.entries.push(ChatEntry::System(format!(
                    "Current base_url: {}",
                    state.cfg.active().base_url
                )));
            } else {
                state.cfg.active_mut().base_url = config::normalize_base_url(arg);
                state.save_cfg_note();
                state.entries.push(ChatEntry::System(format!(
                    "Set base_url to {}",
                    state.cfg.active().base_url
                )));
            }
        }
        "/auto" => {
            state.cfg.auto_approve = !state.cfg.auto_approve;
            state.save_cfg_note();
            state.entries.push(ChatEntry::System(format!(
                "Auto-approve is now {}",
                if state.cfg.auto_approve { "ON" } else { "OFF" }
            )));
        }
        "/mouse" => {
            state.cfg.mouse_capture = !state.cfg.mouse_capture;
            state.save_cfg_note();
            state.entries.push(ChatEntry::System(if state.cfg.mouse_capture {
                "Mouse capture ON (wheel scrolls, hold Shift to select text).".to_string()
            } else {
                "Mouse capture OFF (native mouse drag selection & copy enabled).".to_string()
            }));
        }
        "/new" | "/clear" => {
            state.history.clear();
            state.entries.clear();
            state.undo_stack.clear();
            state.redo_stack.clear();
            state.changed_files.clear();
            state.last_context_tokens = 0;
            state.session_id = session::new_session_id();
            state.scroll_from_bottom = 0;
        }
        "/exit" | "/quit" => {
            state.should_quit = true;
        }
        _ => {
            if let Some(custom) = state
                .custom_commands
                .iter()
                .find(|c| c.slash == name.to_lowercase())
                .cloned()
            {
                let prompt = custom.expand(arg);
                spawn_agent_turn(state, raw.to_string(), prompt, agent_tx, done_tx);
            } else {
                state.entries.push(ChatEntry::System(format!(
                    "Unknown command: {name}. Press Ctrl+P or type /help"
                )));
            }
        }
    }
}

fn finalize_active_thinking(state: &mut AppState) {
    if let Some(ChatEntry::Thinking {
        started_at,
        duration_secs,
        ..
    }) = state.entries.last_mut()
        && duration_secs.is_none()
    {
        *duration_secs = Some(started_at.elapsed().as_secs_f32());
    }
}

fn handle_agent_event(state: &mut AppState, ev: AgentEvent) {
    match ev {
        AgentEvent::ThinkingDelta(delta) => {
            if let Some(ChatEntry::Thinking { text, .. }) = state.entries.last_mut() {
                text.push_str(&delta);
            } else {
                state.entries.push(ChatEntry::Thinking {
                    text: delta,
                    started_at: Instant::now(),
                    duration_secs: None,
                });
            }
        }
        AgentEvent::AssistantDelta(delta) => {
            finalize_active_thinking(state);
            if let Some(ChatEntry::Assistant(text)) = state.entries.last_mut() {
                text.push_str(&delta);
            } else {
                state.entries.push(ChatEntry::Assistant(delta));
            }
        }
        AgentEvent::ToolStarted { name, preview } => {
            finalize_active_thinking(state);
            state.entries.push(ChatEntry::Tool {
                name,
                preview,
                output: None,
                is_error: false,
            });
        }
        AgentEvent::ApprovalRequest {
            name,
            preview,
            responder,
        } => {
            state.pending_approval = Some(PendingApproval {
                name,
                preview,
                scroll: 0,
                responder,
            });
        }
        AgentEvent::FileChanged(snap) => {
            let entry = state
                .changed_files
                .entry(snap.rel_path.clone())
                .or_insert((0, 0));
            entry.0 += snap.added;
            entry.1 += snap.removed;
            state.undo_stack.push(snap);
            state.redo_stack.clear();
        }
        AgentEvent::ToolFinished {
            name,
            output,
            is_error,
        } => {
            if let Some(ChatEntry::Tool {
                name: last_name,
                output: last_out,
                is_error: last_err,
                ..
            }) = state.entries.last_mut()
                && *last_name == name
                && last_out.is_none()
            {
                *last_out = Some(output);
                *last_err = is_error;
            } else {
                state.entries.push(ChatEntry::Tool {
                    name: name.clone(),
                    preview: name.clone(),
                    output: Some(output),
                    is_error,
                });
            }
            if matches!(name.as_str(), "write_file" | "edit_file" | "bash") {
                state.workspace_files = tools::list_workspace_files(&state.work_dir, 500);
                state.git_branch = session::read_git_branch(&state.work_dir);
            }
        }
        AgentEvent::Usage {
            input_tokens,
            output_tokens,
        } => {
            if input_tokens > 0 {
                state.last_context_tokens = input_tokens;
            }
            state.total_in_tokens = state.total_in_tokens.saturating_add(input_tokens);
            state.total_out_tokens = state.total_out_tokens.saturating_add(output_tokens);
        }
        AgentEvent::TurnFinished => {
            finalize_active_thinking(state);
            state.is_busy = false;
            state.active_task = None;
        }
        AgentEvent::Error(err) => {
            finalize_active_thinking(state);
            state.entries.push(ChatEntry::System(format!("Error: {err}")));
            state.is_busy = false;
            state.active_task = None;
        }
    }
}

fn draw_ui(f: &mut Frame, state: &AppState) {
    let pal = state.palette();
    let area = f.area();

    if pal.bg != Color::Reset {
        f.render_widget(Block::default().style(Style::default().bg(pal.bg)), area);
    }

    let approval_height = if let Some(pending) = &state.pending_approval {
        let lines = pending.preview.lines().count() as u16 + 3;
        lines.clamp(6, (area.height / 2).max(8))
    } else {
        0
    };

    let is_agy = matches!(state.cfg.ui_style, UiStyle::Antigravity);
    let is_hero = !is_agy
        && state.entries.is_empty()
        && state.pending_approval.is_none()
        && area.height >= 20
        && area.width >= 56;
    let hero_w = area.width.clamp(40, 86);
    let active_input_w = if is_hero { hero_w } else { area.width };
    let inner_w = active_input_w
        .saturating_sub(if is_agy { 3 } else { 2 })
        .max(20) as usize;
    let line_count = state
        .input
        .split('\n')
        .map(|l| (l.chars().count() / inner_w) + 1)
        .sum::<usize>()
        .max(1);
    let input_height = (line_count as u16 + 2).clamp(3, 7);

    let footer_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };

    let input_rect = if is_hero {
        let main_h = area.height.saturating_sub(1);
        let hero_x = area.x + (area.width.saturating_sub(hero_w)) / 2;
        let header_h: u16 = if state.cfg.show_fox_mascot {
            if main_h >= 27 { 13 } else { 10 }
        } else if hero_w >= 66 && main_h >= 22 {
            6
        } else {
            3
        };
        // header (fox or logo) + blank(1) + status card(5) + blank(1)
        let welcome_h = header_h + 7;
        // welcome + input + blank(0) + keybind hint(1)
        let total_h = welcome_h + input_height + 2;
        let top_y = area.y + main_h.saturating_sub(total_h) / 2;

        let welcome_area = Rect {
            x: hero_x,
            y: top_y,
            width: hero_w,
            height: welcome_h,
        };
        draw_welcome(f, state, welcome_area, pal);

        let inp_rect = Rect {
            x: hero_x,
            y: top_y + welcome_h,
            width: hero_w,
            height: input_height,
        };
        let hint_y = inp_rect.y + inp_rect.height;
        if hint_y < footer_area.y {
            let hint_area = Rect {
                x: hero_x,
                y: hint_y,
                width: hero_w,
                height: 1,
            };
            let binds = Line::from(vec![
                Span::styled("Tab", Style::default().fg(pal.primary)),
                Span::styled(" mode · ", Style::default().fg(pal.muted)),
                Span::styled("Shift+Tab", Style::default().fg(pal.primary)),
                Span::styled(" auto · ", Style::default().fg(pal.muted)),
                Span::styled("Ctrl+P", Style::default().fg(pal.primary)),
                Span::styled(" menu · ", Style::default().fg(pal.muted)),
                Span::styled("Ctrl+X", Style::default().fg(pal.primary)),
                Span::styled(" keys", Style::default().fg(pal.muted)),
            ]);
            f.render_widget(Paragraph::new(binds).alignment(Alignment::Center), hint_area);
        }
        inp_rect
    } else if is_agy {
        let banner_h: u16 = if area.height >= 16 && area.width >= 36 {
            7
        } else {
            0
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(banner_h),
                Constraint::Min(3),
                Constraint::Length(approval_height),
                Constraint::Length(input_height),
                Constraint::Length(1),
            ])
            .split(area);

        if banner_h > 0 {
            draw_antigravity_banner(f, state, chunks[0], pal);
        }

        if !state.entries.is_empty() {
            let content_area = if area.width > 124 {
                Rect {
                    x: chunks[1].x + 1,
                    y: chunks[1].y,
                    width: chunks[1].width.saturating_sub(2).min(116),
                    height: chunks[1].height,
                }
            } else {
                chunks[1]
            };
            draw_chat(f, state, content_area, pal);
        }

        if let Some(pending) = &state.pending_approval {
            draw_permission_panel(f, pending, chunks[2], pal);
        }
        chunks[3]
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(6),
                Constraint::Length(approval_height),
                Constraint::Length(input_height),
                Constraint::Length(1),
            ])
            .split(area);

        let content_area = if area.width > 124 && !state.entries.is_empty() {
            let pad = (area.width - 116) / 2;
            Rect {
                x: chunks[0].x + pad,
                y: chunks[0].y,
                width: 116,
                height: chunks[0].height,
            }
        } else {
            chunks[0]
        };

        if state.entries.is_empty() {
            draw_welcome(f, state, chunks[0], pal);
        } else {
            draw_chat(f, state, content_area, pal);
        }

        if let Some(pending) = &state.pending_approval {
            draw_permission_panel(f, pending, chunks[1], pal);
        }
        chunks[2]
    };

    let spin = state.glyphs.spin(state.spinner_idx);
    if is_agy {
        let hch = if state.glyphs.legacy_console { "-" } else { "─" };
        let rule_col = if state.is_busy || state.leader_pending {
            pal.accent
        } else {
            pal.border
        };
        let top_rule = if state.is_busy {
            let msg = format!(" {spin} thinking... (Esc to stop) ");
            let rem = (input_rect.width as usize).saturating_sub(msg.chars().count() + 2);
            Line::from(vec![
                Span::styled(hch.repeat(2), Style::default().fg(rule_col)),
                Span::styled(msg, Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
                Span::styled(hch.repeat(rem), Style::default().fg(rule_col)),
            ])
        } else if state.leader_pending {
            let msg = " Ctrl+X > [c]onnect [m]odel [t]heme [u]ndo [r]edo [k]compact [p]mcp [e]xport [q]uit ";
            let rem = (input_rect.width as usize).saturating_sub(msg.chars().count() + 1);
            Line::from(vec![
                Span::styled(hch, Style::default().fg(rule_col)),
                Span::styled(msg, Style::default().fg(pal.accent)),
                Span::styled(hch.repeat(rem), Style::default().fg(rule_col)),
            ])
        } else {
            Line::from(Span::styled(
                hch.repeat(input_rect.width as usize),
                Style::default().fg(rule_col),
            ))
        };
        f.render_widget(
            Paragraph::new(top_rule),
            Rect {
                x: input_rect.x,
                y: input_rect.y,
                width: input_rect.width,
                height: 1,
            },
        );

        let mid_h = input_rect.height.saturating_sub(2).max(1);
        let wrapped = if state.input.is_empty() {
            vec![String::new()]
        } else {
            let mut rows = Vec::new();
            for raw_l in state.input.split('\n') {
                for seg in wrap_line_chars(raw_l, inner_w) {
                    rows.push(seg);
                }
            }
            if rows.is_empty() {
                rows.push(String::new());
            }
            rows
        };
        let prompt_lines: Vec<Line> = wrapped
            .into_iter()
            .take(mid_h as usize)
            .enumerate()
            .map(|(i, seg)| {
                let prefix = if i == 0 { "> " } else { "  " };
                Line::from(vec![
                    Span::styled(
                        prefix,
                        Style::default()
                            .fg(pal.primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(seg, Style::default().fg(pal.text)),
                ])
            })
            .collect();
        f.render_widget(
            Paragraph::new(prompt_lines),
            Rect {
                x: input_rect.x,
                y: input_rect.y + 1,
                width: input_rect.width,
                height: mid_h,
            },
        );

        let bot_rule = Line::from(Span::styled(
            hch.repeat(input_rect.width as usize),
            Style::default().fg(pal.border),
        ));
        f.render_widget(
            Paragraph::new(bot_rule),
            Rect {
                x: input_rect.x,
                y: input_rect.y + input_rect.height.saturating_sub(1),
                width: input_rect.width,
                height: 1,
            },
        );
    } else {
        let prompt_title = if state.is_busy {
            format!(" {spin} thinking... (Esc to stop) ")
        } else if state.leader_pending {
            " Ctrl+X > [c]onnect [m]odel [t]heme [u]ndo [r]edo [k]compact [p]mcp [e]xport [q]uit "
                .to_string()
        } else {
            " Ask anything... ".to_string()
        };
        let border_col = if state.is_busy || state.leader_pending {
            pal.accent
        } else {
            pal.border_active
        };

        let input_box = Paragraph::new(state.input.as_str())
            .style(Style::default().fg(pal.text).bg(pal.bg_panel))
            .wrap(Wrap { trim: false })
            .block(styled_block(&prompt_title, border_col, pal));
        f.render_widget(input_box, input_rect);
    }

    if !state.is_busy && state.pending_approval.is_none() && state.overlay.is_none() {
        let before_cursor = &state.input[..state.cursor];
        let mut row: u16 = 0;
        let mut col: u16 = 0;
        for ch in before_cursor.chars() {
            if ch == '\n' || col as usize >= inner_w {
                row += 1;
                col = if ch == '\n' { 0 } else { 1 };
            } else {
                col += 1;
            }
        }
        let max_row = input_height.saturating_sub(3);
        let x_off = if is_agy { 2 } else { 1 };
        f.set_cursor_position((input_rect.x + x_off + col, input_rect.y + 1 + row.min(max_row)));
    }

    draw_footer(f, state, footer_area, pal);

    if state.overlay.is_none() && state.pending_approval.is_none() {
        draw_autocomplete(f, state, input_rect, pal);
    }

    if let Some(overlay) = &state.overlay {
        draw_overlay(f, state, overlay, area, pal);
    }
}

fn draw_permission_panel(f: &mut Frame, pending: &PendingApproval, area: Rect, pal: Palette) {
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled(
            format!(" Approve {}? ", pending.name),
            Style::default()
                .fg(pal.yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "[y] Allow once   [a] Always allow   [n] Reject   [Up/Down] Scroll",
            Style::default().fg(pal.green),
        ),
    ])];

    for l in pending.preview.lines() {
        let trimmed = l.trim_start();
        let style = if trimmed.contains(" - ") || l.starts_with("- ") {
            Style::default().fg(pal.red)
        } else if trimmed.contains(" + ") || l.starts_with("+ ") {
            Style::default().fg(pal.green)
        } else {
            Style::default().fg(pal.primary).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(Span::styled(format!(" {l}"), style)));
    }

    let box_w = Paragraph::new(lines)
        .style(Style::default().bg(pal.bg_panel))
        .scroll((pending.scroll, 0))
        .block(styled_block(" Smart Permission Preview ", pal.yellow, pal));
    f.render_widget(box_w, area);
}

fn draw_antigravity_banner(f: &mut Frame, state: &AppState, area: Rect, pal: Palette) {
    if area.height < 6 || area.width < 28 {
        return;
    }
    let grid = fox3d::render_pixel_fox_emblem(
        state.cfg.theme.is_monochrome(),
        state.glyphs.legacy_console,
    );
    let spec = state.cfg.active_spec();
    let conn_label = if state.cfg.has_key(spec.id) {
        "connected"
    } else if !spec.requires_key {
        "local"
    } else {
        "no api key"
    };
    let ws_raw = state.work_dir.display().to_string().replace('\\', "/");
    let max_info_w = area.width.saturating_sub(21) as usize;
    let ws_display = if ws_raw.chars().count() > max_info_w && max_info_w > 6 {
        let tail: String = ws_raw
            .chars()
            .rev()
            .take(max_info_w - 3)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("...{tail}")
    } else {
        ws_raw
    };
    let mode_tag = if state.cfg.auto_approve {
        format!("{} · Auto", state.mode.as_str())
    } else {
        state.mode.as_str().to_string()
    };

    let mut lines: Vec<Line> = Vec::with_capacity(7);
    for (row_idx, row_cells) in grid.into_iter().enumerate() {
        let mut spans = Vec::with_capacity(20);
        spans.push(Span::raw(" "));
        for cell in row_cells {
            let mut st = Style::default();
            if let Some(fg) = cell.fg {
                st = st.fg(Color::Rgb(fg.r, fg.g, fg.b));
            }
            if let Some(bg) = cell.bg {
                st = st.bg(Color::Rgb(bg.r, bg.g, bg.b));
            }
            spans.push(Span::styled(cell.ch.to_string(), st));
        }
        spans.push(Span::raw("   "));
        match row_idx {
            1 => spans.push(Span::styled(
                format!("OxiCode CLI {}", env!("CARGO_PKG_VERSION")),
                Style::default()
                    .fg(pal.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            2 => spans.push(Span::styled(
                format!("{} ({conn_label})", spec.display_name),
                Style::default().fg(pal.muted),
            )),
            3 => spans.push(Span::styled(
                format!("{} ({mode_tag})", state.cfg.active().model),
                Style::default().fg(pal.text),
            )),
            4 => spans.push(Span::styled(ws_display.clone(), Style::default().fg(pal.muted))),
            _ => {}
        }
        lines.push(Line::from(spans));
    }

    if area.height >= 7 {
        let hch = if state.glyphs.legacy_console { "-" } else { "─" };
        let sep_w = (area.width as usize).min(64).saturating_sub(1);
        lines.push(Line::from(Span::styled(
            format!(" {}", hch.repeat(sep_w)),
            Style::default().fg(pal.border),
        )));
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_welcome(f: &mut Frame, state: &AppState, area: Rect, pal: Palette) {
    let spec = state.cfg.active_spec();
    let connected = state.cfg.has_key(spec.id);
    let key_badge = if connected {
        Span::styled(
            format!("{} connected", state.glyphs.dot_ok),
            Style::default().fg(pal.green).bg(pal.bg_panel),
        )
    } else {
        Span::styled("o no api key", Style::default().fg(pal.red).bg(pal.bg_panel))
    };

    let cur_y = area.y;
    let rem_h = area.height;
    let header_h: u16;

    if state.cfg.show_fox_mascot && rem_h >= 14 {
        let (fox_cols, fox_rows) = if rem_h >= 20 && area.width >= 44 {
            (38usize, 13usize)
        } else {
            (30usize, 10usize)
        };
        let grid = state
            .fox
            .render_ascii_grid_styled(fox_cols, fox_rows, state.cfg.theme.is_monochrome());
        let fox_lines: Vec<Line> = grid
            .into_iter()
            .map(|row| {
                let spans: Vec<Span> = row
                    .into_iter()
                    .map(|cell| {
                        if cell.ch == ' ' {
                            Span::raw(" ")
                        } else {
                            Span::styled(
                                cell.ch.to_string(),
                                Style::default().fg(Color::Rgb(cell.fg.r, cell.fg.g, cell.fg.b)),
                            )
                        }
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();
        header_h = fox_rows as u16;
        let fox_rect = Rect {
            x: area.x,
            y: cur_y,
            width: area.width,
            height: header_h.min(rem_h),
        };
        state.fox_center.set((
            fox_rect.x as f32 + fox_rect.width as f32 * 0.5,
            fox_rect.y as f32 + fox_rect.height as f32 * 0.5,
        ));
        f.render_widget(
            Paragraph::new(fox_lines).alignment(Alignment::Center),
            fox_rect,
        );
    } else {
        let mut logo_lines: Vec<Line> = Vec::new();
        if state.glyphs.legacy_console {
            if area.width >= 52 && rem_h >= 12 {
                let ascii_logo = [
                    ("  ___  __  __ ___  ____  ___  ____  _____ ", true),
                    (" / _ \\ \\ \\/ /|_ _|/ ___|/ _ \\|  _ \\| ____|", true),
                    ("| | | | \\  /  | || |   | | | | | | |  _|  ", true),
                    ("| |_| | /  \\  | || |___| |_| | |_| | |___ ", false),
                    (" \\___/ /_/\\_\\|___|\\____|\\___/|____/|_____|", false),
                ];
                for (row, is_top) in ascii_logo {
                    logo_lines.push(Line::from(Span::styled(
                        row,
                        Style::default()
                            .fg(if is_top { pal.primary } else { pal.accent })
                            .add_modifier(Modifier::BOLD),
                    )));
                }
            } else {
                logo_lines.push(Line::from(Span::styled(
                    "OXICODE",
                    Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
                )));
            }
        } else if area.width >= 66 && rem_h >= 12 {
            let big_logo = [
                (" ██████╗  ██╗  ██╗ ██╗  ██████╗  ██████╗  ██████╗  ███████╗", true),
                ("██╔═══██╗ ╚██╗██╔╝ ██║ ██╔════╝ ██╔═══██╗ ██╔══██╗ ██╔════╝", true),
                ("██║   ██║  ╚███╔╝  ██║ ██║      ██║   ██║ ██║  ██║ █████╗  ", true),
                ("██║   ██║  ██╔██╗  ██║ ██║      ██║   ██║ ██║  ██║ ██╔══╝  ", false),
                ("╚██████╔╝ ██╔╝ ██╗ ██║ ╚██████╗ ╚██████╔╝ ██████╔╝ ███████╗", false),
                (" ╚═════╝  ╚═╝  ╚═╝ ╚═╝  ╚═════╝  ╚═════╝  ╚═════╝  ╚══════╝", false),
            ];
            for (row, is_top) in big_logo {
                logo_lines.push(Line::from(Span::styled(
                    row,
                    Style::default()
                        .fg(if is_top { pal.primary } else { pal.accent })
                        .add_modifier(Modifier::BOLD),
                )));
            }
        } else if area.width >= 40 {
            logo_lines.push(Line::from(Span::styled(
                "█▀▀█ █  █ ▀█▀ █▀▀█ █▀▀█ █▀▀▄ █▀▀▀",
                Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
            )));
            logo_lines.push(Line::from(Span::styled(
                "█  █ ▄▀▀▄  █  █    █  █ █  █ █▀▀▀",
                Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
            )));
            logo_lines.push(Line::from(Span::styled(
                "▀▀▀▀ ▀  ▀ ▀▀▀ ▀▀▀▀ ▀▀▀▀ ▀▀▀  ▀▀▀▀",
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )));
        } else {
            logo_lines.push(Line::from(Span::styled(
                "OXICODE",
                Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
            )));
        }

        header_h = logo_lines.len() as u16;
        let logo_area = Rect {
            x: area.x,
            y: cur_y,
            width: area.width,
            height: header_h.min(rem_h),
        };
        f.render_widget(
            Paragraph::new(logo_lines).alignment(Alignment::Center),
            logo_area,
        );
    }

    let card_y = cur_y + header_h + 1;
    if card_y + 5 > area.y + area.height {
        return;
    }

    let card_w = area.width.min(80);
    let card_x = area.x + (area.width.saturating_sub(card_w)) / 2;
    let card_rect = Rect {
        x: card_x,
        y: card_y,
        width: card_w,
        height: 5,
    };
    let inner_w = card_w.saturating_sub(2) as usize;

    let branch_len = state
        .git_branch
        .as_ref()
        .map(|b| b.chars().count() + 3)
        .unwrap_or(0);
    let max_ws = inner_w.saturating_sub(12 + branch_len);
    let ws_raw = state.work_dir.display().to_string();
    let ws_display = if ws_raw.chars().count() > max_ws && max_ws > 4 {
        let tail: String = ws_raw
            .chars()
            .rev()
            .take(max_ws - 3)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("...{tail}")
    } else {
        ws_raw
    };

    let mut ws_spans = vec![
        Span::styled(" workspace ", Style::default().fg(pal.muted).bg(pal.bg_panel)),
        Span::styled(ws_display, Style::default().fg(pal.cyan).bg(pal.bg_panel)),
    ];
    if let Some(branch) = &state.git_branch {
        ws_spans.push(Span::styled(
            format!(" ({branch})"),
            Style::default().fg(pal.green).bg(pal.bg_panel),
        ));
    }

    let ext_line = vec![
        Span::styled(" skills ", Style::default().fg(pal.muted).bg(pal.bg_panel)),
        Span::styled(
            state.extensions.skills.len().to_string(),
            Style::default().fg(pal.text).bg(pal.bg_panel),
        ),
        Span::styled("  ·  plugins ", Style::default().fg(pal.muted).bg(pal.bg_panel)),
        Span::styled(
            state
                .extensions
                .plugins
                .iter()
                .filter(|p| p.enabled)
                .count()
                .to_string(),
            Style::default().fg(pal.text).bg(pal.bg_panel),
        ),
        Span::styled("  ·  cmds ", Style::default().fg(pal.muted).bg(pal.bg_panel)),
        Span::styled(
            state.custom_commands.len().to_string(),
            Style::default().fg(pal.text).bg(pal.bg_panel),
        ),
    ];

    let card_lines = vec![
        fill_row(
            vec![
                Span::styled(
                    format!(" {} ", spec.display_name),
                    Style::default().fg(pal.primary).bg(pal.bg_panel),
                ),
                Span::styled("/ ", Style::default().fg(pal.muted).bg(pal.bg_panel)),
                Span::styled(
                    &state.cfg.active().model,
                    Style::default()
                        .fg(pal.text)
                        .bg(pal.bg_panel)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ", Style::default().bg(pal.bg_panel)),
                key_badge,
            ],
            inner_w,
            pal.bg_panel,
        ),
        fill_row(ws_spans, inner_w, pal.bg_panel),
        fill_row(ext_line, inner_w, pal.bg_panel),
    ];

    let card = Paragraph::new(card_lines)
        .style(Style::default().bg(pal.bg_panel))
        .block(styled_block("", pal.border, pal));
    f.render_widget(card, card_rect);
}

fn draw_chat(f: &mut Frame, state: &AppState, area: Rect, pal: Palette) {
    let mut lines: Vec<Line> = Vec::new();
    let default_out_limit = if area.height > 40 { 10 } else { 4 };
    let card_w = area.width.saturating_sub(2) as usize;

    let g = state.glyphs;
    let mut idx = 0usize;
    while idx < state.entries.len() {
        match &state.entries[idx] {
            ChatEntry::User(msg) => {
                let text_max = card_w.saturating_sub(4).max(16);
                for raw_l in msg.lines() {
                    let clean_l = sanitize_tabs(raw_l);
                    for l in wrap_line_chars(&clean_l, text_max) {
                        lines.push(fill_row(
                            vec![
                                Span::styled(
                                    g.user_bar,
                                    Style::default()
                                        .fg(pal.accent)
                                        .bg(pal.bg_panel)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    l,
                                    Style::default()
                                        .fg(pal.primary)
                                        .bg(pal.bg_panel)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            ],
                            card_w,
                            pal.bg_panel,
                        ));
                    }
                }
                lines.push(Line::from(""));
                idx += 1;
            }
            ChatEntry::Thinking {
                text,
                started_at,
                duration_secs,
            } => {
                let secs = duration_secs.unwrap_or_else(|| started_at.elapsed().as_secs_f32());
                let summary_raw: String = text
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let summary = clip_text(&sanitize_tabs(&summary_raw), card_w.saturating_sub(28).max(20), g.ellipsis);

                let mut head_spans = vec![Span::styled(
                    if duration_secs.is_some() {
                        format!("  o Thought for {secs:.1}s")
                    } else {
                        format!("  {} Thinking for {secs:.1}s", g.spin(state.spinner_idx))
                    },
                    Style::default().fg(pal.muted),
                )];
                if !state.show_tool_details && !summary.is_empty() {
                    head_spans.push(Span::styled(
                        format!(" ({summary})"),
                        Style::default().fg(pal.muted).add_modifier(Modifier::ITALIC),
                    ));
                }
                lines.push(Line::from(head_spans));

                if state.show_tool_details {
                    for l in text.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("    {}", sanitize_tabs(l)),
                            Style::default().fg(pal.muted).add_modifier(Modifier::ITALIC),
                        )));
                    }
                }
                idx += 1;
            }
            ChatEntry::Assistant(msg) => {
                if idx > 0 {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(vec![
                    Span::styled(
                        g.assistant_mark,
                        Style::default()
                            .fg(pal.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "oxicode",
                        Style::default().fg(pal.muted).add_modifier(Modifier::BOLD),
                    ),
                ]));
                render_markdown_lines(msg, pal, g, state.cfg.theme.is_monochrome(), &mut lines);
                lines.push(Line::from(""));
                idx += 1;
            }
            ChatEntry::Tool { name, .. } => {
                let group_kind = ToolGroupKind::from_name(name);
                let start_idx = idx;
                while idx < state.entries.len() {
                    if let ChatEntry::Tool { name: n, .. } = &state.entries[idx]
                        && ToolGroupKind::from_name(n) == group_kind
                    {
                        idx += 1;
                    } else {
                        break;
                    }
                }
                let is_last_tool_group = !state.entries[idx..]
                    .iter()
                    .any(|e| matches!(e, ChatEntry::Tool { .. }));
                render_tool_group(
                    &state.entries[start_idx..idx],
                    group_kind,
                    state,
                    pal,
                    default_out_limit,
                    is_last_tool_group,
                    card_w,
                    &mut lines,
                );
            }
            ChatEntry::System(msg) => {
                for l in msg.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {l}"),
                        Style::default().fg(pal.yellow),
                    )));
                }
                lines.push(Line::from(""));
                idx += 1;
            }
        }
    }

    if !state.changed_files.is_empty() {
        let total_add: usize = state.changed_files.values().map(|(a, _)| *a).sum();
        let total_rem: usize = state.changed_files.values().map(|(_, r)| *r).sum();
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} {} file(s) changed ", g.changed_mark, state.changed_files.len()),
                Style::default().fg(pal.muted),
            ),
            Span::styled(format!("+{total_add} "), Style::default().fg(pal.green)),
            Span::styled(format!("-{total_rem} "), Style::default().fg(pal.red)),
            Span::styled("· /undo to revert · /diff", Style::default().fg(pal.muted)),
        ]));
        lines.push(Line::from(""));
    }

    let chat_height = area.height.saturating_sub(1) as usize;
    let wrap_w = area.width.max(1) as usize;
    let total_visual_lines = visual_line_count(&lines, wrap_w);
    let max_scroll = total_visual_lines.saturating_sub(chat_height) as u16;
    let clamped_from_bottom = state.scroll_from_bottom.min(max_scroll);
    let scroll_top = max_scroll.saturating_sub(clamped_from_bottom);

    let scroll_tag = if max_scroll == 0 || clamped_from_bottom == 0 {
        "live".to_string()
    } else {
        let pct = ((scroll_top as f64 / max_scroll as f64) * 100.0).round() as u32;
        format!("{pct}%")
    };
    let chat_title = format!(
        " oxicode · {} · {} skills · {} plugins [{scroll_tag}] ",
        state.cfg.theme.name(),
        state.extensions.skills.len(),
        state.extensions.plugins.iter().filter(|p| p.enabled).count(),
    );

    let chat = Paragraph::new(lines)
        .style(Style::default().bg(pal.bg))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(pal.border).bg(pal.bg))
                .style(Style::default().bg(pal.bg))
                .title(Span::styled(chat_title, Style::default().fg(pal.primary))),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_top, 0));
    f.render_widget(chat, area);
}

fn visual_line_count(lines: &[Line<'_>], wrap_w: usize) -> usize {
    if wrap_w == 0 {
        return lines.len();
    }
    let mut total = 0usize;
    for line in lines {
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        if plain.is_empty() {
            total += 1;
            continue;
        }
        let mut cur_w = 0usize;
        let mut line_rows = 1usize;
        for token in plain.split_inclusive(char::is_whitespace) {
            let tw = token.chars().count();
            if cur_w > 0 && cur_w + tw > wrap_w {
                line_rows += 1;
                cur_w = 0;
            }
            if tw > wrap_w {
                let extra = (tw - 1) / wrap_w;
                line_rows += extra;
                cur_w = tw - extra * wrap_w;
            } else {
                cur_w += tw;
            }
        }
        total += line_rows;
    }
    total
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolGroupKind {
    Bash,
    Edit,
    Read,
    Search,
    Other,
}

impl ToolGroupKind {
    fn from_name(name: &str) -> Self {
        match name {
            "bash" => Self::Bash,
            "edit_file" | "write_file" => Self::Edit,
            "read_file" => Self::Read,
            "grep" | "glob" | "list_files" => Self::Search,
            _ => Self::Other,
        }
    }
}

fn clip_text(s: &str, max_chars: usize, ellipsis: &str) -> String {
    let clean: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let total = clean.chars().count();
    let e_len = ellipsis.chars().count();
    if total <= max_chars || max_chars <= e_len + 2 {
        return clean;
    }
    let prefix: String = clean.chars().take(max_chars - e_len).collect();
    format!("{prefix}{ellipsis}")
}

fn extract_compact_target(name: &str, preview: &str) -> String {
    let first = preview.lines().next().unwrap_or("").trim();
    match name {
        "bash" => first.strip_prefix("$ ").unwrap_or(first).trim().to_string(),
        "edit_file" | "write_file" | "read_file" | "list_files" => {
            let without_op = first
                .strip_prefix("edit ")
                .or_else(|| first.strip_prefix("write "))
                .or_else(|| first.strip_prefix("read "))
                .or_else(|| first.strip_prefix("list "))
                .unwrap_or(first);
            let path_part = without_op
                .split(" (")
                .next()
                .unwrap_or(without_op)
                .trim();
            std::path::Path::new(path_part)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or(path_part)
                .to_string()
        }
        "grep" => first
            .strip_prefix("grep ")
            .unwrap_or(first)
            .trim()
            .to_string(),
        "glob" => first
            .strip_prefix("glob ")
            .unwrap_or(first)
            .trim()
            .to_string(),
        _ => first.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tool_group<'a>(
    group: &[ChatEntry],
    kind: ToolGroupKind,
    state: &AppState,
    pal: Palette,
    default_out_limit: usize,
    is_last_tool_group: bool,
    card_w: usize,
    lines: &mut Vec<Line<'a>>,
) {
    let count = group.len();
    let is_running = group
        .iter()
        .any(|e| matches!(e, ChatEntry::Tool { output: None, .. }));
    let has_error = group
        .iter()
        .any(|e| matches!(e, ChatEntry::Tool { is_error: true, .. }));

    let g = state.glyphs;
    let (dot, dot_color) = if is_running {
        (g.spin(state.spinner_idx), pal.yellow)
    } else if has_error {
        (g.dot_err, pal.red)
    } else {
        (g.dot_ok, pal.green)
    };

    let items: Vec<(&str, &str, Option<&str>, bool)> = group
        .iter()
        .filter_map(|e| match e {
            ChatEntry::Tool {
                name,
                preview,
                output,
                is_error,
            } => Some((name.as_str(), preview.as_str(), output.as_deref(), *is_error)),
            _ => None,
        })
        .collect();

    let (verb, summary) = match kind {
        ToolGroupKind::Bash => {
            let v = if is_running { "Running" } else { "Ran" };
            let last_cmd = items
                .last()
                .map(|(n, p, _, _)| extract_compact_target(n, p))
                .unwrap_or_default();
            let clipped = clip_text(&last_cmd, card_w.saturating_sub(36).max(20), g.ellipsis);
            let s = if count == 1 {
                format!("({clipped})")
            } else {
                format!("{count} commands (+{} · {clipped})", count - 1)
            };
            (v, s)
        }
        ToolGroupKind::Edit => {
            let single_create = count == 1 && items[0].0 == "write_file";
            let v = if is_running {
                if single_create { "Creating" } else { "Editing" }
            } else if single_create {
                "Created"
            } else {
                "Edited"
            };
            let names: Vec<String> = items
                .iter()
                .map(|(n, p, _, _)| extract_compact_target(n, p))
                .collect();
            let s = if count == 1 {
                names[0].clone()
            } else {
                let shown: Vec<String> = names.iter().take(3).cloned().collect();
                if names.len() > 3 {
                    format!("{count} files ({}, +{})", shown.join(", "), names.len() - 3)
                } else {
                    format!("{count} files ({})", shown.join(", "))
                }
            };
            (v, s)
        }
        ToolGroupKind::Read => {
            let v = if is_running { "Reading" } else { "Read" };
            let names: Vec<String> = items
                .iter()
                .map(|(n, p, _, _)| extract_compact_target(n, p))
                .collect();
            let s = if count == 1 {
                names[0].clone()
            } else {
                let shown: Vec<String> = names.iter().take(3).cloned().collect();
                if names.len() > 3 {
                    format!("{count} files ({}, +{})", shown.join(", "), names.len() - 3)
                } else {
                    format!("{count} files ({})", shown.join(", "))
                }
            };
            (v, s)
        }
        ToolGroupKind::Search => {
            let v = if is_running {
                "Searching"
            } else if count == 1 && items[0].0 == "list_files" {
                "Listed"
            } else {
                "Searched"
            };
            let targets: Vec<String> = items
                .iter()
                .map(|(n, p, _, _)| clip_text(&extract_compact_target(n, p), 26, g.ellipsis))
                .collect();
            let s = if count == 1 {
                targets[0].clone()
            } else {
                let shown: Vec<String> = targets.iter().take(3).cloned().collect();
                format!("{count} queries ({})", shown.join(", "))
            };
            (v, s)
        }
        ToolGroupKind::Other => {
            let v = if is_running { "Calling" } else { "Called" };
            let s = if count == 1 {
                format!(
                    "{} ({})",
                    items[0].0,
                    clip_text(&extract_compact_target(items[0].0, items[0].1), 36, g.ellipsis)
                )
            } else {
                format!("{count} tools ({}, {})", items[0].0, g.ellipsis)
            };
            (v, s)
        }
    };

    let mut header_spans = vec![
        Span::styled(format!("  {dot} "), Style::default().fg(dot_color)),
        Span::styled(format!("{verb} "), Style::default().fg(pal.yellow)),
        Span::styled(summary, Style::default().fg(pal.primary)),
    ];
    if is_last_tool_group && !state.show_tool_details && !is_running {
        header_spans.push(Span::styled(
            " (ctrl+o to expand)",
            Style::default().fg(pal.muted),
        ));
    }
    lines.push(Line::from(header_spans));

    for (name, preview, output, is_error) in items {
        let should_expand = state.show_tool_details || is_error;
        if !should_expand {
            continue;
        }
        if count > 1 {
            let first = preview.lines().next().unwrap_or(name);
            lines.push(Line::from(Span::styled(
                format!("    · {first}"),
                Style::default().fg(if is_error { pal.red } else { pal.text }),
            )));
        }
        if name == "edit_file" || name == "write_file" {
            for l in preview.lines().skip(1).take(14) {
                let lc = if l.contains(" - ") || l.starts_with("- ") {
                    pal.red
                } else if l.contains(" + ") || l.starts_with("+ ") {
                    pal.green
                } else {
                    pal.muted
                };
                lines.push(Line::from(Span::styled(
                    format!("      {l}"),
                    Style::default().fg(lc),
                )));
            }
        }
        if let Some(out) = output {
            let limit = if is_error { 8 } else { default_out_limit.max(24) };
            for l in out.lines().take(limit) {
                lines.push(Line::from(Span::styled(
                    format!("      {l}"),
                    Style::default().fg(if is_error { pal.red } else { pal.muted }),
                )));
            }
            let total = out.lines().count();
            if total > limit {
                lines.push(Line::from(Span::styled(
                    format!("      ... ({} more lines)", total - limit),
                    Style::default().fg(pal.muted),
                )));
            }
        }
    }
}

fn wrap_line_chars(line: &str, max_chars: usize) -> Vec<String> {
    if line.is_empty() || max_chars == 0 {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut count = 0usize;
    for ch in line.chars() {
        cur.push(ch);
        count += 1;
        if count >= max_chars {
            out.push(std::mem::take(&mut cur));
            count = 0;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn render_markdown_lines<'a>(
    text: &'a str,
    pal: Palette,
    glyphs: Glyphs,
    monochrome: bool,
    out: &mut Vec<Line<'a>>,
) {
    let mut code_lang: Option<String> = None;
    for raw in text.lines() {
        let clean = sanitize_tabs(raw);
        let trimmed = clean.trim_start();
        if let Some(fence_rest) = trimmed.strip_prefix("```") {
            if code_lang.is_none() {
                let lang = fence_rest.trim().to_string();
                let (label, color) = highlight::lang_badge(&lang, pal);
                out.push(Line::from(vec![
                    Span::styled("  ┌─ ", Style::default().fg(pal.border)),
                    Span::styled(
                        label,
                        Style::default()
                            .fg(if monochrome { pal.primary } else { color })
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                code_lang = Some(lang);
            } else {
                out.push(Line::from(Span::styled(
                    "  └─",
                    Style::default().fg(pal.border),
                )));
                code_lang = None;
            }
            continue;
        }
        if let Some(lang) = &code_lang {
            let mut spans = vec![Span::styled("  │ ", Style::default().fg(pal.border))];
            for s in highlight::highlight_code_line(&clean, lang, pal, monochrome) {
                spans.push(Span::styled(s.content.into_owned(), s.style));
            }
            out.push(Line::from(spans));
            continue;
        }
        if trimmed.starts_with('#') {
            out.push(Line::from(Span::styled(
                clean,
                Style::default()
                    .fg(pal.primary)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let indent = clean.len() - trimmed.len();
            let mut spans = vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(glyphs.bullet, Style::default().fg(pal.accent)),
            ];
            for s in parse_inline_md(rest, pal) {
                spans.push(Span::styled(s.content.into_owned(), s.style));
            }
            out.push(Line::from(spans));
            continue;
        }
        let owned_spans: Vec<Span<'static>> = parse_inline_md(&clean, pal)
            .into_iter()
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect();
        out.push(Line::from(owned_spans));
    }
}

fn parse_inline_md<'a>(line: &'a str, pal: Palette) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let next_tick = rest.find('`');
        let next_bold = rest.find("**");
        let tick_first = match (next_tick, next_bold) {
            (Some(t), Some(b)) => t < b,
            (Some(_), None) => true,
            _ => false,
        };

        if tick_first {
            let t = next_tick.unwrap();
            if t > 0 {
                spans.push(Span::styled(&rest[..t], Style::default().fg(pal.text)));
            }
            let after = &rest[t + 1..];
            if let Some(end) = after.find('`') {
                spans.push(Span::styled(
                    &after[..end],
                    Style::default().fg(pal.cyan).bg(pal.bg_element),
                ));
                rest = &after[end + 1..];
            } else {
                spans.push(Span::styled(&rest[t..], Style::default().fg(pal.text)));
                break;
            }
        } else if let Some(b) = next_bold {
            if b > 0 {
                spans.push(Span::styled(&rest[..b], Style::default().fg(pal.text)));
            }
            let after = &rest[b + 2..];
            if let Some(end) = after.find("**") {
                spans.push(Span::styled(
                    &after[..end],
                    Style::default()
                        .fg(pal.primary)
                        .add_modifier(Modifier::BOLD),
                ));
                rest = &after[end + 2..];
            } else {
                spans.push(Span::styled(&rest[b..], Style::default().fg(pal.text)));
                break;
            }
        } else {
            spans.push(Span::styled(rest, Style::default().fg(pal.text)));
            break;
        }
    }
    spans
}

fn format_k(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}K", n as f64 / 1000.0)
    } else {
        format!("{n}")
    }
}

fn draw_footer(f: &mut Frame, state: &AppState, area: Rect, pal: Palette) {
    let spec = state.cfg.active_spec();
    let mode_color = match state.mode {
        AgentMode::Build => pal.primary,
        AgentMode::Plan => pal.secondary,
    };

    if matches!(state.cfg.ui_style, UiStyle::Antigravity) {
        let ctx_used = state.estimated_context_tokens();
        let ctx_max = state.cfg.context_window_tokens();
        let pct = ((ctx_used as f64 / ctx_max as f64) * 100.0).round() as u32;
        let ctx_col = if pct >= 80 {
            pal.red
        } else if pct >= 50 {
            pal.yellow
        } else {
            pal.muted
        };
        let left_text = if state.is_busy {
            format!(" {} esc stop", state.glyphs.spin(state.spinner_idx))
        } else {
            " ? for shortcuts · /style".to_string()
        };
        let auto_tag = if state.cfg.auto_approve {
            " · auto"
        } else {
            ""
        };
        let right_prefix = format!(
            "{} · {}{auto_tag} · ",
            state.cfg.active().model,
            state.mode.as_str().to_lowercase()
        );
        let right_ctx = format!("ctx {pct}% ");
        let total_w = area.width as usize;
        let right_w = right_prefix.chars().count() + right_ctx.chars().count();
        let mut row = Vec::new();
        if total_w > right_w + 4 {
            let max_left = total_w - right_w;
            let trimmed_left: String = left_text.chars().take(max_left).collect();
            let gap = total_w.saturating_sub(trimmed_left.chars().count() + right_w);
            row.push(Span::styled(
                trimmed_left,
                Style::default().fg(if state.is_busy { pal.accent } else { pal.muted }),
            ));
            if gap > 0 {
                row.push(Span::raw(" ".repeat(gap)));
            }
            row.push(Span::styled(right_prefix, Style::default().fg(pal.muted)));
            row.push(Span::styled(right_ctx, Style::default().fg(ctx_col)));
        } else {
            row.push(Span::styled(left_text, Style::default().fg(pal.muted)));
        }
        f.render_widget(Paragraph::new(Line::from(row)), area);
        return;
    }

    let dir_str = state.work_dir.display().to_string();
    let short_dir = if area.width < 110 && dir_str.chars().count() > 22 {
        let tail: String = dir_str
            .chars()
            .rev()
            .take(19)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("...{tail}")
    } else {
        dir_str
    };

    let mut spans = vec![
        Span::styled(
            format!(" {} ", state.mode.as_str()),
            Style::default()
                .fg(pal.bg)
                .bg(mode_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default().bg(pal.bg_panel)),
        Span::styled(
            format!("{}/{}", spec.id, state.cfg.active().model),
            Style::default().fg(pal.text).bg(pal.bg_panel),
        ),
        Span::styled(" | ", Style::default().fg(pal.border).bg(pal.bg_panel)),
        Span::styled(short_dir, Style::default().fg(pal.muted).bg(pal.bg_panel)),
    ];

    if let Some(branch) = &state.git_branch {
        spans.push(Span::styled(
            format!(" ({branch})"),
            Style::default().fg(pal.green).bg(pal.bg_panel),
        ));
    }

    if state.cfg.auto_approve {
        spans.push(Span::styled(
            " [AUTO]",
            Style::default().fg(pal.yellow).bg(pal.bg_panel),
        ));
    }

    let ctx_used = state.estimated_context_tokens();
    let ctx_max = state.cfg.context_window_tokens();
    let pct = ((ctx_used as f64 / ctx_max as f64) * 100.0).round() as u32;
    let ctx_col = if pct >= 80 {
        pal.red
    } else if pct >= 50 {
        pal.yellow
    } else {
        pal.muted
    };

    let right_ctx = format!(
        " ctx: {}/{} ({pct}%) ",
        format_k(ctx_used),
        format_k(ctx_max)
    );
    let right_tail = if state.is_busy {
        format!("{} esc stop ", state.glyphs.spin(state.spinner_idx))
    } else {
        "| ctrl+p ".to_string()
    };

    let line = fill_row(spans, area.width as usize, pal.bg_panel);
    let mut fixed = line.spans;
    let right_len = right_ctx.chars().count() + right_tail.chars().count();
    let total_w = area.width as usize;
    if total_w > right_len + 8 {
        let mut left_budget = total_w - right_len;
        let mut clipped_left: Vec<Span<'_>> = Vec::new();
        for sp in fixed {
            if left_budget == 0 {
                break;
            }
            let w = sp.content.chars().count();
            if w <= left_budget {
                left_budget -= w;
                clipped_left.push(sp);
            } else {
                let take: String = sp.content.chars().take(left_budget).collect();
                clipped_left.push(Span::styled(take, sp.style));
                left_budget = 0;
            }
        }
        if left_budget > 0 {
            clipped_left.push(Span::styled(
                " ".repeat(left_budget),
                Style::default().bg(pal.bg_panel),
            ));
        }
        clipped_left.push(Span::styled(
            right_ctx,
            Style::default().fg(ctx_col).bg(pal.bg_panel),
        ));
        clipped_left.push(Span::styled(
            right_tail,
            Style::default()
                .fg(if state.is_busy { pal.accent } else { pal.muted })
                .bg(pal.bg_panel),
        ));
        fixed = clipped_left;
    }

    f.render_widget(Paragraph::new(Line::from(fixed)), area);
}

fn styled_block<'a>(title: &str, border_color: Color, pal: Palette) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(Glyphs::detect().border)
        .border_style(Style::default().fg(border_color).bg(pal.bg_panel))
        .style(Style::default().bg(pal.bg_panel))
        .title(Span::styled(
            title.to_string(),
            Style::default().fg(pal.primary).bg(pal.bg_panel),
        ))
}

fn visible_window(total: usize, selected: usize, visible_rows: usize) -> (usize, usize) {
    if total == 0 || visible_rows == 0 {
        return (0, 0);
    }
    let sel = selected.min(total - 1);
    if total <= visible_rows {
        return (0, sel);
    }
    let max_start = total - visible_rows;
    let start = sel.saturating_sub(visible_rows / 2).min(max_start);
    (start, sel)
}

fn fill_row<'a>(spans: Vec<Span<'a>>, width: usize, bg: Color) -> Line<'a> {
    let mut clipped: Vec<Span<'a>> = Vec::with_capacity(spans.len() + 1);
    let mut used = 0usize;
    for s in spans {
        if used >= width {
            break;
        }
        let len = s.content.chars().count();
        if used + len <= width {
            used += len;
            clipped.push(s);
        } else {
            let take = width - used;
            let partial: String = s.content.chars().take(take).collect();
            used += take;
            clipped.push(Span::styled(partial, s.style));
            break;
        }
    }
    if used < width {
        clipped.push(Span::styled(" ".repeat(width - used), Style::default().bg(bg)));
    }
    Line::from(clipped)
}

fn fill_list_row<'a>(
    mut spans: Vec<Span<'a>>,
    width: usize,
    active: bool,
    bg: Color,
    pal: Palette,
) -> Line<'a> {
    let suffix = if active {
        if Glyphs::detect().legacy_console { "> " } else { "→ " }
    } else {
        "  "
    };
    let suffix_len = 2;
    let max_content = width.saturating_sub(suffix_len);
    let mut used = 0usize;
    let mut clipped: Vec<Span<'a>> = Vec::with_capacity(spans.len() + 2);
    for s in spans.drain(..) {
        if used >= max_content {
            break;
        }
        let len = s.content.chars().count();
        if used + len <= max_content {
            used += len;
            clipped.push(s);
        } else {
            let take = max_content - used;
            let partial: String = s.content.chars().take(take).collect();
            used += take;
            clipped.push(Span::styled(partial, s.style));
            break;
        }
    }
    if used < max_content {
        clipped.push(Span::styled(
            " ".repeat(max_content - used),
            Style::default().bg(bg),
        ));
    }
    clipped.push(Span::styled(
        suffix,
        Style::default()
            .fg(if active { pal.primary } else { pal.muted })
            .bg(bg),
    ));
    Line::from(clipped)
}

fn modal_hint_row<'a>(hint: &str, width: usize, pal: Palette) -> Line<'a> {
    let clipped: String = format!(" {hint}").chars().take(width).collect();
    fill_row(
        vec![Span::styled(
            clipped,
            Style::default()
                .fg(pal.muted)
                .bg(pal.bg_panel)
                .add_modifier(Modifier::ITALIC),
        )],
        width,
        pal.bg_panel,
    )
}

fn draw_autocomplete(f: &mut Frame, state: &AppState, input_area: Rect, pal: Palette) {
    let at_files = state.filtered_at_files();
    let slash_cmds = state.filtered_slash_cmds();

    let items: Vec<(String, String, Option<highlight::FileIcon>)> = if !at_files.is_empty() {
        at_files
            .into_iter()
            .map(|f| {
                let fi = highlight::file_icon(&f, pal);
                (format!("@{f}"), String::new(), Some(fi))
            })
            .collect()
    } else if !slash_cmds.is_empty() {
        slash_cmds
            .into_iter()
            .map(|(c, d)| (c, d, None))
            .collect()
    } else {
        return;
    };

    let height = (items.len() as u16 + 2).min(12);
    let width = input_area.width;
    let popup_y = if input_area.y > height {
        input_area.y - height
    } else {
        input_area.y + input_area.height
    };
    let rect = Rect {
        x: input_area.x,
        y: popup_y,
        width,
        height,
    };

    let inner_w = rect.width.saturating_sub(2) as usize;
    let list_rows = rect.height.saturating_sub(2) as usize;
    let (start, sel) = visible_window(items.len(), state.ac_selected, list_rows);

    f.render_widget(Clear, rect);
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(start)
        .take(list_rows)
        .map(|(i, (label, desc, icon))| {
            let active = i == sel;
            let bg = if active { pal.bg_element } else { pal.bg_panel };
            let mut spans = vec![Span::styled(
                if active { " > " } else { "   " },
                Style::default().fg(pal.primary).bg(bg),
            )];
            if let Some(fi) = icon {
                spans.push(Span::styled(
                    format!("{} ", fi.badge),
                    Style::default().fg(fi.color).bg(bg),
                ));
            }
            spans.push(Span::styled(
                format!("{label:<14} "),
                Style::default()
                    .fg(if active { pal.primary } else { pal.text })
                    .bg(bg)
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ));
            let prefix_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let avail_desc = inner_w.saturating_sub(prefix_len + 3);
            let clipped_desc: String = desc.chars().take(avail_desc).collect();
            spans.push(Span::styled(
                clipped_desc,
                Style::default().fg(pal.muted).bg(bg),
            ));
            fill_list_row(spans, inner_w, active, bg, pal)
        })
        .collect();

    let title = if items.len() > list_rows {
        format!(
            " Enter: run/insert   Tab: fill ({}/{}) ",
            sel + 1,
            items.len()
        )
    } else {
        " Enter: run/insert   Tab: fill ".to_string()
    };

    let popup = Paragraph::new(lines)
        .style(Style::default().bg(pal.bg_panel))
        .block(styled_block(&title, pal.border_active, pal));
    f.render_widget(popup, rect);
}

fn centered_rect(percent_x: u16, max_w: u16, desired_h: u16, area: Rect) -> Rect {
    let w = (area.width * percent_x / 100)
        .max(42)
        .min(max_w)
        .min(area.width);
    let h = desired_h.min(area.height.saturating_sub(2)).max(6);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn draw_overlay(f: &mut Frame, state: &AppState, overlay: &Overlay, area: Rect, pal: Palette) {
    match overlay {
        Overlay::CommandPalette { filter, selected } => {
            let matches: Vec<(String, String)> = state
                .all_slash_entries()
                .into_iter()
                .filter(|(c, _)| filter.is_empty() || c.contains(&filter.to_lowercase()))
                .collect();
            let rect = centered_rect(68, 86, (matches.len() as u16 + 7).clamp(11, 22), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);

            let mut lines = vec![
                fill_row(
                    vec![
                        Span::styled(" > ", Style::default().fg(pal.accent).bg(pal.bg_panel)),
                        Span::styled(
                            filter.as_str(),
                            Style::default().fg(pal.text).bg(pal.bg_panel),
                        ),
                    ],
                    inner_w,
                    pal.bg_panel,
                ),
                Line::from(Span::styled(
                    "─".repeat(inner_w),
                    Style::default().fg(pal.border).bg(pal.bg_panel),
                )),
            ];
            let list_rows = rect.height.saturating_sub(6) as usize;
            let (start, sel) = visible_window(matches.len(), *selected, list_rows);
            for (i, (cmd, desc)) in matches.iter().enumerate().skip(start).take(list_rows) {
                let active = i == sel;
                let bg = if active { pal.bg_element } else { pal.bg_panel };
                let avail = inner_w.saturating_sub(19);
                let clipped: String = desc.chars().take(avail).collect();
                lines.push(fill_list_row(
                    vec![
                        Span::styled(
                            format!(" {cmd:<14} "),
                            Style::default()
                                .fg(if active { pal.primary } else { pal.text })
                                .bg(bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(clipped, Style::default().fg(pal.muted).bg(bg)),
                    ],
                    inner_w,
                    active,
                    bg,
                    pal,
                ));
            }
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w),
                Style::default().fg(pal.border).bg(pal.bg_panel),
            )));
            lines.push(modal_hint_row(
                "↑/↓ navigate · Enter select · Esc close",
                inner_w,
                pal,
            ));
            let title = if matches.len() > list_rows && !matches.is_empty() {
                format!(" Command Palette ({}/{}) ", sel + 1, matches.len())
            } else {
                " Command Palette ".to_string()
            };
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(&title, pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::ConnectProvider { selected } => {
            let rect = centered_rect(68, 86, (PROVIDERS.len() as u16 + 4).min(18), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            let list_rows = rect.height.saturating_sub(4) as usize;
            let (start, sel) = visible_window(PROVIDERS.len(), *selected, list_rows);
            f.render_widget(Clear, rect);
            let mut lines = Vec::new();
            for (i, spec) in PROVIDERS.iter().enumerate().skip(start).take(list_rows) {
                let active = i == sel;
                let bg = if active { pal.bg_element } else { pal.bg_panel };
                let status = if !spec.requires_key {
                    ("local/custom", pal.cyan)
                } else if state.cfg.has_key(spec.id) {
                    ("key set", pal.green)
                } else {
                    ("no key", pal.muted)
                };
                let cur = if state.cfg.provider == spec.id { "*" } else { " " };
                lines.push(fill_list_row(
                    vec![
                        Span::styled(
                            format!(" {cur} {:<25} ", spec.display_name),
                            Style::default()
                                .fg(if active { pal.primary } else { pal.text })
                                .bg(bg)
                                .add_modifier(if active {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(
                            format!("[{}]", status.0),
                            Style::default().fg(status.1).bg(bg),
                        ),
                    ],
                    inner_w,
                    active,
                    bg,
                    pal,
                ));
            }
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w),
                Style::default().fg(pal.border).bg(pal.bg_panel),
            )));
            lines.push(modal_hint_row(
                if state.overlay_back.is_empty() {
                    "↑/↓ navigate · Enter configure · Esc close"
                } else {
                    "↑/↓ navigate · Enter configure · Esc back"
                },
                inner_w,
                pal,
            ));
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(" Connect Provider ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::ConnectForm {
            provider_id,
            base_url,
            api_key,
            model,
            focus,
        } => {
            let spec = config::find_spec(provider_id);
            let rect = centered_rect(76, 96, 11, area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);
            let masked_key = if api_key.is_empty() {
                "(empty - Ctrl+V or Right-Click to paste)".to_string()
            } else {
                let len = api_key.chars().count();
                let tail: String = api_key
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                if len > 6 {
                    format!("{}...{tail} ({len} chars)", "*".repeat((len - 4).min(20)))
                } else {
                    "*".repeat(len)
                }
            };
            let field_line = |idx: usize, label: &str, val: &str| {
                let active = *focus == idx;
                let bg = if active { pal.bg_element } else { pal.bg_panel };
                fill_row(
                    vec![
                        Span::styled(
                            format!(" {label:<10}: "),
                            Style::default()
                                .fg(if active { pal.primary } else { pal.muted })
                                .bg(bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(val.to_string(), Style::default().fg(pal.text).bg(bg)),
                    ],
                    inner_w,
                    bg,
                )
            };
            let lines = vec![
                fill_row(
                    vec![Span::styled(
                        format!(" Provider: {} ({})", spec.display_name, spec.id),
                        Style::default()
                            .fg(pal.accent)
                            .bg(pal.bg_panel)
                            .add_modifier(Modifier::BOLD),
                    )],
                    inner_w,
                    pal.bg_panel,
                ),
                fill_row(vec![], inner_w, pal.bg_panel),
                field_line(0, "API Key", &masked_key),
                field_line(1, "Base URL", base_url),
                field_line(2, "Model", model),
                fill_row(vec![], inner_w, pal.bg_panel),
                fill_row(
                    vec![Span::styled(
                        " Ctrl+V: paste   Ctrl+M: models   Enter: save   Esc: back",
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ),
            ];
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(" Configure API Provider ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::ModelPicker {
            filter,
            selected,
            models,
            loading,
            error,
        } => {
            let rect = centered_rect(76, 98, (area.height * 65 / 100).max(14), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);
            let mut lines = vec![
                fill_row(
                    vec![
                        Span::styled(" Filter: ", Style::default().fg(pal.accent).bg(pal.bg_panel)),
                        Span::styled(
                            filter.as_str(),
                            Style::default().fg(pal.text).bg(pal.bg_panel),
                        ),
                    ],
                    inner_w,
                    pal.bg_panel,
                ),
                Line::from(Span::styled(
                    "─".repeat(inner_w),
                    Style::default().fg(pal.border).bg(pal.bg_panel),
                )),
            ];
            let esc_action = if state.overlay_back.is_empty() {
                "close"
            } else {
                "back"
            };
            let mut title = format!(" Select Model (Enter to apply, Esc to {esc_action}) ");
            if *loading {
                lines.push(fill_row(
                    vec![Span::styled(
                        " Fetching models from provider...",
                        Style::default().fg(pal.yellow).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
            } else if let Some(err) = error {
                lines.push(fill_row(
                    vec![Span::styled(
                        format!(" {err}"),
                        Style::default().fg(pal.red).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
                lines.push(fill_row(
                    vec![Span::styled(
                        " Tip: type or paste any model ID above and press Enter.",
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
            } else {
                let matches: Vec<&String> = models
                    .iter()
                    .filter(|m| {
                        filter.is_empty() || m.to_lowercase().contains(&filter.to_lowercase())
                    })
                    .collect();
                let list_rows = rect.height.saturating_sub(6) as usize;
                let (start, sel) = visible_window(matches.len(), *selected, list_rows);
                if matches.len() > list_rows {
                    title = format!(" Select Model ({}/{}) ", sel + 1, matches.len());
                }
                for (i, m) in matches.iter().enumerate().skip(start).take(list_rows) {
                    let active = i == sel;
                    let bg = if active { pal.bg_element } else { pal.bg_panel };
                    let cur = if *m == &state.cfg.active().model { "*" } else { " " };
                    lines.push(fill_list_row(
                        vec![Span::styled(
                            format!(" {cur} {m}"),
                            Style::default()
                                .fg(if active { pal.primary } else { pal.text })
                                .bg(bg)
                                .add_modifier(if active {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        )],
                        inner_w,
                        active,
                        bg,
                        pal,
                    ));
                }
                lines.push(Line::from(Span::styled(
                    "─".repeat(inner_w),
                    Style::default().fg(pal.border).bg(pal.bg_panel),
                )));
                lines.push(modal_hint_row(
                    if state.overlay_back.is_empty() {
                        "↑/↓ navigate · Enter apply · Esc close"
                    } else {
                        "↑/↓ navigate · Enter apply · Esc back"
                    },
                    inner_w,
                    pal,
                ));
            }
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(&title, pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::ThemePicker {
            selected,
            original: _,
        } => {
            let themes = ThemeKind::all();
            let rect = centered_rect(64, 76, (themes.len() as u16 + 4).min(20), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);
            let list_rows = rect.height.saturating_sub(4) as usize;
            let (start, sel) = visible_window(themes.len(), *selected, list_rows);
            let mut lines: Vec<Line> = themes
                .iter()
                .enumerate()
                .skip(start)
                .take(list_rows)
                .map(|(i, t)| {
                    let active = i == sel;
                    let tp = palette_for(*t);
                    let bg = if active { pal.bg_element } else { pal.bg_panel };
                    fill_list_row(
                        vec![
                            Span::styled(
                                if active { state.glyphs.list_cursor } else { "   " },
                                Style::default().fg(pal.primary).bg(bg),
                            ),
                            Span::styled("■", Style::default().fg(tp.primary).bg(bg)),
                            Span::styled("■", Style::default().fg(tp.accent).bg(bg)),
                            Span::styled("■", Style::default().fg(tp.green).bg(bg)),
                            Span::styled("■ ", Style::default().fg(tp.red).bg(bg)),
                            Span::styled(
                                t.label(),
                                Style::default()
                                    .fg(if active { pal.primary } else { pal.text })
                                    .bg(bg)
                                    .add_modifier(if active {
                                        Modifier::BOLD
                                    } else {
                                        Modifier::empty()
                                    }),
                            ),
                        ],
                        inner_w,
                        active,
                        bg,
                        pal,
                    )
                })
                .collect();
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w),
                Style::default().fg(pal.border).bg(pal.bg_panel),
            )));
            lines.push(modal_hint_row(
                if state.overlay_back.is_empty() {
                    "↑/↓ preview · Enter apply · Esc revert"
                } else {
                    "↑/↓ preview · Enter apply · Esc back"
                },
                inner_w,
                pal,
            ));
            let title = format!(" Theme Picker ({}/{}) ", sel + 1, themes.len());
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(&title, pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::StylePicker {
            selected,
            original: _,
            original_fox: _,
        } => {
            let styles = UiStyle::all();
            let rect = centered_rect(76, 86, (styles.len() as u16 * 2 + 5).min(14), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);
            let sel = (*selected).min(styles.len().saturating_sub(1));
            let mut lines: Vec<Line> = Vec::new();
            for (i, st) in styles.iter().enumerate() {
                let active = i == sel;
                let bg = if active { pal.bg_element } else { pal.bg_panel };
                lines.push(fill_list_row(
                    vec![
                        Span::styled(
                            if active { state.glyphs.list_cursor } else { "   " },
                            Style::default().fg(pal.primary).bg(bg),
                        ),
                        Span::styled(
                            st.label(),
                            Style::default()
                                .fg(if active { pal.primary } else { pal.text })
                                .bg(bg)
                                .add_modifier(if active {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                    ],
                    inner_w,
                    active,
                    bg,
                    pal,
                ));
                lines.push(fill_list_row(
                    vec![Span::styled(
                        format!("     {}", st.description()),
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    false,
                    pal.bg_panel,
                    pal,
                ));
            }
            let hch = if state.glyphs.legacy_console { "-" } else { "─" };
            lines.push(Line::from(Span::styled(
                hch.repeat(inner_w),
                Style::default().fg(pal.border).bg(pal.bg_panel),
            )));
            lines.push(modal_hint_row(
                if state.overlay_back.is_empty() {
                    "↑/↓ preview · Enter apply · Esc revert"
                } else {
                    "↑/↓ preview · Enter apply · Esc back"
                },
                inner_w,
                pal,
            ));
            let title = format!(" UI Style Picker ({}/{}) ", sel + 1, styles.len());
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(&title, pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::DirPicker { path_input, error } => {
            let rect = centered_rect(76, 96, 8, area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);
            let exists = std::path::Path::new(path_input.trim().trim_matches('"')).is_dir();
            let status_span = if let Some(err) = error {
                Span::styled(format!(" {err}"), Style::default().fg(pal.red).bg(pal.bg_panel))
            } else if exists {
                Span::styled(
                    format!(" {} valid directory", state.glyphs.check_ok),
                    Style::default().fg(pal.green).bg(pal.bg_panel),
                )
            } else {
                Span::styled(
                    format!(" {} directory not found", state.glyphs.check_err),
                    Style::default().fg(pal.yellow).bg(pal.bg_panel),
                )
            };

            let lines = vec![
                fill_row(
                    vec![
                        Span::styled(
                            " Path: ",
                            Style::default()
                                .fg(pal.primary)
                                .bg(pal.bg_element)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            path_input.as_str(),
                            Style::default().fg(pal.text).bg(pal.bg_element),
                        ),
                    ],
                    inner_w,
                    pal.bg_element,
                ),
                fill_row(vec![status_span], inner_w, pal.bg_panel),
                fill_row(vec![], inner_w, pal.bg_panel),
                fill_row(
                    vec![Span::styled(
                        if state.overlay_back.is_empty() {
                            " Ctrl+V / Right-Click: paste path   Ctrl+U: clear   Enter: switch"
                        } else {
                            " Ctrl+V: paste   Ctrl+U: clear   Enter: switch   Esc: back"
                        },
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ),
            ];
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(
                    " Workspace / Project Directory ",
                    pal.primary,
                    pal,
                ));
            f.render_widget(widget, rect);
        }
        Overlay::SessionPicker { sessions, selected } => {
            let rect = centered_rect(78, 100, (sessions.len() as u16 + 4).clamp(9, 20), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);
            let list_rows = rect.height.saturating_sub(4) as usize;
            let (start, sel) = visible_window(sessions.len(), *selected, list_rows);
            let mut lines: Vec<Line> = if sessions.is_empty() {
                vec![fill_row(
                    vec![Span::styled(
                        " No saved sessions yet.",
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                )]
            } else {
                sessions
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(list_rows)
                    .map(|(i, s)| {
                        let active = i == sel;
                        let bg = if active { pal.bg_element } else { pal.bg_panel };
                        let title_clip: String = s.title.chars().take(36).collect();
                        fill_list_row(
                            vec![
                                Span::styled(
                                    if active { " › " } else { "   " },
                                    Style::default().fg(pal.primary).bg(bg),
                                ),
                                Span::styled(
                                    format!("{title_clip:<37} "),
                                    Style::default()
                                        .fg(if active { pal.primary } else { pal.text })
                                        .bg(bg)
                                        .add_modifier(if active {
                                            Modifier::BOLD
                                        } else {
                                            Modifier::empty()
                                        }),
                                ),
                                Span::styled(
                                    format!("{} ({} msgs)", s.model, s.messages.len()),
                                    Style::default().fg(pal.muted).bg(bg),
                                ),
                            ],
                            inner_w,
                            active,
                            bg,
                            pal,
                        )
                    })
                    .collect()
            };
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w),
                Style::default().fg(pal.border).bg(pal.bg_panel),
            )));
            lines.push(modal_hint_row(
                "↑/↓ navigate · Enter resume · d/Del delete · Esc close",
                inner_w,
                pal,
            ));
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(" Saved Sessions ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::McpStatus { servers, loading } => {
            let rect = centered_rect(76, 94, (servers.len() as u16 + 7).clamp(9, 18), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            f.render_widget(Clear, rect);
            let mut lines = Vec::new();
            if *loading {
                lines.push(fill_row(
                    vec![Span::styled(
                        " Checking MCP servers...",
                        Style::default().fg(pal.yellow).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
            } else if servers.is_empty() {
                lines.push(fill_row(
                    vec![Span::styled(
                        " No MCP servers configured.",
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
                lines.push(fill_row(vec![], inner_w, pal.bg_panel));
                lines.push(fill_row(
                    vec![Span::styled(
                        " Add [mcp_servers.<name>] to config.toml or .oxicode.toml:",
                        Style::default().fg(pal.primary).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
                lines.push(fill_row(
                    vec![Span::styled(
                        "   command = \"npx\"",
                        Style::default().fg(pal.cyan).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
                lines.push(fill_row(
                    vec![Span::styled(
                        "   args = [\"-y\", \"@modelcontextprotocol/server-filesystem\", \".\"]",
                        Style::default().fg(pal.cyan).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
            } else {
                for (name, count, err) in servers {
                    if let Some(e) = err {
                        lines.push(fill_row(
                            vec![
                                Span::styled(
                                    format!(" {} {name}: ", state.glyphs.check_err),
                                    Style::default().fg(pal.red).bg(pal.bg_panel),
                                ),
                                Span::styled(
                                    e.as_str(),
                                    Style::default().fg(pal.muted).bg(pal.bg_panel),
                                ),
                            ],
                            inner_w,
                            pal.bg_panel,
                        ));
                    } else {
                        lines.push(fill_row(
                            vec![
                                Span::styled(
                                    format!(" {} {name}: ", state.glyphs.check_ok),
                                    Style::default().fg(pal.green).bg(pal.bg_panel),
                                ),
                                Span::styled(
                                    format!("{count} tools loaded"),
                                    Style::default().fg(pal.text).bg(pal.bg_panel),
                                ),
                            ],
                            inner_w,
                            pal.bg_panel,
                        ));
                    }
                }
            }
            lines.push(fill_row(vec![], inner_w, pal.bg_panel));
            lines.push(fill_row(
                vec![Span::styled(
                    " [r] Reload MCP servers   [Esc] Close",
                    Style::default().fg(pal.muted).bg(pal.bg_panel),
                )],
                inner_w,
                pal.bg_panel,
            ));
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(" MCP Servers ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::Skills { selected, status } => {
            let skills = &state.extensions.skills;
            let rect = centered_rect(76, 94, (skills.len() as u16 + 6).clamp(10, 18), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            let list_rows = rect.height.saturating_sub(4) as usize;
            let (start, sel) = visible_window(skills.len(), *selected, list_rows);
            f.render_widget(Clear, rect);
            let mut lines = Vec::new();
            if skills.is_empty() {
                lines.push(fill_row(
                    vec![Span::styled(
                        " No skills found in .oxicode/skills/<name>/SKILL.md",
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
            } else {
                for (i, sk) in skills.iter().enumerate().skip(start).take(list_rows) {
                    let active = i == sel;
                    let bg = if active { pal.bg_element } else { pal.bg_panel };
                    let desc: String = sk
                        .description
                        .chars()
                        .take(inner_w.saturating_sub(36))
                        .collect();
                    lines.push(fill_row(
                        vec![
                            Span::styled(
                                if active { " > " } else { "   " },
                                Style::default().fg(pal.primary).bg(bg),
                            ),
                            Span::styled(
                                format!("{:<16} ", sk.name),
                                Style::default()
                                    .fg(if active { pal.primary } else { pal.text })
                                    .bg(bg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("[{}] ", sk.source),
                                Style::default().fg(pal.cyan).bg(bg),
                            ),
                            Span::styled(desc, Style::default().fg(pal.muted).bg(bg)),
                        ],
                        inner_w,
                        bg,
                    ));
                }
            }
            lines.push(fill_row(vec![], inner_w, pal.bg_panel));
            let footer_msg = status
                .as_deref()
                .unwrap_or("Enter: insert skill   n: create starter SKILL.md   Esc: close");
            lines.push(fill_row(
                vec![Span::styled(
                    format!(" {footer_msg}"),
                    Style::default().fg(pal.muted).bg(pal.bg_panel),
                )],
                inner_w,
                pal.bg_panel,
            ));
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(" Skills ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::Plugins { selected, status } => {
            let plugins = &state.extensions.plugins;
            let rect = centered_rect(76, 94, (plugins.len() as u16 + 6).clamp(10, 18), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            let list_rows = rect.height.saturating_sub(4) as usize;
            let (start, sel) = visible_window(plugins.len(), *selected, list_rows);
            f.render_widget(Clear, rect);
            let mut lines = Vec::new();
            if plugins.is_empty() {
                lines.push(fill_row(
                    vec![Span::styled(
                        " No plugins found in .oxicode/plugins/<name>/plugin.toml",
                        Style::default().fg(pal.muted).bg(pal.bg_panel),
                    )],
                    inner_w,
                    pal.bg_panel,
                ));
            } else {
                for (i, pl) in plugins.iter().enumerate().skip(start).take(list_rows) {
                    let active = i == sel;
                    let bg = if active { pal.bg_element } else { pal.bg_panel };
                    let (badge, bcol) = if pl.enabled {
                        (format!("{} on ", state.glyphs.check_ok), pal.green)
                    } else {
                        ("o off".to_string(), pal.muted)
                    };
                    lines.push(fill_row(
                        vec![
                            Span::styled(
                                if active { " > " } else { "   " },
                                Style::default().fg(pal.primary).bg(bg),
                            ),
                            Span::styled(
                                format!("[{badge}] "),
                                Style::default().fg(bcol).bg(bg),
                            ),
                            Span::styled(
                                format!("{} v{} ", pl.name, pl.version),
                                Style::default()
                                    .fg(if active { pal.primary } else { pal.text })
                                    .bg(bg)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!(
                                    "({}t/{}s/{}c/{}m) {}",
                                    pl.tool_count,
                                    pl.skill_count,
                                    pl.command_count,
                                    pl.mcp_count,
                                    pl.description
                                ),
                                Style::default().fg(pal.muted).bg(bg),
                            ),
                        ],
                        inner_w,
                        bg,
                    ));
                }
            }
            lines.push(fill_row(vec![], inner_w, pal.bg_panel));
            let footer_msg = status
                .as_deref()
                .unwrap_or("Space/Enter: toggle   n: create starter plugin   Esc: close");
            lines.push(fill_row(
                vec![Span::styled(
                    format!(" {footer_msg}"),
                    Style::default().fg(pal.muted).bg(pal.bg_panel),
                )],
                inner_w,
                pal.bg_panel,
            ));
            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(" Plugins ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::Settings { selected } => {
            let spec = state.cfg.active_spec();
            let enabled_plugins = state.extensions.plugins.iter().filter(|p| p.enabled).count();
            let rows: Vec<(&str, String, Color)> = vec![
                (
                    "Provider & API",
                    format!(
                        "{} [{}]",
                        spec.display_name,
                        if state.cfg.has_key(spec.id) {
                            "key set"
                        } else if !spec.requires_key {
                            "local/custom"
                        } else {
                            "no key"
                        }
                    ),
                    if state.cfg.has_key(spec.id) || !spec.requires_key {
                        pal.green
                    } else {
                        pal.red
                    },
                ),
                ("Model", state.cfg.active().model.clone(), pal.cyan),
                ("Theme", state.cfg.theme.label().to_string(), pal.primary),
                (
                    "UI Style",
                    format!("{} (←/→ or Enter)", state.cfg.ui_style.label()),
                    pal.accent,
                ),
                (
                    "Default Mode",
                    if state.cfg.default_plan_mode {
                        "PLAN (read-only analysis)".to_string()
                    } else {
                        "BUILD (full edits & shell)".to_string()
                    },
                    if state.cfg.default_plan_mode {
                        pal.secondary
                    } else {
                        pal.primary
                    },
                ),
                (
                    "Auto-Approve",
                    if state.cfg.auto_approve {
                        "ON (skip confirmations)".to_string()
                    } else {
                        "OFF (prompt before edits/shell)".to_string()
                    },
                    if state.cfg.auto_approve {
                        pal.yellow
                    } else {
                        pal.green
                    },
                ),
                (
                    "Max Output Tokens",
                    format!("{} tokens (←/→)", state.cfg.max_tokens),
                    pal.text,
                ),
                (
                    "Tool Output Details",
                    if state.cfg.show_tool_details {
                        "Expanded by default".to_string()
                    } else {
                        "Compact by default".to_string()
                    },
                    pal.text,
                ),
                (
                    "Mouse Capture",
                    if state.cfg.mouse_capture {
                        "ON (wheel scroll / hover fox)".to_string()
                    } else {
                        "OFF (native select & copy)".to_string()
                    },
                    if state.cfg.mouse_capture {
                        pal.yellow
                    } else {
                        pal.green
                    },
                ),
                (
                    "Workspace Dir",
                    state.work_dir.display().to_string(),
                    pal.cyan,
                ),
                (
                    "Plugins & Skills",
                    format!(
                        "{} skills · {}/{} plugins · {} cmds",
                        state.extensions.skills.len(),
                        enabled_plugins,
                        state.extensions.plugins.len(),
                        state.custom_commands.len()
                    ),
                    pal.text,
                ),
                (
                    "MCP Servers",
                    format!("{} configured (Enter to inspect)", state.cfg.mcp_servers.len()),
                    pal.text,
                ),
            ];

            let rect = centered_rect(82, 94, (rows.len() as u16 + 4).min(20), area);
            let inner_w = rect.width.saturating_sub(2) as usize;
            let list_rows = rect.height.saturating_sub(4) as usize;
            let (start, sel) = visible_window(rows.len(), *selected, list_rows);
            f.render_widget(Clear, rect);

            let mut lines: Vec<Line> = rows
                .iter()
                .enumerate()
                .skip(start)
                .take(list_rows)
                .map(|(i, (label, val, val_col))| {
                    let active = i == sel;
                    let bg = if active { pal.bg_element } else { pal.bg_panel };
                    fill_list_row(
                        vec![
                            Span::styled(
                                if active { state.glyphs.list_cursor } else { "   " },
                                Style::default().fg(pal.primary).bg(bg),
                            ),
                            Span::styled(
                                format!("{label:<21} "),
                                Style::default()
                                    .fg(if active { pal.primary } else { pal.text })
                                    .bg(bg)
                                    .add_modifier(if active {
                                        Modifier::BOLD
                                    } else {
                                        Modifier::empty()
                                    }),
                            ),
                            Span::styled(val.clone(), Style::default().fg(*val_col).bg(bg)),
                        ],
                        inner_w,
                        active,
                        bg,
                        pal,
                    )
                })
                .collect();

            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w),
                Style::default().fg(pal.border).bg(pal.bg_panel),
            )));
            let cfg_hint = Config::global_config_path()
                .and_then(|p| p.file_name().map(|n| format!("↑/↓ navigate · Enter/←/→ change · Esc close · {}", n.to_string_lossy())))
                .unwrap_or_else(|| "↑/↓ navigate · Enter/←/→ change · Esc close".to_string());
            lines.push(modal_hint_row(&cfg_hint, inner_w, pal));

            let widget = Paragraph::new(lines)
                .style(Style::default().bg(pal.bg_panel))
                .block(styled_block(" Settings ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
        Overlay::Help => {
            let rect = centered_rect(72, 90, 23, area);
            f.render_widget(Clear, rect);
            let lines = vec![
                Line::from(Span::styled(
                    " Keybindings",
                    Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
                )),
                Line::from("  Ctrl+P          Command palette"),
                Line::from("  Ctrl+X o        Unified Settings (/settings)"),
                Line::from("  Ctrl+X c        Connect / configure API provider"),
                Line::from("  Ctrl+X m        Model picker (live from provider)"),
                Line::from("  Ctrl+X t        Theme picker (15 themes)"),
                Line::from("  Ctrl+X w        Change workspace directory (/dir)"),
                Line::from("  Ctrl+X s        Resume saved session (/sessions)"),
                Line::from("  Ctrl+X u / r    Undo / Redo file edits"),
                Line::from("  Ctrl+X k        Compact conversation (/compact)"),
                Line::from("  Ctrl+X p        MCP servers status (/mcp)"),
                Line::from("  Ctrl+X e        Export session to Markdown (/export)"),
                Line::from("  Ctrl+X y        Copy last AI response (/copy)"),
                Line::from("  Ctrl+X n        New session"),
                Line::from("  Ctrl+X a        Toggle auto-approve"),
                Line::from("  Ctrl+O          Expand / collapse thoughts & tool steps"),
                Line::from("  Tab / Shift+Tab Switch BUILD/PLAN · Toggle auto-approve"),
                Line::from("  Ctrl+W          Delete previous word"),
                Line::from("  Ctrl+V / RClick Paste text / API key / path"),
                Line::from("  @ / !           Attach file / Run inline shell"),
                Line::from("  Esc             Close menu / Interrupt generation"),
                Line::from("  Ctrl+C / Ctrl+D Exit"),
            ];
            let widget = Paragraph::new(lines)
                .style(Style::default().fg(pal.text).bg(pal.bg_panel))
                .block(styled_block(" Help (Esc to close) ", pal.primary, pal));
            f.render_widget(widget, rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppState, ChatEntry, ToolGroupKind, draw_ui, extract_compact_target, parse_inline_md,
        prev_word_boundary, visible_window, wrap_line_chars,
    };
    use oxicode_core::config::Config;
    use oxicode_core::extensions::ExtensionRegistry;
    use oxicode_core::mcp::McpManager;
    use oxicode_core::theme::ThemeKind;
    use ratatui::{Terminal, backend::TestBackend, style::Modifier};
    use std::path::PathBuf;
    use std::time::Instant;

    #[test]
    fn visible_window_keeps_selection_in_bounds() {
        assert_eq!(visible_window(18, 0, 10), (0, 0));
        let (start, sel) = visible_window(18, 15, 10);
        assert_eq!(sel, 15);
        assert!(sel >= start && sel < start + 10);
        assert_eq!(start, 8);

        let (start2, sel2) = visible_window(18, 99, 10);
        assert_eq!(sel2, 17);
        assert_eq!(start2, 8);

        assert_eq!(visible_window(5, 3, 10), (0, 3));
    }

    #[test]
    fn inline_md_parses_bold_and_code() {
        let pal = super::palette_for(ThemeKind::OpenCodeBeige);
        let spans = parse_inline_md("Use **oxicode** with `cargo test` now", pal);
        assert!(spans.iter().any(|s| {
            s.content == "oxicode" && s.style.add_modifier.contains(Modifier::BOLD)
        }));
        assert!(spans.iter().any(|s| s.content == "cargo test"));
    }

    #[test]
    fn word_boundary_and_wrap_handle_cyrillic_utf8() {
        let s = "привет мир fox";
        let end = s.len();
        let b1 = prev_word_boundary(s, end);
        assert_eq!(&s[b1..], "fox");
        let b2 = prev_word_boundary(s, b1);
        assert_eq!(&s[b2..b1], "мир ");
        let wrapped = wrap_line_chars("абвгде", 2);
        assert_eq!(wrapped, vec!["аб", "вг", "де"]);

        assert_eq!(ToolGroupKind::from_name("bash"), ToolGroupKind::Bash);
        assert_eq!(ToolGroupKind::from_name("edit_file"), ToolGroupKind::Edit);
        assert_eq!(ToolGroupKind::from_name("write_file"), ToolGroupKind::Edit);
        assert_eq!(
            extract_compact_target("bash", "$ Remove-Item -Path \"test.txt\" -Force"),
            "Remove-Item -Path \"test.txt\" -Force"
        );
        assert_eq!(
            extract_compact_target("edit_file", "edit src/example.py (+3 -1)"),
            "example.py"
        );
    }

    #[test]
    fn draw_ui_renders_grouped_compact_antigravity_steps() {
        let mut state = AppState::new(
            Config::default(),
            PathBuf::from("."),
            McpManager::empty(),
            ExtensionRegistry::default(),
            None,
        );
        state.entries = vec![
            ChatEntry::User("Create demo files and clean temp".to_string()),
            ChatEntry::Thinking {
                text: "The test directory is empty, creating files first...".to_string(),
                started_at: Instant::now(),
                duration_secs: Some(8.0),
            },
            ChatEntry::Tool {
                name: "bash".to_string(),
                preview: "$ ls -la".to_string(),
                output: Some("exit code: 0".to_string()),
                is_error: false,
            },
            ChatEntry::Tool {
                name: "bash".to_string(),
                preview: "$ Get-ChildItem -Path \"F:\\antigravity\\cli\\test\"".to_string(),
                output: Some("exit code: 0".to_string()),
                is_error: false,
            },
            ChatEntry::Tool {
                name: "edit_file".to_string(),
                preview: "edit README.md (+2 -0)".to_string(),
                output: Some("Updated README.md".to_string()),
                is_error: false,
            },
            ChatEntry::Tool {
                name: "write_file".to_string(),
                preview: "write example.py (5 lines)".to_string(),
                output: Some("Wrote example.py".to_string()),
                is_error: false,
            },
            ChatEntry::Tool {
                name: "write_file".to_string(),
                preview: "write temp_remove.txt (1 lines)".to_string(),
                output: Some("Wrote temp_remove.txt".to_string()),
                is_error: false,
            },
            ChatEntry::Tool {
                name: "bash".to_string(),
                preview: "$ Remove-Item -Path \"F:\\antigravity\\cli\\test\\temp_remove.txt\" -Force".to_string(),
                output: Some("exit code: 0".to_string()),
                is_error: false,
            },
        ];

        let backend = TestBackend::new(110, 22);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &state)).unwrap();

        let buf = terminal.backend().buffer();
        let mut rendered_lines = Vec::new();
        for y in 0..buf.area.height {
            let mut row = String::new();
            for x in 0..buf.area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            rendered_lines.push(row.trim_end().to_string());
        }
        let joined = rendered_lines.join("\n");
        println!("{joined}");

        assert!(joined.contains("o Thought for 8.0s"));
        assert!(joined.contains("● Ran 2 commands (+1 · Get-ChildItem"));
        assert!(joined.contains("● Edited 3 files (README.md, example.py, temp_remove.txt)"));
        assert!(joined.contains("● Ran (Remove-Item"));
        assert!(joined.contains("(ctrl+o to expand)"));
    }

    #[test]
    fn legacy_console_glyphs_use_pure_ascii() {
        let mut state = AppState::new(
            Config::default(),
            PathBuf::from("."),
            McpManager::empty(),
            ExtensionRegistry::default(),
            None,
        );
        state.glyphs = super::Glyphs::legacy();
        state.entries = vec![ChatEntry::Tool {
            name: "bash".to_string(),
            preview: "$ echo\t1".to_string(),
            output: Some("1".to_string()),
            is_error: false,
        }];
        let backend = TestBackend::new(90, 18);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &state)).unwrap();

        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        assert!(out.contains("+ Ran (echo 1)"));
        assert!(!out.contains('●'));
        assert!(!out.contains('⠋'));
    }

    #[test]
    fn draw_ui_renders_antigravity_style_with_static_pixel_fox() {
        let mut state = AppState::new(
            Config::default(),
            PathBuf::from("."),
            McpManager::empty(),
            ExtensionRegistry::default(),
            None,
        );
        state.apply_ui_style(oxicode_core::config::UiStyle::Antigravity);
        state.input = "hello world".to_string();
        state.cursor = state.input.len();
        state.aim_fox_at_input();
        assert_eq!(state.fox.target_yaw, 0.0);
        assert_eq!(state.fox.target_pitch, 0.0);

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &state)).unwrap();

        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        assert!(out.contains("OxiCode CLI"));
        assert!(out.contains("> hello world"));
        assert!(out.contains("? for shortcuts"));
        assert!(out.contains('█') || out.contains('▀') || out.contains('▄'));
    }

    #[test]
    fn esc_walks_back_from_nested_overlays_and_mouse_capture_defaults_off() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut state = AppState::new(
            Config::default(),
            PathBuf::from("."),
            McpManager::empty(),
            ExtensionRegistry::default(),
            None,
        );
        assert!(!state.cfg.mouse_capture);

        let (agent_tx, _arx) = tokio::sync::mpsc::unbounded_channel();
        let (done_tx, _drx) = tokio::sync::mpsc::unbounded_channel();
        let (async_tx, _urx) = tokio::sync::mpsc::unbounded_channel();

        // Open Settings -> Provider & API (0) -> ConnectForm
        state.overlay = Some(super::Overlay::Settings { selected: 0 });
        super::handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent_tx,
            &done_tx,
            &async_tx,
        );
        assert!(matches!(state.overlay, Some(super::Overlay::ConnectProvider { .. })));

        super::handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent_tx,
            &done_tx,
            &async_tx,
        );
        assert!(matches!(state.overlay, Some(super::Overlay::ConnectForm { .. })));

        // First Esc -> back to ConnectProvider
        super::handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &agent_tx,
            &done_tx,
            &async_tx,
        );
        assert!(matches!(state.overlay, Some(super::Overlay::ConnectProvider { .. })));

        // Second Esc -> back to Settings
        super::handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &agent_tx,
            &done_tx,
            &async_tx,
        );
        assert!(matches!(state.overlay, Some(super::Overlay::Settings { selected: 0 })));

        // Third Esc -> closes completely
        super::handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &agent_tx,
            &done_tx,
            &async_tx,
        );
        assert!(state.overlay.is_none());
    }
}
