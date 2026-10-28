//! Conversations Claude Code saved for a directory, to resume them.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// Session id, as passed to `claude --resume`.
    pub id: String,
    pub title: String,
    pub updated: DateTime<Local>,
}

/// Where Claude Code keeps its conversations: `$CLAUDE_CONFIG_DIR/projects`,
/// or `~/.claude/projects`.
pub fn projects_dir() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => Some(PathBuf::from(dir).join("projects")),
        None => Some(dirs::home_dir()?.join(".claude").join("projects")),
    }
}

/// Conversations started in `cwd`, most recent first. Best effort: unreadable
/// files are left out.
// shortcut: Claude shortens very long directory names with a hash, so such a
// `cwd` lists nothing; compute the hash if it happens.
pub fn conversations(projects_dir: &Path, cwd: &Path) -> Vec<Conversation> {
    let encoded: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let Ok(entries) = std::fs::read_dir(projects_dir.join(encoded)) else {
        return Vec::new();
    };
    let mut conversations: Vec<Conversation> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "jsonl" {
                return None;
            }
            let updated = std::fs::metadata(&path).ok()?.modified().ok()?.into();
            let title = title(&std::fs::read_to_string(&path).ok()?)?;
            let id = path.file_stem()?.to_string_lossy().into_owned();
            Some(Conversation { id, title, updated })
        })
        .collect();
    conversations.sort_by_key(|c| std::cmp::Reverse(c.updated));
    conversations
}

/// Title Claude shows for a transcript: the name given with `--name` or
/// `/rename`, else the generated title, else the first prompt. None for a
/// transcript without prompt.
fn title(transcript: &str) -> Option<String> {
    let (mut custom, mut generated, mut prompt) = (None, None, None);
    // Lines are filtered before parsing: transcripts can weigh megabytes.
    for line in transcript.lines() {
        let field = if line.contains(r#""type":"custom-title""#) {
            &mut custom
        } else if line.contains(r#""type":"ai-title""#) {
            &mut generated
        } else if prompt.is_none() && line.contains(r#""type":"user""#) {
            &mut prompt
        } else {
            continue;
        };
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let text = match entry["type"].as_str() {
            Some("custom-title") => entry["customTitle"].as_str(),
            Some("ai-title") => entry["aiTitle"].as_str(),
            _ if entry["isMeta"] == true => None,
            _ => prompt_text(&entry["message"]["content"]),
        };
        if let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) {
            *field = Some(text.lines().next().unwrap_or(text).to_string());
        }
    }
    custom.or(generated).or(prompt)
}

/// Text of a user message: a string, or the first text block. Tool results
/// have none.
fn prompt_text(content: &Value) -> Option<&str> {
    match content {
        Value::String(text) => Some(text),
        Value::Array(blocks) => blocks
            .iter()
            .find(|b| b["type"] == "text")
            .and_then(|b| b["text"].as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    use super::*;

    const USER: &str = r#"{"type":"user","message":{"role":"user","content":"Fix the retry loop\nin payments"}}"#;
    const META: &str = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<caveat>"}}"#;
    const TOOL: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#;
    const AI: &str = r#"{"type":"ai-title","aiTitle":"Payment retries","sessionId":"x"}"#;
    const CUSTOM: &str = r#"{"type":"custom-title","customTitle":"review group/api!7","sessionId":"x"}"#;

    #[test]
    fn title_prefers_custom_then_generated_then_first_prompt() {
        assert_eq!(title(&[META, TOOL, USER].join("\n")).as_deref(), Some("Fix the retry loop"));
        assert_eq!(title(&[USER, AI].join("\n")).as_deref(), Some("Payment retries"));
        assert_eq!(title(&[CUSTOM, USER, AI].join("\n")).as_deref(), Some("review group/api!7"));
        assert_eq!(title(&[META, TOOL].join("\n")), None);
    }

    #[test]
    fn lists_conversations_of_a_directory_most_recent_first() {
        let projects = tempfile::tempdir().unwrap();
        let cwd = Path::new("/home/me/.local/share/usine/worktrees/group-api_7");
        let dir = projects.path().join("-home-me--local-share-usine-worktrees-group-api-7");
        std::fs::create_dir_all(dir.join("old")).unwrap();
        let write = |name: &str, content: &str, age: u64| {
            let path = dir.join(name);
            std::fs::write(&path, content).unwrap();
            let at = SystemTime::now() - Duration::from_secs(age);
            File::options().write(true).open(&path).unwrap().set_modified(at).unwrap();
        };
        write("old.jsonl", USER, 3600);
        write("new.jsonl", &[USER, AI].join("\n"), 60);
        write("empty.jsonl", META, 0);
        write("notes.txt", USER, 0);

        let found: Vec<(String, String)> = conversations(projects.path(), cwd)
            .into_iter()
            .map(|c| (c.id, c.title))
            .collect();
        assert_eq!(
            found,
            [
                ("new".to_string(), "Payment retries".to_string()),
                ("old".to_string(), "Fix the retry loop".to_string()),
            ]
        );
        assert!(conversations(projects.path(), Path::new("/elsewhere")).is_empty());
    }
}
