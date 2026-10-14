use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use super::parse_remote;

/// How deep below each root clones are searched (`~/git/<group>/<repo>` is depth 2).
const MAX_DEPTH: usize = 3;

/// Finds a local clone whose `origin` remote is `host`/`project`.
pub fn find_clone(roots: &[PathBuf], host: &str, project: &str) -> Option<PathBuf> {
    roots.iter().find_map(|root| search(root, host, project, 0))
}

fn search(dir: &Path, host: &str, project: &str, depth: usize) -> Option<PathBuf> {
    if dir.join(".git").exists() {
        let url = git_output(dir, &["config", "--get", "remote.origin.url"]).ok()?;
        let (remote_host, remote_project) = parse_remote(url.trim())?;
        let matches = remote_host == host.to_lowercase() && remote_project == project;
        // Clones are not searched for nested clones.
        return matches.then(|| dir.to_path_buf());
    }
    if depth >= MAX_DEPTH {
        return None;
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .collect();
    children.sort();
    children
        .iter()
        .find_map(|child| search(child, host, project, depth + 1))
}

/// Fetches MR `iid` and its target branch into `clone`, then checks the MR out in a
/// detached worktree at `dest` (created, or updated if it already exists).
pub fn prepare_mr_worktree(clone: &Path, iid: u64, target_branch: &str, dest: &Path) -> Result<()> {
    let mr_ref = format!("refs/usine/mr/{iid}");
    git(
        clone,
        &[
            "fetch",
            "--quiet",
            "origin",
            &format!("+refs/merge-requests/{iid}/head:{mr_ref}"),
            &format!("+refs/heads/{target_branch}:refs/remotes/origin/{target_branch}"),
        ],
    )?;
    if dest.exists() {
        git(dest, &["checkout", "--quiet", "--detach", &mr_ref])
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let dest = dest.to_str().context("worktree path is not valid UTF-8")?;
        git(clone, &["worktree", "add", "--quiet", "--detach", dest, &mr_ref])
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    git_output(dir, args).map(|_| ())
}

fn git_output(dir: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        // Never prompt: usine runs git behind its own UI.
        .env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let output = command.output().context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
