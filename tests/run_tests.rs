use runner::run::{do_run, RunRequest};
use std::collections::HashMap;

fn mk_req(command: &str, timeout: u64, files: &[(&str, &str)]) -> RunRequest {
    let mut map = HashMap::new();
    for (k, v) in files.iter() {
        map.insert((*k).to_string(), (*v).to_string());
    }
    RunRequest {
        files: map,
        command: command.to_string(),
        timeout_seconds: timeout,
    }
}

#[tokio::test]
async fn echo_stdout_and_stderr() {
    let req = mk_req("echo -n hello; echo -n world 1>&2", 5, &[]);
    let res = do_run(req).await.unwrap();
    assert_eq!(res.stdout, "hello");
    assert_eq!(res.stderr, "world");
}

#[tokio::test]
async fn writes_and_reads_files() {
    let req = mk_req(
        "cat input.txt > out.txt; echo -n done",
        5,
        &[("input.txt", "data")],
    );
    let res = do_run(req).await.unwrap();
    assert_eq!(res.stdout, "done");
    assert!(res.stderr.is_empty());
}

#[tokio::test]
async fn rejects_invalid_paths() {
    let req = mk_req("true", 5, &[("../evil", "x")]);
    let err = do_run(req).await.unwrap_err().to_string();
    assert!(err.contains("invalid file name"));
}

#[tokio::test]
async fn times_out_and_kills_process_group() {
    // Start a process that spawns a child, both should be killed on timeout
    let script = r#"
        set -e
        bash -lc 'sleep 1000' &
        # Parent waits forever as well
        sleep 1000
    "#;
    let req = mk_req(script, 1, &[]);
    let err = do_run(req).await.unwrap_err().to_string();
    assert!(err.contains("timed out after"));
} 