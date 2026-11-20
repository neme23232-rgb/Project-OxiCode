mod tui;
mod tui_theme;

use anyhow::Result;
use clap::Parser;
use oxicode_core::agent::{self, AgentEvent, AgentMode, ApprovalDecision, TurnParams};
use oxicode_core::config::{Config, PROVIDERS};
use oxicode_core::extensions::ExtensionRegistry;
use oxicode_core::llm::{self, Message};
use oxicode_core::mcp::McpManager;
use oxicode_core::session::{self, SavedSession};
use serde_json::json;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const CLI_EXAMPLES: &str = "\
Examples:
  oxicode                              Open interactive TUI
  oxicode \"Explain main.rs\"            Run single prompt headless
  oxicode -y \"Fix cargo clippy warns\"  Auto-approve writes and shell commands
  git diff | oxicode --plan \"Review diff\"  Read piped stdin in PLAN mode
  oxicode -c                           Continue most recent session in TUI
  oxicode --json \"List TODOs\"          Stream NDJSON events for scripting";

#[derive(Parser, Debug)]
#[command(
    name = "oxicode",
    version,
    about = "Fast, low-memory terminal AI coding agent",
    after_help = CLI_EXAMPLES
)]
struct Cli {
    /// Optional prompt to execute (runs headless unless -i / --tui is set)
    #[arg(value_name = "PROMPT")]
    task: Option<String>,

    /// LLM provider: anymodel, opencode, openrouter, nvidia, google, openai, anthropic, deepseek, ollama, custom
    #[arg(short, long)]
    provider: Option<String>,

    /// Model name override
    #[arg(short, long)]
    model: Option<String>,

    /// Base URL override
    #[arg(short = 'u', long)]
    base_url: Option<String>,

    /// Workspace directory (defaults to current directory)
    #[arg(short = 'd', long)]
    dir: Option<PathBuf>,

    /// Auto-approve file writes and shell commands
    #[arg(short = 'y', long)]
    auto: bool,

    /// Start in Plan mode (read-only analysis & planning)
    #[arg(long)]
    plan: bool,

    /// Force interactive TUI mode (optionally with an initial prompt)
    #[arg(short = 'i', long)]
    tui: bool,

    /// Continue the most recent saved session
    #[arg(short = 'c', long = "continue")]
    continue_last: bool,

    /// Resume a specific saved session by ID (exact or prefix)
    #[arg(long, value_name = "SESSION_ID")]
    id: Option<String>,

    /// Run a single prompt in headless mode and exit
    #[arg(short = 'q', long)]
    prompt: Option<String>,

    /// Stream headless events as newline-delimited JSON (NDJSON)
    #[arg(long)]
    json: bool,

    /// Show elapsed time and token summary after headless runs
    #[arg(short = 'v', long)]
    verbose: bool,

    /// List built-in API providers and exit
    #[arg(long)]
    providers: bool,

    /// Fetch and print available models for the active provider
    #[arg(long)]
    models: bool,

    /// List saved sessions and exit
    #[arg(long)]
    history: bool,

    /// Diagnose configuration, provider, workspace, and MCP status
    #[arg(long)]
    doctor: bool,

    /// Open interactive TUI directly on the Settings screen
    #[arg(long = "config")]
    config_screen: bool,

    /// Optional timeout in seconds for headless runs (0 = no timeout)
    #[arg(short = 't', long, default_value_t = 0)]
    timeout: u64,

    /// Install oxicode binary into user PATH (~/.cargo/bin) and exit
    #[arg(long)]
    install: bool,
}

#[cfg(windows)]
fn configure_windows_console() {
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    unsafe extern "system" {
        fn SetConsoleCP(wCodePageID: u32) -> i32;
        fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
        fn GetStdHandle(nStdHandle: u32) -> isize;
        fn GetConsoleMode(hConsoleHandle: isize, lpMode: *mut u32) -> i32;
        fn SetConsoleMode(hConsoleHandle: isize, dwMode: u32) -> i32;
    }

    unsafe {
        SetConsoleCP(65001);
        SetConsoleOutputCP(65001);
        for handle_id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let h = GetStdHandle(handle_id);
            if h != 0 && h != -1 {
                let mut mode = 0u32;
                if GetConsoleMode(h, &mut mode) != 0 {
                    let _ = SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn configure_windows_console() {}

fn install_self() -> Result<()> {
    let exe = std::env::current_exe()?;
    let bin_name = if cfg!(windows) { "oxicode.exe" } else { "oxicode" };
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("Could not resolve home directory"))?;
    let cargo_bin = home.join(".cargo").join("bin");
    let target_dir = if cargo_bin.is_dir() {
        cargo_bin
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.clone())
            .join("oxicode")
            .join("bin")
    } else {
        home.join(".local").join("bin")
    };
    std::fs::create_dir_all(&target_dir)?;
    let dest = target_dir.join(bin_name);
    if exe.canonicalize().ok() != dest.canonicalize().ok() {
        std::fs::copy(&exe, &dest)?;
    }

    let dir_str = target_dir.display().to_string();
    let in_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|item| item == target_dir))
        .unwrap_or(false);

    #[cfg(windows)]
    if !in_path {
        let escaped = dir_str.replace('\'', "''");
        let ps = format!(
            "$u=[Environment]::GetEnvironmentVariable('PATH','User'); if ($u -notlike '*{escaped}*') {{ [Environment]::SetEnvironmentVariable('PATH', ($u.TrimEnd(';') + ';{escaped}'), 'User') }}"
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .status();
    }

    println!("Installed oxicode to {}", dest.display());
    if in_path {
        println!("You can now run `oxicode` from any terminal.");
    } else {
        println!("Added {dir_str} to user PATH (open a new terminal and run `oxicode`).");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    configure_windows_console();
    let cli = Cli::parse();

    if cli.install {
        return install_self();
    }

    if cli.providers {
        for spec in PROVIDERS {
            println!(
                "{:<12} {:<26} {}",
                spec.id, spec.display_name, spec.default_base_url
            );
        }
        return Ok(());
    }

    if cli.history {
        print_history();
        return Ok(());
    }

    let raw_dir = cli
        .dir
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let work_dir = raw_dir
        .canonicalize()
        .map(|p| session::strip_unc_prefix(&p))
        .unwrap_or(raw_dir);

    let mut cfg = Config::load(
        &work_dir,
        cli.provider.as_deref(),
        cli.model.as_deref(),
        cli.base_url.as_deref(),
    )?;
    if cli.auto {
        cfg.auto_approve = true;
    }

    if cli.models {
        let client = oxicode_core::build_http_client();
        let list = llm::fetch_models(&client, &cfg).await?;
        for m in list {
            println!("{m}");
        }
        return Ok(());
    }

    let extensions = ExtensionRegistry::load(&work_dir, &cfg.disabled_plugins);
    let mut mcp_configs = cfg.mcp_servers.clone();
    mcp_configs.extend(extensions.extra_mcp.clone());
    let mcp = McpManager::connect_all(&mcp_configs).await;

    if cli.doctor {
        print_doctor(&cfg, &work_dir, &extensions, &mcp).await;
        return Ok(());
    }

    let initial_mode = if cli.plan || cfg.default_plan_mode {
        AgentMode::Plan
    } else {
        AgentMode::Build
    };
    let piped_stdin = read_piped_stdin();
    let cli_prompt = cli.prompt.or(cli.task);
    let combined_prompt = merge_prompt_and_stdin(cli_prompt.clone(), piped_stdin.clone());
    let resume = resolve_resume(cli.continue_last, cli.id.as_deref());

    if !cli.tui
        && !cli.config_screen
        && let Some(prompt) = combined_prompt
    {
        return run_headless(
            cfg,
            work_dir,
            mcp,
            extensions,
            resume,
            initial_mode,
            prompt,
            cli.json,
            cli.verbose,
            cli.timeout,
        )
        .await;
    }

    let initial_prompt = if cli.tui { cli_prompt } else { None };
    tui::run(
        cfg,
        work_dir,
        mcp,
        extensions,
        resume,
        initial_mode,
        initial_prompt,
        cli.config_screen,
    )
    .await
}

fn resolve_resume(continue_last: bool, id: Option<&str>) -> Option<SavedSession> {
    if let Some(needle) = id {
        return session::list_sessions(200)
            .into_iter()
            .find(|s| s.id == needle || s.id.starts_with(needle));
    }
    if continue_last {
        return session::list_sessions(1).into_iter().next();
    }
    None
}

fn print_history() {
    let sessions = session::list_sessions(30);
    if sessions.is_empty() {
        println!("No saved sessions found.");
        return;
    }
    for s in sessions {
        let title: String = s.title.chars().take(48).collect();
        println!(
            "{}  {:48}  {} ({} msgs)",
            s.id,
            title,
            s.model,
            s.messages.len()
        );
    }
}

async fn print_doctor(
    cfg: &Config,
    work_dir: &Path,
    extensions: &ExtensionRegistry,
    mcp: &McpManager,
) {
    let spec = cfg.active_spec();
    let active = cfg.active();
    let key_status = if !spec.requires_key {
        if active.api_key.trim().is_empty() {
            "not required (local/custom)"
        } else {
            "configured (optional)"
        }
    } else if cfg.has_key(spec.id) {
        "configured"
    } else {
        "missing (run /connect)"
    };
    let branch = session::read_git_branch(work_dir).unwrap_or_else(|| "none".to_string());
    let cfg_path = Config::global_config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let sessions_dir = session::sessions_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let mcp_snap = mcp.status_snapshot().await;

    println!("oxicode version  : {}", env!("CARGO_PKG_VERSION"));
    println!("config file      : {cfg_path}");
    println!("sessions dir     : {sessions_dir} ({} saved)", session::list_sessions(200).len());
    println!("workspace        : {} (git: {branch})", work_dir.display());
    println!("provider         : {} ({})", spec.display_name, spec.id);
    println!("api key          : {key_status}");
    println!("base url         : {}", active.base_url);
    println!("model            : {}", active.model);
    println!("theme            : {}", cfg.theme.name());
    println!(
        "extensions       : {} skills, {} plugins ({} enabled), {} rules, {} custom cmds",
        extensions.skills.len(),
        extensions.plugins.len(),
        extensions.plugins.iter().filter(|p| p.enabled).count(),
        extensions.rules.len(),
        extensions.custom_commands.len()
    );
    if mcp_snap.is_empty() {
        println!("mcp servers      : 0 configured");
    } else {
        for (name, count, err) in mcp_snap {
            match err {
                Some(e) => println!("mcp [{name}]       : error ({e})"),
                None => println!("mcp [{name}]       : connected ({count} tools)"),
            }
        }
    }
}

fn read_piped_stdin() -> Option<String> {
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut chunk = [0u8; 8192];
        loop {
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(chunk[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Wait briefly for first chunk so idle redirected pipes (IDEs/runners) never hang startup.
    let first = match rx.recv_timeout(Duration::from_millis(60)) {
        Ok(bytes) => bytes,
        Err(_) => return None,
    };
    let mut buf = first;
    while let Ok(more) = rx.recv_timeout(Duration::from_secs(5)) {
        buf.extend_from_slice(&more);
        if buf.len() >= 1_048_576 {
            break;
        }
    }
    let s = String::from_utf8_lossy(&buf).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn merge_prompt_and_stdin(prompt: Option<String>, stdin_text: Option<String>) -> Option<String> {
    match (prompt, stdin_text) {
        (Some(p), Some(s)) => Some(format!("{p}\n\n```stdin\n{s}\n```")),
        (Some(p), None) => Some(p),
        (None, Some(s)) => Some(s),
        (None, None) => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_headless(
    cfg: Config,
    work_dir: PathBuf,
    mcp: McpManager,
    extensions: ExtensionRegistry,
    resume: Option<SavedSession>,
    mode: AgentMode,
    prompt: String,
    json_output: bool,
    verbose: bool,
    timeout_secs: u64,
) -> Result<()> {
    let started = Instant::now();
    let client = oxicode_core::build_http_client();
    let (session_id, mut history) = match resume {
        Some(s) => (s.id, s.messages),
        None => (session::new_session_id(), Vec::new()),
    };
    history.push(Message::user(prompt));

    let (tx, mut rx) = mpsc::unbounded_channel();
    let cfg_save = cfg.clone();
    let dir_save = work_dir.clone();

    let handle = tokio::spawn(async move {
        agent::run_turn(
            TurnParams {
                client,
                cfg,
                work_dir,
                mode,
                mcp,
                extensions,
            },
            history,
            tx,
        )
        .await
    });

    let mut total_in = 0u32;
    let mut total_out = 0u32;
    let mut had_error = false;
    let deadline = if timeout_secs > 0 {
        Some(tokio::time::sleep(Duration::from_secs(timeout_secs)))
    } else {
        None
    };
    tokio::pin!(deadline);

    loop {
        let ev_opt = if let Some(timer) = deadline.as_mut().as_pin_mut() {
            tokio::select! {
                _ = timer => {
                    had_error = true;
                    handle.abort();
                    if json_output {
                        println!("{}", json!({ "type": "error", "error": format!("Timed out after {timeout_secs}s") }));
                    } else {
                        eprintln!("\nError: Timed out after {timeout_secs}s");
                    }
                    break;
                }
                ev = rx.recv() => ev,
            }
        } else {
            rx.recv().await
        };

        let Some(ev) = ev_opt else {
            break;
        };

        match ev {
            AgentEvent::ThinkingDelta(_) => {}
            AgentEvent::AssistantDelta(d) => {
                if json_output {
                    println!("{}", json!({ "type": "assistant_delta", "text": d }));
                } else {
                    print!("{d}");
                    let _ = std::io::stdout().flush();
                }
            }
            AgentEvent::ToolStarted { name, preview } => {
                if json_output {
                    println!(
                        "{}",
                        json!({ "type": "tool_started", "name": name, "preview": preview })
                    );
                } else {
                    eprintln!("\n[{name}] {preview}");
                }
            }
            AgentEvent::ApprovalRequest {
                name,
                preview,
                responder,
            } => {
                if json_output {
                    println!(
                        "{}",
                        json!({ "type": "approval_rejected", "name": name, "preview": preview })
                    );
                } else {
                    eprintln!("\n[blocked] {name} requires approval (re-run with -y to allow)");
                }
                let _ = responder.send(ApprovalDecision::Reject);
            }
            AgentEvent::ToolFinished {
                name,
                is_error,
                output,
            } => {
                if json_output {
                    println!(
                        "{}",
                        json!({
                            "type": "tool_finished",
                            "name": name,
                            "is_error": is_error,
                            "output": output
                        })
                    );
                } else if is_error {
                    eprintln!("[{name} error] {output}");
                }
            }
            AgentEvent::Error(e) => {
                had_error = true;
                if json_output {
                    println!("{}", json!({ "type": "error", "error": e }));
                } else {
                    eprintln!("\nError: {e}");
                }
            }
            AgentEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                total_in += input_tokens;
                total_out += output_tokens;
                if json_output {
                    println!(
                        "{}",
                        json!({
                            "type": "usage",
                            "input_tokens": input_tokens,
                            "output_tokens": output_tokens
                        })
                    );
                }
            }
            AgentEvent::FileChanged(fc) => {
                if json_output {
                    println!(
                        "{}",
                        json!({
                            "type": "file_changed",
                            "path": fc.rel_path,
                            "added": fc.added,
                            "removed": fc.removed
                        })
                    );
                }
            }
            AgentEvent::TurnFinished => {
                if json_output {
                    println!(
                        "{}",
                        json!({
                            "type": "done",
                            "elapsed_ms": started.elapsed().as_millis(),
                            "input_tokens": total_in,
                            "output_tokens": total_out
                        })
                    );
                } else {
                    println!();
                }
                break;
            }
        }
    }

    if let Ok((final_history, _)) = handle.await {
        let _ = session::save_session(
            &session_id,
            &dir_save,
            &cfg_save.provider,
            &cfg_save.active().model,
            &final_history,
        );
    }

    if verbose && !json_output {
        eprintln!(
            "[done in {:.2}s | tokens: {} in / {} out]",
            started.elapsed().as_secs_f64(),
            total_in,
            total_out
        );
    }
    if had_error {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::merge_prompt_and_stdin;

    #[test]
    fn merges_prompt_and_stdin_cleanly() {
        assert_eq!(merge_prompt_and_stdin(None, None), None);
        assert_eq!(
            merge_prompt_and_stdin(Some("hi".into()), None),
            Some("hi".into())
        );
        assert_eq!(
            merge_prompt_and_stdin(None, Some("log".into())),
            Some("log".into())
        );
        let merged = merge_prompt_and_stdin(Some("fix".into()), Some("err".into())).unwrap();
        assert!(merged.contains("fix"));
        assert!(merged.contains("```stdin\nerr\n```"));
    }
}
