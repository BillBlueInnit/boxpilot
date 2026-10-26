//! Built-in subscription converter: turns v2ray share links (plain or base64),
//! Clash / Clash.Meta YAML and sing-box JSON into sing-box outbounds.

mod clash;
mod uri;
mod util;

use serde_json::{Value, json};

use crate::model::Node;
pub use uri::parse_uri;
pub use util::split_host_port;

pub fn decode_base64(s: &str) -> Option<String> {
    util::b64_decode(s)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    SingBox,
    Clash,
    ShareLinks,
}

impl SourceFormat {
    pub fn label(&self) -> &'static str {
        match self {
            SourceFormat::SingBox => "sing-box",
            SourceFormat::Clash => "Clash",
            SourceFormat::ShareLinks => "Share links",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParseResult {
    pub format: SourceFormat,
    pub nodes: Vec<Node>,
    /// Human readable reasons for entries that could not be converted.
    pub skipped: Vec<String>,
}

/// Detects the subscription format and converts every node it can.
pub fn parse_subscription(content: &str) -> Result<ParseResult, String> {
    parse_inner(content, 0)
}

fn parse_inner(content: &str, depth: u8) -> Result<ParseResult, String> {
    let text = content.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Err("subscription is empty".into());
    }
    if text.starts_with('{') {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
        if v.get("outbounds").is_some() || v.get("endpoints").is_some() {
            return parse_singbox(&v);
        }
        if v.get("proxies").is_some() {
            return parse_clash_result(text);
        }
        return Err("JSON document has no `outbounds` or `proxies`".into());
    }
    if looks_like_clash(text) {
        return parse_clash_result(text);
    }
    if text.contains("://") {
        return Ok(parse_links(text));
    }
    if depth == 0 {
        if let Some(decoded) = util::b64_decode(text) {
            return parse_inner(&decoded, depth + 1);
        }
    }
    Err("unrecognized subscription format".into())
}

fn looks_like_clash(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("proxies:") || l.starts_with("\"proxies\":"))
}

fn parse_clash_result(text: &str) -> Result<ParseResult, String> {
    let (nodes, skipped) = clash::parse_clash(text)?;
    Ok(ParseResult {
        format: SourceFormat::Clash,
        nodes,
        skipped,
    })
}

fn parse_links(text: &str) -> ParseResult {
    let mut nodes = Vec::new();
    let mut skipped = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !line.contains("://") {
            continue;
        }
        match parse_uri(line) {
            Ok(n) => nodes.push(n),
            Err(e) => {
                let preview: String = line.chars().take(48).collect();
                skipped.push(format!("{preview}…: {e}"));
            }
        }
    }
    ParseResult {
        format: SourceFormat::ShareLinks,
        nodes,
        skipped,
    }
}

const PROXY_TYPES: &[&str] = &[
    "shadowsocks",
    "vmess",
    "vless",
    "trojan",
    "hysteria",
    "hysteria2",
    "tuic",
    "socks",
    "http",
    "anytls",
    "ssh",
    "shadowtls",
    "naive",
    "wireguard",
];

fn parse_singbox(v: &Value) -> Result<ParseResult, String> {
    let mut nodes = Vec::new();
    let mut skipped = Vec::new();
    let items = v
        .get("outbounds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(v.get("endpoints").and_then(Value::as_array).into_iter().flatten());
    for item in items {
        let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
        let tag = item
            .get("tag")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !PROXY_TYPES.contains(&ty) {
            continue;
        }
        if item.get("detour").is_some() {
            skipped.push(format!("{tag}: chained outbounds (detour) are not imported"));
            continue;
        }
        let mut out = item.clone();
        let obj = out.as_object_mut().unwrap();
        obj.remove("tag");
        if ty == "wireguard" && obj.contains_key("server") {
            // Legacy WireGuard outbound (removed in sing-box 1.13) -> endpoint.
            migrate_legacy_wireguard(obj);
        }
        let (server, port) = if ty == "wireguard" {
            let peer = obj.get("peers").and_then(|p| p.get(0));
            (
                peer.and_then(|p| p.get("address")).and_then(Value::as_str).unwrap_or(""),
                peer.and_then(|p| p.get("port")).and_then(Value::as_u64).unwrap_or(0),
            )
        } else {
            (
                obj.get("server").and_then(Value::as_str).unwrap_or(""),
                obj.get("server_port").and_then(Value::as_u64).unwrap_or(0),
            )
        };
        nodes.push(Node {
            name: if tag.is_empty() { format!("{server}:{port}") } else { tag },
            protocol: ty.to_string(),
            server: server.to_string(),
            port: port as u16,
            outbound: out,
        });
    }
    Ok(ParseResult {
        format: SourceFormat::SingBox,
        nodes,
        skipped,
    })
}

fn migrate_legacy_wireguard(obj: &mut serde_json::Map<String, Value>) {
    let server = obj.remove("server").unwrap_or(Value::Null);
    let port = obj.remove("server_port").unwrap_or(Value::Null);
    let public_key = obj.remove("peer_public_key").unwrap_or(Value::Null);
    let psk = obj.remove("pre_shared_key");
    let reserved = obj.remove("reserved");
    if let Some(addr) = obj.remove("local_address") {
        obj.insert("address".into(), addr);
    }
    let mut peer = json!({
        "address": server,
        "port": port,
        "public_key": public_key,
        "allowed_ips": ["0.0.0.0/0", "::/0"],
    });
    if let Some(psk) = psk {
        peer["pre_shared_key"] = psk;
    }
    if let Some(r) = reserved {
        peer["reserved"] = r;
    }
    obj.insert("peers".into(), json!([peer]));
}

/// Applies a subscription's include / exclude regex filters to node names.
pub fn filter_nodes(nodes: Vec<Node>, include: &str, exclude: &str) -> Result<Vec<Node>, String> {
    let build = |p: &str| -> Result<Option<regex::Regex>, String> {
        let p = p.trim();
        if p.is_empty() {
            return Ok(None);
        }
        regex::RegexBuilder::new(p)
            .case_insensitive(true)
            .build()
            .map(Some)
            .map_err(|e| format!("invalid filter `{p}`: {e}"))
    };
    let include = build(include)?;
    let exclude = build(exclude)?;
    Ok(nodes
        .into_iter()
        .filter(|n| include.as_ref().is_none_or(|r| r.is_match(&n.name)))
        .filter(|n| exclude.as_ref().is_none_or(|r| !r.is_match(&n.name)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_base64_links() {
        let links = "trojan://pw@a.com:443#A\nss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@1.2.3.4:8388#B\nssr://xyz\n";
        let encoded = util::b64_encode(links);
        let r = parse_subscription(&encoded).unwrap();
        assert_eq!(r.format, SourceFormat::ShareLinks);
        assert_eq!(r.nodes.len(), 2);
        assert_eq!(r.skipped.len(), 1);
    }

    #[test]
    fn singbox_json() {
        let doc = r#"{"outbounds":[
            {"type":"selector","tag":"proxy","outbounds":["a"]},
            {"type":"vless","tag":"a","server":"x.com","server_port":443,"uuid":"u"},
            {"type":"wireguard","tag":"wg","server":"1.1.1.1","server_port":51820,"local_address":["10.0.0.2/32"],"private_key":"k","peer_public_key":"p"},
            {"type":"direct","tag":"direct"}]}"#;
        let r = parse_subscription(doc).unwrap();
        assert_eq!(r.format, SourceFormat::SingBox);
        assert_eq!(r.nodes.len(), 2);
        assert!(r.nodes[0].outbound.get("tag").is_none());
        let wg = &r.nodes[1];
        assert_eq!(wg.server, "1.1.1.1");
        assert_eq!(wg.outbound["peers"][0]["public_key"], "p");
        assert_eq!(wg.outbound["address"][0], "10.0.0.2/32");
    }

    #[test]
    fn filters() {
        let r = parse_subscription("trojan://a@a.com:1#HK 01\ntrojan://a@a.com:1#US 01\ntrojan://a@a.com:1#剩余流量").unwrap();
        let kept = filter_nodes(r.nodes, "hk|us", "").unwrap();
        assert_eq!(kept.len(), 2);
        let kept = filter_nodes(kept, "", "^us").unwrap();
        assert_eq!(kept[0].name, "HK 01");
    }
}
