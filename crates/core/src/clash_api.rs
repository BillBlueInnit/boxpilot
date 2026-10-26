//! Client for sing-box's Clash-compatible controller API.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, bail};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone)]
pub struct ClashApi {
    base: String,
    secret: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DelayHistory {
    pub delay: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyInfo {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub now: Option<String>,
    #[serde(default)]
    pub all: Option<Vec<String>>,
    #[serde(default)]
    pub history: Vec<DelayHistory>,
}

impl ProxyInfo {
    pub fn is_group(&self) -> bool {
        self.all.is_some()
    }

    pub fn last_delay(&self) -> Option<u32> {
        self.history.last().map(|h| h.delay)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ProxiesResponse {
    proxies: HashMap<String, ProxyInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnMetadata {
    #[serde(default)]
    pub network: String,
    #[serde(default, rename = "type")]
    pub inbound: String,
    #[serde(default, rename = "sourceIP")]
    pub source_ip: String,
    #[serde(default, rename = "destinationIP")]
    pub destination_ip: String,
    #[serde(default)]
    pub destination_port: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub process_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    #[serde(default)]
    pub metadata: ConnMetadata,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub rule_payload: String,
}

impl Connection {
    pub fn target(&self) -> String {
        let host = if self.metadata.host.is_empty() {
            &self.metadata.destination_ip
        } else {
            &self.metadata.host
        };
        if self.metadata.destination_port.is_empty() {
            host.clone()
        } else {
            format!("{host}:{}", self.metadata.destination_port)
        }
    }

    pub fn process(&self) -> &str {
        let p = &self.metadata.process_path;
        p.rsplit(['\\', '/']).next().unwrap_or(p)
    }

    /// Outbound actually used (last element of the chain).
    pub fn outbound(&self) -> &str {
        self.chains.first().map(String::as_str).unwrap_or("")
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionsSnapshot {
    #[serde(default)]
    pub download_total: u64,
    #[serde(default)]
    pub upload_total: u64,
    #[serde(default)]
    pub connections: Option<Vec<Connection>>,
    #[serde(default)]
    pub memory: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub struct Traffic {
    pub up: u64,
    pub down: u64,
}

impl ClashApi {
    pub fn new(port: u16, secret: &str) -> Self {
        Self {
            base: format!("http://127.0.0.1:{port}"),
            secret: secret.to_string(),
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client"),
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self.http.request(method, format!("{}{}", self.base, path));
        if !self.secret.is_empty() {
            r = r.bearer_auth(&self.secret);
        }
        r
    }

    pub async fn version(&self) -> Result<String> {
        let v: serde_json::Value = self
            .req(reqwest::Method::GET, "/version")
            .timeout(Duration::from_secs(2))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v["version"].as_str().unwrap_or_default().to_string())
    }

    pub async fn proxies(&self) -> Result<HashMap<String, ProxyInfo>> {
        let r: ProxiesResponse = self
            .req(reqwest::Method::GET, "/proxies")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(r.proxies)
    }

    pub async fn select(&self, group: &str, name: &str) -> Result<()> {
        let resp = self
            .req(reqwest::Method::PUT, &format!("/proxies/{}", enc(group)))
            .json(&json!({ "name": name }))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("select failed: HTTP {}", resp.status());
        }
        Ok(())
    }

    /// Returns the delay in ms, or an error for timeouts / failures.
    pub async fn delay(&self, name: &str, url: &str, timeout_ms: u32) -> Result<u32> {
        let resp = self
            .req(reqwest::Method::GET, &format!("/proxies/{}/delay", enc(name)))
            .query(&[("url", url), ("timeout", &timeout_ms.to_string())])
            .timeout(Duration::from_millis(timeout_ms as u64 + 3000))
            .send()
            .await?;
        let status = resp.status();
        let v: serde_json::Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{}", v["message"].as_str().unwrap_or("timeout"));
        }
        match v["delay"].as_u64() {
            Some(d) if d > 0 => Ok(d as u32),
            _ => bail!("timeout"),
        }
    }

    pub async fn connections(&self) -> Result<ConnectionsSnapshot> {
        Ok(self
            .req(reqwest::Method::GET, "/connections")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn close_connection(&self, id: &str) -> Result<()> {
        self.req(reqwest::Method::DELETE, &format!("/connections/{}", enc(id)))
            .send()
            .await?;
        Ok(())
    }

    pub async fn close_all_connections(&self) -> Result<()> {
        self.req(reqwest::Method::DELETE, "/connections").send().await?;
        Ok(())
    }

    pub async fn set_mode(&self, mode: &str) -> Result<()> {
        let resp = self
            .req(reqwest::Method::PATCH, "/configs")
            .json(&json!({ "mode": mode }))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("set mode failed: HTTP {}", resp.status());
        }
        Ok(())
    }

    /// Streams `/traffic` (one sample per second) until the stream ends or `on_sample` returns false.
    pub async fn stream_traffic(&self, mut on_sample: impl FnMut(Traffic) -> bool) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/traffic", self.base))
            .bearer_auth(&self.secret)
            .timeout(Duration::from_secs(60 * 60 * 24 * 365))
            .send()
            .await?
            .error_for_status()?;
        let mut stream = resp.bytes_stream();
        let mut buf = Vec::new();
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk?);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if let Ok(t) = serde_json::from_slice::<Traffic>(&line) {
                    if !on_sample(t) {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}
