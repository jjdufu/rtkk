//! Integration tests for Step 02 Daemon core: token/session auth, single-flight mutating
//! [`JobManager`] mutex (`409 CONFLICT_ACTIVE_JOB`), and gapless JSONL + live SSE event replay.

use axum::response::IntoResponse as _;
use rtkk::daemon::jobs::{JobConcurrencyClass, JobStatus};
use rtkk::daemon::{ServeOptions, build_app_router, initialize_daemon_state};
use rtkk::logging::{ExecutionEvent, emit_event};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Notify;

struct TempDaemonDir {
    path: PathBuf,
}

impl TempDaemonDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("rtkk-s02-{prefix}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for TempDaemonDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

#[tokio::test]
async fn mutating_job_mutex_blocks_concurrent_writes() -> Result<(), String> {
    let dir = TempDaemonDir::new("mutex")?;
    let opts = ServeOptions {
        listen: "127.0.0.1:0".to_string(),
        work_dir: dir.path.clone(),
        token_file: Some(dir.path.join(".rtkk-daemon/admin.token")),
    };
    let (state, _token_path, bootstrapped) =
        initialize_daemon_state(&opts).map_err(|e| e.to_string())?;
    ensure(bootstrapped, "token must bootstrap on empty workspace")?;

    let gate = Arc::new(Notify::new());
    let gate_for_job = Arc::clone(&gate);

    let first = state
        .jobs()
        .spawn_job(
            "deploy",
            "Cluster deploy",
            JobConcurrencyClass::Mutating,
            move |_cancel| async move {
                gate_for_job.notified().await;
                Ok(())
            },
        )
        .map_err(|_| "first mutating job must succeed".to_string())?;

    // Second mutating job while first is running must return 409 CONFLICT_ACTIVE_JOB
    let Err(conflict_err) = state.jobs().spawn_job(
        "cleanup",
        "Cluster cleanup",
        JobConcurrencyClass::Mutating,
        |_cancel| async move { Ok(()) },
    ) else {
        return Err("expected second mutating job to be blocked".to_string());
    };
    let resp = conflict_err.into_response();
    ensure(
        resp.status() == axum::http::StatusCode::CONFLICT,
        &format!("expected 409 CONFLICT, got {}", resp.status()),
    )?;

    // Read-only job must NOT be blocked while mutating job holds the lock
    let readonly = state
        .jobs()
        .spawn_job(
            "check",
            "Read-only preflight check",
            JobConcurrencyClass::Readonly,
            |_cancel| async move { Ok(()) },
        )
        .map_err(|_| "readonly job must run concurrently alongside mutating job".to_string())?;

    gate.notify_one();

    for _ in 0..50 {
        let d1 = state.jobs().get_job(&first.id);
        let d2 = state.jobs().get_job(&readonly.id);
        if d1.is_some_and(|d| d.summary.status == JobStatus::Succeeded)
            && d2.is_some_and(|d| d.summary.status == JobStatus::Succeeded)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    ensure(
        state.jobs().active_mutating_job_id().is_none(),
        "active mutating lock must clear once first job completes",
    )?;
    Ok(())
}

#[tokio::test]
async fn sse_replays_persisted_jsonl_and_tails_live_events() -> Result<(), String> {
    let dir = TempDaemonDir::new("sse")?;
    let token_path = dir.path.join(".rtkk-daemon/admin.token");
    let opts = ServeOptions {
        listen: "127.0.0.1:0".to_string(),
        work_dir: dir.path.clone(),
        token_file: Some(token_path.clone()),
    };
    let (state, _, _) = initialize_daemon_state(&opts).map_err(|e| e.to_string())?;
    let token = std::fs::read_to_string(&token_path)
        .map_err(|e| e.to_string())?
        .trim()
        .to_string();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let app = build_app_router(state.clone());
    let server_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let resume_live = Arc::new(Notify::new());
    let resume_clone = Arc::clone(&resume_live);
    let summary = state
        .jobs()
        .spawn_job(
            "playbook_run",
            "SSE replay & live tail test",
            JobConcurrencyClass::Mutating,
            move |_cancel| async move {
                let _ = emit_event(ExecutionEvent::PlayStarted {
                    play_name: "Historical Play".to_string(),
                    hosts: vec!["c23-p61".to_string()],
                });
                resume_clone.notified().await;
                let _ = emit_event(ExecutionEvent::TaskFinished {
                    task_name: "Live Task".to_string(),
                    host: "c23-p61".to_string(),
                    changed: true,
                    failed: false,
                    skipped: false,
                    stdout: "applied".to_string(),
                    stderr: String::new(),
                    duration_ms: 15,
                });
                Ok(())
            },
        )
        .map_err(|_| "failed to spawn SSE test job".to_string())?;

    // Allow event #1 to flush to JSONL before SSE client connects
    tokio::time::sleep(Duration::from_millis(60)).await;

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let sse_url = format!("http://{addr}/api/v1/jobs/{}/events", summary.id);
    let sse_fut = tokio::spawn({
        let c = client.clone();
        let tok = token.clone();
        let url = sse_url.clone();
        async move {
            c.get(&url)
                .bearer_auth(&tok)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .text()
                .await
                .map_err(|e| e.to_string())
        }
    });

    tokio::time::sleep(Duration::from_millis(60)).await;
    resume_live.notify_one();

    let live_sse_body = sse_fut.await.map_err(|e| e.to_string())??;
    ensure(
        live_sse_body.contains("Historical Play"),
        "SSE stream must replay historical JSONL event",
    )?;
    ensure(
        live_sse_body.contains("Live Task"),
        "SSE stream must tail live incremental event",
    )?;
    ensure(
        live_sse_body.contains("event: job_state") && live_sse_body.contains("succeeded"),
        "SSE stream must emit terminal job_state frame",
    )?;

    // Also verify pure post-completion JSONL replay
    let post_body = client
        .get(&sse_url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        post_body.contains("Historical Play") && post_body.contains("Live Task"),
        "post-completion SSE replay must return all persisted events",
    )?;

    server_handle.abort();
    Ok(())
}
