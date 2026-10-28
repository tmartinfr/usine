//! Review of a GitLab merge request by a Claude Code session running in a
//! dedicated git worktree.

use anyhow::{Context, Result, bail};

use crate::connectors::claude::{ClaudeConfig, Session, Tmux};
use crate::connectors::git::{GitConfig, find_clone, prepare_mr_worktree};
use crate::connectors::gitlab::GitlabConfig;
use crate::model::MergeRequest;

const SESSION_PREFIX: &str = "usine-review-";

/// `group/sub.project` -> `group-sub-project`
fn slug(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

/// Name of the tmux session reviewing `mr`.
pub fn session_name(mr: &MergeRequest) -> String {
    format!("{SESSION_PREFIX}{}-{}", slug(&mr.project), mr.iid)
}

/// Review sessions, running or finished. Their subject is the MR reference.
pub fn list_reviews(tmux: &Tmux) -> Result<Vec<Session>> {
    tmux.sessions(SESSION_PREFIX)
}

/// Checks the MR out in a worktree and starts Claude on it. Blocking (runs git).
/// Returns the session name.
pub fn start_review(
    gitlab: &GitlabConfig,
    git: &GitConfig,
    claude: &ClaudeConfig,
    tmux: &Tmux,
    mr: &MergeRequest,
) -> Result<String> {
    let name = session_name(mr);
    if list_reviews(tmux)?.iter().any(|s| s.name == name) {
        bail!("a review of {} is already running", mr.reference);
    }

    let host = gitlab.host()?;
    let clone = find_clone(&git.root_paths(), &host, &mr.project).with_context(|| {
        format!("no local clone of {host}/{} found under {}", mr.project, git.roots.join(", "))
    })?;
    let worktree = git
        .worktrees_path()?
        .join(format!("{}-mr-{}", slug(&mr.project), mr.iid));
    prepare_mr_worktree(&clone, mr.iid, &mr.target_branch, &worktree)?;

    let mut command = vec![claude.command.clone()];
    command.extend(claude.args.iter().cloned());
    command.extend([
        "--name".into(),
        format!("review {}", mr.reference),
        claude.review_prompt_for(&mr.url, &mr.target_branch),
    ]);
    tmux.start(&name, &mr.reference, &worktree, &command)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use chrono::DateTime;

    use super::*;
    use crate::model::{Role, Source};

    #[test]
    fn session_names_are_tmux_safe() {
        assert_eq!(slug("paylead/infra/nomad.v2"), "paylead-infra-nomad-v2");
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    struct Server(String);

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = Command::new("tmux").args(["-L", &self.0, "kill-server"]).output();
        }
    }

    /// Full flow with real git and tmux; `sh` stands in for Claude.
    #[test]
    fn starts_review_in_mr_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "--quiet", "-b", "main"]);
        git(&origin, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
        git(&origin, &["commit", "--quiet", "--allow-empty", "-m", "mr"]);
        let mr_commit = git(&origin, &["rev-parse", "HEAD"]);
        git(&origin, &["update-ref", "refs/merge-requests/7/head", &mr_commit]);
        git(&origin, &["reset", "--quiet", "--hard", "HEAD~1"]);

        // A clone whose origin looks like a GitLab remote but points to `origin`.
        let clone = tmp.path().join("src/group/api");
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        git(tmp.path(), &["clone", "--quiet", origin.to_str().unwrap(), clone.to_str().unwrap()]);
        let remote = "git@gitlab.example.com:group/api.git";
        git(&clone, &["remote", "set-url", "origin", remote]);
        git(&clone, &["config", &format!("url.{}.insteadOf", origin.display()), remote]);

        let gitlab = GitlabConfig { url: "https://gitlab.example.com".into(), token: "t".into() };
        let git_config = GitConfig {
            roots: vec![tmp.path().join("src").to_str().unwrap().into()],
            worktrees_dir: Some(tmp.path().join("worktrees").to_str().unwrap().into()),
        };
        let claude = ClaudeConfig {
            command: "sh".into(),
            args: vec!["-c".into(), "echo \"name=$1\"; echo \"prompt=$2\"; pwd; sleep 30".into()],
            ..ClaudeConfig::default()
        };
        let socket = format!("usine-test-{}-review", std::process::id());
        let _server = Server(socket.clone());
        let tmux = Tmux::with_socket(&socket);
        let mr = MergeRequest {
            source: Source::Gitlab,
            reference: "group/api!7".into(),
            project: "group/api".into(),
            iid: 7,
            title: "Add retry".into(),
            description: String::new(),
            draft: false,
            merge_status: "mergeable".into(),
            pipeline: None,
            role: Role::Author,
            source_branch: "feat".into(),
            target_branch: "main".into(),
            author: "me".into(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: "https://gitlab.example.com/group/api/-/merge_requests/7".into(),
            worktree: None,
        };

        let name = start_review(&gitlab, &git_config, &claude, &tmux, &mr).unwrap();

        assert_eq!(name, "usine-review-group-api-7");
        let worktree = tmp.path().join("worktrees/group-api-mr-7");
        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), mr_commit);
        let reviews = list_reviews(&tmux).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].subject, "group/api!7");

        let start = Instant::now();
        let screen = loop {
            let screen = tmux.capture(&name, false).unwrap();
            if screen.contains("group-api-mr-7") || start.elapsed() > Duration::from_secs(5) {
                break screen;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(screen.contains("name=review group/api!7"), "{screen}");
        assert!(
            screen.contains("prompt=Review the GitLab merge request https://gitlab.example.com/group/api/-/merge_requests/7."),
            "{screen}"
        );
        assert!(screen.contains("group-api-mr-7"), "{screen}");

        let err = start_review(&gitlab, &git_config, &claude, &tmux, &mr).unwrap_err();
        assert!(err.to_string().contains("already running"), "{err}");
    }

    #[test]
    fn reports_missing_clone() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = format!("usine-test-{}-noclone", std::process::id());
        let _server = Server(socket.clone());
        let gitlab = GitlabConfig { url: "https://gitlab.com".into(), token: "t".into() };
        let git_config = GitConfig { roots: vec![tmp.path().to_str().unwrap().into()], worktrees_dir: None };
        let mr = MergeRequest {
            source: Source::Gitlab,
            reference: "group/api!7".into(),
            project: "group/api".into(),
            iid: 7,
            title: String::new(),
            description: String::new(),
            draft: false,
            merge_status: String::new(),
            pipeline: None,
            role: Role::Author,
            source_branch: String::new(),
            target_branch: "main".into(),
            author: String::new(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: String::new(),
            worktree: None,
        };

        let err = start_review(&gitlab, &git_config, &ClaudeConfig::default(), &Tmux::with_socket(&socket), &mr)
            .unwrap_err();
        assert!(err.to_string().contains("no local clone of gitlab.com/group/api"), "{err}");
    }
}
