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
    /// Matches go through a node group of their own, shown on the Proxies page.
    pub own_group: bool,
    /// How that group picks its node; `None` leaves it to the user.
    pub pick: Option<crate::model::AutoStrategy>,
    /// Country code the group's picks stay in; empty for any.
    pub region: String,
    /// Curated service preset id (e.g. `"ai"`, `"youtube"`), if added from the library / profile.
    pub preset: Option<String>,
    /// Short human-friendly description shown instead of raw URLs.
    pub description: String,
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
            own_group: false,
            pick: None,
            region: String::new(),
            preset: None,
            description: String::new(),
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

    /// Whether matches can go through a group of their own; rejected ones go nowhere.
    pub fn can_own_group(&self) -> bool {
        !matches!(self.target.to_ascii_lowercase().as_str(), "reject" | "block")
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

/// A user-defined group inside Custom Rules that owns the routing target for
/// all rules inside it, and can expose its own node group on the Proxies page.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CustomRuleGroup {
    pub id: String,
    pub name: String,
    /// `proxy`, `direct`, `reject`, or an outbound / group name.
    pub target: String,
    pub enabled: bool,
    /// Matches go through a node group of their own, shown on the Proxies page.
    pub own_group: bool,
    /// How that group picks its node; `None` leaves it to the user (Manual).
    pub pick: Option<crate::model::AutoStrategy>,
    /// Country code the group's picks stay in; empty for any.
    pub region: String,
}

impl Default for CustomRuleGroup {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            target: "proxy".into(),
            enabled: true,
            own_group: true,
            pick: None,
            region: String::new(),
        }
    }
}

impl CustomRuleGroup {
    /// Whether matches can go through a group of their own; rejected ones go nowhere.
    pub fn can_own_group(&self) -> bool {
        !matches!(self.target.to_ascii_lowercase().as_str(), "reject" | "block")
    }

    /// Target string displayed and written to `CustomRule.list` for member rules.
    pub fn display_target(&self) -> &str {
        if self.own_group && self.can_own_group() && !self.name.trim().is_empty() {
            self.name.trim()
        } else {
            self.target.as_str()
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
/// The domain a site is registered under (`www.bbc.co.uk` -> `bbc.co.uk`), the
/// natural match for "this site and its subdomains". `None` for IP addresses
/// and single-label names.
pub fn base_domain(host: &str) -> Option<String> {
    const SECOND_LEVEL: [&str; 8] = ["com", "net", "org", "gov", "edu", "ac", "co", "or"];
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty()) {
        return None;
    }
    // Two-letter countries with a second level: example.com.cn, example.co.jp.
    let n = labels.len();
    let keep = if n >= 3 && labels[n - 1].len() == 2 && SECOND_LEVEL.contains(&labels[n - 2]) { 3 } else { 2 };
    Some(labels[n - keep..].join("."))
}

/// Prefix used in `CustomRule.list` for a temporarily disabled rule.
/// Standard `#` comment so older builds and external editors treat it cleanly.
pub const DISABLED_PREFIX: &str = "#!disabled ";

/// Section marker used in `CustomRule.list` to place following rules into a named
/// custom rule group. Standard `#` comment so external editors and older builds
/// remain 100% compatible.
pub const GROUP_PREFIX: &str = "#!group ";

fn rule_dedup_key(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed.strip_prefix(DISABLED_PREFIX).map(str::trim).unwrap_or(trimmed);
    if body.is_empty() || body.starts_with('#') || body.starts_with("//") {
        return None;
    }
    let mut parts = body.splitn(3, ',').map(str::trim);
    let (kind, value) = (parts.next()?, parts.next()?);
    Some(format!("{},{}", kind.to_ascii_lowercase(), value.to_ascii_lowercase()))
}

fn parse_group_header(line: &str) -> Option<&str> {
    line.trim().strip_prefix(GROUP_PREFIX).map(str::trim)
}

fn find_first_group_header(lines: &[String]) -> Option<usize> {
    lines.iter().position(|l| parse_group_header(l).is_some())
}

fn find_group_section(lines: &[String], group_name: &str) -> Option<(usize, usize)> {
    let wanted = group_name.trim();
    if wanted.is_empty() {
        return None;
    }
    let start = lines.iter().position(|l| {
        parse_group_header(l).is_some_and(|g| g.eq_ignore_ascii_case(wanted))
    })?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| parse_group_header(l).is_some())
        .map(|off| start + 1 + off)
        .unwrap_or(lines.len());
    Some((start, end))
}

fn join_custom_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

fn replace_rule_line_target(raw: &str, new_target: &str) -> String {
    let trimmed = raw.trim();
    let (disabled, body) = match trimmed.strip_prefix(DISABLED_PREFIX) {
        Some(rest) => (true, rest.trim()),
        None => (false, trimmed),
    };
    let parts: Vec<&str> = body.splitn(3, ',').map(str::trim).collect();
    if parts.len() != 3 {
        return raw.to_string();
    }
    let rebuilt = format!("{},{},{}", parts[0], parts[1], new_target.trim());
    if disabled { format!("{DISABLED_PREFIX}{rebuilt}") } else { rebuilt }
}

/// Removes any `#!group ...` section that contains no rule lines and cleans up
/// adjacent redundant blank lines.
fn prune_empty_group_headers(lines: &mut Vec<String>) {
    let mut i = 0;
    while i < lines.len() {
        if parse_group_header(&lines[i]).is_some() {
            let end = lines[i + 1..]
                .iter()
                .position(|l| parse_group_header(l).is_some())
                .map(|off| i + 1 + off)
                .unwrap_or(lines.len());
            let has_content = lines[i + 1..end].iter().any(|l| !l.trim().is_empty());
            if !has_content {
                lines.drain(i..end);
                // Remove leading blank line if it's now trailing or duplicated.
                if i > 0 && lines[i - 1].trim().is_empty() && (i >= lines.len() || lines[i].trim().is_empty()) {
                    lines.remove(i - 1);
                    i = i.saturating_sub(1);
                }
                continue;
            }
        }
        i += 1;
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
}

/// Adds a `TYPE,VALUE,TARGET` line to the ungrouped (top-priority) section of
/// custom rules. An ungrouped line for the same type and value is replaced (and
/// re-enabled) where it stands; otherwise it is inserted at the end of the
/// ungrouped section (ahead of any `#!group ` sections). Returns the updated
/// text, or `None` if the exact enabled rule is already there.
pub fn merge_custom_rule(text: &str, rule: &str) -> Option<String> {
    let wanted = rule_dedup_key(rule)?;
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let ungrouped_end = find_first_group_header(&lines).unwrap_or(lines.len());
    match lines[..ungrouped_end].iter().position(|l| rule_dedup_key(l).as_deref() == Some(wanted.as_str())) {
        Some(i) if lines[i].trim().eq_ignore_ascii_case(rule.trim()) => return None,
        Some(i) => lines[i] = rule.trim().to_string(),
        None => {
            let mut insert_at = ungrouped_end;
            while insert_at > 0 && lines[insert_at - 1].trim().is_empty() && ungrouped_end < lines.len() {
                insert_at -= 1;
            }
            lines.insert(insert_at, rule.trim().to_string());
        }
    }
    Some(join_custom_lines(&lines))
}

/// Adds a `TYPE,VALUE,TARGET` line into either the ungrouped section (`None`)
/// or the named `#!group <name>` section (`Some(name)`).
pub fn add_custom_rule_in_group(text: &str, rule: &str, group: Option<&str>) -> Option<String> {
    let Some(group_name) = group.map(str::trim).filter(|g| !g.is_empty()) else {
        return merge_custom_rule(text, rule);
    };
    let wanted = rule_dedup_key(rule)?;
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    if let Some((start, end)) = find_group_section(&lines, group_name) {
        if let Some(rel) = lines[start + 1..end]
            .iter()
            .position(|l| rule_dedup_key(l).as_deref() == Some(wanted.as_str()))
        {
            let i = start + 1 + rel;
            if lines[i].trim().eq_ignore_ascii_case(rule.trim()) {
                return None;
            }
            lines[i] = rule.trim().to_string();
        } else {
            let mut insert_at = end;
            while insert_at > start + 1 && lines[insert_at - 1].trim().is_empty() {
                insert_at -= 1;
            }
            lines.insert(insert_at, rule.trim().to_string());
        }
    } else {
        if !lines.is_empty() && !lines.last().unwrap().trim().is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("{GROUP_PREFIX}{group_name}"));
        lines.push(rule.trim().to_string());
    }
    Some(join_custom_lines(&lines))
}

// ------------------------------------------------ curated presets & profiles

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceCategory {
    AiDev,
    Streaming,
    SocialGaming,
    DomesticPlatforms,
    General,
}

impl ServiceCategory {
    pub const ALL: [Self; 5] = [
        Self::AiDev,
        Self::Streaming,
        Self::SocialGaming,
        Self::DomesticPlatforms,
        Self::General,
    ];
}

/// One built-in service in the curated library, backed by sing-box rule-sets.
#[derive(Debug, Clone, Copy)]
pub struct ServicePreset {
    pub id: &'static str,
    pub category: ServiceCategory,
    /// Display name used when no translation overrides it (also the default group tag).
    pub name: &'static str,
    /// `geosite-{tag}` name substituted into `settings.geosite_url`.
    pub geosite: &'static str,
    /// Default target: `"proxy"`, `"direct"`, `"reject"`.
    pub default_target: &'static str,
    /// Whether the Daily/Streaming profiles give it an independent node group.
    pub default_own_group: bool,
    /// Representative domains used by the offline rule tester when the `.srs` is binary.
    pub sample_domains: &'static [&'static str],
}

pub const SERVICE_PRESETS: &[ServicePreset] = &[
    ServicePreset {
        id: "ai",
        category: ServiceCategory::AiDev,
        name: "AI Services",
        geosite: "category-ai-!cn",
        default_target: "proxy",
        default_own_group: true,
        sample_domains: &[
            "openai.com",
            "chatgpt.com",
            "oaistatic.com",
            "claude.ai",
            "anthropic.com",
            "gemini.google.com",
            "aistudio.google.com",
            "perplexity.ai",
            "grok.com",
            "x.ai",
            "cursor.sh",
            "cursor.com",
            "poe.com",
            "copilot.microsoft.com",
        ],
    },
    ServicePreset {
        id: "github",
        category: ServiceCategory::AiDev,
        name: "GitHub",
        geosite: "github",
        default_target: "proxy",
        default_own_group: false,
        sample_domains: &["github.com", "githubusercontent.com", "github.io", "githubassets.com"],
    },
    ServicePreset {
        id: "google",
        category: ServiceCategory::AiDev,
        name: "Google",
        geosite: "google",
        default_target: "proxy",
        default_own_group: false,
        sample_domains: &["google.com", "googleapis.com", "gstatic.com", "gmail.com", "googleusercontent.com"],
    },
    ServicePreset {
        id: "youtube",
        category: ServiceCategory::Streaming,
        name: "YouTube",
        geosite: "youtube",
        default_target: "proxy",
        default_own_group: true,
        sample_domains: &["youtube.com", "youtu.be", "ytimg.com", "googlevideo.com"],
    },
    ServicePreset {
        id: "netflix",
        category: ServiceCategory::Streaming,
        name: "Netflix",
        geosite: "netflix",
        default_target: "proxy",
        default_own_group: true,
        sample_domains: &["netflix.com", "nflxvideo.net", "nflximg.net", "fast.com"],
    },
    ServicePreset {
        id: "disney",
        category: ServiceCategory::Streaming,
        name: "Disney+",
        geosite: "disney",
        default_target: "proxy",
        default_own_group: true,
        sample_domains: &["disneyplus.com", "disney-plus.net", "bamgrid.com"],
    },
    ServicePreset {
        id: "spotify",
        category: ServiceCategory::Streaming,
        name: "Spotify",
        geosite: "spotify",
        default_target: "proxy",
        default_own_group: false,
        sample_domains: &["spotify.com", "scdn.co", "spoti.fi"],
    },
    ServicePreset {
        id: "tiktok",
        category: ServiceCategory::Streaming,
        name: "TikTok",
        geosite: "tiktok",
        default_target: "proxy",
        default_own_group: true,
        sample_domains: &["tiktok.com", "tiktokv.com", "tiktokcdn.com"],
    },
    ServicePreset {
        id: "telegram",
        category: ServiceCategory::SocialGaming,
        name: "Telegram",
        geosite: "telegram",
        default_target: "proxy",
        default_own_group: false,
        sample_domains: &["telegram.org", "t.me", "tdesktop.com", "telegra.ph"],
    },
    ServicePreset {
        id: "steam",
        category: ServiceCategory::SocialGaming,
        name: "Steam",
        geosite: "steam",
        default_target: "proxy",
        default_own_group: false,
        sample_domains: &["steampowered.com", "steamcommunity.com", "steamstatic.com"],
    },
    ServicePreset {
        id: "bilibili",
        category: ServiceCategory::DomesticPlatforms,
        name: "Bilibili",
        geosite: "bilibili",
        default_target: "direct",
        default_own_group: false,
        sample_domains: &["bilibili.com", "bilivideo.com", "hdslb.com", "b23.tv"],
    },
    ServicePreset {
        id: "apple",
        category: ServiceCategory::DomesticPlatforms,
        name: "Apple CN",
        geosite: "apple-cn",
        default_target: "direct",
        default_own_group: false,
        sample_domains: &["apple.com.cn", "icloud.com.cn", "cdn-apple.com"],
    },
    ServicePreset {
        id: "microsoft",
        category: ServiceCategory::DomesticPlatforms,
        name: "Microsoft CN",
        geosite: "microsoft@cn",
        default_target: "direct",
        default_own_group: false,
        sample_domains: &["microsoft.com", "live.com", "office.com", "windowsupdate.com"],
    },
    ServicePreset {
        id: "overseas",
        category: ServiceCategory::General,
        name: "Overseas Sites",
        geosite: "geolocation-!cn",
        default_target: "proxy",
        default_own_group: false,
        sample_domains: &["x.com", "twitter.com", "reddit.com", "wikipedia.org", "discord.com", "instagram.com"],
    },
];

pub fn service_preset(id: &str) -> Option<&'static ServicePreset> {
    SERVICE_PRESETS.iter().find(|p| p.id == id)
}

impl ServicePreset {
    pub fn to_provider(&self, geosite_url: &str, own_group: Option<bool>) -> RuleProvider {
        RuleProvider {
            id: crate::model::new_id(),
            name: self.name.to_string(),
            source: geosite_url.replace("{name}", self.geosite),
            format: RuleFormat::SingBoxBinary,
            target: self.default_target.to_string(),
            enabled: true,
            interval_hours: 24,
            own_group: own_group.unwrap_or(self.default_own_group),
            preset: Some(self.id.to_string()),
            description: format!("geosite-{}", self.geosite),
            ..Default::default()
        }
    }
}

/// One-click routing profile presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleProfile {
    Daily,
    MinimalBypassCn,
    StreamingGaming,
    DirectFirst,
}

impl RuleProfile {
    pub const ALL: [Self; 4] = [
        Self::Daily,
        Self::MinimalBypassCn,
        Self::StreamingGaming,
        Self::DirectFirst,
    ];

    pub fn spec(self) -> RuleProfileSpec {
        match self {
            Self::Daily => RuleProfileSpec {
                bypass_cn: true,
                block_ads: true,
                final_direct: false,
                services: &[("ai", true), ("youtube", true), ("telegram", false), ("github", false)],
            },
            Self::MinimalBypassCn => RuleProfileSpec {
                bypass_cn: true,
                block_ads: false,
                final_direct: false,
                services: &[],
            },
            Self::StreamingGaming => RuleProfileSpec {
                bypass_cn: true,
                block_ads: true,
                final_direct: false,
                services: &[
                    ("ai", true),
                    ("youtube", true),
                    ("netflix", true),
                    ("disney", true),
                    ("tiktok", true),
                    ("telegram", false),
                    ("steam", false),
                    ("bilibili", false),
                ],
            },
            Self::DirectFirst => RuleProfileSpec {
                bypass_cn: false,
                block_ads: true,
                final_direct: true,
                services: &[
                    ("ai", true),
                    ("youtube", true),
                    ("telegram", false),
                    ("github", false),
                    ("google", false),
                    ("overseas", false),
                ],
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RuleProfileSpec {
    pub bypass_cn: bool,
    pub block_ads: bool,
    pub final_direct: bool,
    /// `(service_preset_id, own_group)` in priority order.
    pub services: &'static [(&'static str, bool)],
}

// ------------------------------------------------ structured custom rules

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomRuleKind {
    DomainSuffix,
    Domain,
    DomainKeyword,
    ProcessName,
    IpCidr,
    DstPort,
    GeoSite,
    GeoIp,
    RuleSet,
    DomainRegex,
    ProcessPath,
    SrcIpCidr,
    Network,
}

impl CustomRuleKind {
    /// Common types shown first in the UI dropdown.
    pub const ALL: [Self; 13] = [
        Self::DomainSuffix,
        Self::Domain,
        Self::DomainKeyword,
        Self::ProcessName,
        Self::IpCidr,
        Self::DstPort,
        Self::GeoSite,
        Self::GeoIp,
        Self::RuleSet,
        Self::DomainRegex,
        Self::ProcessPath,
        Self::SrcIpCidr,
        Self::Network,
    ];

    /// Primary everyday types for quick-add.
    pub const QUICK: [Self; 8] = [
        Self::DomainSuffix,
        Self::Domain,
        Self::DomainKeyword,
        Self::ProcessName,
        Self::IpCidr,
        Self::DstPort,
        Self::GeoSite,
        Self::GeoIp,
    ];

    pub fn token(self) -> &'static str {
        match self {
            Self::DomainSuffix => "DOMAIN-SUFFIX",
            Self::Domain => "DOMAIN",
            Self::DomainKeyword => "DOMAIN-KEYWORD",
            Self::ProcessName => "PROCESS-NAME",
            Self::IpCidr => "IP-CIDR",
            Self::DstPort => "DST-PORT",
            Self::GeoSite => "GEOSITE",
            Self::GeoIp => "GEOIP",
            Self::RuleSet => "RULE-SET",
            Self::DomainRegex => "DOMAIN-REGEX",
            Self::ProcessPath => "PROCESS-PATH",
            Self::SrcIpCidr => "SRC-IP-CIDR",
            Self::Network => "NETWORK",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "domain-suffix" => Some(Self::DomainSuffix),
            "domain" => Some(Self::Domain),
            "domain-keyword" => Some(Self::DomainKeyword),
            "domain-regex" => Some(Self::DomainRegex),
            "ip-cidr" | "ip-cidr6" | "ip6-cidr" => Some(Self::IpCidr),
            "src-ip-cidr" | "source-ip-cidr" => Some(Self::SrcIpCidr),
            "process-name" => Some(Self::ProcessName),
            "process-path" => Some(Self::ProcessPath),
            "dst-port" | "dest-port" | "port" => Some(Self::DstPort),
            "network" => Some(Self::Network),
            "geosite" => Some(Self::GeoSite),
            "geoip" => Some(Self::GeoIp),
            "rule-set" => Some(Self::RuleSet),
            _ => None,
        }
    }
}

/// One parsed rule line from `CustomRule.list`, with its exact line index for lossless edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomRule {
    pub line_index: usize,
    pub kind: Option<CustomRuleKind>,
    pub kind_raw: String,
    pub value: String,
    pub target: String,
    pub enabled: bool,
    pub raw: String,
    pub error: Option<String>,
    /// Name of the `#!group <name>` section this rule sits under, or `None` if ungrouped.
    pub group: Option<String>,
}

/// Cleans user input for a custom rule value (e.g. pasting a full URL or executable path).
pub fn normalize_custom_value(kind: CustomRuleKind, raw: &str) -> String {
    let s = raw.trim().trim_matches(['"', '\'']);
    if s.is_empty() {
        return String::new();
    }
    match kind {
        CustomRuleKind::DomainSuffix | CustomRuleKind::Domain => {
            let had_scheme = s.contains("://");
            let host = strip_url_to_host(s);
            let host = host.trim_start_matches("*.").trim_start_matches("+.").trim_start_matches('.').trim_end_matches('.');
            if kind == CustomRuleKind::DomainSuffix && had_scheme {
                base_domain(host).unwrap_or_else(|| host.to_ascii_lowercase())
            } else {
                host.to_ascii_lowercase()
            }
        }
        CustomRuleKind::IpCidr | CustomRuleKind::SrcIpCidr => {
            if let Ok(ip) = s.parse::<std::net::IpAddr>() {
                let bits = if ip.is_ipv4() { 32 } else { 128 };
                format!("{ip}/{bits}")
            } else {
                s.to_string()
            }
        }
        CustomRuleKind::ProcessName => {
            // If a full file path was pasted or picked, extract its process name.
            let name = s.rsplit(['/', '\\']).next().unwrap_or(s);
            // macOS .app bundle picked from Finder: strip `.app` suffix.
            name.strip_suffix(".app").unwrap_or(name).to_string()
        }
        CustomRuleKind::GeoSite | CustomRuleKind::GeoIp | CustomRuleKind::Network => s.to_ascii_lowercase(),
        _ => s.to_string(),
    }
}

fn strip_url_to_host(s: &str) -> &str {
    let rest = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    if let Some(bracketed) = hostport.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or(hostport)
    } else if hostport.matches(':').count() == 1 {
        hostport.split(':').next().unwrap_or(hostport)
    } else {
        hostport
    }
}

/// Validates a custom rule value; returns `Ok(())` or a concise reason string.
pub fn validate_custom_value(kind: CustomRuleKind, value: &str) -> Result<(), String> {
    let v = value.trim();
    if v.is_empty() {
        return Err("empty value".into());
    }
    if v.contains(',') {
        return Err("cannot contain commas".into());
    }
    let parts: Vec<&str> = v.split('|').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err("empty value".into());
    }
    match kind {
        CustomRuleKind::IpCidr | CustomRuleKind::SrcIpCidr => {
            for p in parts {
                if p.parse::<IpNet>().is_err() {
                    return Err(format!("invalid CIDR: {p}"));
                }
            }
        }
        CustomRuleKind::DstPort => {
            for p in parts {
                if p.parse::<u16>().is_err() {
                    return Err(format!("invalid port: {p}"));
                }
            }
        }
        CustomRuleKind::Network => {
            for p in parts {
                if !matches!(p.to_ascii_lowercase().as_str(), "tcp" | "udp") {
                    return Err(format!("expected tcp or udp: {p}"));
                }
            }
        }
        CustomRuleKind::DomainRegex => {
            for p in parts {
                if let Err(e) = regex::Regex::new(p) {
                    return Err(format!("invalid regex: {e}"));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn format_custom_rule_line(kind: CustomRuleKind, value: &str, target: &str, enabled: bool) -> String {
    let body = format!("{},{},{}", kind.token(), value.trim(), target.trim());
    if enabled { body } else { format!("{DISABLED_PREFIX}{body}") }
}

/// Parses `CustomRule.list` into structured rows, keeping disabled rules (`#!disabled …`),
/// tracking `#!group <name>` sections, and surfacing per-line syntax errors while
/// ignoring ordinary comments & blank lines.
pub fn parse_custom_rule_list(text: &str) -> Vec<CustomRule> {
    let mut out = Vec::new();
    let mut current_group: Option<String> = None;
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(group_name) = parse_group_header(trimmed) {
            current_group = (!group_name.is_empty()).then(|| group_name.to_string());
            continue;
        }
        let (enabled, body) = if let Some(rest) = trimmed.strip_prefix(DISABLED_PREFIX) {
            (false, rest.trim())
        } else if trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        } else {
            (true, trimmed)
        };
        let parts: Vec<&str> = body.splitn(3, ',').map(str::trim).collect();
        if parts.len() != 3 {
            out.push(CustomRule {
                line_index: idx,
                kind: None,
                kind_raw: parts.first().copied().unwrap_or("").to_string(),
                value: parts.get(1).copied().unwrap_or(body).to_string(),
                target: parts.get(2).copied().unwrap_or("").to_string(),
                enabled,
                raw: raw.to_string(),
                error: Some("expected TYPE,VALUE,TARGET".into()),
                group: current_group.clone(),
            });
            continue;
        }
        let kind = CustomRuleKind::parse(parts[0]);
        let error = match kind {
            None => Some(format!("unknown type `{}`", parts[0])),
            Some(k) => validate_custom_value(k, parts[1]).err().or_else(|| {
                parts[2].is_empty().then(|| "missing target".into())
            }),
        };
        out.push(CustomRule {
            line_index: idx,
            kind,
            kind_raw: parts[0].to_string(),
            value: parts[1].to_string(),
            target: parts[2].to_string(),
            enabled,
            raw: raw.to_string(),
            error,
            group: current_group.clone(),
        });
    }
    out
}

pub fn set_custom_rule_enabled(text: &str, line_index: usize, enabled: bool) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let line = lines.get_mut(line_index)?;
    let trimmed = line.trim();
    let body = trimmed.strip_prefix(DISABLED_PREFIX).map(str::trim).unwrap_or(trimmed).to_string();
    *line = if enabled { body } else { format!("{DISABLED_PREFIX}{body}") };
    Some(join_custom_lines(&lines))
}

pub fn update_custom_rule_line(
    text: &str,
    line_index: usize,
    kind: CustomRuleKind,
    value: &str,
    target: &str,
    enabled: bool,
) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let line = lines.get_mut(line_index)?;
    *line = format_custom_rule_line(kind, value, target, enabled);
    Some(join_custom_lines(&lines))
}

pub fn remove_custom_rule_line(text: &str, line_index: usize) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    if line_index >= lines.len() {
        return None;
    }
    lines.remove(line_index);
    prune_empty_group_headers(&mut lines);
    Some(join_custom_lines(&lines))
}

/// Moves a custom rule line up (`delta < 0`) or down (`delta > 0`) relative to
/// neighboring rule lines in the same group (or ungrouped section), preserving
/// comments in place.
pub fn move_custom_rule_line(text: &str, line_index: usize, delta: isize) -> Option<String> {
    let parsed = parse_custom_rule_list(text);
    let me = parsed.iter().find(|r| r.line_index == line_index)?;
    let peers: Vec<&CustomRule> = parsed.iter().filter(|r| r.group == me.group).collect();
    let pos = peers.iter().position(|r| r.line_index == line_index)?;
    let target_pos = (pos as isize + delta).clamp(0, peers.len() as isize - 1) as usize;
    if pos == target_pos {
        return None;
    }
    let other_line = peers[target_pos].line_index;
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    lines.swap(line_index, other_line);
    Some(join_custom_lines(&lines))
}

/// Moves a custom rule line into `target_group` (`None` = Ungrouped, `Some(name)` = `#!group name`),
/// optionally updating its target column to stay consistent with the destination group.
pub fn move_custom_rule_to_group(
    text: &str,
    line_index: usize,
    target_group: Option<&str>,
    new_target: Option<&str>,
) -> Option<String> {
    let parsed = parse_custom_rule_list(text);
    let rule = parsed.iter().find(|r| r.line_index == line_index)?;
    let target_group = target_group.map(str::trim).filter(|g| !g.is_empty());
    let same_group = match (rule.group.as_deref(), target_group) {
        (None, None) => true,
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    };
    if same_group {
        return None;
    }
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    if line_index >= lines.len() {
        return None;
    }
    let raw = lines.remove(line_index);
    let moved_line = match new_target.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => replace_rule_line_target(&raw, t),
        None => raw,
    };
    prune_empty_group_headers(&mut lines);
    match target_group {
        None => {
            let ungrouped_end = find_first_group_header(&lines).unwrap_or(lines.len());
            let mut insert_at = ungrouped_end;
            while insert_at > 0 && lines[insert_at - 1].trim().is_empty() && ungrouped_end < lines.len() {
                insert_at -= 1;
            }
            lines.insert(insert_at, moved_line);
        }
        Some(gname) => {
            if let Some((start, end)) = find_group_section(&lines, gname) {
                let mut insert_at = end;
                while insert_at > start + 1 && lines[insert_at - 1].trim().is_empty() {
                    insert_at -= 1;
                }
                lines.insert(insert_at, moved_line);
            } else {
                if !lines.is_empty() && !lines.last().unwrap().trim().is_empty() {
                    lines.push(String::new());
                }
                lines.push(format!("{GROUP_PREFIX}{gname}"));
                lines.push(moved_line);
            }
        }
    }
    Some(join_custom_lines(&lines))
}

/// Renames a `#!group <old_name>` section header in `CustomRule.list` to `new_name`
/// and optionally updates the target field of its member rules.
pub fn rename_custom_rule_group_in_text(
    text: &str,
    old_name: &str,
    new_name: &str,
    new_target: Option<&str>,
) -> String {
    let old = old_name.trim();
    let new = new_name.trim();
    if old.is_empty() || new.is_empty() {
        return text.to_string();
    }
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut in_group = false;
    let mut changed = false;
    for line in &mut lines {
        if let Some(g) = parse_group_header(line) {
            if g.eq_ignore_ascii_case(old) {
                *line = format!("{GROUP_PREFIX}{new}");
                in_group = true;
                changed = true;
            } else {
                in_group = false;
            }
            continue;
        }
        if in_group {
            if let Some(t) = new_target.map(str::trim).filter(|t| !t.is_empty()) {
                let trimmed = line.trim();
                let is_rule = !trimmed.is_empty()
                    && (trimmed.starts_with(DISABLED_PREFIX)
                        || (!trimmed.starts_with('#') && !trimmed.starts_with("//")));
                if is_rule {
                    *line = replace_rule_line_target(line, t);
                    changed = true;
                }
            }
        }
    }
    if changed { join_custom_lines(&lines) } else { text.to_string() }
}

/// Updates the target column of every rule inside `#!group <group_name>` in `CustomRule.list`.
pub fn sync_custom_group_target_in_text(text: &str, group_name: &str, new_target: &str) -> String {
    rename_custom_rule_group_in_text(text, group_name, group_name, Some(new_target))
}

/// Removes a `#!group <group_name>` section header and safely moves its member rules
/// back to the Ungrouped section (ahead of any remaining `#!group ` sections).
pub fn ungroup_custom_rule_group_in_text(text: &str, group_name: &str, fallback_target: Option<&str>) -> String {
    let wanted = group_name.trim();
    if wanted.is_empty() {
        return text.to_string();
    }
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let Some((start, end)) = find_group_section(&lines, wanted) else {
        return text.to_string();
    };
    let section: Vec<String> = lines.drain(start..end).collect();
    let mut rescued: Vec<String> = section
        .into_iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| match fallback_target.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => replace_rule_line_target(&l, t),
            None => l,
        })
        .collect();
    prune_empty_group_headers(&mut lines);
    if !rescued.is_empty() {
        let ungrouped_end = find_first_group_header(&lines).unwrap_or(lines.len());
        let mut insert_at = ungrouped_end;
        while insert_at > 0 && lines[insert_at - 1].trim().is_empty() && ungrouped_end < lines.len() {
            insert_at -= 1;
        }
        for (i, line) in rescued.drain(..).enumerate() {
            lines.insert(insert_at + i, line);
        }
    }
    join_custom_lines(&lines)
}

// ------------------------------------------------ offline rule match tester

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleMatchLayer {
    PrivateIp,
    CustomRule { line_index: usize },
    CustomGroup { id: String, name: String },
    RuleSet { id: String, name: String },
    BuiltIn { name: String },
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatchResult {
    pub query: String,
    pub layer: RuleMatchLayer,
    /// Human-readable description of the matched rule (e.g. `DOMAIN-SUFFIX,openai.com`).
    pub rule: String,
    /// Target (`proxy`, `direct`, `reject`, or group name).
    pub target: String,
    /// True when the result came from a curated/heuristic hint or bypassed opaque binary `.srs` sets.
    pub approximate: bool,
}

/// Evaluates `query` (URL, domain, IP, `host:port`, or process name) against the
/// exact routing priority:
/// Private IP -> Ungrouped Custom Rules -> Custom Rule Groups (in order) -> Enabled Rule-sets -> Built-in -> Fallback.
pub fn match_rule_query(
    query: &str,
    settings: &crate::model::Settings,
    custom_groups: &[CustomRuleGroup],
    providers: &[RuleProvider],
    rule_set_dir: &std::path::Path,
    ip_is_cn: impl Fn(std::net::IpAddr) -> bool,
) -> Option<RuleMatchResult> {
    let raw = query.trim();
    if raw.is_empty() {
        return None;
    }

    // Parse query into (domain, ip, port, process).
    let (domain, ip, port, process) = parse_test_query(raw);

    // 1. Private IP always goes direct.
    if let Some(addr) = ip {
        if is_private_ip(addr) {
            return Some(RuleMatchResult {
                query: raw.to_string(),
                layer: RuleMatchLayer::PrivateIp,
                rule: "ip_is_private".into(),
                target: "direct".into(),
                approximate: false,
            });
        }
    }

    // Helper to test one parsed custom rule against the query.
    let mut saw_opaque_geosite = false;
    let mut eval_custom = |r: &CustomRule| -> Option<bool> {
        if !r.enabled || r.error.is_some() {
            return None;
        }
        let kind = r.kind?;
        let values: Vec<&str> = r.value.split('|').map(str::trim).filter(|v| !v.is_empty()).collect();
        let matched = match kind {
            CustomRuleKind::Domain => domain.as_deref().is_some_and(|d| values.iter().any(|v| d.eq_ignore_ascii_case(v))),
            CustomRuleKind::DomainSuffix => domain.as_deref().is_some_and(|d| values.iter().any(|v| domain_matches_suffix(d, v))),
            CustomRuleKind::DomainKeyword => domain.as_deref().is_some_and(|d| values.iter().any(|v| d.contains(&v.to_ascii_lowercase()))),
            CustomRuleKind::DomainRegex => domain.as_deref().is_some_and(|d| {
                values.iter().any(|v| regex::Regex::new(v).is_ok_and(|re| re.is_match(d)))
            }),
            CustomRuleKind::IpCidr => ip.is_some_and(|addr| values.iter().any(|v| ip_in_cidr(addr, v))),
            CustomRuleKind::DstPort => port.is_some_and(|p| values.iter().any(|v| v.parse::<u16>().ok() == Some(p))),
            CustomRuleKind::ProcessName => process.as_deref().is_some_and(|p| values.iter().any(|v| p.eq_ignore_ascii_case(v))),
            CustomRuleKind::ProcessPath => process.as_deref().is_some_and(|p| values.iter().any(|v| p.eq_ignore_ascii_case(v))),
            CustomRuleKind::GeoSite => {
                saw_opaque_geosite = true;
                domain.as_deref().is_some_and(|d| values.iter().any(|v| domain_matches_geosite_hint(d, v)))
            }
            CustomRuleKind::GeoIp => ip.is_some_and(|addr| values.iter().any(|v| v.eq_ignore_ascii_case("cn") && ip_is_cn(addr))),
            CustomRuleKind::RuleSet => {
                saw_opaque_geosite = true;
                false
            }
            CustomRuleKind::SrcIpCidr | CustomRuleKind::Network => false,
        };
        matched.then_some(matches!(kind, CustomRuleKind::GeoSite))
    };

    let parsed_custom = parse_custom_rule_list(&settings.custom_rules);

    // 2a. Ungrouped custom rules (highest user priority).
    for r in parsed_custom.iter().filter(|r| r.group.is_none()) {
        if let Some(approx) = eval_custom(r) {
            return Some(RuleMatchResult {
                query: raw.to_string(),
                layer: RuleMatchLayer::CustomRule { line_index: r.line_index },
                rule: format!("{},{}", r.kind.unwrap().token(), r.value),
                target: r.target.clone(),
                approximate: approx,
            });
        }
    }

    // 2b. Custom rule groups in group priority order (each group owns its target).
    for g in custom_groups.iter().filter(|g| g.enabled) {
        let gtarget = g.display_target().to_string();
        for r in parsed_custom
            .iter()
            .filter(|r| r.group.as_deref().is_some_and(|gn| gn.eq_ignore_ascii_case(&g.name)))
        {
            if let Some(approx) = eval_custom(r) {
                return Some(RuleMatchResult {
                    query: raw.to_string(),
                    layer: RuleMatchLayer::CustomGroup { id: g.id.clone(), name: g.name.clone() },
                    rule: format!("{},{}", r.kind.unwrap().token(), r.value),
                    target: gtarget,
                    approximate: approx,
                });
            }
        }
    }

    // 2c. Any ad-hoc `#!group` in CustomRule.list not registered in custom_groups.
    for r in parsed_custom.iter().filter(|r| {
        r.group.as_deref().is_some_and(|gn| {
            !custom_groups.iter().any(|g| g.name.eq_ignore_ascii_case(gn))
        })
    }) {
        if let Some(approx) = eval_custom(r) {
            return Some(RuleMatchResult {
                query: raw.to_string(),
                layer: RuleMatchLayer::CustomRule { line_index: r.line_index },
                rule: format!("{},{}", r.kind.unwrap().token(), r.value),
                target: r.target.clone(),
                approximate: approx,
            });
        }
    }

    // 3. Enabled rule-sets in order.
    let mut saw_binary_provider = false;
    for p in providers.iter().filter(|p| p.enabled) {
        let target = if p.own_group && p.can_own_group() {
            p.name.clone()
        } else {
            p.target.clone()
        };
        // Exact match for local JSON source rule-sets.
        if p.stored == Some(RuleFormat::SingBoxSource) {
            if let Some(path) = crate::config::provider_file(rule_set_dir, p) {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                        if json_ruleset_matches(&v, domain.as_deref(), ip, port, process.as_deref()) {
                            return Some(RuleMatchResult {
                                query: raw.to_string(),
                                layer: RuleMatchLayer::RuleSet { id: p.id.clone(), name: p.name.clone() },
                                rule: p.name.clone(),
                                target,
                                approximate: false,
                            });
                        }
                    }
                }
            }
            continue;
        }
        // Binary `.srs`: check curated preset sample domains / geosite hint.
        saw_binary_provider = true;
        if let Some(d) = domain.as_deref() {
            let preset_hit = p.preset.as_deref().and_then(service_preset).is_some_and(|sp| {
                sp.sample_domains.iter().any(|s| domain_matches_suffix(d, s))
            });
            if preset_hit {
                return Some(RuleMatchResult {
                    query: raw.to_string(),
                    layer: RuleMatchLayer::RuleSet { id: p.id.clone(), name: p.name.clone() },
                    rule: p.name.clone(),
                    target,
                    approximate: true,
                });
            }
        }
    }

    // 4. Built-in rules.
    if settings.block_ads {
        if let Some(d) = domain.as_deref() {
            if domain_matches_geosite_hint(d, "category-ads-all") {
                return Some(RuleMatchResult {
                    query: raw.to_string(),
                    layer: RuleMatchLayer::BuiltIn { name: "block_ads".into() },
                    rule: "geosite-category-ads-all".into(),
                    target: "reject".into(),
                    approximate: true,
                });
            }
        }
    }
    if settings.bypass_cn {
        if let Some(d) = domain.as_deref() {
            if domain_matches_geosite_hint(d, "cn") {
                return Some(RuleMatchResult {
                    query: raw.to_string(),
                    layer: RuleMatchLayer::BuiltIn { name: "bypass_cn".into() },
                    rule: "geosite-cn".into(),
                    target: "direct".into(),
                    approximate: true,
                });
            }
        }
        if let Some(addr) = ip {
            if ip_is_cn(addr) {
                return Some(RuleMatchResult {
                    query: raw.to_string(),
                    layer: RuleMatchLayer::BuiltIn { name: "bypass_cn".into() },
                    rule: "geoip-cn".into(),
                    target: "direct".into(),
                    approximate: false,
                });
            }
        }
    }

    // 5. Fallback.
    Some(RuleMatchResult {
        query: raw.to_string(),
        layer: RuleMatchLayer::Fallback,
        rule: if settings.final_direct { "final → direct".into() } else { "final → proxy".into() },
        target: if settings.final_direct { "direct".into() } else { "proxy".into() },
        approximate: saw_binary_provider || saw_opaque_geosite || settings.bypass_cn || settings.block_ads,
    })
}

fn parse_test_query(raw: &str) -> (Option<String>, Option<std::net::IpAddr>, Option<u16>, Option<String>) {
    let trimmed = raw.trim();
    if let Ok(ip) = trimmed.parse::<std::net::IpAddr>() {
        return (None, Some(ip), None, None);
    }
    // Plain port number.
    if let Ok(port) = trimmed.parse::<u16>() {
        return (None, None, Some(port), None);
    }
    // URL or host(:port).
    let stripped = trimmed.split_once("://").map(|(_, r)| r).unwrap_or(trimmed);
    let authority = stripped.split(['/', '?', '#']).next().unwrap_or(stripped);
    let (host, port) = if let Some(b) = authority.strip_prefix('[') {
        let (h, rest) = b.split_once(']').unwrap_or((b, ""));
        (h, rest.strip_prefix(':').and_then(|p| p.parse::<u16>().ok()))
    } else if authority.matches(':').count() == 1 {
        let (h, p) = authority.split_once(':').unwrap();
        (h, p.parse::<u16>().ok())
    } else {
        (authority, None)
    };
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return (None, Some(ip), port, None);
    }
    // Heuristic: looks like an app/process if ends with .exe/.app or has no dot and isn't a URL.
    let lower = host.to_ascii_lowercase();
    let looks_process = lower.ends_with(".exe")
        || lower.ends_with(".app")
        || trimmed.contains('\\')
        || (!trimmed.contains("://") && !lower.contains('.'));
    if looks_process {
        let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
        let name = name.strip_suffix(".app").unwrap_or(name).to_string();
        return (None, None, port, Some(name));
    }
    (Some(lower.trim_end_matches('.').to_string()), None, port, None)
}

fn domain_matches_suffix(domain: &str, suffix: &str) -> bool {
    let d = domain.trim_end_matches('.').to_ascii_lowercase();
    let s = suffix.trim().trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    if s.is_empty() {
        return false;
    }
    d == s || (d.len() > s.len() && d.ends_with(&s) && d.as_bytes()[d.len() - s.len() - 1] == b'.')
}

fn ip_in_cidr(ip: std::net::IpAddr, cidr: &str) -> bool {
    let Some((net_s, prefix_s)) = cidr.trim().split_once('/') else {
        return cidr.trim().parse::<std::net::IpAddr>().ok() == Some(ip);
    };
    let (Ok(net), Ok(prefix)) = (net_s.parse::<std::net::IpAddr>(), prefix_s.parse::<u32>()) else {
        return false;
    };
    match (ip, net) {
        (std::net::IpAddr::V4(a), std::net::IpAddr::V4(b)) if prefix <= 32 => {
            let mask = if prefix == 0 { 0 } else { !0u32 << (32 - prefix) };
            (u32::from(a) & mask) == (u32::from(b) & mask)
        }
        (std::net::IpAddr::V6(a), std::net::IpAddr::V6(b)) if prefix <= 128 => {
            let mask = if prefix == 0 { 0 } else { !0u128 << (128 - prefix) };
            (u128::from(a) & mask) == (u128::from(b) & mask)
        }
        _ => false,
    }
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

fn json_ruleset_matches(
    v: &Value,
    domain: Option<&str>,
    ip: Option<std::net::IpAddr>,
    port: Option<u16>,
    process: Option<&str>,
) -> bool {
    let Some(rules) = v.get("rules").and_then(Value::as_array) else { return false };
    fn str_list<'a>(obj: &'a serde_json::Map<String, Value>, k: &str) -> Vec<&'a str> {
        match obj.get(k) {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
            Some(Value::String(s)) => vec![s.as_str()],
            _ => Vec::new(),
        }
    }
    for rule in rules {
        let Some(obj) = rule.as_object() else { continue };
        if let Some(d) = domain {
            if str_list(obj, "domain").iter().any(|x| d.eq_ignore_ascii_case(x)) {
                return true;
            }
            if str_list(obj, "domain_suffix").iter().any(|x| domain_matches_suffix(d, x)) {
                return true;
            }
            if str_list(obj, "domain_keyword").iter().any(|x| d.contains(&x.to_ascii_lowercase())) {
                return true;
            }
            if str_list(obj, "domain_regex").iter().any(|x| regex::Regex::new(x).is_ok_and(|re| re.is_match(d))) {
                return true;
            }
        }
        if let Some(addr) = ip {
            if str_list(obj, "ip_cidr").iter().any(|x| ip_in_cidr(addr, x)) {
                return true;
            }
        }
        if let Some(p) = port {
            if let Some(Value::Array(ports)) = obj.get("port") {
                if ports.iter().any(|x| x.as_u64() == Some(p as u64)) {
                    return true;
                }
            }
        }
        if let Some(proc) = process {
            if str_list(obj, "process_name").iter().any(|x| proc.eq_ignore_ascii_case(x))
                || str_list(obj, "process_path").iter().any(|x| proc.eq_ignore_ascii_case(x))
            {
                return true;
            }
        }
    }
    false
}

fn domain_matches_geosite_hint(domain: &str, tag: &str) -> bool {
    let tag = tag.to_ascii_lowercase();
    if let Some(p) = SERVICE_PRESETS.iter().find(|p| p.geosite == tag) {
        return p.sample_domains.iter().any(|s| domain_matches_suffix(domain, s));
    }
    match tag.as_str() {
        "cn" => {
            domain.ends_with(".cn")
                || [
                    "baidu.com",
                    "qq.com",
                    "weixin.qq.com",
                    "taobao.com",
                    "tmall.com",
                    "jd.com",
                    "bilibili.com",
                    "zhihu.com",
                    "weibo.com",
                    "douyin.com",
                    "163.com",
                    "126.com",
                    "aliyun.com",
                    "mi.com",
                    "meituan.com",
                    "xiaohongshu.com",
                    "feishu.cn",
                    "dingtalk.com",
                    "deepseek.com",
                ]
                .iter()
                .any(|s| domain_matches_suffix(domain, s))
        }
        "category-ads-all" => [
            "doubleclick.net",
            "googlesyndication.com",
            "googleadservices.com",
            "adservice.google.com",
            "ads.youtube.com",
            "pagead2.googlesyndication.com",
            "adnxs.com",
            "cpro.baidu.com",
            "pos.baidu.com",
        ]
        .iter()
        .any(|s| domain_matches_suffix(domain, s)),
        _ => false,
    }
}

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
        // A group's first member is its default, and may be another group
        // (`📺 哔哩哔哩` starts on `🎯 全球直连`, which starts on DIRECT).
        let mut name = group.to_string();
        let mut first = None;
        for _ in 0..group_first.len() {
            let Some((_, f)) = group_first.iter().find(|(n, _)| *n == name) else { break };
            first = Some(f.to_ascii_uppercase());
            match f.strip_prefix("[]") {
                Some(next) if group_first.iter().any(|(n, _)| n == next) => name = next.to_string(),
                _ => break,
            }
        }
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

    #[test]
    fn acl_group_defaults_follow_other_groups() {
        // ACL4SSR's Bilibili group starts on the direct group, not on DIRECT itself.
        let ini = r#"
ruleset=📺 哔哩哔哩,https://x/Bilibili.list
ruleset=📹 油管视频,https://x/YouTube.list
ruleset=🛑 拦截链,https://x/Ads.list
ruleset=🔁 成环,https://x/Loop.list
custom_proxy_group=🚀 节点选择`select`[]♻️ 自动选择`[]DIRECT
custom_proxy_group=♻️ 自动选择`url-test`.*
custom_proxy_group=🎯 全球直连`select`[]DIRECT`[]🚀 节点选择
custom_proxy_group=📺 哔哩哔哩`select`[]🎯 全球直连`[]🚀 节点选择
custom_proxy_group=📹 油管视频`select`[]🚀 节点选择`[]DIRECT
custom_proxy_group=🛑 广告拦截`select`[]REJECT
custom_proxy_group=🛑 拦截链`select`[]🛑 广告拦截
custom_proxy_group=🔁 成环`select`[]🔁 成环
"#;
        let sets = parse_acl_ini(ini, "g{name}", "i{name}");
        let target = |name: &str| sets.iter().find(|s| s.group == name).unwrap().target.as_str();
        assert_eq!(target("📺 哔哩哔哩"), "direct");
        assert_eq!(target("📹 油管视频"), "proxy");
        assert_eq!(target("🛑 拦截链"), "reject");
        // A group that leads back to itself ends instead of hanging.
        assert_eq!(target("🔁 成环"), "proxy");
    }

    #[test]
    fn base_domains() {
        assert_eq!(base_domain("www.bilibili.com").as_deref(), Some("bilibili.com"));
        assert_eq!(base_domain("d1--cn-gotcha04.bilivideo.com").as_deref(), Some("bilivideo.com"));
        assert_eq!(base_domain("www.bbc.co.uk").as_deref(), Some("bbc.co.uk"));
        assert_eq!(base_domain("a.b.example.com.cn").as_deref(), Some("example.com.cn"));
        assert_eq!(base_domain("WWW.Example.ORG.").as_deref(), Some("example.org"));
        assert_eq!(base_domain("example.co").as_deref(), Some("example.co"));
        assert_eq!(base_domain("1.2.3.4"), None);
        assert_eq!(base_domain("2001:db8::1"), None);
        assert_eq!(base_domain("localhost"), None);
    }

    #[test]
    fn merging_custom_rules() {
        let text = "# mine\nDOMAIN-SUFFIX,a.com,proxy\n";
        // New rules go last and keep what is there.
        let added = merge_custom_rule(text, "DOMAIN,b.com,direct").unwrap();
        assert_eq!(added, "# mine\nDOMAIN-SUFFIX,a.com,proxy\nDOMAIN,b.com,direct\n");
        // The same match with a new target replaces it in place.
        let changed = merge_custom_rule(&added, "domain-suffix,A.com,reject").unwrap();
        assert_eq!(changed, "# mine\ndomain-suffix,A.com,reject\nDOMAIN,b.com,direct\n");
        // Already there: nothing to do.
        assert_eq!(merge_custom_rule(text, "domain-suffix,a.com,PROXY"), None);
        // Works from empty, and comments never match.
        assert_eq!(merge_custom_rule("", "DOMAIN,c.com,direct").unwrap(), "DOMAIN,c.com,direct\n");
        assert!(merge_custom_rule("#DOMAIN,c.com,direct", "DOMAIN,c.com,direct").is_some());
        // Re-adding a disabled rule re-enables it in place.
        let dis = "#!disabled DOMAIN-SUFFIX,a.com,direct\nDOMAIN,b.com,proxy\n";
        assert_eq!(
            merge_custom_rule(dis, "DOMAIN-SUFFIX,a.com,proxy").unwrap(),
            "DOMAIN-SUFFIX,a.com,proxy\nDOMAIN,b.com,proxy\n"
        );
    }

    #[test]
    fn structured_custom_rules_roundtrip() {
        let text = "# user header\nDOMAIN-SUFFIX,openai.com,proxy\n#!disabled PROCESS-NAME,steam.exe,direct\nbadline\n";
        let rules = parse_custom_rule_list(text);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].kind, Some(CustomRuleKind::DomainSuffix));
        assert!(rules[0].enabled);
        assert!(rules[0].error.is_none());
        assert_eq!(rules[1].kind, Some(CustomRuleKind::ProcessName));
        assert!(!rules[1].enabled);
        assert!(rules[2].error.is_some());

        // Toggle disabled rule on, preserving header comment.
        let toggled = set_custom_rule_enabled(text, 2, true).unwrap();
        assert!(toggled.contains("# user header\n"));
        assert!(toggled.contains("\nPROCESS-NAME,steam.exe,direct\n"));

        // Move rule 1 up before rule 0.
        let moved = move_custom_rule_line(&toggled, 2, -1).unwrap();
        let after = parse_custom_rule_list(&moved);
        assert_eq!(after[0].kind, Some(CustomRuleKind::ProcessName));
        assert_eq!(after[1].kind, Some(CustomRuleKind::DomainSuffix));

        // Remove rule 0.
        let removed = remove_custom_rule_line(&moved, after[0].line_index).unwrap();
        assert_eq!(parse_custom_rule_list(&removed).len(), 2);
    }

    #[test]
    fn normalize_and_match_rules() {
        assert_eq!(normalize_custom_value(CustomRuleKind::DomainSuffix, "https://chat.openai.com/c/123"), "openai.com");
        assert_eq!(normalize_custom_value(CustomRuleKind::Domain, "https://api.openai.com:443/v1"), "api.openai.com");
        assert_eq!(normalize_custom_value(CustomRuleKind::IpCidr, "1.1.1.1"), "1.1.1.1/32");
        assert_eq!(normalize_custom_value(CustomRuleKind::ProcessName, r"C:\Games\Steam\steam.exe"), "steam.exe");
        assert_eq!(normalize_custom_value(CustomRuleKind::ProcessName, "/Applications/Telegram.app"), "Telegram");

        let mut s = crate::model::Settings::default();
        s.custom_rules = "DOMAIN-SUFFIX,example.com,direct\nPROCESS-NAME,steam.exe,proxy\n".into();
        let providers = [service_preset("ai").unwrap().to_provider(&s.geosite_url, Some(true))];
        let dir = std::env::temp_dir();

        let m1 = match_rule_query("192.168.1.5", &s, &[], &providers, &dir, |_| false).unwrap();
        assert_eq!(m1.layer, RuleMatchLayer::PrivateIp);
        assert_eq!(m1.target, "direct");

        let m2 = match_rule_query("https://sub.example.com/page", &s, &[], &providers, &dir, |_| false).unwrap();
        assert!(matches!(m2.layer, RuleMatchLayer::CustomRule { .. }));
        assert_eq!(m2.target, "direct");

        let m3 = match_rule_query("chatgpt.com", &s, &[], &providers, &dir, |_| false).unwrap();
        assert!(matches!(m3.layer, RuleMatchLayer::RuleSet { .. }));
        assert_eq!(m3.target, "AI Services");

        let m4 = match_rule_query("steam.exe", &s, &[], &providers, &dir, |_| false).unwrap();
        assert!(matches!(m4.layer, RuleMatchLayer::CustomRule { .. }));
        assert_eq!(m4.target, "proxy");

        s.bypass_cn = false;
        s.final_direct = true;
        let m5 = match_rule_query("unknown-site.org", &s, &[], &providers, &dir, |_| false).unwrap();
        assert_eq!(m5.layer, RuleMatchLayer::Fallback);
        assert_eq!(m5.target, "direct");
    }

    #[test]
    fn custom_rule_groups_text_and_match() {
        let mut text = "DOMAIN-SUFFIX,top.example,direct\n".to_string();
        text = add_custom_rule_in_group(&text, "DOMAIN-SUFFIX,openai.com,My AI", Some("My AI")).unwrap();
        text = add_custom_rule_in_group(&text, "DOMAIN-SUFFIX,claude.ai,My AI", Some("My AI")).unwrap();
        text = add_custom_rule_in_group(&text, "DOMAIN-SUFFIX,jira.local,Work", Some("Work")).unwrap();

        let parsed = parse_custom_rule_list(&text);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].group, None);
        assert_eq!(parsed[1].group.as_deref(), Some("My AI"));
        assert_eq!(parsed[2].group.as_deref(), Some("My AI"));
        assert_eq!(parsed[3].group.as_deref(), Some("Work"));

        // Adding an ungrouped rule inserts BEFORE `#!group My AI`.
        let merged = merge_custom_rule(&text, "DOMAIN,override.openai.com,direct").unwrap();
        let parsed2 = parse_custom_rule_list(&merged);
        assert_eq!(parsed2[1].value, "override.openai.com");
        assert_eq!(parsed2[1].group, None);

        // Move claude.ai from My AI to Work (updating its target).
        let claude_line = parsed2.iter().find(|r| r.value == "claude.ai").unwrap().line_index;
        let moved = move_custom_rule_to_group(&merged, claude_line, Some("Work"), Some("direct")).unwrap();
        let parsed3 = parse_custom_rule_list(&moved);
        let claude = parsed3.iter().find(|r| r.value == "claude.ai").unwrap();
        assert_eq!(claude.group.as_deref(), Some("Work"));
        assert_eq!(claude.target, "direct");

        // Rename group My AI -> AI Tools.
        let renamed = rename_custom_rule_group_in_text(&moved, "My AI", "AI Tools", Some("AI Tools"));
        let parsed4 = parse_custom_rule_list(&renamed);
        let oai = parsed4.iter().find(|r| r.value == "openai.com").unwrap();
        assert_eq!(oai.group.as_deref(), Some("AI Tools"));
        assert_eq!(oai.target, "AI Tools");

        // Matcher respects ungrouped override > enabled custom group > disabled custom group.
        let groups = vec![
            CustomRuleGroup {
                id: "g1".into(),
                name: "AI Tools".into(),
                target: "proxy".into(),
                enabled: true,
                own_group: true,
                ..Default::default()
            },
            CustomRuleGroup {
                id: "g2".into(),
                name: "Work".into(),
                target: "direct".into(),
                enabled: false,
                own_group: false,
                ..Default::default()
            },
        ];
        let mut s = crate::model::Settings::default();
        s.custom_rules = renamed.clone();
        s.bypass_cn = false;
        let dir = std::env::temp_dir();
        let m_override = match_rule_query("override.openai.com", &s, &groups, &[], &dir, |_| false).unwrap();
        assert!(matches!(m_override.layer, RuleMatchLayer::CustomRule { .. }));
        assert_eq!(m_override.target, "direct");

        let m_group = match_rule_query("api.openai.com", &s, &groups, &[], &dir, |_| false).unwrap();
        assert_eq!(m_group.layer, RuleMatchLayer::CustomGroup { id: "g1".into(), name: "AI Tools".into() });
        assert_eq!(m_group.target, "AI Tools");

        // Disabled group is skipped -> falls through to fallback.
        let m_disabled = match_rule_query("jira.local", &s, &groups, &[], &dir, |_| false).unwrap();
        assert_eq!(m_disabled.layer, RuleMatchLayer::Fallback);

        // Ungrouping Work rescues its rules into Ungrouped without losing them.
        let ungrouped = ungroup_custom_rule_group_in_text(&renamed, "Work", Some("direct"));
        let parsed5 = parse_custom_rule_list(&ungrouped);
        assert_eq!(parsed5.len(), 5);
        assert!(parsed5.iter().find(|r| r.value == "jira.local").unwrap().group.is_none());
    }
}
