use crate::llm::Message;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub id: String,
    pub updated_at: u64,
    pub work_dir: String,
    pub provider: String,
    pub model: String,
    pub title: String,
    pub messages: Vec<Message>,
}

pub fn sessions_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("oxicode").join("sessions"))
}

pub fn new_session_id() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("session_{secs}_{}", std::process::id())
}

pub fn save_session(
    id: &str,
    work_dir: &Path,
    provider: &str,
    model: &str,
    messages: &[Message],
) -> Result<()> {
    if messages.is_empty() {
        return Ok(());
    }
    let Some(dir) = sessions_dir() else {
        return Ok(());
    };
    std::fs::create_dir_all(&dir)?;

    let title = messages
        .iter()
        .find(|m| m.role == crate::llm::Role::User)
        .map(|m| {
            let first = m.content.lines().next().unwrap_or("Chat").trim();
            let truncated: String = first.chars().take(60).collect();
            if first.chars().count() > 60 {
                format!("{truncated}...")
            } else {
                truncated
            }
        })
        .unwrap_or_else(|| "Chat".to_string());

    let updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let session = SavedSession {
        id: id.to_string(),
        updated_at,
        work_dir: strip_unc_prefix(work_dir).display().to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        title,
        messages: messages.to_vec(),
    };

    let path = dir.join(format!("{id}.json"));
    let json = serde_json::to_string(&session)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn list_sessions(limit: usize) -> Vec<SavedSession> {
    let Some(dir) = sessions_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&p)
            && let Ok(parsed) = serde_json::from_str::<SavedSession>(&raw)
        {
            sessions.push(parsed);
        }
    }
    sessions.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
    sessions.truncate(limit);
    sessions
}

pub fn delete_session(id: &str) -> Result<bool> {
    let Some(dir) = sessions_dir() else {
        return Ok(false);
    };
    let path = dir.join(format!("{id}.json"));
    if path.exists() {
        std::fs::remove_file(path)?;
        return Ok(true);
    }
    Ok(false)
}

/// Strip Windows `\\?\` extended-length UNC prefix so paths render cleanly (`F:\oxicode`).
pub fn strip_unc_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// Read the active git branch name directly from `.git/HEAD` without spawning a process.
pub fn read_git_branch(work_dir: &Path) -> Option<String> {
    let mut cur = Some(work_dir);
    while let Some(dir) = cur {
        let head = dir.join(".git").join("HEAD");
        if head.is_file() {
            let content = std::fs::read_to_string(head).ok()?;
            let trimmed = content.trim();
            if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                return Some(branch.to_string());
            }
            return Some(trimmed.chars().take(7).collect());
        }
        cur = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_windows_unc_prefix() {
        let p = Path::new(r"\\?\F:\oxicode");
        assert_eq!(strip_unc_prefix(p), PathBuf::from(r"F:\oxicode"));

        let unc = Path::new(r"\\?\UNC\server\share\dir");
        assert_eq!(strip_unc_prefix(unc), PathBuf::from(r"\\server\share\dir"));

        let normal = Path::new(r"F:\oxicode");
        assert_eq!(strip_unc_prefix(normal), PathBuf::from(r"F:\oxicode"));
    }
}
