//! Tests against a real tmux server on a private socket.

use std::time::{Duration, Instant};

use super::{Session, Tmux};

/// Kills the private tmux server when the test ends, even on failure.
struct Server(Tmux, String);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.1, "kill-server"])
            .output();
    }
}

fn server(test: &str) -> Server {
    let socket = format!("usine-test-{}-{test}", std::process::id());
    Server(Tmux::with_socket(&socket), socket)
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn sh(script: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), script.into()]
}

#[test]
fn starts_lists_captures_and_kills_sessions() {
    let server = server("lifecycle");
    let tmux = &server.0;
    assert_eq!(tmux.sessions("usine-").unwrap(), []);

    let cwd = tempfile::tempdir().unwrap();
    tmux.start("usine-review-g-p-1", "g/p!1", cwd.path(), &sh("pwd; echo hello; sleep 30"))
        .unwrap();
    tmux.start("other", "", cwd.path(), &sh("sleep 30")).unwrap();

    assert_eq!(
        tmux.sessions("usine-").unwrap(),
        [Session { name: "usine-review-g-p-1".into(), subject: "g/p!1".into(), finished: false }]
    );
    wait_for(|| tmux.capture("usine-review-g-p-1").unwrap().contains("hello"));
    let screen = tmux.capture("usine-review-g-p-1").unwrap();
    let cwd_name = cwd.path().file_name().unwrap().to_str().unwrap();
    assert!(screen.contains(cwd_name), "{screen}");

    tmux.kill("usine-review-g-p-1").unwrap();
    assert_eq!(tmux.sessions("usine-").unwrap(), []);
}

#[test]
fn keeps_finished_sessions_visible() {
    let server = server("finished");
    let tmux = &server.0;
    let cwd = tempfile::tempdir().unwrap();

    tmux.start("usine-x", "x", cwd.path(), &sh("echo 'command not found'; exit 1"))
        .unwrap();

    wait_for(|| tmux.sessions("usine-").unwrap()[0].finished);
    let screen = tmux.capture("usine-x").unwrap();
    assert!(screen.starts_with("command not found\n\nPane is dead"), "{screen:?}");
}

#[test]
fn errors_on_unknown_session() {
    let server = server("unknown");
    let tmux = &server.0;
    let cwd = tempfile::tempdir().unwrap();
    tmux.start("usine-a", "a", cwd.path(), &sh("sleep 30")).unwrap();

    assert!(tmux.capture("usine-missing").is_err());
    assert!(tmux.kill("usine-missing").is_err());
}
