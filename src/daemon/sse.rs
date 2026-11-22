//! Server-Sent Events (`SSE`) historical JSONL replay and gapless live event stream builder.

use crate::daemon::jobs::{JobDetail, LiveJobFrame, SequencedExecutionEvent};
use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

fn format_sse_frame(id: u64, event_type: &str, json_payload: &str) -> Bytes {
    Bytes::from(format!(
        "id: {id}\nevent: {event_type}\ndata: {json_payload}\n\n"
    ))
}

fn replay_persisted_events(
    jsonl_path: &PathBuf,
    after_seq: u64,
    tx: &mpsc::Sender<Result<Bytes, std::io::Error>>,
) -> Option<u64> {
    let mut high_water = after_seq;
    let Ok(file) = std::fs::File::open(jsonl_path) else {
        return Some(high_water);
    };
    let reader = BufReader::new(file);
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<SequencedExecutionEvent>(trimmed) else {
            continue;
        };
        if parsed.seq <= high_water {
            continue;
        }
        high_water = parsed.seq;
        if tx
            .blocking_send(Ok(format_sse_frame(parsed.seq, "execution", trimmed)))
            .is_err()
        {
            return None;
        }
    }
    Some(high_water)
}

async fn pump_live_sse(
    detail: JobDetail,
    jsonl_path: PathBuf,
    mut rx: broadcast::Receiver<LiveJobFrame>,
    after_seq: u64,
    tx: mpsc::Sender<Result<Bytes, std::io::Error>>,
) {
    let replayed = tokio::task::spawn_blocking({
        let tx_clone = tx.clone();
        move || replay_persisted_events(&jsonl_path, after_seq, &tx_clone)
    })
    .await
    .ok()
    .flatten();

    let Some(mut high_water) = replayed else {
        return;
    };

    if detail.summary.status.is_terminal() {
        let state_seq = high_water.saturating_add(1);
        let payload = serde_json::json!({
            "seq": state_seq,
            "ts_ms": detail.summary.finished_at_ms.unwrap_or(detail.summary.created_at_ms),
            "status": detail.summary.status,
            "error": detail.summary.error,
        })
        .to_string();
        let _ = tx
            .send(Ok(format_sse_frame(state_seq, "job_state", &payload)))
            .await;
        return;
    }

    let mut keepalive = tokio::time::interval(Duration::from_secs(15));
    let _ = keepalive.tick().await;
    loop {
        tokio::select! {
            _ = keepalive.tick() => {
                if tx.send(Ok(Bytes::from_static(b": keep-alive\n\n"))).await.is_err() {
                    return;
                }
            }
            recv_res = rx.recv() => {
                match recv_res {
                    Ok(LiveJobFrame::Execution(seq_ev)) => {
                        if seq_ev.seq <= high_water {
                            continue;
                        }
                        high_water = seq_ev.seq;
                        if let Ok(json_str) = serde_json::to_string(&seq_ev)
                            && tx.send(Ok(format_sse_frame(seq_ev.seq, "execution", &json_str))).await.is_err()
                        {
                            return;
                        }
                    }
                    Ok(LiveJobFrame::State { seq, ts_ms, status, error }) => {
                        let final_seq = seq.max(high_water.saturating_add(1));
                        let payload = serde_json::json!({
                            "seq": final_seq,
                            "ts_ms": ts_ms,
                            "status": status,
                            "error": error,
                        })
                        .to_string();
                        let _ = tx.send(Ok(format_sse_frame(final_seq, "job_state", &payload))).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }
}

/// Builds an HTTP `200 text/event-stream` response replaying JSONL history then tailing live frames.
#[must_use]
pub fn build_job_sse_response(
    detail: JobDetail,
    jsonl_path: PathBuf,
    rx: broadcast::Receiver<LiveJobFrame>,
    after_seq: u64,
) -> Response {
    let (tx, mut mpsc_rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(256);
    tokio::spawn(pump_live_sse(detail, jsonl_path, rx, after_seq, tx));

    let byte_stream = futures_util::stream::poll_fn(move |cx| mpsc_rx.poll_recv(cx));
    let mut response = Response::new(Body::from_stream(byte_stream));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}
