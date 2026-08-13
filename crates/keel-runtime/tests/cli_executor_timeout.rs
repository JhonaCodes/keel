use keel_runtime::{CliModelExecutor, ModelExecutor, ModelMessage, ModelRequest};
use std::time::{Duration, Instant};

fn write_script(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    path.to_str().unwrap().to_string()
}

/// A governed CLI that runs longer than the executor's timeout is killed, and
/// `complete` returns an error promptly — a slow external auditor can never hang
/// the single-threaded MCP server.
#[test]
fn cli_executor_times_out_and_kills_a_slow_child() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "slow.sh",
        "#!/bin/sh\ncat >/dev/null\nsleep 10\necho done\n",
    );
    let mut exec = CliModelExecutor::new(vec!["sh".to_string(), script], dir.path(), "slow-cli")
        .with_timeout(Duration::from_millis(400));

    let start = Instant::now();
    let res = exec.complete(ModelRequest::new("s", vec![ModelMessage::user("hi")]));
    let elapsed = start.elapsed();

    assert!(
        res.is_err(),
        "a timed-out child yields an error, not empty success"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "complete must return promptly after the timeout, took {elapsed:?}"
    );
}

/// A fast child still returns its stdout when a timeout is set.
#[test]
fn cli_executor_with_timeout_returns_fast_child_output() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "fast.sh",
        "#!/bin/sh\ncat >/dev/null\nprintf 'ok'\n",
    );
    let mut exec = CliModelExecutor::new(vec!["sh".to_string(), script], dir.path(), "fast-cli")
        .with_timeout(Duration::from_secs(5));

    let res = exec
        .complete(ModelRequest::new("s", vec![ModelMessage::user("hi")]))
        .unwrap();
    assert_eq!(res.content, "ok");
}
