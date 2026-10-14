//! Local git clones and worktrees.

mod config;
mod remote;
mod repo;
#[cfg(test)]
mod tests;

pub use config::GitConfig;
pub use remote::parse_remote;
pub use repo::{find_clone, prepare_mr_worktree};
