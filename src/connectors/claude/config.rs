use serde::Deserialize;

/// `[claude]` section of `config.toml`; every key is optional.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// Claude Code executable.
    pub command: String,
    /// Extra arguments, e.g. `["--model", "opus"]`.
    pub args: Vec<String>,
    /// Prompt of review sessions; `{url}` and `{target_branch}` are replaced.
    pub review_prompt: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            command: "claude".into(),
            args: Vec::new(),
            review_prompt: "Review the GitLab merge request {url}. Its code is checked out in the \
                            current directory; the target branch is origin/{target_branch}."
                .into(),
        }
    }
}

impl ClaudeConfig {
    pub fn review_prompt_for(&self, url: &str, target_branch: &str) -> String {
        self.review_prompt
            .replace("{url}", url)
            .replace("{target_branch}", target_branch)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn defaults_without_section() {
        let claude = Config::parse("").unwrap().claude;
        assert_eq!(claude.command, "claude");
        assert!(claude.args.is_empty());
        assert_eq!(
            claude.review_prompt_for("https://gitlab.com/g/p/-/merge_requests/1", "main"),
            "Review the GitLab merge request https://gitlab.com/g/p/-/merge_requests/1. Its code \
             is checked out in the current directory; the target branch is origin/main."
        );
    }

    #[test]
    fn custom_prompt_and_args() {
        let claude = Config::parse(
            "[claude]\nargs = [\"--model\", \"opus\"]\nreview_prompt = \"/code-review {url} against {target_branch}\"\n",
        )
        .unwrap()
        .claude;
        assert_eq!(claude.command, "claude");
        assert_eq!(claude.args, ["--model", "opus"]);
        assert_eq!(claude.review_prompt_for("U", "develop"), "/code-review U against develop");
    }
}
