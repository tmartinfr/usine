//! Tests against real git repositories in a temporary directory.

use std::path::Path;
use std::process::Command;

use super::{branch_worktrees, find_clone, prepare_mr_worktree};

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

fn init_repo(dir: &Path, remote: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet", "-b", "main"]);
    git(dir, &["remote", "add", "origin", remote]);
}

#[test]
fn finds_clone_by_remote() {
    let root = tempfile::tempdir().unwrap();
    init_repo(&root.path().join("work/api"), "git@gitlab.com:group/api.git");
    init_repo(&root.path().join("work/nomad"), "https://gitlab.com/group/infra/nomad.git");
    init_repo(&root.path().join("other/nomad"), "git@github.com:group/infra/nomad.git");
    let roots = [root.path().to_path_buf()];

    assert_eq!(
        find_clone(&roots, "gitlab.com", "group/infra/nomad"),
        Some(root.path().join("work/nomad"))
    );
    assert_eq!(find_clone(&roots, "gitlab.com", "group/missing"), None);
}

#[test]
fn prepares_and_updates_mr_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "--quiet", "-b", "main"]);
    git(&origin, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
    git(&origin, &["commit", "--quiet", "--allow-empty", "-m", "mr v1"]);
    let v1 = git(&origin, &["rev-parse", "HEAD"]);
    git(&origin, &["update-ref", "refs/merge-requests/7/head", &v1]);
    git(&origin, &["reset", "--quiet", "--hard", "HEAD~1"]);

    let clone = tmp.path().join("clone");
    git(tmp.path(), &["clone", "--quiet", origin.to_str().unwrap(), "clone"]);
    let dest = tmp.path().join("worktrees/group-api-mr-7");

    prepare_mr_worktree(&clone, 7, "main", &dest).unwrap();
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), v1);

    // The MR gets a new commit: preparing again updates the existing worktree.
    git(&origin, &["commit", "--quiet", "--allow-empty", "-m", "mr v2"]);
    let v2 = git(&origin, &["rev-parse", "HEAD"]);
    git(&origin, &["update-ref", "refs/merge-requests/7/head", &v2]);

    prepare_mr_worktree(&clone, 7, "main", &dest).unwrap();
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), v2);
    assert!(!git(&clone, &["rev-parse", "origin/main"]).is_empty());
}

#[test]
fn reports_missing_mr() {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "--quiet", "-b", "main"]);
    git(&origin, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
    git(tmp.path(), &["clone", "--quiet", origin.to_str().unwrap(), "clone"]);

    let err = prepare_mr_worktree(&tmp.path().join("clone"), 99, "main", &tmp.path().join("wt"))
        .unwrap_err();
    assert!(err.to_string().contains("git fetch"), "{err}");
}

#[test]
fn lists_worktrees_on_a_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let clone = tmp.path().join("clone");
    init_repo(&clone, "git@gitlab.com:group/api.git");
    git(&clone, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
    let feat = tmp.path().join("feat");
    git(&clone, &["worktree", "add", "--quiet", "-b", "feat", feat.to_str().unwrap()]);
    let detached = tmp.path().join("detached");
    git(&clone, &["worktree", "add", "--quiet", "--detach", detached.to_str().unwrap()]);

    let canonical = |path: &Path| path.canonicalize().unwrap();
    assert_eq!(
        branch_worktrees(&clone).unwrap(),
        [(canonical(&clone), "main".to_string()), (canonical(&feat), "feat".to_string())]
    );
}
