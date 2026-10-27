//! Which country a proxy server is in: from hints in the node name (flags,
//! place names, codes) or, failing that, a GeoIP lookup of the server IP.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::fetch::{self, Route};

/// GeoLite2-compatible country database (MetaCubeX build, ~7.5 MB), then a CDN mirror.
pub const DB_URLS: [&str; 2] = [
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb",
    "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/country.mmdb",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    /// Named by the provider (flag, place name or code in the node name).
    Name,
    /// GeoIP of the server address.
    Ip(IpAddr),
    /// Asked an IP service through the node: where traffic really leaves.
    Exit(IpAddr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Country {
    /// ISO 3166-1 alpha-2, upper case.
    pub code: String,
    pub origin: Origin,
}

/// Place names in node names, most specific first: "印度尼西亚" must win over "印度".
const PLACES: &[(&str, &[&str])] = &[
    ("HK", &["香港", "hong kong", "hongkong"]),
    ("MO", &["澳门", "澳門", "macau", "macao"]),
    ("TW", &["台湾", "台灣", "臺灣", "台北", "新北", "taiwan", "taipei"]),
    ("JP", &["日本", "东京", "東京", "大阪", "埼玉", "japan", "tokyo", "osaka"]),
    ("SG", &["新加坡", "狮城", "獅城", "singapore"]),
    ("KR", &["韩国", "韓國", "首尔", "首爾", "春川", "korea", "seoul"]),
    (
        "US",
        &[
            "美国", "美國", "洛杉矶", "洛杉磯", "圣何塞", "聖何塞", "西雅图", "芝加哥", "纽约", "紐約", "硅谷",
            "达拉斯", "凤凰城", "united states", "america", "los angeles", "san jose", "seattle", "new york",
            "chicago", "dallas", "silicon valley", "phoenix", "ashburn",
        ],
    ),
    ("GB", &["英国", "英國", "伦敦", "倫敦", "united kingdom", "britain", "england", "london"]),
    ("DE", &["德国", "德國", "法兰克福", "法蘭克福", "germany", "frankfurt", "berlin"]),
    ("FR", &["法国", "法國", "巴黎", "france", "paris"]),
    ("NL", &["荷兰", "荷蘭", "阿姆斯特丹", "netherlands", "holland", "amsterdam"]),
    ("RU", &["俄罗斯", "俄羅斯", "莫斯科", "russia", "moscow"]),
    ("CA", &["加拿大", "多伦多", "温哥华", "canada", "toronto", "vancouver", "montreal"]),
    ("AU", &["澳大利亚", "澳大利亞", "澳洲", "悉尼", "墨尔本", "australia", "sydney", "melbourne"]),
    ("ID", &["印度尼西亚", "印度尼西亞", "印尼", "雅加达", "indonesia", "jakarta"]),
    ("IN", &["印度", "孟买", "india", "mumbai"]),
    ("TR", &["土耳其", "伊斯坦布尔", "turkey", "türkiye", "istanbul"]),
    ("AR", &["阿根廷", "argentina"]),
    ("BR", &["巴西", "圣保罗", "brazil", "sao paulo"]),
    ("MY", &["马来西亚", "馬來西亞", "吉隆坡", "malaysia", "kuala lumpur"]),
    ("TH", &["泰国", "泰國", "曼谷", "thailand", "bangkok"]),
    ("VN", &["越南", "vietnam", "hanoi"]),
    ("PH", &["菲律宾", "菲律賓", "马尼拉", "philippines", "manila"]),
    ("IT", &["意大利", "義大利", "米兰", "italy", "milan"]),
    ("ES", &["西班牙", "马德里", "spain", "madrid"]),
    ("CH", &["瑞士", "苏黎世", "switzerland", "zurich"]),
    ("SE", &["瑞典", "斯德哥尔摩", "sweden", "stockholm"]),
    ("IE", &["爱尔兰", "愛爾蘭", "都柏林", "ireland", "dublin"]),
    ("PL", &["波兰", "波蘭", "华沙", "poland", "warsaw"]),
    ("UA", &["乌克兰", "烏克蘭", "基辅", "ukraine", "kyiv", "kiev"]),
    ("IL", &["以色列", "israel"]),
    ("AE", &["阿联酋", "阿聯酋", "迪拜", "杜拜", "emirates", "dubai"]),
    ("ZA", &["南非", "约翰内斯堡", "south africa", "johannesburg"]),
    ("MX", &["墨西哥", "mexico"]),
    ("CL", &["智利", "chile"]),
    ("EG", &["埃及", "egypt"]),
    ("NG", &["尼日利亚", "nigeria"]),
    ("FI", &["芬兰", "芬蘭", "赫尔辛基", "finland", "helsinki"]),
    ("NO", &["挪威", "norway", "oslo"]),
    ("DK", &["丹麦", "丹麥", "denmark", "copenhagen"]),
    ("AT", &["奥地利", "奧地利", "维也纳", "austria", "vienna"]),
    ("BE", &["比利时", "比利時", "belgium", "brussels"]),
    ("PT", &["葡萄牙", "portugal", "lisbon"]),
    ("GR", &["希腊", "希臘", "greece", "athens"]),
    ("CZ", &["捷克", "czech", "prague"]),
    ("HU", &["匈牙利", "hungary", "budapest"]),
    ("RO", &["罗马尼亚", "羅馬尼亞", "romania", "bucharest"]),
    ("BG", &["保加利亚", "bulgaria", "sofia"]),
    ("NZ", &["新西兰", "紐西蘭", "new zealand", "auckland"]),
    ("KZ", &["哈萨克斯坦", "kazakhstan", "almaty"]),
    ("PK", &["巴基斯坦", "pakistan"]),
    ("BD", &["孟加拉", "bangladesh"]),
    ("KH", &["柬埔寨", "cambodia"]),
    ("MN", &["蒙古", "mongolia"]),
    ("SA", &["沙特", "saudi"]),
    ("QA", &["卡塔尔", "qatar"]),
    ("IS", &["冰岛", "冰島", "iceland", "reykjavik"]),
    ("LU", &["卢森堡", "盧森堡", "luxembourg"]),
    ("PE", &["秘鲁", "秘魯", "peru"]),
    ("CO", &["哥伦比亚", "colombia"]),
    ("NP", &["尼泊尔", "nepal"]),
    ("MM", &["缅甸", "myanmar"]),
    ("LA", &["老挝", "laos"]),
];

/// Upper-case codes taken as-is when they stand alone ("HK 01", "US-LAX").
/// Codes that read like words in node names (NO, IS, AT, BE, ME, TV, AI…) are left out.
const CODES: &[&str] = &[
    "HK", "MO", "TW", "JP", "SG", "KR", "US", "GB", "DE", "FR", "NL", "RU", "CA", "AU", "ID", "IN", "TR", "AR",
    "BR", "MY", "TH", "VN", "PH", "IT", "ES", "CH", "SE", "IE", "PL", "UA", "IL", "AE", "ZA", "MX", "CL", "EG",
    "NG", "FI", "DK", "PT", "GR", "CZ", "HU", "RO", "BG", "NZ", "KZ", "PK", "BD", "KH", "MN", "SA", "QA", "LU",
    "PE", "CO", "NP", "MM", "CN",
];

/// Three-letter forms that are unambiguous in node names.
const ALIASES: &[(&str, &str)] = &[
    ("UK", "GB"), ("USA", "US"), ("HKG", "HK"), ("SGP", "SG"), ("JPN", "JP"), ("TWN", "TW"), ("KOR", "KR"),
    ("GBR", "GB"), ("DEU", "DE"), ("NLD", "NL"), ("RUS", "RU"), ("AUS", "AU"), ("TUR", "TR"), ("ARG", "AR"),
    ("BRA", "BR"), ("MYS", "MY"), ("THA", "TH"), ("VNM", "VN"), ("PHL", "PH"), ("IDN", "ID"),
];

/// The country a node name points at, if any.
pub fn from_name(name: &str) -> Option<&'static str> {
    if let Some(code) = flag(name) {
        return Some(code);
    }
    // Earliest mention wins ("香港-日本" is a Hong Kong node); on a tie the longer name.
    let lower = name.to_ascii_lowercase();
    let mut best: Option<(usize, usize, &'static str)> = None;
    let mut consider = |pos: usize, len: usize, code: &'static str| {
        if best.is_none_or(|(p, l, _)| pos < p || (pos == p && len > l)) {
            best = Some((pos, len, code));
        }
    };
    for (code, names) in PLACES {
        for n in *names {
            let pos = if n.is_ascii() { find_word(&lower, n) } else { name.find(n) };
            if let Some(pos) = pos {
                consider(pos, n.len(), code);
            }
        }
    }
    for (pos, run) in upper_runs(name) {
        let code = match run.len() {
            2 => CODES.iter().find(|c| **c == run).copied(),
            _ => None,
        }
        .or_else(|| ALIASES.iter().find(|(a, _)| *a == run).map(|(_, c)| *c));
        if let Some(code) = code {
            consider(pos, run.len(), code);
        }
    }
    if let Some((_, _, code)) = best {
        return Some(code);
    }
    // "中国" alone means mainland China; "中国香港" was caught above as HK.
    ["中国", "中國", "国内", "回国", "china"]
        .iter()
        .any(|n| if n.is_ascii() { find_word(&lower, n).is_some() } else { name.contains(n) })
        .then_some("CN")
}

/// `needle` as a whole word: "peru" must not match inside "superuser".
fn find_word(hay: &str, needle: &str) -> Option<usize> {
    let b = hay.as_bytes();
    hay.match_indices(needle).map(|(i, _)| i).find(|&i| {
        let end = i + needle.len();
        (i == 0 || !b[i - 1].is_ascii_alphabetic()) && (end == b.len() || !b[end].is_ascii_alphabetic())
    })
}

/// A regional-indicator flag emoji, e.g. 🇯🇵 → JP.
fn flag(name: &str) -> Option<&'static str> {
    let ri = |c: char| (0x1F1E6..=0x1F1FF).contains(&(c as u32)).then(|| (b'A' + (c as u32 - 0x1F1E6) as u8) as char);
    let chars: Vec<char> = name.chars().collect();
    chars.windows(2).find_map(|w| {
        let code: String = [ri(w[0])?, ri(w[1])?].iter().collect();
        // Flags are unambiguous, so the word-like codes count here too.
        CODES.iter().chain(["NO", "IS", "AT", "BE"].iter()).find(|c| **c == code).copied()
    })
}

/// Maximal runs of ASCII upper-case letters bounded by non-letters, with byte offsets.
fn upper_runs(s: &str) -> Vec<(usize, &str)> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_uppercase() {
                i += 1;
            }
            let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphabetic();
            let after_ok = i == bytes.len() || !bytes[i].is_ascii_alphabetic();
            if before_ok && after_ok {
                out.push((start, &s[start..i]));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// `tag → server host` for every proxy outbound/endpoint in a sing-box config.
pub fn servers(config: &Value) -> Vec<(String, String)> {
    let list = |key: &str| config.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    list("outbounds")
        .into_iter()
        .chain(list("endpoints"))
        .filter_map(|o| {
            let tag = o.get("tag")?.as_str()?.to_string();
            let server = o
                .get("server")
                .and_then(Value::as_str)
                // WireGuard endpoints keep the address on the peer.
                .or_else(|| o.pointer("/peers/0/address").and_then(Value::as_str))?;
            Some((tag, server.trim_matches(['[', ']']).to_string()))
        })
        .collect()
}

/// sing-box fake-IP ranges: answers from these say nothing about the server.
fn is_fake(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 198 && (o[1] & 0xFE) == 18
        }
        IpAddr::V6(v6) => (v6.segments()[0] & 0xFFC0) == 0xFC00,
    }
}

/// Resolves a server host to a real IP. With TUN + fake-IP the system resolver
/// hands out fake addresses, so DNS-over-HTTPS is the fallback.
pub async fn resolve(host: &str, routes: &[Route]) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    let system = tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 0))).await;
    if let Ok(Ok(addrs)) = system {
        let ips: Vec<IpAddr> = addrs.map(|a| a.ip()).filter(|ip| !is_fake(ip)).collect();
        if let Some(ip) = ips.iter().find(|ip| ip.is_ipv4()).or(ips.first()) {
            return Some(*ip);
        }
    }
    for route in routes {
        for doh in ["https://223.5.5.5/resolve", "https://1.1.1.1/dns-query"] {
            if let Ok(Some(ip)) = doh_query(doh, host, route).await {
                return Some(ip);
            }
        }
    }
    None
}

async fn doh_query(endpoint: &str, host: &str, route: &Route) -> Result<Option<IpAddr>> {
    let client = fetch::client(route, Duration::from_secs(6), "BoxPilot")?;
    let body: Value = client
        .get(endpoint)
        .query(&[("name", host), ("type", "A")])
        .header("accept", "application/dns-json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(body
        .get("Answer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| a.get("type").and_then(Value::as_u64) == Some(1))
        .find_map(|a| a.get("data")?.as_str()?.parse::<IpAddr>().ok())
        .filter(|ip| !is_fake(ip)))
}

/// Shared anycast/CDN ranges (Cloudflare, Fastly): a node fronted by a CDN has
/// one of these as its address, and its GeoIP says nothing about the exit.
const ANYCAST: &[&str] = &[
    "173.245.48.0/20", "103.21.244.0/22", "103.22.200.0/22", "103.31.4.0/22", "141.101.64.0/18",
    "108.162.192.0/18", "190.93.240.0/20", "188.114.96.0/20", "197.234.240.0/22", "198.41.128.0/17",
    "162.158.0.0/15", "104.16.0.0/13", "104.24.0.0/14", "172.64.0.0/13", "131.0.72.0/22",
    "151.101.0.0/16", "199.232.0.0/16",
    "2400:cb00::/32", "2606:4700::/32", "2803:f800::/32", "2405:b500::/32", "2405:8100::/32",
    "2a06:98c0::/29", "2c0f:f248::/32", "2a04:4e40::/32",
];

fn in_cidr(ip: &IpAddr, cidr: &str) -> bool {
    let Some((net, bits)) = cidr.split_once('/') else { return false };
    let (Ok(net), Ok(bits)) = (net.parse::<IpAddr>(), bits.parse::<u32>()) else { return false };
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            u32::from(*a) & mask == u32::from(n) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => {
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - bits) };
            u128::from(*a) & mask == u128::from(n) & mask
        }
        _ => false,
    }
}

/// The server address can't place the node: anycast/CDN, or not a public
/// unicast address at all (private, broadcast, placeholder nodes).
pub fn needs_exit_check(ip: &IpAddr) -> bool {
    let local = match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_documentation()
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (v6.segments()[0] & 0xFE00) == 0xFC00
        }
    };
    local || is_fake(ip) || ANYCAST.iter().any(|c| in_cidr(ip, c))
}

/// Asks IP services, through the HTTP proxy at `proxy_url`, which address and
/// country the request left from.
pub async fn exit_country(proxy_url: &str) -> Option<(IpAddr, String)> {
    // A fresh client per check: a pooled connection would reuse the previous node.
    let client = fetch::client(&Route::Proxy(proxy_url.to_string()), Duration::from_secs(10), "BoxPilot").ok()?;
    let valid = |code: &str| code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase());
    if let Ok(resp) = client.get("https://www.cloudflare.com/cdn-cgi/trace").send().await {
        if let Ok(text) = resp.text().await {
            let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(str::trim);
            if let (Some(ip), Some(loc)) = (field("ip=").and_then(|v| v.parse().ok()), field("loc=")) {
                if valid(loc) {
                    return Some((ip, loc.to_string()));
                }
            }
        }
    }
    let body: Value = client.get("https://api.ip.sb/geoip").send().await.ok()?.json().await.ok()?;
    let ip = body.get("ip")?.as_str()?.parse().ok()?;
    let code = body.get("country_code")?.as_str()?.to_ascii_uppercase();
    valid(&code).then_some((ip, code))
}

/// Whether `data` is a readable GeoIP database (checked before replacing the cached one).
pub fn is_database(data: &[u8]) -> bool {
    maxminddb::Reader::from_source(data).is_ok()
}

/// Looks up `ips` in the database at `path`. The file is read, used and
/// dropped, so nothing stays in memory between refreshes.
pub fn lookup(path: &Path, ips: &[IpAddr]) -> Result<HashMap<IpAddr, String>> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let reader = maxminddb::Reader::from_source(data).context("open GeoIP database")?;
    let mut out = HashMap::new();
    for ip in ips {
        if let Ok(Some(c)) = reader.lookup::<maxminddb::geoip2::Country>(*ip) {
            let code = c.country.and_then(|c| c.iso_code).or(c.registered_country.and_then(|c| c.iso_code));
            if let Some(code) = code {
                out.insert(*ip, code.to_ascii_uppercase());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_country_hints_from_names() {
        assert_eq!(from_name("🇯🇵 Tokyo 01"), Some("JP"));
        assert_eq!(from_name("🇬🇧 London"), Some("GB"));
        assert_eq!(from_name("香港 IPLC 02"), Some("HK"));
        assert_eq!(from_name("中国香港-01"), Some("HK"));
        assert_eq!(from_name("香港-日本 中转"), Some("HK"));
        assert_eq!(from_name("印度尼西亚 01"), Some("ID"));
        assert_eq!(from_name("印度 01"), Some("IN"));
        assert_eq!(from_name("US-LAX 2x"), Some("US"));
        assert_eq!(from_name("HK01"), Some("HK"));
        assert_eq!(from_name("Singapore premium"), Some("SG"));
        assert_eq!(from_name("UK London"), Some("GB"));
        assert_eq!(from_name("回国 上海"), Some("CN"));
        // No hint: JMS-style names fall through to GeoIP.
        assert_eq!(from_name("JMS-1405634@c62s2.portablesubmarines.com:443"), None);
        assert_eq!(from_name("VIP NO.1 WS"), None);
        assert_eq!(from_name("superuser-01"), None);
    }

    #[test]
    fn finds_servers_in_config() {
        let cfg = serde_json::json!({
            "outbounds": [
                { "type": "vless", "tag": "a", "server": "1.2.3.4" },
                { "type": "selector", "tag": "proxy", "outbounds": ["a"] },
                { "type": "direct", "tag": "direct" }
            ],
            "endpoints": [{ "type": "wireguard", "tag": "wg", "peers": [{ "address": "[2001:db8::1]" }] }]
        });
        assert_eq!(servers(&cfg), vec![("a".into(), "1.2.3.4".into()), ("wg".into(), "2001:db8::1".into())]);
        assert!(is_fake(&"198.18.0.7".parse().unwrap()));
        assert!(is_fake(&"198.19.255.1".parse().unwrap()));
        assert!(!is_fake(&"198.20.0.1".parse().unwrap()));
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(needs_exit_check(&ip("104.21.3.4")));
        assert!(needs_exit_check(&ip("2606:4700::6810:1")));
        assert!(needs_exit_check(&ip("255.255.255.255")));
        assert!(needs_exit_check(&ip("192.168.1.2")));
        assert!(!needs_exit_check(&ip("45.32.1.2")));
    }
}
