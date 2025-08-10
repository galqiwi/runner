use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{fs, process::Command};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RunRequest {
    pub files: HashMap<String, String>,
    pub command: String,
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

        let output = Command::new("bash")
            .arg("-lc")
            .arg(&request.command)
            .current_dir(&working_dir)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

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
                candidate = base_tmp.join(format!(
                    "runner-{process_id}-{timestamp_nanos}-{counter}"
                ));
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
