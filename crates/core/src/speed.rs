//! Download speed and real-connection latency through one node, measured with
//! Cloudflare's speed-test endpoints (the ones speed.cloudflare.com uses).

use std::time::{Duration, Instant};

use futures_util::StreamExt;

use crate::fetch::{self, Route};

/// An empty response: the time it takes is a full round trip through the
/// node over a fresh connection (proxy handshake, TLS, request, reply).
const LATENCY_URL: &str = "https://speed.cloudflare.com/__down?bytes=0";
/// Download size cap for a test the user starts; the time cap usually ends it first.
pub const DOWNLOAD_BYTES: u64 = 20_000_000;
/// Smaller cap for the Auto group's periodic measurements.
pub const AUTO_BYTES: u64 = 8_000_000;
const DOWNLOAD_TIME: Duration = Duration::from_secs(8);

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
    let client = fetch::client(&Route::Proxy(proxy_url.to_string()), Duration::from_secs(10), "BoxPilot").ok()?;
    let start = Instant::now();
    let resp = client.get(LATENCY_URL).send().await.ok()?.error_for_status().ok()?;
    let _ = resp.bytes().await;
    Some(start.elapsed().as_millis().min(u32::MAX as u128) as u32)
}

/// Measures through the HTTP proxy at `proxy_url` (the probe inbound, already
/// pointed at the node). A fresh client, so nothing is reused from earlier tests.
pub async fn measure(proxy_url: &str, bytes: u64) -> SpeedResult {
    let Ok(client) = fetch::client(&Route::Proxy(proxy_url.to_string()), Duration::from_secs(15), "BoxPilot") else {
        return SpeedResult::default();
    };
    let start = Instant::now();
    let latency_ms = match client.get(LATENCY_URL).send().await.and_then(|r| r.error_for_status()) {
        Ok(resp) => {
            let _ = resp.bytes().await;
            Some(start.elapsed().as_millis().min(u32::MAX as u128) as u32)
        }
        // A node that can't fetch an empty file won't download anything either.
        Err(_) => return SpeedResult::default(),
    };
    SpeedResult { latency_ms, bytes_per_sec: download(&client, bytes).await }
}

async fn download(client: &reqwest::Client, bytes: u64) -> Option<u64> {
    let url = format!("https://speed.cloudflare.com/__down?bytes={bytes}");
    let resp = client.get(url).send().await.ok()?.error_for_status().ok()?;
    let mut body = resp.bytes_stream();
    let start = Instant::now();
    let deadline = tokio::time::Instant::now() + DOWNLOAD_TIME;
    let mut total = 0u64;
    // Chunks are counted and dropped, so memory stays flat.
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(deadline, body.next()).await {
        total += chunk.len() as u64;
    }
    let secs = start.elapsed().as_secs_f64();
    (total > 0 && secs > 0.0).then(|| (total as f64 / secs) as u64)
}
