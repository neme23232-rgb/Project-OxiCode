use crate::llm::ToolDefinition;
use anyhow::{Context, Result, anyhow};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

const MAX_OUTPUT_CHARS: usize = 24_000;
const COMMAND_TIMEOUT_SECS: u64 = 60;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read file contents with line numbers. Optionally specify 1-based offset and limit.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to file (relative to workspace or absolute)" },
                    "offset": { "type": "integer", "description": "1-based starting line number" },
                    "limit": { "type": "integer", "description": "Maximum number of lines to read" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Create a new file or overwrite an existing file with the given content.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to file" },
                    "content": { "type": "string", "description": "Full file content" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "edit_file".to_string(),
            description: "Replace an exact unique substring in a file with new text.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to file" },
                    "old_text": { "type": "string", "description": "Exact existing text to replace" },
                    "new_text": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old_text", "new_text"]
            }),
        },
        ToolDefinition {
            name: "list_files".to_string(),
            description: "List files in the workspace respecting .gitignore.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path (defaults to workspace root)" },
                    "max_depth": { "type": "integer", "description": "Maximum recursion depth (default 4)" }
                }
            }),
        },
        ToolDefinition {
            name: "grep".to_string(),
            description: "Search for a regex pattern across files respecting .gitignore.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                    "path": { "type": "string", "description": "Directory or file to search in" }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "glob".to_string(),
            description: "Find workspace files matching a glob pattern (e.g. '**/*.rs', 'src/*.ts'). Respects .gitignore.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern to match relative file paths" },
                    "path": { "type": "string", "description": "Base directory (defaults to workspace root)" }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "bash".to_string(),
            description: "Execute a shell command in the workspace directory.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute" }
                },
                "required": ["command"]
            }),
        },
    ]
}

pub fn definitions_for_mode(read_only: bool) -> Vec<ToolDefinition> {
    if read_only {
        definitions()
            .into_iter()
            .filter(|t| !requires_approval(&t.name))
            .collect()
    } else {
        definitions()
    }
}

pub fn list_workspace_files(work_dir: &Path, limit: usize) -> Vec<String> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(work_dir)
        .max_depth(Some(6))
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walker.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let rel = p
            .strip_prefix(work_dir)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        files.push(rel);
        if files.len() >= limit {
            break;
        }
    }
    files.sort();
    files
}

#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub path: PathBuf,
    pub rel_path: String,
    pub previous_content: Option<String>,
    pub new_content: Option<String>,
    pub added: usize,
    pub removed: usize,
}

pub fn requires_approval(name: &str) -> bool {
    matches!(name, "write_file" | "edit_file" | "bash")
        || name.starts_with("mcp__")
        || name.starts_with("plugin__")
}

pub fn capture_snapshot(work_dir: &Path, name: &str, args: &Value) -> Option<FileSnapshot> {
    if !matches!(name, "write_file" | "edit_file") {
        return None;
    }
    let raw = args.get("path").and_then(Value::as_str)?;
    let path = resolve_path(work_dir, raw);
    let rel_path = path
        .strip_prefix(work_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    let previous_content = std::fs::read_to_string(&path).ok();

    let (new_content, added, removed) = match name {
        "write_file" => {
            let content = args.get("content").and_then(Value::as_str).unwrap_or("");
            let old_lines = previous_content.as_ref().map(|s| s.lines().count()).unwrap_or(0);
            let new_lines = content.lines().count();
            (Some(content.to_string()), new_lines, old_lines)
        }
        "edit_file" => {
            let old_text = args.get("old_text").and_then(Value::as_str).unwrap_or("");
            let new_text = args.get("new_text").and_then(Value::as_str).unwrap_or("");
            let updated = previous_content.as_ref().map(|c| {
                if let Some((actual_old, actual_new)) = match_edit_target(c, old_text, new_text) {
                    c.replacen(&actual_old, &actual_new, 1)
                } else {
                    c.replacen(old_text, new_text, 1)
                }
            });
            (updated, new_text.lines().count(), old_text.lines().count())
        }
        _ => (None, 0, 0),
    };

    Some(FileSnapshot {
        path,
        rel_path,
        previous_content,
        new_content,
        added,
        removed,
    })
}

pub fn restore_snapshot(snap: &FileSnapshot) -> Result<String> {
    match &snap.previous_content {
        Some(prev) => {
            if let Some(parent) = snap.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&snap.path, prev)
                .with_context(|| format!("Failed to restore {}", snap.path.display()))?;
            Ok(format!("Restored {} (-{} +{})", snap.rel_path, snap.added, snap.removed))
        }
        None => {
            if snap.path.exists() {
                std::fs::remove_file(&snap.path)
                    .with_context(|| format!("Failed to remove {}", snap.path.display()))?;
            }
            Ok(format!("Removed created file {}", snap.rel_path))
        }
    }
}

pub fn reapply_snapshot(snap: &FileSnapshot) -> Result<String> {
    if let Some(next) = &snap.new_content {
        if let Some(parent) = snap.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&snap.path, next)
            .with_context(|| format!("Failed to reapply {}", snap.path.display()))?;
        Ok(format!("Reapplied {} (+{} -{})", snap.rel_path, snap.added, snap.removed))
    } else {
        Err(anyhow!("Nothing to redo for {}", snap.rel_path))
    }
}

pub fn smart_diff_preview(work_dir: &Path, name: &str, args: &Value) -> String {
    match name {
        "edit_file" => {
            let raw = args.get("path").and_then(Value::as_str).unwrap_or("");
            let old = args.get("old_text").and_then(Value::as_str).unwrap_or("");
            let new = args.get("new_text").and_then(Value::as_str).unwrap_or("");
            let path = resolve_path(work_dir, raw);
            let start_line = std::fs::read_to_string(&path)
                .ok()
                .and_then(|c| {
                    let needle = if c.contains(old) {
                        old.to_string()
                    } else {
                        old.replace("\r\n", "\n").replace('\n', "\r\n")
                    };
                    c.find(&needle)
                        .map(|idx| c[..idx].bytes().filter(|&b| b == b'\n').count() + 1)
                })
                .unwrap_or(1);

            let mut out = format!(
                "edit {raw} (+{} -{})\n",
                new.lines().count(),
                old.lines().count()
            );
            for (i, line) in old.lines().enumerate() {
                out.push_str(&format!("{:>4} - {line}\n", start_line + i));
            }
            for (i, line) in new.lines().enumerate() {
                out.push_str(&format!("{:>4} + {line}\n", start_line + i));
            }
            out.trim_end().to_string()
        }
        "write_file" => {
            let raw = args.get("path").and_then(Value::as_str).unwrap_or("");
            let content = args.get("content").and_then(Value::as_str).unwrap_or("");
            let path = resolve_path(work_dir, raw);
            let prev_lines = std::fs::read_to_string(&path)
                .ok()
                .map(|c| c.lines().count())
                .unwrap_or(0);
            let mut out = format!("write {raw} (+{} -{prev_lines})\n", content.lines().count());
            for (i, line) in content.lines().take(24).enumerate() {
                out.push_str(&format!("{:>4} + {line}\n", i + 1));
            }
            let total = content.lines().count();
            if total > 24 {
                out.push_str(&format!("... ({} more lines)", total - 24));
            }
            out.trim_end().to_string()
        }
        _ => preview_tool(name, args),
    }
}

pub fn preview_tool(name: &str, args: &Value) -> String {
    match name {
        "bash" => {
            let cmd = args.get("command").and_then(Value::as_str).unwrap_or("");
            format!("$ {cmd}")
        }
        "write_file" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            let lines = args
                .get("content")
                .and_then(Value::as_str)
                .map(|c| c.lines().count())
                .unwrap_or(0);
            format!("write {path} ({lines} lines)")
        }
        "edit_file" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            let old = args.get("old_text").and_then(Value::as_str).unwrap_or("");
            let new = args.get("new_text").and_then(Value::as_str).unwrap_or("");
            let mut out = format!("edit {path}\n");
            for line in old.lines().take(6) {
                out.push_str(&format!("- {line}\n"));
            }
            for line in new.lines().take(6) {
                out.push_str(&format!("+ {line}\n"));
            }
            out.trim_end().to_string()
        }
        "read_file" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            format!("read {path}")
        }
        "list_files" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
            format!("list {path}")
        }
        "grep" => {
            let pat = args.get("pattern").and_then(Value::as_str).unwrap_or("");
            format!("grep /{pat}/")
        }
        "glob" => {
            let pat = args.get("pattern").and_then(Value::as_str).unwrap_or("");
            format!("glob {pat}")
        }
        _ => format!("{name}({args})"),
    }
}

fn resolve_path(work_dir: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        work_dir.join(p)
    }
}

fn truncate_output(mut s: String) -> String {
    if s.len() > MAX_OUTPUT_CHARS {
        let mut end = MAX_OUTPUT_CHARS;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n... [output truncated]");
    }
    s
}

pub async fn execute(work_dir: &Path, name: &str, args: &Value) -> Result<String> {
    let result = match name {
        "read_file" => exec_read_file(work_dir, args).await,
        "write_file" => exec_write_file(work_dir, args).await,
        "edit_file" => exec_edit_file(work_dir, args).await,
        "list_files" => exec_list_files(work_dir, args),
        "grep" => exec_grep(work_dir, args),
        "glob" => exec_glob(work_dir, args),
        "bash" => exec_bash(work_dir, args).await,
        _ => Err(anyhow!("Unknown tool: {name}")),
    }?;
    Ok(truncate_output(result))
}

async fn exec_read_file(work_dir: &Path, args: &Value) -> Result<String> {
    let raw = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'path' argument"))?;
    let path = resolve_path(work_dir, raw);
    let content = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(300) as usize;

    let mut out = String::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line_no < offset {
            continue;
        }
        if line_no >= offset + limit {
            out.push_str("... [more lines available]\n");
            break;
        }
        out.push_str(&format!("{line_no:>4} | {line}\n"));
    }
    if out.is_empty() {
        out.push_str("(empty file or offset beyond end)");
    }
    Ok(out)
}

async fn exec_write_file(work_dir: &Path, args: &Value) -> Result<String> {
    let raw = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'path' argument"))?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'content' argument"))?;
    let path = resolve_path(work_dir, raw);

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, content)
        .await
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(format!(
        "Wrote {} bytes to {}",
        content.len(),
        path.display()
    ))
}

/// Matches `old_text` directly or via CRLF/LF normalization so edits work seamlessly on Windows files.
fn match_edit_target(content: &str, old_text: &str, new_text: &str) -> Option<(String, String)> {
    if content.contains(old_text) {
        return Some((old_text.to_string(), new_text.to_string()));
    }
    let norm_old = old_text.replace("\r\n", "\n").replace('\n', "\r\n");
    if norm_old != old_text && content.contains(&norm_old) {
        let norm_new = new_text.replace("\r\n", "\n").replace('\n', "\r\n");
        return Some((norm_old, norm_new));
    }
    let lf_old = old_text.replace("\r\n", "\n");
    if lf_old != old_text && content.contains(&lf_old) {
        let lf_new = new_text.replace("\r\n", "\n");
        return Some((lf_old, lf_new));
    }
    None
}

async fn exec_edit_file(work_dir: &Path, args: &Value) -> Result<String> {
    let raw = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'path' argument"))?;
    let old_text = args
        .get("old_text")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'old_text' argument"))?;
    let new_text = args
        .get("new_text")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'new_text' argument"))?;
    let path = resolve_path(work_dir, raw);

    let content = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let Some((actual_old, actual_new)) = match_edit_target(&content, old_text, new_text) else {
        let total_lines = content.lines().count();
        return Err(anyhow!(
            "old_text not found in {} ({total_lines} lines). Re-read the exact section and check whitespace.",
            path.display()
        ));
    };
    let count = content.matches(&actual_old).count();
    if count > 1 {
        return Err(anyhow!(
            "old_text matched {count} times in {}; provide more surrounding context to make it unique",
            path.display()
        ));
    }

    let updated = content.replacen(&actual_old, &actual_new, 1);
    tokio::fs::write(&path, updated)
        .await
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(format!("Updated {}", path.display()))
}

fn exec_list_files(work_dir: &Path, args: &Value) -> Result<String> {
    let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let max_depth = args
        .get("max_depth")
        .and_then(Value::as_u64)
        .unwrap_or(4)
        .min(10) as usize;
    let target = resolve_path(work_dir, raw);

    let mut entries = Vec::new();
    let walker = WalkBuilder::new(&target)
        .max_depth(Some(max_depth))
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();

    for entry in walker.flatten() {
        let p = entry.path();
        if p == target {
            continue;
        }
        let rel = p.strip_prefix(work_dir).unwrap_or(p);
        let suffix = if p.is_dir() { "/" } else { "" };
        entries.push(format!(
            "{}{}",
            rel.to_string_lossy().replace('\\', "/"),
            suffix
        ));
        if entries.len() >= 250 {
            entries.push("... [truncated at 250 entries]".to_string());
            break;
        }
    }
    Ok(entries.join("\n"))
}

fn exec_grep(work_dir: &Path, args: &Value) -> Result<String> {
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'pattern' argument"))?;
    let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let target = resolve_path(work_dir, raw);
    let re = Regex::new(pattern).with_context(|| format!("Invalid regex: {pattern}"))?;

    let mut matches = Vec::new();
    let walker = WalkBuilder::new(&target)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walker.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(p) else {
            continue;
        };
        let rel = p
            .strip_prefix(work_dir)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        for (idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                matches.push(format!("{}:{}: {}", rel, idx + 1, line.trim_end()));
                if matches.len() >= 100 {
                    matches.push("... [truncated at 100 matches]".to_string());
                    return Ok(matches.join("\n"));
                }
            }
        }
    }

    if matches.is_empty() {
        Ok("No matches found.".to_string())
    } else {
        Ok(matches.join("\n"))
    }
}

fn glob_to_regex(pattern: &str) -> Result<Regex> {
    let norm = pattern.trim().replace('\\', "/");
    let chars: Vec<char> = norm.chars().collect();
    let mut re = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    if i + 2 < chars.len() && chars[i + 2] == '/' {
                        re.push_str("(?:.*/)?");
                        i += 3;
                        continue;
                    }
                    re.push_str(".*");
                    i += 2;
                    continue;
                }
                re.push_str("[^/]*");
            }
            '?' => re.push_str("[^/]"),
            '{' => re.push_str("(?:"),
            '}' => re.push(')'),
            ',' => re.push('|'),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '[' | ']' | '\\' => {
                re.push('\\');
                re.push(chars[i]);
            }
            c => re.push(c),
        }
        i += 1;
    }
    re.push('$');
    Regex::new(&re).with_context(|| format!("Invalid glob pattern: {pattern}"))
}

fn exec_glob(work_dir: &Path, args: &Value) -> Result<String> {
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'pattern' argument"))?;
    let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let target = resolve_path(work_dir, raw);
    let re = glob_to_regex(pattern)?;

    let mut matched = Vec::new();
    let walker = WalkBuilder::new(&target)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walker.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let rel = p
            .strip_prefix(&target)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        if re.is_match(&rel) {
            let from_root = p
                .strip_prefix(work_dir)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            matched.push(from_root);
            if matched.len() >= 250 {
                matched.sort();
                matched.push("... [truncated at 250 matches]".to_string());
                return Ok(matched.join("\n"));
            }
        }
    }
    matched.sort();
    if matched.is_empty() {
        Ok("No files matched pattern.".to_string())
    } else {
        Ok(matched.join("\n"))
    }
}

async fn exec_bash(work_dir: &Path, args: &Value) -> Result<String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'command' argument"))?;

    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("powershell");
        let ps_cmd = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; [Console]::InputEncoding = [System.Text.Encoding]::UTF8; $OutputEncoding = [System.Text.Encoding]::UTF8; {command}"
        );
        c.args(["-NoProfile", "-Command", &ps_cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(work_dir);

    let output = tokio::time::timeout(Duration::from_secs(COMMAND_TIMEOUT_SECS), cmd.output())
        .await
        .map_err(|_| anyhow!("Command timed out after {COMMAND_TIMEOUT_SECS}s"))?
        .context("Failed to execute shell command")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap_or(-1);

    let mut res = format!("exit code: {code}\n");
    if !stdout.is_empty() {
        res.push_str(&format!("stdout:\n{stdout}\n"));
    }
    if !stderr.is_empty() {
        res.push_str(&format!("stderr:\n{stderr}\n"));
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_write_edit_cycle() {
        let tmp = std::env::temp_dir().join(format!("oxicode_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let write_res = execute(
            &tmp,
            "write_file",
            &json!({ "path": "hello.txt", "content": "line 1\nline 2\nline 3\n" }),
        )
        .await
        .unwrap();
        assert!(write_res.contains("Wrote"));

        let edit_res = execute(
            &tmp,
            "edit_file",
            &json!({ "path": "hello.txt", "old_text": "line 2", "new_text": "line TWO" }),
        )
        .await
        .unwrap();
        assert!(edit_res.contains("Updated"));

        let read_res = execute(&tmp, "read_file", &json!({ "path": "hello.txt" }))
            .await
            .unwrap();
        assert!(read_res.contains("2 | line TWO"));

        let grep_res = execute(&tmp, "grep", &json!({ "pattern": "TWO" }))
            .await
            .unwrap();
        assert!(grep_res.contains("hello.txt:2: line TWO"));

        // Snapshot undo & redo cycle
        let args = json!({ "path": "hello.txt", "old_text": "line TWO", "new_text": "line THREE" });
        let snap = capture_snapshot(&tmp, "edit_file", &args).unwrap();
        execute(&tmp, "edit_file", &args).await.unwrap();
        assert!(std::fs::read_to_string(tmp.join("hello.txt")).unwrap().contains("THREE"));

        restore_snapshot(&snap).unwrap();
        assert!(std::fs::read_to_string(tmp.join("hello.txt")).unwrap().contains("TWO"));

        reapply_snapshot(&snap).unwrap();
        assert!(std::fs::read_to_string(tmp.join("hello.txt")).unwrap().contains("THREE"));

        // CRLF Windows file edit with LF old_text
        std::fs::write(tmp.join("crlf.txt"), "first\r\nsecond\r\nthird\r\n").unwrap();
        execute(
            &tmp,
            "edit_file",
            &json!({ "path": "crlf.txt", "old_text": "first\nsecond", "new_text": "first\nSECOND" }),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.join("crlf.txt")).unwrap(),
            "first\r\nSECOND\r\nthird\r\n"
        );

        // Glob pattern matching
        let glob_res = execute(&tmp, "glob", &json!({ "pattern": "*.txt" }))
            .await
            .unwrap();
        assert!(glob_res.contains("hello.txt"));
        assert!(glob_res.contains("crlf.txt"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
