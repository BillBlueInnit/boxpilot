//! Download speed and real-connection latency through one node, measured with
//! Cloudflare's speed-test endpoints (the ones speed.cloudflare.com uses).
//!
//! Designed for both desktop and mobile (`crates/core`):
//! - Tight connect/TTFB timeouts so dead nodes fail quickly.
//! - Two parallel streams to saturate high-RTT links without TCP slow-start drag.
//! - Per-chunk stall timeout + early-stop once throughput stabilizes.
//! - Cancellation support so batch tests can stop immediately.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;

use crate::fetch::{self, Route};

/// An empty response: the time it takes is a full round trip through the
/// node over a fresh connection (proxy handshake, TLS, request, reply).
const LATENCY_URL: &str = "https://speed.cloudflare.com/__down?bytes=0";
/// Download size cap for a user-triggered test (split across parallel streams).
pub const DOWNLOAD_BYTES: u64 = 12_000_000;
/// Smaller cap for the Auto group's periodic measurements.
pub const AUTO_BYTES: u64 = 6_000_000;

const LATENCY_TIMEOUT: Duration = Duration::from_secs(4);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const DOWNLOAD_TIME: Duration = Duration::from_secs(4);
const STALL_TIMEOUT: Duration = Duration::from_millis(1800);
const WARMUP_TIME: Duration = Duration::from_millis(300);
const MIN_STABLE_TIME: Duration = Duration::from_millis(1600);
const MIN_STABLE_BYTES: u64 = 2_000_000;
const STREAMS: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpeedResult {
    /// Real-connection latency in ms; `None` when the node didn't answer.
    pub latency_ms: Option<u32>,
    /// Download speed in bytes per second.
    pub bytes_per_sec: Option<u64>,
}

/// Real-connection latency alone: one empty HTTPS request over a fresh
/// connection through the node. `None` when it fails.
pub async fn latency(proxy_url: &str) -> Option<u32> {
    let client = fetch::client(&Route::Proxy(proxy_url.to_string()), LATENCY_TIMEOUT, "BoxPilot").ok()?;
    let start = Instant::now();
    let resp = tokio::time::timeout(LATENCY_TIMEOUT, client.get(LATENCY_URL).send())
        .await
        .ok()?
        .ok()?
        .error_for_status()
        .ok()?;
    let _ = tokio::time::timeout(LATENCY_TIMEOUT, resp.bytes()).await;
    Some(start.elapsed().as_millis().min(u32::MAX as u128) as u32)
}

/// Measures through the HTTP proxy at `proxy_url` (a probe slot already pointed
/// at the node). A fresh client, so nothing is reused from earlier nodes.
pub async fn measure(proxy_url: &str, bytes: u64) -> SpeedResult {
    let never_cancel = AtomicBool::new(false);
    measure_cancellable(proxy_url, bytes, &never_cancel).await
}

/// Same as [`measure`], stopping early when `cancel` turns true.
pub async fn measure_cancellable(proxy_url: &str, bytes: u64, cancel: &AtomicBool) -> SpeedResult {
    if cancel.load(Ordering::Relaxed) {
        return SpeedResult::default();
    }
    let Ok(client) = fetch::client(&Route::Proxy(proxy_url.to_string()), REQUEST_TIMEOUT, "BoxPilot") else {
        return SpeedResult::default();
    };
    let start = Instant::now();
    let latency_ms = match tokio::time::timeout(LATENCY_TIMEOUT, client.get(LATENCY_URL).send()).await {
        Ok(Ok(resp)) => match resp.error_for_status() {
            Ok(resp) => {
                let _ = tokio::time::timeout(LATENCY_TIMEOUT, resp.bytes()).await;
                Some(start.elapsed().as_millis().min(u32::MAX as u128) as u32)
            }
            Err(_) => return SpeedResult::default(),
        },
        // A node that can't fetch an empty file won't download anything either.
        _ => return SpeedResult::default(),
    };
    if cancel.load(Ordering::Relaxed) {
        return SpeedResult { latency_ms, bytes_per_sec: None };
    }
    SpeedResult {
        latency_ms,
        bytes_per_sec: download_parallel(&client, bytes, cancel).await,
    }
}

async fn download_parallel(client: &reqwest::Client, bytes: u64, cancel: &AtomicBool) -> Option<u64> {
    let per_stream = (bytes / STREAMS).max(1_000_000);
    let url = format!("https://speed.cloudflare.com/__down?bytes={per_stream}");
    let total = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    let deadline = tokio::time::Instant::now() + DOWNLOAD_TIME;

    let mut handles = Vec::with_capacity(STREAMS as usize);
    for _ in 0..STREAMS {
        let client = client.clone();
        let url = url.clone();
        let total = total.clone();
        let stop = stop.clone();
        handles.push(tokio::spawn(async move {
            let resp = match tokio::time::timeout(LATENCY_TIMEOUT, client.get(&url).send()).await {
                Ok(Ok(r)) => match r.error_for_status() {
                    Ok(r) => r,
                    Err(_) => return,
                },
                _ => return,
            };
            let mut body = resp.bytes_stream();
            while !stop.load(Ordering::Relaxed) {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                let step_deadline = (now + STALL_TIMEOUT).min(deadline);
                match tokio::time::timeout_at(step_deadline, body.next()).await {
                    Ok(Some(Ok(chunk))) => {
                        total.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    }
                    _ => break,
                }
            }
        }));
    }

    // Monitor for cancellation, warmup snapshot and early-stop on steady throughput.
    let mut warmup_snapshot: Option<(Duration, u64)> = None;
    let mut prev_sample: Option<(Duration, u64)> = None;
    let mut prev_rate: Option<f64> = None;

    loop {
        if cancel.load(Ordering::Relaxed) {
            stop.store(true, Ordering::Relaxed);
            break;
        }
        if handles.iter().all(|h| h.is_finished()) {
            break;
        }
        let elapsed = start.elapsed();
        if elapsed >= DOWNLOAD_TIME {
            stop.store(true, Ordering::Relaxed);
            break;
        }
        let bytes_now = total.load(Ordering::Relaxed);
        if warmup_snapshot.is_none() && elapsed >= WARMUP_TIME {
            warmup_snapshot = Some((elapsed, bytes_now));
        }
        if let Some((pt, pb)) = prev_sample {
            let dt = (elapsed - pt).as_secs_f64();
            if dt >= 0.45 {
                let rate = bytes_now.saturating_sub(pb) as f64 / dt;
                if elapsed >= MIN_STABLE_TIME && bytes_now >= MIN_STABLE_BYTES {
                    if let Some(last_rate) = prev_rate {
                        let hi = rate.max(last_rate);
                        if hi > 0.0 && (rate - last_rate).abs() / hi <= 0.15 {
                            stop.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
                prev_rate = Some(rate);
                prev_sample = Some((elapsed, bytes_now));
            }
        } else {
            prev_sample = Some((elapsed, bytes_now));
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    stop.store(true, Ordering::Relaxed);
    for h in handles {
        h.abort();
    }

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    let elapsed = start.elapsed();
    let total_bytes = total.load(Ordering::Relaxed);
    if total_bytes == 0 || elapsed.as_secs_f64() <= 0.0 {
        return None;
    }

    // Prefer steady-state speed (excluding initial TCP slow-start warmup) when enough steady time elapsed.
    if let Some((wt, wb)) = warmup_snapshot {
        let steady_secs = (elapsed - wt).as_secs_f64();
        let steady_bytes = total_bytes.saturating_sub(wb);
        if steady_secs >= 0.6 && steady_bytes > 0 {
            return Some((steady_bytes as f64 / steady_secs) as u64);
        }
    }
    Some((total_bytes as f64 / elapsed.as_secs_f64()) as u64)
}
