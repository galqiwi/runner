use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{fs, io::AsyncReadExt, process::Command, time::timeout};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RunRequest {
    pub files: HashMap<String, String>,
    pub command: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RunResponse {
    pub stdout: String,
    pub stderr: String,
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OkRunResponse {
    pub stdout: String,
    pub stderr: String,
}

pub async fn run(request: RunRequest) -> RunResponse {
    let response = do_run(request).await;

    match response {
        Ok(response) => RunResponse {
            stdout: response.stdout,
            stderr: response.stderr,
            error: String::new(),
        },
        Err(error) => RunResponse {
            stdout: String::new(),
            stderr: String::new(),
            error: error.to_string(),
        },
    }
}

pub async fn do_run(request: RunRequest) -> anyhow::Result<OkRunResponse> {
    let working_dir = create_unique_temp_dir().await?;

    let run_result = async {
        for (name, content) in request.files {
            let file_path = sanitize_and_join(&working_dir, &name)?;

            if let Some(parent_dir) = file_path.parent() {
                fs::create_dir_all(parent_dir).await?;
            }

            fs::write(&file_path, content).await?;
        }

        // Build the command so we can set up a new process group (session) before spawn
        let mut cmd = Command::new("bash");
        cmd.arg("-lc")
            .arg(&request.command)
            .current_dir(&working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        unsafe {
            use nix::unistd::setsid;
            // Create a new session so the spawned process becomes the leader of a new process group
            cmd.pre_exec(|| match setsid() {
                Ok(_) => Ok(()),
                Err(err) => Err(std::io::Error::other(
                    format!("setsid failed: {err}"),
                )),
            });
        }

        let mut child = cmd.spawn()?;

        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();

        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(ref mut out) = stdout_pipe {
                let _ = out.read_to_end(&mut buf).await;
            }
            buf
        });

        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(ref mut err) = stderr_pipe {
                let _ = err.read_to_end(&mut buf).await;
            }
            buf
        });

        if request.timeout_seconds > 0 {
            let duration = Duration::from_secs(request.timeout_seconds);
            match timeout(duration, child.wait()).await {
                Ok(wait_res) => {
                    let _status = wait_res?;
                }
                Err(_) => {
                    // On timeout, send SIGKILL to the entire process group created via setsid()
                    #[cfg(unix)]
                    {
                        use nix::sys::signal::{kill, Signal};
                        use nix::unistd::Pid;
                        if let Some(raw_pid_u32) = child.id() {
                            let pid = raw_pid_u32 as i32;
                            if pid > 0 {
                                let _ = kill(Pid::from_raw(-pid), Signal::SIGKILL);
                            }
                        }
                    }
                    // Fallback: ensure the direct child is also killed
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    let _ = stdout_task.await.unwrap_or_default();
                    let stderr_bytes = stderr_task.await.unwrap_or_default();
                    let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
                    anyhow::bail!(
                        "timed out after {} seconds\n{}",
                        request.timeout_seconds,
                        stderr
                    )
                }
            }
        } else {
            let _status = child.wait().await?;
        }

        let stdout_bytes = stdout_task.await.unwrap_or_default();
        let stderr_bytes = stderr_task.await.unwrap_or_default();

        let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
        let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

        Ok::<OkRunResponse, anyhow::Error>(OkRunResponse { stdout, stderr })
    }
    .await;

    let _ = fs::remove_dir_all(&working_dir).await;

    run_result
}

async fn create_unique_temp_dir() -> anyhow::Result<PathBuf> {
    let base_tmp = std::env::temp_dir();
    let process_id = std::process::id();
    let timestamp_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut candidate = base_tmp.join(format!("runner-{process_id}-{timestamp_nanos}"));

    let mut counter: u32 = 0;
    loop {
        match fs::create_dir(&candidate).await {
            Ok(_) => return Ok(candidate),
            Err(e) if counter < 10 && e.kind() == std::io::ErrorKind::AlreadyExists => {
                counter += 1;
                candidate =
                    base_tmp.join(format!("runner-{process_id}-{timestamp_nanos}-{counter}"));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn sanitize_and_join(base: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let candidate = Path::new(name);

    if candidate.is_absolute()
        || candidate
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        anyhow::bail!("invalid file name: must be a relative path without '..'");
    }

    Ok(base.join(candidate))
}
