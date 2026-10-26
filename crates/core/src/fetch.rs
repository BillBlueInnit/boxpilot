//! HTTP downloads for subscriptions, with proxy / direct fallback and retries.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use reqwest::header::HeaderMap;

use crate::model::SubInfo;

/// One way of reaching the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Direct,
    /// Local mixed inbound, e.g. `http://127.0.0.1:7890`.
    Proxy(String),
}

impl Route {
    pub fn label(&self) -> &'static str {
        match self {
            Route::Direct => "direct",
            Route::Proxy(_) => "proxy",
        }
    }
}

pub fn client(route: &Route, timeout: Duration, user_agent: &str) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(timeout.min(Duration::from_secs(15)))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(8));
    b = match route {
        Route::Direct => b.no_proxy(),
        Route::Proxy(url) => b.proxy(reqwest::Proxy::all(url)?),
    };
    Ok(b.build()?)
}

#[derive(Debug, Clone, Default)]
pub struct Fetched {
    pub body: String,
    pub info: Option<SubInfo>,
    pub title: Option<String>,
    pub update_interval_hours: Option<u32>,
    pub web_page: Option<String>,
    pub route: &'static str,
}

/// Downloads a subscription, trying each route in order, each up to `attempts` times.
pub async fn fetch_subscription(
    url: &str,
    user_agent: &str,
    timeout: Duration,
    routes: &[Route],
    attempts: u32,
) -> Result<Fetched> {
    let mut errors = Vec::new();
    for route in routes {
        let client = client(route, timeout, user_agent)?;
        for attempt in 0..attempts.max(1) {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
            match fetch_once(&client, url).await {
                Ok(mut f) => {
                    f.route = route.label();
                    return Ok(f);
                }
                Err(e) => {
                    log::warn!("subscription fetch via {} failed: {e:#}", route.label());
                    errors.push(format!("{}: {e:#}", route.label()));
                    // Client errors will not fix themselves on retry.
                    if e.to_string().starts_with("HTTP 4") {
                        break;
                    }
                }
            }
        }
    }
    let last = errors.pop().unwrap_or_else(|| "no route available".into());
    Err(anyhow!(last))
}

async fn fetch_once(client: &reqwest::Client, url: &str) -> Result<Fetched> {
    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("HTTP {}", status.as_u16());
    }
    let headers = resp.headers().clone();
    let body = resp.text().await?;
    Ok(Fetched {
        body,
        info: header(&headers, "subscription-userinfo").and_then(|v| parse_userinfo(&v)),
        title: header(&headers, "profile-title")
            .map(|t| decode_title(&t))
            .or_else(|| header(&headers, "content-disposition").and_then(|v| disposition_filename(&v))),
        update_interval_hours: header(&headers, "profile-update-interval")
            .and_then(|v| v.trim().parse().ok()),
        web_page: header(&headers, "profile-web-page-url"),
        route: "",
    })
}

fn header(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(name)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).trim().to_string())
        .filter(|v| !v.is_empty())
}

/// `upload=1; download=2; total=3; expire=1700000000`
pub fn parse_userinfo(v: &str) -> Option<SubInfo> {
    let mut info = SubInfo::default();
    let mut any = false;
    for part in v.split(';') {
        let Some((k, val)) = part.split_once('=') else { continue };
        let num = val.trim().parse::<f64>().unwrap_or(0.0);
        match k.trim().to_ascii_lowercase().as_str() {
            "upload" => info.upload = num as u64,
            "download" => info.download = num as u64,
            "total" => info.total = num as u64,
            "expire" => info.expire = num as i64,
            _ => continue,
        }
        any = true;
    }
    any.then_some(info)
}

fn decode_title(t: &str) -> String {
    match t.strip_prefix("base64:") {
        Some(b) => crate::convert::decode_base64(b).unwrap_or_else(|| t.to_string()),
        None => t.to_string(),
    }
}

/// `attachment; filename*=UTF-8''%E6%9C%BA%E5%9C%BA` or `filename="name.yaml"`.
pub fn disposition_filename(v: &str) -> Option<String> {
    let mut plain = None;
    for part in v.split(';').map(str::trim) {
        if let Some(rest) = part.strip_prefix("filename*=") {
            let encoded = rest.rsplit("''").next().unwrap_or(rest);
            let name = percent_encoding::percent_decode_str(encoded.trim_matches('"'))
                .decode_utf8_lossy()
                .into_owned();
            return Some(strip_ext(&name));
        }
        if let Some(rest) = part.strip_prefix("filename=") {
            plain = Some(strip_ext(rest.trim_matches('"')));
        }
    }
    plain.filter(|s| !s.is_empty())
}

fn strip_ext(name: &str) -> String {
    for ext in [".yaml", ".yml", ".json", ".txt", ".conf"] {
        if let Some(n) = name.strip_suffix(ext) {
            return n.to_string();
        }
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo() {
        let i = parse_userinfo("upload=1024; download=2048; total=10737418240; expire=1767225600").unwrap();
        assert_eq!(i.used(), 3072);
        assert_eq!(i.expire, 1767225600);
        assert!(parse_userinfo("garbage").is_none());
    }

    #[test]
    fn filenames() {
        assert_eq!(
            disposition_filename("attachment; filename*=UTF-8''%E6%9C%BA%E5%9C%BA.yaml").unwrap(),
            "机场"
        );
        assert_eq!(disposition_filename("attachment; filename=\"My Sub.yaml\"").unwrap(), "My Sub");
    }
}
