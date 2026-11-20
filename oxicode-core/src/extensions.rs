use crate::commands::{self, CustomCommand};
use crate::config::McpServerConfig;
use crate::llm::ToolDefinition;
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub source: String,
    pub path: PathBuf,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginToolManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_params")]
    pub parameters: Value,
}

fn default_params() -> Value {
    json!({
        "type": "object",
        "properties": {
            "input": { "type": "string", "description": "Input argument for the tool" }
        }
    })
}

#[derive(Debug, Clone, Deserialize)]
struct PluginManifest {
    #[serde(default)]
    name: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    tools: Vec<PluginToolManifest>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub tool_count: usize,
    pub skill_count: usize,
    pub command_count: usize,
    pub mcp_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    pub skills: Vec<SkillEntry>,
    pub plugins: Vec<PluginEntry>,
    pub custom_commands: Vec<CustomCommand>,
    pub rules: Vec<String>,
    pub extra_mcp: BTreeMap<String, McpServerConfig>,
    pub plugin_tools: Vec<(String, PluginToolManifest)>, // (full_tool_name, manifest)
}

impl ExtensionRegistry {
    pub fn load(work_dir: &Path, disabled_plugins: &[String]) -> Self {
        let mut reg = Self::default();
        let mut skill_map: BTreeMap<String, SkillEntry> = BTreeMap::new();
        let mut cmd_map: BTreeMap<String, CustomCommand> = BTreeMap::new();

        for cc in commands::load_custom_commands(work_dir) {
            cmd_map.insert(cc.slash.clone(), cc);
        }

        if let Some(cfg_dir) = dirs::config_dir() {
            let global_root = cfg_dir.join("oxicode");
            load_skills_dir(&global_root.join("skills"), "global", &mut skill_map);
            load_rules_dir(&global_root.join("rules"), &mut reg.rules);
            load_plugins_dir(
                &global_root.join("plugins"),
                disabled_plugins,
                &mut reg,
                &mut skill_map,
                &mut cmd_map,
            );
        }

        let local_root = work_dir.join(".oxicode");
        load_skills_dir(&local_root.join("skills"), "project", &mut skill_map);
        load_rules_dir(&local_root.join("rules"), &mut reg.rules);
        load_plugins_dir(
            &local_root.join("plugins"),
            disabled_plugins,
            &mut reg,
            &mut skill_map,
            &mut cmd_map,
        );

        reg.skills = skill_map.into_values().collect();
        reg.custom_commands = cmd_map.into_values().collect();
        reg
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = Vec::new();
        if !self.skills.is_empty() {
            defs.push(ToolDefinition {
                name: "load_skill".to_string(),
                description: "Load the full instructions of an installed skill by name.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Skill name to load" }
                    },
                    "required": ["name"]
                }),
            });
        }
        for (full_name, spec) in &self.plugin_tools {
            defs.push(ToolDefinition {
                name: full_name.clone(),
                description: spec.description.clone(),
                parameters: spec.parameters.clone(),
            });
        }
        defs
    }

    pub fn find_skill(&self, name: &str) -> Option<&SkillEntry> {
        let needle = name.trim().to_lowercase();
        self.skills.iter().find(|s| s.name.to_lowercase() == needle)
    }

    pub fn is_plugin_tool(&self, name: &str) -> bool {
        self.plugin_tools.iter().any(|(n, _)| n == name)
    }

    pub async fn execute_plugin_tool(
        &self,
        full_name: &str,
        args: &Value,
        work_dir: &Path,
    ) -> Result<String> {
        let (_, spec) = self
            .plugin_tools
            .iter()
            .find(|(n, _)| n == full_name)
            .ok_or_else(|| anyhow!("Unknown plugin tool: {full_name}"))?;

        let mut cmd = Command::new(&spec.command);
        cmd.args(&spec.args)
            .current_dir(work_dir)
            .env("OXICODE_ARGS_JSON", args.to_string())
            .env("OXICODE_WORK_DIR", work_dir.display().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn plugin tool '{full_name}'"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(args.to_string().as_bytes()).await;
        }
        let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
            .await
            .map_err(|_| anyhow!("Plugin tool '{full_name}' timed out"))??;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            return Err(anyhow!(
                "Plugin tool exited with {}: {}",
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() { stdout } else { stderr }
            ));
        }
        Ok(if stdout.is_empty() { stderr } else { stdout })
    }
}

fn load_skills_dir(dir: &Path, source: &str, out: &mut BTreeMap<String, SkillEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let skill_file = if p.is_dir() {
            p.join("SKILL.md")
        } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
            p.clone()
        } else {
            continue;
        };
        if !skill_file.is_file() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&skill_file) else {
            continue;
        };
        let fallback_name = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("skill")
            .to_string();
        let (name, description, body) = parse_skill_md(&raw, &fallback_name);
        out.insert(
            name.to_lowercase(),
            SkillEntry {
                name,
                description,
                source: source.to_string(),
                path: skill_file,
                body,
            },
        );
    }
}

fn load_rules_dir(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    files.sort();
    for p in files {
        if let Ok(raw) = std::fs::read_to_string(&p) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
    }
}

fn load_plugins_dir(
    dir: &Path,
    disabled_plugins: &[String],
    reg: &mut ExtensionRegistry,
    skill_map: &mut BTreeMap<String, SkillEntry>,
    cmd_map: &mut BTreeMap<String, CustomCommand>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let root = entry.path();
        if !root.is_dir() {
            continue;
        }
        let Some(folder_id) = root.file_name().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        let manifest_toml = root.join("plugin.toml");
        let manifest_json = root.join("plugin.json");
        let manifest: PluginManifest = if manifest_toml.is_file() {
            let Ok(raw) = std::fs::read_to_string(&manifest_toml) else {
                continue;
            };
            let Ok(parsed) = toml::from_str(&raw) else {
                continue;
            };
            parsed
        } else if manifest_json.is_file() {
            let Ok(raw) = std::fs::read_to_string(&manifest_json) else {
                continue;
            };
            let Ok(parsed) = serde_json::from_str(&raw) else {
                continue;
            };
            parsed
        } else {
            continue;
        };

        let id = sanitize(&folder_id);
        let name = if manifest.name.trim().is_empty() {
            folder_id.clone()
        } else {
            manifest.name.clone()
        };
        let enabled = !disabled_plugins.iter().any(|d| d == &id);

        let mut plugin_skills = BTreeMap::new();
        load_skills_dir(&root.join("skills"), &format!("plugin:{id}"), &mut plugin_skills);

        let mut plugin_cmds = BTreeMap::new();
        load_plugin_commands(&root.join("commands"), &mut plugin_cmds);

        let entry = PluginEntry {
            id: id.clone(),
            name,
            version: manifest.version,
            description: manifest.description,
            enabled,
            tool_count: manifest.tools.len(),
            skill_count: plugin_skills.len(),
            command_count: plugin_cmds.len(),
            mcp_count: manifest.mcp_servers.len(),
        };
        reg.plugins.push(entry);

        if !enabled {
            continue;
        }

        if !manifest.instructions.trim().is_empty() {
            reg.rules.push(manifest.instructions.trim().to_string());
        }
        load_rules_dir(&root.join("rules"), &mut reg.rules);
        skill_map.extend(plugin_skills);
        cmd_map.extend(plugin_cmds);

        for (srv_name, srv_cfg) in manifest.mcp_servers {
            reg.extra_mcp.insert(format!("{id}_{srv_name}"), srv_cfg);
        }
        for tool in manifest.tools {
            let full = format!("plugin__{id}__{}", sanitize(&tool.name));
            reg.plugin_tools.push((full, tool));
        }
    }
}

fn load_plugin_commands(dir: &Path, out: &mut BTreeMap<String, CustomCommand>) {
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
        let (_, desc, body) = parse_skill_md(&raw, stem);
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

fn parse_skill_md(raw: &str, fallback_name: &str) -> (String, String, String) {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        let front = &rest[..end];
        let body = rest[end + 4..].trim().to_string();
        let mut name = fallback_name.to_string();
        let mut desc = "Skill instructions".to_string();
        for line in front.lines() {
            let l = line.trim();
            if let Some(v) = l.strip_prefix("name:") {
                let clean = v.trim().trim_matches('"').trim_matches('\'');
                if !clean.is_empty() {
                    name = clean.to_string();
                }
            } else if let Some(v) = l.strip_prefix("description:") {
                let clean = v.trim().trim_matches('"').trim_matches('\'');
                if !clean.is_empty() {
                    desc = clean.to_string();
                }
            }
        }
        return (name, desc, body);
    }

    let desc = trimmed
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches('#').trim())
        .map(|l| l.chars().take(60).collect::<String>())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "Skill instructions".to_string());

    (fallback_name.to_string(), desc, trimmed.to_string())
}

pub fn scaffold_starter_skill(work_dir: &Path, raw_name: &str) -> Result<PathBuf> {
    let slug = sanitize(if raw_name.trim().is_empty() {
        "example-skill"
    } else {
        raw_name.trim()
    });
    let dir = work_dir.join(".oxicode").join("skills").join(&slug);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("SKILL.md");
    if !file.exists() {
        let content = format!(
            "---\nname: {slug}\ndescription: Describe when oxicode should use this skill\n---\n\n\
             # {slug}\n\n\
             Write step-by-step instructions, checklists, or conventions here.\n"
        );
        std::fs::write(&file, content)?;
    }
    Ok(file)
}

pub fn scaffold_starter_plugin(work_dir: &Path, raw_name: &str) -> Result<PathBuf> {
    let slug = sanitize(if raw_name.trim().is_empty() {
        "example-plugin"
    } else {
        raw_name.trim()
    });
    let dir = work_dir.join(".oxicode").join("plugins").join(&slug);
    std::fs::create_dir_all(dir.join("skills"))?;
    std::fs::create_dir_all(dir.join("commands"))?;
    let manifest = dir.join("plugin.toml");
    if !manifest.exists() {
        let content = format!(
            "name = \"{slug}\"\n\
             version = \"0.1.0\"\n\
             description = \"Custom oxicode plugin\"\n\
             instructions = \"\"\n\n\
             # Optional custom CLI tools exposed to the agent:\n\
             # [[tools]]\n\
             # name = \"hello\"\n\
             # description = \"Run a custom script\"\n\
             # command = \"powershell\"\n\
             # args = [\"-NoProfile\", \"-Command\", \"Write-Output 'hello from plugin'\"]\n"
        );
        std::fs::write(&manifest, content)?;
    }
    Ok(manifest)
}

fn sanitize(s: &str) -> String {
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

    #[test]
    fn loads_skills_rules_and_plugins_from_project() {
        let tmp = std::env::temp_dir().join(format!("oxicode_ext_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let skill_file = scaffold_starter_skill(&tmp, "rust-review").unwrap();
        assert!(skill_file.exists());

        let plugin_file = scaffold_starter_plugin(&tmp, "my-plugin").unwrap();
        assert!(plugin_file.exists());

        std::fs::create_dir_all(tmp.join(".oxicode").join("rules")).unwrap();
        std::fs::write(
            tmp.join(".oxicode").join("rules").join("style.md"),
            "Always write tests.",
        )
        .unwrap();

        let reg = ExtensionRegistry::load(&tmp, &[]);
        assert!(reg.find_skill("rust-review").is_some());
        assert_eq!(reg.plugins.len(), 1);
        assert!(reg.plugins[0].enabled);
        assert_eq!(reg.rules.len(), 1);

        let reg_disabled = ExtensionRegistry::load(&tmp, &["my-plugin".to_string()]);
        assert_eq!(reg_disabled.plugins.len(), 1);
        assert!(!reg_disabled.plugins[0].enabled);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
