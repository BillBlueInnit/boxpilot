//! Rule-set import: Clash / ACL4SSR rule lists and YAML providers are converted
//! to sing-box source rule-sets; sing-box `.srs` and `.json` rule-sets are used
//! as-is. Also parses ACL4SSR / subconverter `.ini` configs into providers.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuleFormat {
    /// Detected from the file name and content.
    #[default]
    Auto,
    /// sing-box binary rule-set (`.srs`).
    SingBoxBinary,
    /// sing-box source rule-set (`.json`).
    SingBoxSource,
    /// Clash classical rules (`DOMAIN-SUFFIX,example.com`), ACL4SSR `.list` files
    /// and `payload:` YAML.
    Clash,
    /// Clash `behavior: domain` lists (`+.example.com`).
    ClashDomain,
    /// Clash `behavior: ipcidr` lists.
    ClashIpCidr,
}

impl RuleFormat {
    pub const ALL: [RuleFormat; 6] = [
        RuleFormat::Auto,
        RuleFormat::SingBoxBinary,
        RuleFormat::SingBoxSource,
        RuleFormat::Clash,
        RuleFormat::ClashDomain,
        RuleFormat::ClashIpCidr,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            RuleFormat::Auto => "Auto",
            RuleFormat::SingBoxBinary => "sing-box .srs",
            RuleFormat::SingBoxSource => "sing-box .json",
            RuleFormat::Clash => "Clash / ACL4SSR",
            RuleFormat::ClashDomain => "Clash domain",
            RuleFormat::ClashIpCidr => "Clash ipcidr",
        }
    }
}

/// A rule-set the user imported, applied with `target` when it matches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RuleProvider {
    pub id: String,
    pub name: String,
    /// `http(s)://…` or a local file path.
    pub source: String,
    pub format: RuleFormat,
    /// `proxy`, `direct`, `reject` or an outbound / group name.
    pub target: String,
    pub enabled: bool,
    /// Hours between refreshes; 0 disables automatic updates.
    pub interval_hours: u32,
    pub last_update: Option<i64>,
    pub last_attempt: Option<i64>,
    pub last_error: Option<String>,
    pub rule_count: usize,
    /// Format of the stored file after conversion.
    pub stored: Option<RuleFormat>,
}

impl Default for RuleProvider {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            source: String::new(),
            format: RuleFormat::Auto,
            target: "proxy".into(),
            enabled: true,
            interval_hours: 24,
            last_update: None,
            last_attempt: None,
            last_error: None,
            rule_count: 0,
            stored: None,
        }
    }
}

impl RuleProvider {
    pub fn is_remote(&self) -> bool {
        let s = self.source.trim();
        s.starts_with("http://") || s.starts_with("https://")
    }

    pub fn tag(&self) -> String {
        format!("rp-{}", self.id)
    }

    pub fn next_due(&self) -> Option<i64> {
        if !self.enabled || self.interval_hours == 0 {
            return None;
        }
        let Some(last) = self.last_update else {
            return Some(self.last_attempt.map(|t| t + 600).unwrap_or(0));
        };
        let regular = last + self.interval_hours as i64 * 3600;
        match (self.last_error.is_some(), self.last_attempt) {
            (true, Some(a)) if a > last => Some(regular.min(a + 1800)),
            _ => Some(regular),
        }
    }
}

/// Result of converting downloaded rule content.
pub struct Converted {
    pub format: RuleFormat,
    pub bytes: Vec<u8>,
    pub rule_count: usize,
    pub skipped: usize,
}

const SRS_MAGIC: &[u8] = b"SRS";

/// Converts downloaded content to something sing-box can load.
pub fn convert_rules(source: &str, format: RuleFormat, data: &[u8]) -> Result<Converted, String> {
    let format = match format {
        RuleFormat::Auto => detect(source, data),
        f => f,
    };
    match format {
        RuleFormat::SingBoxBinary => {
            if !data.starts_with(SRS_MAGIC) {
                return Err("not a sing-box binary rule-set".into());
            }
            Ok(Converted { format, bytes: data.to_vec(), rule_count: 0, skipped: 0 })
        }
        RuleFormat::SingBoxSource => {
            let v: Value = serde_json::from_slice(data).map_err(|e| format!("invalid JSON: {e}"))?;
            let rules = v.get("rules").and_then(Value::as_array).ok_or("no `rules` array")?;
            let count = rules.iter().map(count_items).sum();
            Ok(Converted { format, bytes: data.to_vec(), rule_count: count, skipped: 0 })
        }
        RuleFormat::Clash | RuleFormat::ClashDomain | RuleFormat::ClashIpCidr => {
            let text = String::from_utf8_lossy(data);
            let (set, skipped) = clash_to_source(&text, format);
            if set.is_empty() {
                return Err(if skipped > 0 {
                    format!("none of the {skipped} rules can be used by sing-box")
                } else {
                    "no rules found".into()
                });
            }
            let count = set.count();
            Ok(Converted {
                format: RuleFormat::SingBoxSource,
                bytes: serde_json::to_vec(&set.to_json()).unwrap(),
                rule_count: count,
                skipped,
            })
        }
        RuleFormat::Auto => unreachable!(),
    }
}

fn count_items(rule: &Value) -> usize {
    rule.as_object()
        .map(|o| o.values().map(|v| v.as_array().map_or(1, Vec::len)).sum())
        .unwrap_or(0)
}

fn detect(source: &str, data: &[u8]) -> RuleFormat {
    if data.starts_with(SRS_MAGIC) {
        return RuleFormat::SingBoxBinary;
    }
    let lower = source.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or(&lower);
    let text = String::from_utf8_lossy(&data[..data.len().min(4096)]);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('{') {
        return RuleFormat::SingBoxSource;
    }
    if path.contains("ipcidr") || path.contains("/ip/") {
        if !trimmed.lines().any(|l| l.contains(',')) {
            return RuleFormat::ClashIpCidr;
        }
    }
    // A list without any `TYPE,` prefix is a plain domain or CIDR list.
    let entries: Vec<&str> = trimmed
        .lines()
        .map(|l| l.trim().trim_start_matches("- ").trim_matches(['\'', '"']))
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//") && *l != "payload:")
        .take(50)
        .collect();
    if !entries.is_empty() && entries.iter().all(|l| !l.contains(',')) {
        if entries.iter().all(|l| l.contains('/') && l.parse::<IpNet>().is_ok()) {
            return RuleFormat::ClashIpCidr;
        }
        return RuleFormat::ClashDomain;
    }
    RuleFormat::Clash
}

struct IpNet;
impl std::str::FromStr for IpNet {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        let (ip, len) = s.split_once('/').ok_or(())?;
        ip.parse::<std::net::IpAddr>().map_err(|_| ())?;
        len.parse::<u8>().map_err(|_| ())?;
        Ok(IpNet)
    }
}

#[derive(Default)]
pub struct SourceSet {
    domain: Vec<String>,
    domain_suffix: Vec<String>,
    domain_keyword: Vec<String>,
    domain_regex: Vec<String>,
    ip_cidr: Vec<String>,
    source_ip_cidr: Vec<String>,
    port: Vec<u16>,
    port_range: Vec<String>,
    source_port: Vec<u16>,
    process_name: Vec<String>,
    process_path: Vec<String>,
}

impl SourceSet {
    fn is_empty(&self) -> bool {
        self.count() == 0
    }

    fn count(&self) -> usize {
        self.domain.len()
            + self.domain_suffix.len()
            + self.domain_keyword.len()
            + self.domain_regex.len()
            + self.ip_cidr.len()
            + self.source_ip_cidr.len()
            + self.port.len()
            + self.port_range.len()
            + self.source_port.len()
            + self.process_name.len()
            + self.process_path.len()
    }

    /// Fields of one headless rule are AND-ed across categories in sing-box,
    /// so each category gets its own rule (rules in a set are OR-ed).
    fn to_json(&self) -> Value {
        let mut rules = Vec::new();
        let mut domains = serde_json::Map::new();
        for (k, v) in [
            ("domain", &self.domain),
            ("domain_suffix", &self.domain_suffix),
            ("domain_keyword", &self.domain_keyword),
            ("domain_regex", &self.domain_regex),
            ("ip_cidr", &self.ip_cidr),
        ] {
            if !v.is_empty() {
                domains.insert(k.into(), json!(v));
            }
        }
        if !domains.is_empty() {
            rules.push(Value::Object(domains));
        }
        if !self.source_ip_cidr.is_empty() {
            rules.push(json!({ "source_ip_cidr": self.source_ip_cidr }));
        }
        if !self.port.is_empty() || !self.port_range.is_empty() {
            let mut r = serde_json::Map::new();
            if !self.port.is_empty() {
                r.insert("port".into(), json!(self.port));
            }
            if !self.port_range.is_empty() {
                r.insert("port_range".into(), json!(self.port_range));
            }
            rules.push(Value::Object(r));
        }
        if !self.source_port.is_empty() {
            rules.push(json!({ "source_port": self.source_port }));
        }
        if !self.process_name.is_empty() {
            rules.push(json!({ "process_name": self.process_name }));
        }
        if !self.process_path.is_empty() {
            rules.push(json!({ "process_path": self.process_path }));
        }
        json!({ "version": 2, "rules": rules })
    }

    fn add_domain_entry(&mut self, d: &str) {
        let d = d.trim().trim_end_matches('.');
        if d.is_empty() {
            return;
        }
        if let Some(rest) = d.strip_prefix("+.") {
            self.domain_suffix.push(rest.into());
        } else if let Some(rest) = d.strip_prefix("*.") {
            self.domain_suffix.push(format!(".{rest}"));
        } else if let Some(rest) = d.strip_prefix('.') {
            self.domain_suffix.push(rest.into());
        } else if d.contains('*') {
            self.domain_regex.push(wildcard_regex(d));
        } else {
            self.domain.push(d.into());
        }
    }
}

fn wildcard_regex(pattern: &str) -> String {
    let mut re = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => re.push_str("[^.]*"),
            '?' => re.push('.'),
            c if "\\.+()[]{}^$|".contains(c) => {
                re.push('\\');
                re.push(c);
            }
            c => re.push(c),
        }
    }
    re.push('$');
    re
}

/// Converts Clash rules; returns the set and how many entries were skipped.
pub fn clash_to_source(text: &str, format: RuleFormat) -> (SourceSet, usize) {
    let mut set = SourceSet::default();
    let mut skipped = 0;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        let line = line.strip_prefix("- ").unwrap_or(line).trim().trim_matches(['\'', '"']);
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") || line.ends_with(':') {
            continue;
        }
        match format {
            RuleFormat::ClashDomain if !line.contains(',') => {
                set.add_domain_entry(line);
                continue;
            }
            RuleFormat::ClashIpCidr if !line.contains(',') => {
                set.ip_cidr.push(line.into());
                continue;
            }
            _ => {}
        }
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() < 2 {
            // A bare entry in a classical list.
            if line.parse::<IpNet>().is_ok() {
                set.ip_cidr.push(line.into());
            } else if line.contains('.') {
                set.add_domain_entry(line);
            } else {
                skipped += 1;
            }
            continue;
        }
        let value = parts[1].to_string();
        match parts[0].to_ascii_uppercase().as_str() {
            "DOMAIN" => set.domain.push(value),
            "DOMAIN-SUFFIX" => set.domain_suffix.push(value),
            "DOMAIN-KEYWORD" => set.domain_keyword.push(value),
            "DOMAIN-REGEX" => set.domain_regex.push(value),
            "DOMAIN-WILDCARD" => set.domain_regex.push(wildcard_regex(&value)),
            "IP-CIDR" | "IP-CIDR6" | "IP6-CIDR" => set.ip_cidr.push(value),
            "SRC-IP-CIDR" => set.source_ip_cidr.push(value),
            "DST-PORT" | "DEST-PORT" => match value.split_once('-') {
                Some((a, b)) => set.port_range.push(format!("{}:{}", a.trim(), b.trim())),
                None => match value.parse() {
                    Ok(p) => set.port.push(p),
                    Err(_) => skipped += 1,
                },
            },
            "SRC-PORT" => match value.parse() {
                Ok(p) => set.source_port.push(p),
                Err(_) => skipped += 1,
            },
            "PROCESS-NAME" => set.process_name.push(value),
            "PROCESS-PATH" => set.process_path.push(value),
            // GEOIP / GEOSITE / IP-ASN / USER-AGENT / logical rules have no
            // equivalent inside a sing-box rule-set.
            _ => skipped += 1,
        }
    }
    (set, skipped)
}

/// One `ruleset=` line of an ACL4SSR / subconverter config.
#[derive(Debug, Clone, PartialEq)]
pub struct IniRuleSet {
    pub group: String,
    pub target: String,
    pub name: String,
    pub source: String,
    pub format: RuleFormat,
}

/// Parses an ACL4SSR / subconverter `.ini` config. `geosite_url` / `geoip_url`
/// templates turn `[]GEOSITE,x` / `[]GEOIP,x` into sing-box rule-sets.
pub fn parse_acl_ini(text: &str, geosite_url: &str, geoip_url: &str) -> Vec<IniRuleSet> {
    // Group name -> first member, to learn which groups mean DIRECT / REJECT.
    let mut group_first: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(def) = line.strip_prefix("custom_proxy_group=") {
            let mut parts = def.split('`');
            let name = parts.next().unwrap_or("").trim().to_string();
            let _kind = parts.next();
            let first = parts.next().unwrap_or("").trim().to_string();
            group_first.push((name, first));
        }
    }
    let target_of = |group: &str| -> String {
        let first = group_first
            .iter()
            .find(|(n, _)| n == group)
            .map(|(_, f)| f.to_ascii_uppercase());
        match first.as_deref() {
            Some("[]DIRECT") => "direct".into(),
            Some("[]REJECT") | Some("[]REJECT-TINYGIF") => "reject".into(),
            Some(_) => "proxy".into(),
            None => {
                let g = group.to_lowercase();
                if g.contains("直连") || g.contains("直連") || g.contains("direct") {
                    "direct".into()
                } else if ["拦截", "攔截", "广告", "廣告", "净化", "淨化", "reject", "block", "adblock"]
                    .iter()
                    .any(|k| g.contains(k))
                {
                    "reject".into()
                } else {
                    "proxy".into()
                }
            }
        }
    };
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        let Some(def) = line.strip_prefix("ruleset=") else { continue };
        let Some((group, rest)) = def.split_once(',') else { continue };
        let group = group.trim().to_string();
        let rest = rest.trim();
        let target = target_of(&group);
        if let Some(inline) = rest.strip_prefix("[]") {
            let (kind, value) = inline.split_once(',').unwrap_or((inline, ""));
            let value = value.split(',').next().unwrap_or("").trim().to_ascii_lowercase();
            let (name, url) = match kind.trim().to_ascii_uppercase().as_str() {
                "GEOIP" if !value.is_empty() => (format!("geoip-{value}"), geoip_url.replace("{name}", &value)),
                "GEOSITE" if !value.is_empty() => (format!("geosite-{value}"), geosite_url.replace("{name}", &value)),
                // FINAL / MATCH is the default route; other inline rules are skipped.
                _ => continue,
            };
            out.push(IniRuleSet {
                name: format!("{group} · {name}"),
                group,
                target,
                source: url,
                format: RuleFormat::SingBoxBinary,
            });
            continue;
        }
        // subconverter prefixes: clash-domain:, clash-ipcidr:, clash-classic:, surge:, quanx:
        let (format, url) = match rest.split_once(':') {
            Some((p, u)) if !p.contains('/') && !p.eq_ignore_ascii_case("https") && !p.eq_ignore_ascii_case("http") => {
                let f = match p.to_ascii_lowercase().as_str() {
                    "clash-domain" => RuleFormat::ClashDomain,
                    "clash-ipcidr" => RuleFormat::ClashIpCidr,
                    _ => RuleFormat::Clash,
                };
                (f, u.trim())
            }
            _ => (RuleFormat::Auto, rest),
        };
        let url = url.split(',').next().unwrap_or(url).trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        let stem = url
            .rsplit('/')
            .next()
            .unwrap_or(url)
            .split(['.', '?'])
            .next()
            .unwrap_or(url)
            .to_string();
        out.push(IniRuleSet {
            name: format!("{group} · {stem}"),
            group,
            target,
            source: url.to_string(),
            format,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acl_list() {
        let text = "# comment\nDOMAIN-SUFFIX,google.com\nDOMAIN,www.x.com\nDOMAIN-KEYWORD,goog\nIP-CIDR,1.1.1.0/24,no-resolve\nIP-CIDR6,2001:db8::/32\nGEOIP,CN\nPROCESS-NAME,steam.exe\nDST-PORT,1000-2000\nUSER-AGENT,*Foo*\n";
        let c = convert_rules("https://x/Google.list", RuleFormat::Auto, text.as_bytes()).unwrap();
        assert_eq!(c.format, RuleFormat::SingBoxSource);
        assert_eq!(c.rule_count, 7);
        assert_eq!(c.skipped, 2);
        let v: Value = serde_json::from_slice(&c.bytes).unwrap();
        assert_eq!(v["rules"][0]["domain_suffix"][0], "google.com");
        assert_eq!(v["rules"][0]["ip_cidr"].as_array().unwrap().len(), 2);
        assert_eq!(v["rules"][1]["port_range"][0], "1000:2000");
        assert_eq!(v["rules"][2]["process_name"][0], "steam.exe");
    }

    #[test]
    fn clash_yaml_providers() {
        let domain = "payload:\n  - '+.google.com'\n  - 'www.apple.com'\n  - '*.cdn.com'\n";
        let c = convert_rules("https://x/proxy.yaml", RuleFormat::Auto, domain.as_bytes()).unwrap();
        let v: Value = serde_json::from_slice(&c.bytes).unwrap();
        assert_eq!(v["rules"][0]["domain_suffix"], json!(["google.com", ".cdn.com"]));
        assert_eq!(v["rules"][0]["domain"], json!(["www.apple.com"]));
        let ip = "payload:\n  - '10.0.0.0/8'\n  - '2001:db8::/32'\n";
        let c = convert_rules("https://x/lan.yaml", RuleFormat::Auto, ip.as_bytes()).unwrap();
        let v: Value = serde_json::from_slice(&c.bytes).unwrap();
        assert_eq!(v["rules"][0]["ip_cidr"].as_array().unwrap().len(), 2);
        let classic = "payload:\n  - DOMAIN-SUFFIX,x.com\n  - IP-CIDR,1.0.0.0/8\n";
        let c = convert_rules("https://x/c.yaml", RuleFormat::Auto, classic.as_bytes()).unwrap();
        assert_eq!(c.rule_count, 2);
    }

    #[test]
    fn srs_passthrough() {
        let c = convert_rules("x.srs", RuleFormat::Auto, b"SRS\x01data").unwrap();
        assert_eq!(c.format, RuleFormat::SingBoxBinary);
        assert!(convert_rules("x.srs", RuleFormat::SingBoxBinary, b"nope").is_err());
    }

    #[test]
    fn acl_ini() {
        let ini = r#"
[custom]
ruleset=🎯 全球直连,https://raw.githubusercontent.com/ACL4SSR/ACL4SSR/master/Clash/LocalAreaNetwork.list
ruleset=🛑 广告拦截,https://raw.githubusercontent.com/ACL4SSR/ACL4SSR/master/Clash/BanAD.list
ruleset=📲 电报消息,clash-ipcidr:https://example.com/telegram.yaml
ruleset=🎯 全球直连,[]GEOIP,CN
ruleset=🐟 漏网之鱼,[]FINAL
custom_proxy_group=🚀 节点选择`select`[]♻️ 自动选择`[]DIRECT`.*
custom_proxy_group=🎯 全球直连`select`[]DIRECT`[]🚀 节点选择
custom_proxy_group=🛑 广告拦截`select`[]REJECT`[]DIRECT
custom_proxy_group=📲 电报消息`select`[]🚀 节点选择`[]DIRECT
"#;
        let sets = parse_acl_ini(ini, "https://g/geosite-{name}.srs", "https://g/geoip-{name}.srs");
        assert_eq!(sets.len(), 4);
        assert_eq!(sets[0].target, "direct");
        assert_eq!(sets[0].name, "🎯 全球直连 · LocalAreaNetwork");
        assert_eq!(sets[1].target, "reject");
        assert_eq!(sets[2].target, "proxy");
        assert_eq!(sets[2].format, RuleFormat::ClashIpCidr);
        assert_eq!(sets[3].source, "https://g/geoip-cn.srs");
        assert_eq!(sets[3].target, "direct");
    }
}
