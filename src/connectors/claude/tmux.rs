use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// User option attached to sessions to remember what they are about.
const SUBJECT_OPTION: &str = "@usine_subject";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub name: String,
    /// What the session works on, e.g. an MR reference.
    pub subject: String,
    /// The program exited; its last screen is kept until the session is closed.
    pub finished: bool,
}

/// Thin wrapper over the `tmux` CLI.
#[derive(Debug, Clone, Default)]
pub struct Tmux {
    /// Private server socket (`tmux -L`), used by tests.
    socket: Option<String>,
}

impl Tmux {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn with_socket(socket: &str) -> Self {
        Self { socket: Some(socket.into()) }
    }

    /// Starts `command` in a new detached session. The pane stays open after the
    /// command exits so its output remains visible.
    pub fn start(&self, name: &str, subject: &str, cwd: &Path, command: &[String]) -> Result<()> {
        let cwd = cwd.to_str().context("session directory is not valid UTF-8")?;
        // Exact-match pane target: `-t name` alone would also match `name-2`.
        let target = format!("={name}:");
        // The session starts with a shell, gets its options, then the shell is
        // replaced by the command: an immediately failing command stays visible.
        self.run(&["new-session", "-d", "-s", name, "-c", cwd, "-x", "200", "-y", "50"])?;
        self.run(&["set-option", "-t", &target, SUBJECT_OPTION, subject])?;
        self.run(&["set-option", "-w", "-t", &target, "remain-on-exit", "on"])?;
        let mut respawn = vec!["respawn-pane", "-k", "-t", &target];
        respawn.extend(command.iter().map(String::as_str));
        self.run(&respawn)?;
        Ok(())
    }

    /// Sessions whose name starts with `prefix`.
    pub fn sessions(&self, prefix: &str) -> Result<Vec<Session>> {
        let format = format!("#{{session_name}}\t#{{pane_dead}}\t#{{{SUBJECT_OPTION}}}");
        let output = match self.run(&["list-sessions", "-F", &format]) {
            Ok(output) => output,
            // No server running means no session.
            Err(err)
                if ["no server running", "error connecting to"]
                    .iter()
                    .any(|m| err.to_string().contains(m)) =>
            {
                return Ok(Vec::new());
            }
            Err(err) => return Err(err),
        };
        Ok(output
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(3, '\t');
                let name = fields.next()?;
                let dead = fields.next()?;
                let subject = fields.next().unwrap_or_default();
                name.starts_with(prefix).then(|| Session {
                    name: name.into(),
                    subject: subject.into(),
                    finished: dead == "1",
                })
            })
            .collect())
    }

    /// Recent content of the session's active pane (screen plus some history), as
    /// plain text with runs of blank lines collapsed and trailing blank lines removed.
    pub fn capture(&self, name: &str) -> Result<String> {
        let text = self.run(&["capture-pane", "-p", "-S", "-200", "-t", &format!("={name}:")])?;
        let mut lines: Vec<&str> = Vec::new();
        for line in text.lines().map(str::trim_end) {
            if !(line.is_empty() && lines.last().is_some_and(|l| l.is_empty())) {
                lines.push(line);
            }
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        Ok(lines.join("\n"))
    }

    pub fn kill(&self, name: &str) -> Result<()> {
        self.run(&["kill-session", "-t", &format!("={name}")]).map(|_| ())
    }

    /// Command attaching the current terminal to the session. Inside tmux, switches
    /// the current client instead (returns immediately).
    pub fn attach_command(&self, name: &str) -> Command {
        let mut command = self.command();
        let target = format!("={name}");
        if std::env::var_os("TMUX").is_some() {
            command.args(["switch-client", "-t", &target]);
        } else {
            command.args(["attach-session", "-t", &target]);
        }
        command
    }

    fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        if let Some(socket) = &self.socket {
            command.args(["-L", socket]);
        }
        command
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let output = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .context("cannot run tmux (is it installed?)")?;
        if !output.status.success() {
            bail!(
                "tmux {} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}
