use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CustomCommand {
    pub slash: String,
    pub description: String,
    pub template: String,
}

impl CustomCommand {
    pub fn expand(&self, args: &str) -> String {
        let trimmed = args.trim();
        if self.template.contains("$ARGUMENTS") {
            self.template.replace("$ARGUMENTS", trimmed)
        } else if trimmed.is_empty() {
            self.template.clone()
        } else {
            format!("{}\n\n{trimmed}", self.template)
        }
    }
}

pub fn load_custom_commands(work_dir: &Path) -> Vec<CustomCommand> {
    let mut map: BTreeMap<String, CustomCommand> = BTreeMap::new();

    if let Some(cfg_dir) = dirs::config_dir() {
        load_dir(&cfg_dir.join("oxicode").join("commands"), &mut map);
    }
    load_dir(&work_dir.join(".oxicode").join("commands"), &mut map);

    map.into_values().collect()
}

fn load_dir(dir: &Path, out: &mut BTreeMap<String, CustomCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(raw) = std::fs::read_to_string(&p) else {
            continue;
        };
        let (desc, body) = parse_command_md(&raw);
        let slash = format!("/{}", stem.trim().to_lowercase());
        out.insert(
            slash.clone(),
            CustomCommand {
                slash,
                description: desc,
                template: body,
            },
        );
    }
}

fn parse_command_md(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        let front = &rest[..end];
        let body = rest[end + 4..].trim().to_string();
        let desc = front
            .lines()
            .find_map(|l| l.trim().strip_prefix("description:"))
            .map(|d| d.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "Custom command".to_string());
        return (desc, body);
    }

    let desc = trimmed
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches('#').trim())
        .map(|l| l.chars().take(48).collect::<String>())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "Custom command".to_string());

    (desc, trimmed.to_string())
}

pub fn export_markdown(work_dir: &Path, target: Option<&str>, content: &str) -> Result<PathBuf> {
    let path = match target {
        Some(raw) if !raw.trim().is_empty() => {
            let p = PathBuf::from(raw.trim());
            if p.is_absolute() { p } else { work_dir.join(p) }
        }
        _ => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            work_dir.join(format!("oxicode_export_{ts}.md"))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)
        .with_context(|| format!("Failed to export session to {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_expands_args() {
        let md = "---\ndescription: Review staged changes\n---\nCheck $ARGUMENTS carefully.";
        let (desc, body) = parse_command_md(md);
        assert_eq!(desc, "Review staged changes");
        let cmd = CustomCommand {
            slash: "/review".to_string(),
            description: desc,
            template: body,
        };
        assert_eq!(cmd.expand("src/main.rs"), "Check src/main.rs carefully.");
    }
}
