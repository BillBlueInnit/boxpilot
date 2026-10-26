use base64::Engine;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, general_purpose};
use serde_json::{Map, Value, json};

const LENIENT: GeneralPurposeConfig = GeneralPurposeConfig::new()
    .with_decode_padding_mode(DecodePaddingMode::Indifferent)
    .with_decode_allow_trailing_bits(true);
const STANDARD_LENIENT: GeneralPurpose = GeneralPurpose::new(&base64::alphabet::STANDARD, LENIENT);
const URL_SAFE_LENIENT: GeneralPurpose = GeneralPurpose::new(&base64::alphabet::URL_SAFE, LENIENT);

/// Decodes base64 in any of the common flavours (standard / url-safe, padded or not,
/// with embedded whitespace).
pub fn b64_decode(input: &str) -> Option<String> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return None;
    }
    let bytes = STANDARD_LENIENT
        .decode(&cleaned)
        .or_else(|_| URL_SAFE_LENIENT.decode(&cleaned))
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[allow(dead_code)]
pub fn b64_encode(input: &str) -> String {
    general_purpose::STANDARD.encode(input)
}

pub fn pct_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

/// Query strings use `+` for spaces in some generators.
pub fn query_decode(s: &str) -> String {
    pct_decode(&s.replace('+', " "))
}

/// A share link split into its raw parts. Unlike `url::Url`, this accepts the
/// non-standard authorities used by proxy links (port ranges, unbracketed
/// userinfo characters, ...).
#[derive(Debug, Default, Clone)]
pub struct UriParts {
    pub scheme: String,
    /// Raw (still percent-encoded) userinfo.
    pub userinfo: Option<String>,
    pub host: String,
    /// Raw port text, e.g. `443` or `443,20000-30000`.
    pub port: String,
    #[allow(dead_code)]
    pub path: String,
    pub query: Vec<(String, String)>,
    /// Decoded fragment (node name).
    pub fragment: String,
}

impl UriParts {
    pub fn parse(link: &str) -> Option<Self> {
        let (scheme, rest) = link.split_once("://")?;
        let scheme = scheme.trim().to_ascii_lowercase();
        let (rest, fragment) = match rest.split_once('#') {
            Some((r, f)) => (r, pct_decode(f.trim())),
            None => (rest, String::new()),
        };
        let (rest, query_str) = match rest.split_once('?') {
            Some((r, q)) => (r, q),
            None => (rest, ""),
        };
        // The authority ends at the first '/' after the last '@'.
        let at = rest.rfind('@');
        let search_from = at.map(|i| i + 1).unwrap_or(0);
        let (authority, path) = match rest[search_from..].find('/') {
            Some(i) => rest.split_at(search_from + i),
            None => (rest, ""),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(i) => (Some(authority[..i].to_string()), &authority[i + 1..]),
            None => (None, authority),
        };
        let (host, port) = split_host_port(hostport);
        let query = query_str
            .split('&')
            .filter(|kv| !kv.is_empty())
            .map(|kv| match kv.split_once('=') {
                Some((k, v)) => (query_decode(k), query_decode(v)),
                None => (query_decode(kv), String::new()),
            })
            .collect();
        Some(Self {
            scheme,
            userinfo,
            host,
            port,
            path: path.to_string(),
            query,
            fragment,
        })
    }

    pub fn q(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }

    pub fn q_any(&self, keys: &[&str]) -> Option<&str> {
        keys.iter().find_map(|k| self.q(k))
    }

    pub fn q_bool(&self, keys: &[&str]) -> bool {
        self.q_any(keys).map(truthy).unwrap_or(false)
    }

    pub fn port_u16(&self) -> Option<u16> {
        self.port
            .split([',', '-'])
            .next()
            .and_then(|p| p.trim().parse().ok())
    }

    pub fn userinfo_decoded(&self) -> Option<String> {
        self.userinfo.as_deref().map(pct_decode)
    }
}

pub fn split_host_port(hostport: &str) -> (String, String) {
    let hostport = hostport.trim();
    if let Some(stripped) = hostport.strip_prefix('[') {
        if let Some((host, rest)) = stripped.split_once(']') {
            return (
                host.to_string(),
                rest.strip_prefix(':').unwrap_or("").to_string(),
            );
        }
    }
    match hostport.rsplit_once(':') {
        // A bare IPv6 address without brackets has several colons; treat it as host only.
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.to_string()),
        _ => (hostport.to_string(), String::new()),
    }
}

pub fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn split_list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Inserts a key only when the value is meaningful (non-empty string, non-null).
pub fn put(map: &mut Map<String, Value>, key: &str, value: impl Into<Value>) {
    let value = value.into();
    let empty = match &value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    };
    if !empty {
        map.insert(key.to_string(), value);
    }
}

#[derive(Debug, Default, Clone)]
pub struct TlsOpts {
    pub enabled: bool,
    pub server_name: Option<String>,
    pub insecure: bool,
    pub alpn: Vec<String>,
    pub fingerprint: Option<String>,
    pub reality_public_key: Option<String>,
    pub reality_short_id: Option<String>,
    pub disable_sni: bool,
}

impl TlsOpts {
    pub fn to_value(&self) -> Option<Value> {
        if !self.enabled {
            return None;
        }
        let mut tls = Map::new();
        tls.insert("enabled".into(), json!(true));
        if let Some(sni) = &self.server_name {
            put(&mut tls, "server_name", sni.as_str());
        }
        if self.insecure {
            tls.insert("insecure".into(), json!(true));
        }
        if self.disable_sni {
            tls.insert("disable_sni".into(), json!(true));
        }
        if !self.alpn.is_empty() {
            tls.insert("alpn".into(), json!(self.alpn));
        }
        let reality = self.reality_public_key.as_ref().filter(|k| !k.is_empty());
        let fp = self
            .fingerprint
            .as_ref()
            .filter(|f| !f.is_empty() && f.as_str() != "none");
        // REALITY requires uTLS; default to chrome when no fingerprint is given.
        if fp.is_some() || reality.is_some() {
            let fp = fp.cloned().unwrap_or_else(|| "chrome".into());
            let fp = if fp == "random" || fp == "randomized" {
                "randomized".to_string()
            } else {
                fp
            };
            tls.insert("utls".into(), json!({ "enabled": true, "fingerprint": fp }));
        }
        if let Some(pk) = reality {
            let mut r = Map::new();
            r.insert("enabled".into(), json!(true));
            r.insert("public_key".into(), json!(pk));
            if let Some(sid) = &self.reality_short_id {
                r.insert("short_id".into(), json!(sid));
            }
            tls.insert("reality".into(), Value::Object(r));
        }
        Some(Value::Object(tls))
    }
}

#[derive(Debug, Default, Clone)]
pub struct TransportOpts {
    pub network: String,
    pub host: Option<String>,
    pub path: Option<String>,
    pub service_name: Option<String>,
    pub header_type: Option<String>,
    pub method: Option<String>,
    pub headers: Vec<(String, String)>,
    pub max_early_data: Option<u32>,
    pub early_data_header: Option<String>,
}

impl TransportOpts {
    /// Returns `Ok(None)` for plain TCP, `Err` for transports sing-box lacks.
    pub fn to_value(&self) -> Result<Option<Value>, String> {
        let net = self.network.trim().to_ascii_lowercase();
        let mut t = Map::new();
        match net.as_str() {
            "" | "tcp" | "raw" => {
                if self.header_type.as_deref() != Some("http") {
                    return Ok(None);
                }
                t.insert("type".into(), json!("http"));
                if let Some(host) = &self.host {
                    t.insert("host".into(), json!(split_list(host)));
                }
                if let Some(path) = &self.path {
                    put(&mut t, "path", path.split(',').next().unwrap_or("/"));
                }
                if let Some(m) = &self.method {
                    put(&mut t, "method", m.as_str());
                }
            }
            "ws" | "websocket" => {
                t.insert("type".into(), json!("ws"));
                let mut path = self.path.clone().unwrap_or_else(|| "/".into());
                let mut early = self.max_early_data;
                // v2ray style early data: /path?ed=2048
                if let Some((p, q)) = path.clone().split_once('?') {
                    let mut rest = Vec::new();
                    for kv in q.split('&') {
                        match kv.split_once('=') {
                            Some(("ed", v)) => early = v.parse().ok().or(early),
                            _ => rest.push(kv),
                        }
                    }
                    path = if rest.is_empty() {
                        p.to_string()
                    } else {
                        format!("{p}?{}", rest.join("&"))
                    };
                }
                t.insert("path".into(), json!(path));
                let mut headers = Map::new();
                if let Some(host) = &self.host {
                    put(&mut headers, "Host", host.as_str());
                }
                for (k, v) in &self.headers {
                    if !k.eq_ignore_ascii_case("host") || !headers.contains_key("Host") {
                        put(&mut headers, k, v.as_str());
                    }
                }
                put(&mut t, "headers", Value::Object(headers));
                if let Some(ed) = early.filter(|e| *e > 0) {
                    t.insert("max_early_data".into(), json!(ed));
                    t.insert(
                        "early_data_header_name".into(),
                        json!(
                            self.early_data_header
                                .clone()
                                .unwrap_or_else(|| "Sec-WebSocket-Protocol".into())
                        ),
                    );
                }
            }
            "grpc" => {
                t.insert("type".into(), json!("grpc"));
                put(
                    &mut t,
                    "service_name",
                    self.service_name
                        .clone()
                        .or_else(|| self.path.clone())
                        .unwrap_or_default(),
                );
            }
            "h2" | "http" => {
                t.insert("type".into(), json!("http"));
                if let Some(host) = &self.host {
                    t.insert("host".into(), json!(split_list(host)));
                }
                if let Some(path) = &self.path {
                    put(&mut t, "path", path.as_str());
                }
                if let Some(m) = &self.method {
                    put(&mut t, "method", m.as_str());
                }
            }
            "httpupgrade" => {
                t.insert("type".into(), json!("httpupgrade"));
                if let Some(host) = &self.host {
                    put(&mut t, "host", host.as_str());
                }
                put(&mut t, "path", self.path.clone().unwrap_or_else(|| "/".into()));
            }
            "quic" => {
                t.insert("type".into(), json!("quic"));
            }
            other => return Err(format!("unsupported transport `{other}`")),
        }
        Ok(Some(Value::Object(t)))
    }
}

/// Parses bandwidth values such as `100`, `100 Mbps`, `1 Gbps`, `50mbps` into Mbps.
pub fn parse_mbps(v: &str) -> Option<u64> {
    let s = v.trim().to_ascii_lowercase();
    let digits: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let n: f64 = digits.parse().ok()?;
    let unit = s[digits.len()..].trim();
    let mbps = if unit.starts_with('g') {
        n * 1000.0
    } else if unit.starts_with('k') {
        n / 1000.0
    } else {
        n
    };
    Some(mbps.max(1.0) as u64)
}

/// `20000-30000,443` -> `["20000:30000", "443:443"]` for hysteria2 port hopping.
pub fn port_ranges(v: &str) -> Vec<String> {
    v.split([',', '/'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|p| match p.split_once('-') {
            Some((a, b)) => format!("{}:{}", a.trim(), b.trim()),
            None => format!("{p}:{p}"),
        })
        .collect()
}
