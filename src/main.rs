use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use clap::Parser;
use runner::run::{run, RunRequest, RunResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

#[derive(Parser, Debug)]
#[command(name = "runner")]
struct Cli {
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long, default_value = "runner.log")]
    log_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub request: RunRequest,
    pub response: RunResponse,
}

#[derive(Clone)]
struct AppState {
    log_path: Arc<String>,
}

async fn log_entry(log_path: &str, entry: LogEntry) -> anyhow::Result<()> {
    let entry = serde_json::to_string(&entry)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .await?;
    file.write_all(entry.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

async fn handle_run(
    State(state): State<AppState>,
    Json(request): Json<RunRequest>,
) -> impl IntoResponse {
    let mut response: RunResponse = run(request.clone()).await;

    let log_result = log_entry(
        &state.log_path,
        LogEntry {
            request,
            response: response.clone(),
        },
    )
    .await;
    if let Err(e) = log_result {
        response = RunResponse {
            stdout: String::new(),
            stderr: String::new(),
            error: e.to_string(),
        };
    }

    (StatusCode::OK, Json(response))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    let addr = format!("{}:{}", args.host, args.port);

    let state = AppState {
        log_path: Arc::new(args.log_path),
    };

    let app = Router::new()
        .route("/run", post(handle_run))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    println!("listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}
