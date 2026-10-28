//! Local worktrees of GitLab merge requests.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::connectors::git::{GitConfig, branch_worktrees, find_clone};
use crate::connectors::gitlab::GitlabConfig;
use crate::model::MergeRequest;

/// Sets the worktree of each MR: the worktree of a local clone of its project
/// that has its source branch checked out. Blocking (runs git), best effort:
/// an MR whose clone or worktree is not found keeps None.
// shortcut: an MR from a fork is matched in the clone of the target project, so a
// local branch of the same name matches; compare the source project if it bites.
pub fn find_worktrees(gitlab: &GitlabConfig, git: &GitConfig, mrs: &mut [MergeRequest]) {
    let Ok(host) = gitlab.host() else { return };
    let roots = git.root_paths();
    let mut by_project: HashMap<String, Vec<(PathBuf, String)>> = HashMap::new();
    for mr in mrs {
        let worktrees = by_project.entry(mr.project.clone()).or_insert_with(|| {
            find_clone(&roots, &host, &mr.project)
                .and_then(|clone| branch_worktrees(&clone).ok())
                .unwrap_or_default()
        });
        mr.worktree = worktrees
            .iter()
            .find(|(_, branch)| *branch == mr.source_branch)
            .map(|(path, _)| path.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use chrono::DateTime;

    use super::*;
    use crate::model::{Role, Source};

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn mr(project: &str, branch: &str) -> MergeRequest {
        MergeRequest {
            source: Source::Gitlab,
            reference: format!("{project}!1"),
            project: project.into(),
            iid: 1,
            title: String::new(),
            description: String::new(),
            draft: false,
            merge_status: String::new(),
            pipeline: None,
            source_branch: branch.into(),
            target_branch: "main".into(),
            author: String::new(),
            role: Role::Author,
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: String::new(),
            worktree: None,
        }
    }

    #[test]
    fn finds_the_worktree_of_the_source_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let clone = tmp.path().join("src/group/api");
        std::fs::create_dir_all(&clone).unwrap();
        git(&clone, &["init", "--quiet", "-b", "main"]);
        git(&clone, &["remote", "add", "origin", "git@gitlab.example.com:group/api.git"]);
        git(&clone, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
        let feat = tmp.path().join("api-feat");
        git(&clone, &["worktree", "add", "--quiet", "-b", "feat", feat.to_str().unwrap()]);

        let gitlab = GitlabConfig { url: "https://gitlab.example.com".into(), token: "t".into() };
        let git_config = GitConfig {
            roots: vec![tmp.path().join("src").to_str().unwrap().into()],
            worktrees_dir: None,
        };
        let mut mrs = [mr("group/api", "feat"), mr("group/api", "other"), mr("group/web", "feat")];

        find_worktrees(&gitlab, &git_config, &mut mrs);

        let worktrees: Vec<_> = mrs.iter().map(|mr| mr.worktree.clone()).collect();
        assert_eq!(worktrees, [Some(feat.canonicalize().unwrap()), None, None]);
    }
}
