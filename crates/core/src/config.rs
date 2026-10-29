//! Generates a complete sing-box configuration from settings and nodes.

use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;

use serde_json::{Map, Value, json};

use crate::model::{Node, Settings};
use crate::rules::{RuleFormat, RuleProvider};

pub const TAG_PROXY: &str = "Proxy";
pub const TAG_AUTO: &str = "Auto";
pub const TAG_DIRECT: &str = "direct";
/// Hidden selector behind the probe inbound: the exit-location check points it
/// at one node at a time without touching the user's selection.
pub const TAG_PROBE: &str = "BoxPilot-probe";
const PROBE_INBOUND: &str = "probe-in";
const RESERVED: &[&str] = &[TAG_PROXY, TAG_AUTO, TAG_DIRECT, TAG_PROBE, "GLOBAL", "REJECT", "DIRECT"];

/// sing-box version, used to emit options the running core understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CoreVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl CoreVersion {
    pub const MIN_SUPPORTED: CoreVersion = CoreVersion { major: 1, minor: 12, patch: 0 };

    /// Parses `sing-box version 1.14.4` or `1.14.4-beta.1`.
    pub fn parse(text: &str) -> Option<Self> {
        let token = text
            .split_whitespace()
            .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))?;
        let mut parts = token
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<u32>().ok());
        Some(Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next().flatten().unwrap_or(0),
        })
    }

    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

impl std::fmt::Display for CoreVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Nodes of one enabled subscription.
pub struct NodeGroup<'a> {
    pub name: &'a str,
    pub nodes: &'a [Node],
}

pub struct BuildInput<'a> {
    pub settings: &'a Settings,
    pub groups: Vec<NodeGroup<'a>>,
    pub version: CoreVersion,
    /// When set, rule-sets are referenced as local files `<dir>/<tag>.srs`
    /// managed by the app; rules whose file is missing are left out.
    /// When `None`, sing-box downloads them itself (remote rule-sets).
    pub rule_set_dir: Option<&'a std::path::Path>,
    /// Imported rule-sets, applied in order after the custom rules.
    pub providers: &'a [RuleProvider],
}

/// Where a provider's converted rule-set is stored.
pub fn provider_file(dir: &std::path::Path, p: &RuleProvider) -> Option<std::path::PathBuf> {
    let ext = match p.stored? {
        RuleFormat::SingBoxBinary => "srs",
        _ => "json",
    };
    Some(dir.join(format!("{}.{ext}", p.tag())))
}

fn apply_target(rule: &mut Map<String, Value>, target: &str, known_tags: &HashSet<String>) -> bool {
    match target.to_ascii_lowercase().as_str() {
        "proxy" => {
            rule.insert("outbound".into(), json!(TAG_PROXY));
        }
        "direct" => {
            rule.insert("outbound".into(), json!(TAG_DIRECT));
        }
        "reject" | "block" => {
            rule.insert("action".into(), json!("reject"));
        }
        _ if known_tags.contains(target) => {
            rule.insert("outbound".into(), json!(target));
        }
        _ => {
            rule.insert("outbound".into(), json!(TAG_PROXY));
            return false;
        }
    }
    true
}

/// A rule-set the configuration wants, with where to download it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSetSpec {
    pub tag: String,
    pub url: String,
}

pub struct BuiltConfig {
    pub config: Value,
    pub node_count: usize,
    /// Warnings about custom rules that were ignored.
    pub warnings: Vec<String>,
    /// Every rule-set referenced by the settings (present or not).
    pub rule_sets: Vec<RuleSetSpec>,
    /// Rule-sets that were left out because their local file is missing.
    pub missing_rule_sets: Vec<RuleSetSpec>,
    /// The node group of each rule-set that has one: (provider id, group tag).
    pub provider_groups: Vec<(String, String)>,
}

pub fn rule_set_file(dir: &std::path::Path, tag: &str) -> std::path::PathBuf {
    dir.join(format!("{tag}.srs"))
}

struct TagAllocator {
    used: HashSet<String>,
}

impl TagAllocator {
    fn new() -> Self {
        Self {
            used: RESERVED.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn alloc(&mut self, wanted: &str) -> String {
        let base = if wanted.trim().is_empty() { "node" } else { wanted.trim() };
        if self.used.insert(base.to_string()) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base} ({i})"))
            .find(|t| self.used.insert(t.clone()))
            .unwrap()
    }
}

/// The TUN peer address. DNS sent there enters TUN and the DNS hijack
/// answers it; macOS points its network services here while TUN runs.
pub const TUN_DNS: &str = "172.19.0.2";

/// Sends the core's system-resolver lookups (`local` DNS servers, used to
/// find DoH hosts by name) to `upstream`. While the system's DNS points at
/// TUN they would otherwise come back to the core itself.
pub fn use_upstream_for_local_dns(config: &mut Value, upstream: IpAddr) {
    let Some(servers) = config.pointer_mut("/dns/servers").and_then(Value::as_array_mut) else {
        return;
    };
    for server in servers.iter_mut().filter(|s| s["type"] == "local") {
        let tag = server["tag"].clone();
        *server = json!({ "type": "udp", "tag": tag, "server": upstream.to_string() });
    }
}

pub fn build_config(input: &BuildInput) -> BuiltConfig {
    let s = input.settings;
    let v = input.version;
    let mut tags = TagAllocator::new();
    let mut outbounds: Vec<Value> = Vec::new();
    let mut endpoints: Vec<Value> = Vec::new();
    let mut all_tags: Vec<String> = Vec::new();
    let mut group_tags: Vec<String> = Vec::new();
    let mut group_outbounds: Vec<Value> = Vec::new();

    let multi = input.groups.iter().filter(|g| !g.nodes.is_empty()).count() > 1;
    for group in &input.groups {
        if group.nodes.is_empty() {
            continue;
        }
        let mut members = Vec::new();
        for node in group.nodes {
            let tag = tags.alloc(&node.name);
            let mut ob = node.outbound.clone();
            if let Some(obj) = ob.as_object_mut() {
                obj.insert("tag".into(), json!(tag));
                obj.insert("type".into(), json!(node.protocol));
            }
            if node.is_endpoint() {
                endpoints.push(ob);
            } else {
                outbounds.push(ob);
            }
            members.push(tag.clone());
            all_tags.push(tag);
        }
        if multi {
            let gtag = tags.alloc(group.name);
            group_outbounds.push(json!({
                "type": "selector",
                "tag": gtag,
                "outbounds": members,
                "interrupt_exist_connections": true,
            }));
            group_tags.push(gtag);
        }
    }

    let mut head: Vec<Value> = Vec::new();
    let mut proxy_members: Vec<String> = Vec::new();
    if !all_tags.is_empty() {
        proxy_members.push(TAG_AUTO.into());
        proxy_members.extend(group_tags.iter().cloned());
        proxy_members.extend(all_tags.iter().cloned());
    }
    proxy_members.push(TAG_DIRECT.into());
    head.push(json!({
        "type": "selector",
        "tag": TAG_PROXY,
        "outbounds": proxy_members,
        "default": if all_tags.is_empty() { TAG_DIRECT } else { TAG_AUTO },
        "interrupt_exist_connections": true,
    }));
    if !all_tags.is_empty() {
        // BoxPilot picks Auto's node through the Clash API (the engine's auto
        // loop), so changing how it picks or which region never touches the
        // config, and never restarts the core.
        head.push(json!({
            "type": "selector",
            "tag": TAG_AUTO,
            "outbounds": all_tags,
            "default": all_tags[0],
            "interrupt_exist_connections": false,
        }));
    }
    let probe = !all_tags.is_empty() && s.probe_port != 0;
    if probe {
        head.push(json!({
            "type": "selector",
            "tag": TAG_PROBE,
            "outbounds": all_tags,
            "interrupt_exist_connections": false,
        }));
    }
    head.extend(group_outbounds);
    // Rule-set groups go in ahead of the nodes, after the subscription groups.
    let outbounds_len = outbounds.len();
    head.extend(outbounds);
    head.push(json!({ "type": "direct", "tag": TAG_DIRECT }));

    let mut rule_sets = RuleSets::new(s, v, input.rule_set_dir);
    let mut warnings = Vec::new();
    let known = all_known_tags(&head, &endpoints);
    let mut custom = parse_custom_rules(&s.custom_rules, &mut rule_sets, &known, &mut warnings);
    let mut provider_groups = Vec::new();
    if let Some(dir) = input.rule_set_dir {
        for p in input.providers.iter().filter(|p| p.enabled) {
            let Some(path) = provider_file(dir, p).filter(|f| f.is_file()) else { continue };
            let tag = p.tag();
            let format = if p.stored == Some(RuleFormat::SingBoxBinary) { "binary" } else { "source" };
            rule_sets.sets.insert(
                tag.clone(),
                json!({ "type": "local", "tag": tag, "format": format, "path": path.to_string_lossy() }),
            );
            let mut rule = Map::new();
            rule.insert("rule_set".into(), json!([tag]));
            if !apply_target(&mut rule, &p.target, &known) {
                warnings.push(format!("rule-set {}: unknown target `{}`, using Proxy", p.name, p.target));
            }
            if p.own_group && p.can_own_group() && !all_tags.is_empty() {
                // A selector of its own: it starts on the rule-set's target and
                // can be switched by hand or picked for like Auto.
                let gtag = tags.alloc(if p.name.trim().is_empty() { "Rule-set" } else { &p.name });
                let start = rule.get("outbound").and_then(Value::as_str).unwrap_or(TAG_PROXY).to_string();
                let mut members = vec![TAG_PROXY.to_string()];
                members.extend(group_tags.iter().cloned());
                members.extend(all_tags.iter().cloned());
                members.push(TAG_DIRECT.into());
                let start = if members.contains(&start) { start } else { TAG_PROXY.to_string() };
                head.insert(
                    head.len() - 1 - outbounds_len,
                    json!({
                        "type": "selector",
                        "tag": gtag,
                        "outbounds": members,
                        "default": start,
                        "interrupt_exist_connections": true,
                    }),
                );
                rule.insert("outbound".into(), json!(gtag));
                provider_groups.push((p.id.clone(), gtag));
            }
            custom.push(Value::Object(rule));
        }
    }

    let mut route = build_route(s, &mut rule_sets, custom);
    if probe {
        // Ahead of the mode and private-IP rules, so it always reaches the chosen node.
        if let Some(rules) = route["rules"].as_array_mut() {
            rules.insert(0, json!({ "inbound": [PROBE_INBOUND], "outbound": TAG_PROBE }));
        }
    }
    let dns = build_dns(s, &mut rule_sets);

    let mut config = Map::new();
    config.insert(
        "log".into(),
        json!({ "level": s.log_level, "timestamp": true }),
    );
    config.insert("dns".into(), dns);
    let mut inbounds = build_inbounds(s);
    if probe {
        inbounds.push(json!({
            "type": "mixed",
            "tag": PROBE_INBOUND,
            "listen": "127.0.0.1",
            "listen_port": s.probe_port,
        }));
    }
    config.insert("inbounds".into(), json!(inbounds));
    if !endpoints.is_empty() {
        config.insert("endpoints".into(), json!(endpoints));
    }
    config.insert("outbounds".into(), json!(head));
    let uses_remote = rule_sets.has_remote();
    let (sets, wanted, missing) = rule_sets.finish();
    route["rule_set"] = json!(sets);
    if v.at_least(1, 14) && s.ruleset_via_proxy && uses_remote {
        config.insert(
            "http_clients".into(),
            json!([{ "tag": "ruleset-client", "detour": TAG_PROXY }]),
        );
        route["default_http_client"] = json!("ruleset-client");
    }
    config.insert("route".into(), route);
    config.insert(
        "experimental".into(),
        json!({
            "clash_api": {
                "external_controller": format!("127.0.0.1:{}", s.clash_api_port),
                "secret": s.clash_api_secret,
                "default_mode": s.mode.as_str(),
            },
            "cache_file": {
                "enabled": true,
                "path": "cache.db",
                "store_fakeip": s.fake_ip,
            },
        }),
    );

    BuiltConfig {
        config: Value::Object(config),
        node_count: input.groups.iter().map(|g| g.nodes.len()).sum(),
        warnings,
        rule_sets: wanted,
        missing_rule_sets: missing,
        provider_groups,
    }
}

fn all_known_tags(outbounds: &[Value], endpoints: &[Value]) -> HashSet<String> {
    outbounds
        .iter()
        .chain(endpoints)
        .filter_map(|o| o.get("tag").and_then(Value::as_str).map(String::from))
        .collect()
}

fn build_inbounds(s: &Settings) -> Vec<Value> {
    let mut inbounds = vec![json!({
        "type": "mixed",
        "tag": "mixed-in",
        "listen": if s.allow_lan { "::" } else { "127.0.0.1" },
        "listen_port": s.mixed_port,
    })];
    if s.tun_enabled {
        let mut address = vec!["172.19.0.1/30"];
        if s.ipv6 {
            address.push("fdfe:dcba:9876::1/126");
        }
        let mut tun = json!({
            "type": "tun",
            "tag": "tun-in",
            "address": address,
            "mtu": s.tun_mtu,
            "auto_route": true,
            "strict_route": s.tun_strict_route,
            "stack": s.tun_stack.as_str(),
        });
        // Apple systems only take `utunN` names; sing-box picks a free one there.
        if !cfg!(any(target_os = "macos", target_os = "ios")) {
            tun["interface_name"] = json!("BoxPilot");
        }
        inbounds.push(tun);
    }
    inbounds
}

struct RuleSets<'a> {
    sets: BTreeMap<String, Value>,
    wanted: Vec<RuleSetSpec>,
    missing: Vec<RuleSetSpec>,
    geosite_url: String,
    geoip_url: String,
    legacy_detour: bool,
    dir: Option<&'a std::path::Path>,
}

impl<'a> RuleSets<'a> {
    fn new(s: &Settings, v: CoreVersion, dir: Option<&'a std::path::Path>) -> Self {
        Self {
            sets: BTreeMap::new(),
            wanted: Vec::new(),
            missing: Vec::new(),
            geosite_url: s.geosite_url.clone(),
            geoip_url: s.geoip_url.clone(),
            legacy_detour: s.ruleset_via_proxy && !v.at_least(1, 14),
            dir,
        }
    }

    /// Registers a rule-set; returns its tag, or `None` when it is unavailable.
    fn remote(&mut self, tag: String, url: String) -> Option<String> {
        if let Some(existing) = self.sets.get(&tag) {
            return (!existing.is_null()).then_some(tag);
        }
        let spec = RuleSetSpec { tag: tag.clone(), url: url.clone() };
        self.wanted.push(spec.clone());
        let entry = match self.dir {
            Some(dir) => {
                let path = rule_set_file(dir, &tag);
                if path.is_file() {
                    json!({
                        "type": "local",
                        "tag": tag,
                        "format": "binary",
                        "path": path.to_string_lossy(),
                    })
                } else {
                    self.missing.push(spec);
                    Value::Null
                }
            }
            None => {
                let format = if url.ends_with(".json") { "source" } else { "binary" };
                let mut rs = json!({
                    "type": "remote",
                    "tag": tag,
                    "format": format,
                    "url": url,
                    "update_interval": "1d",
                });
                if self.legacy_detour {
                    rs["download_detour"] = json!(TAG_PROXY);
                }
                rs
            }
        };
        let ok = !entry.is_null();
        self.sets.insert(tag.clone(), entry);
        ok.then_some(tag)
    }

    fn geosite(&mut self, name: &str) -> Option<String> {
        let url = self.geosite_url.replace("{name}", name);
        self.remote(format!("geosite-{name}"), url)
    }

    fn geoip(&mut self, name: &str) -> Option<String> {
        let url = self.geoip_url.replace("{name}", name);
        self.remote(format!("geoip-{name}"), url)
    }

    fn has_remote(&self) -> bool {
        self.sets.values().any(|v| v["type"] == "remote")
    }

    fn finish(self) -> (Vec<Value>, Vec<RuleSetSpec>, Vec<RuleSetSpec>) {
        (
            self.sets.into_values().filter(|v| !v.is_null()).collect(),
            self.wanted,
            self.missing,
        )
    }
}

fn build_route(s: &Settings, rs: &mut RuleSets, custom: Vec<Value>) -> Value {
    let mut rules = vec![
        json!({ "action": "sniff" }),
        json!({ "protocol": "dns", "action": "hijack-dns" }),
    ];
    // After sniffing (QUIC is recognised there) and DNS, before any routing.
    if s.block_quic {
        rules.push(json!({ "protocol": "quic", "action": "reject" }));
    }
    if !s.proxy_udp {
        rules.push(json!({ "network": "udp", "outbound": TAG_DIRECT }));
    }
    rules.extend([
        json!({ "ip_is_private": true, "outbound": TAG_DIRECT }),
        json!({ "clash_mode": "Direct", "outbound": TAG_DIRECT }),
        json!({ "clash_mode": "Global", "outbound": TAG_PROXY }),
    ]);
    rules.extend(custom);
    if s.block_ads {
        if let Some(ads) = rs.geosite("category-ads-all") {
            rules.push(json!({ "rule_set": [ads], "action": "reject" }));
        }
    }
    if s.bypass_cn {
        if let Some(cn) = rs.geosite("cn") {
            rules.push(json!({ "rule_set": [cn], "outbound": TAG_DIRECT }));
        }
        if let Some(not_cn) = rs.geosite("geolocation-!cn") {
            rules.push(json!({ "rule_set": [not_cn], "outbound": TAG_PROXY }));
        }
        if let Some(cn_ip) = rs.geoip("cn") {
            rules.push(json!({ "action": "resolve", "server": "dns-remote" }));
            rules.push(json!({ "rule_set": [cn_ip], "outbound": TAG_DIRECT }));
        }
    }
    json!({
        "rules": rules,
        "final": TAG_PROXY,
        "auto_detect_interface": true,
        "find_process": true,
        "default_domain_resolver": { "server": "dns-direct" },
    })
}

fn build_dns(s: &Settings, rs: &mut RuleSets) -> Value {
    let mut servers = vec![
        json!({ "type": "local", "tag": "dns-local" }),
        dns_server(&s.dns_remote, "dns-remote", Some(TAG_PROXY)),
        dns_server(&s.dns_direct, "dns-direct", None),
    ];
    let mut rules = vec![
        json!({ "clash_mode": "Direct", "server": "dns-direct" }),
        json!({ "clash_mode": "Global", "server": "dns-remote" }),
    ];
    if s.block_ads {
        if let Some(ads) = rs.geosite("category-ads-all") {
            rules.push(json!({ "rule_set": [ads], "action": "reject" }));
        }
    }
    if s.bypass_cn {
        if let Some(cn) = rs.geosite("cn") {
            rules.push(json!({ "rule_set": [cn], "server": "dns-direct" }));
        }
    }
    if s.fake_ip {
        servers.push(json!({
            "type": "fakeip",
            "tag": "dns-fakeip",
            "inet4_range": "198.18.0.0/15",
            "inet6_range": "fc00::/18",
        }));
        rules.push(json!({ "query_type": ["A", "AAAA"], "server": "dns-fakeip" }));
    }
    json!({
        "servers": servers,
        "rules": rules,
        "final": "dns-remote",
        "strategy": if s.ipv6 { "prefer_ipv4" } else { "ipv4_only" },
    })
}

/// Converts `https://1.1.1.1/dns-query`, `tls://dns.google`, `8.8.8.8`, `local`, ...
/// into the sing-box 1.12+ DNS server format.
pub fn dns_server(addr: &str, tag: &str, detour: Option<&str>) -> Value {
    let addr = addr.trim();
    if addr.is_empty() || addr.eq_ignore_ascii_case("local") {
        return json!({ "type": "local", "tag": tag });
    }
    if addr.eq_ignore_ascii_case("dhcp") || addr.starts_with("dhcp://") {
        return json!({ "type": "dhcp", "tag": tag });
    }
    let (scheme, rest) = addr.split_once("://").unwrap_or(("udp", addr));
    let ty = match scheme.to_ascii_lowercase().as_str() {
        "https" | "doh" => "https",
        "h3" => "h3",
        "tls" | "dot" => "tls",
        "quic" | "doq" => "quic",
        "tcp" => "tcp",
        _ => "udp",
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (host, port) = crate::convert::split_host_port(hostport);
    let port: Option<u16> = port.parse().ok();
    let mut server = Map::new();
    server.insert("type".into(), json!(ty));
    server.insert("tag".into(), json!(tag));
    server.insert("server".into(), json!(host));
    if let Some(p) = port {
        server.insert("server_port".into(), json!(p));
    }
    if matches!(ty, "https" | "h3") && !path.is_empty() && path != "/dns-query" {
        server.insert("path".into(), json!(path));
    }
    if host.parse::<std::net::IpAddr>().is_err() {
        server.insert("domain_resolver".into(), json!("dns-local"));
    }
    if let Some(d) = detour {
        server.insert("detour".into(), json!(d));
    }
    Value::Object(server)
}

/// Custom rule lines: `TYPE,VALUE,TARGET` (Clash style names are accepted).
/// TARGET is `proxy`, `direct`, `reject` or any outbound / group tag.
fn parse_custom_rules(
    text: &str,
    rs: &mut RuleSets,
    known_tags: &HashSet<String>,
    warnings: &mut Vec<String>,
) -> Vec<Value> {
    let mut rules = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, ',').map(str::trim).collect();
        if parts.len() != 3 {
            warnings.push(format!("rule {}: expected TYPE,VALUE,TARGET", i + 1));
            continue;
        }
        let (kind, value, target) = (parts[0].to_ascii_lowercase(), parts[1], parts[2]);
        let values: Vec<String> = value.split('|').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
        let key = match kind.replace('_', "-").as_str() {
            "domain" => "domain",
            "domain-suffix" => "domain_suffix",
            "domain-keyword" => "domain_keyword",
            "domain-regex" => "domain_regex",
            "ip-cidr" | "ip-cidr6" => "ip_cidr",
            "src-ip-cidr" | "source-ip-cidr" => "source_ip_cidr",
            "process-name" => "process_name",
            "process-path" => "process_path",
            "dst-port" | "port" => "port",
            "network" => "network",
            "geosite" | "geoip" | "rule-set" => "rule_set",
            _ => {
                warnings.push(format!("rule {}: unknown type `{}`", i + 1, parts[0]));
                continue;
            }
        };
        let mut rule = Map::new();
        match (kind.as_str(), key) {
            (_, "rule_set") => {
                let tags: Vec<String> = values
                    .iter()
                    .enumerate()
                    .filter_map(|(j, v)| match kind.as_str() {
                        "geosite" => rs.geosite(&v.to_ascii_lowercase()),
                        "geoip" => rs.geoip(&v.to_ascii_lowercase()),
                        _ => rs.remote(format!("custom-{}-{}", i + 1, j + 1), v.clone()),
                    })
                    .collect();
                if tags.is_empty() {
                    // Rule-set not downloaded yet; the rule is added once it is.
                    continue;
                }
                rule.insert("rule_set".into(), json!(tags));
            }
            (_, "port") => {
                let ports: Vec<u16> = values.iter().filter_map(|p| p.parse().ok()).collect();
                rule.insert("port".into(), json!(ports));
            }
            (_, "network") => {
                rule.insert("network".into(), json!(values));
            }
            _ => {
                rule.insert(key.into(), json!(values));
            }
        }
        if !apply_target(&mut rule, target, known_tags) {
            warnings.push(format!("rule {}: unknown target `{target}`, using Proxy", i + 1));
        }
        rules.push(Value::Object(rule));
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::parse_subscription;

    #[test]
    fn version_parse() {
        let v = CoreVersion::parse("sing-box version 1.14.4\n\nEnvironment: go1.25").unwrap();
        assert_eq!(v, CoreVersion { major: 1, minor: 14, patch: 4 });
        assert!(CoreVersion::parse("1.13.0-beta.2").unwrap().at_least(1, 13));
    }

    #[test]
    fn dns_formats() {
        assert_eq!(dns_server("https://1.1.1.1/dns-query", "r", Some("Proxy"))["type"], "https");
        let d = dns_server("tls://dns.google:853", "r", None);
        assert_eq!(d["server_port"], 853);
        assert_eq!(d["domain_resolver"], "dns-local");
        assert_eq!(dns_server("223.5.5.5", "d", None)["type"], "udp");
    }

    #[test]
    fn full_config() {
        let a = parse_subscription("trojan://p@a.com:443#Same\nvless://u@b.com:443#Same").unwrap();
        let b = parse_subscription("wireguard://k@1.1.1.1:51820?publickey=p#WG\nhy2://x@c.com:443#Proxy").unwrap();
        let mut settings = Settings::default();
        settings.tun_enabled = true;
        settings.custom_rules = "DOMAIN-SUFFIX,example.com,direct\ngeosite,netflix,proxy\nbad".into();
        let out = build_config(&BuildInput {
            settings: &settings,
            groups: vec![
                NodeGroup { name: "Sub A", nodes: &a.nodes },
                NodeGroup { name: "Sub B", nodes: &b.nodes },
            ],
            version: CoreVersion { major: 1, minor: 14, patch: 0 },
            rule_set_dir: None,
            providers: &[],
        });
        let c = &out.config;
        let tags: Vec<&str> = c["outbounds"].as_array().unwrap().iter().map(|o| o["tag"].as_str().unwrap()).collect();
        assert_eq!(tags[0], "Proxy");
        assert!(tags.contains(&"Same"));
        assert!(tags.contains(&"Same (2)"));
        // A node named like a reserved tag is renamed.
        assert!(tags.contains(&"Proxy (2)"));
        assert!(tags.contains(&"Sub A"));
        assert_eq!(c["endpoints"][0]["tag"], "WG");
        assert_eq!(c["inbounds"][1]["type"], "tun");
        assert_eq!(c["http_clients"][0]["detour"], "Proxy");
        let rs: Vec<&str> = c["route"]["rule_set"].as_array().unwrap().iter().map(|r| r["tag"].as_str().unwrap()).collect();
        assert!(rs.contains(&"geosite-netflix"));
        assert!(rs.contains(&"geoip-cn"));
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(c["route"]["rules"][5]["domain_suffix"][0], "example.com");
    }

    #[test]
    fn rule_sets_with_their_own_group() {
        let dir = std::env::temp_dir().join(format!("bp-own-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let nodes = parse_subscription("trojan://p@a.com:443#A
vless://u@b.com:443#B").unwrap();
        let provider = |id: &str, name: &str, target: &str, own: bool| {
            let p = RuleProvider {
                id: id.into(),
                name: name.into(),
                target: target.into(),
                own_group: own,
                stored: Some(RuleFormat::SingBoxSource),
                ..Default::default()
            };
            std::fs::write(provider_file(&dir, &p).unwrap(), b"{}").unwrap();
            p
        };
        let providers = [
            provider("g", "Google", "proxy", true),
            provider("d", "Domestic", "direct", true),
            provider("r", "Ads", "reject", true),
            provider("n", "Netflix", "proxy", false),
        ];
        let out = build_config(&BuildInput {
            settings: &Settings::default(),
            groups: vec![NodeGroup { name: "Sub", nodes: &nodes.nodes }],
            version: CoreVersion { major: 1, minor: 14, patch: 4 },
            rule_set_dir: Some(&dir),
            providers: &providers,
        });
        assert_eq!(
            out.provider_groups,
            vec![("g".to_string(), "Google".to_string()), ("d".to_string(), "Domestic".to_string())]
        );
        let obs = out.config["outbounds"].as_array().unwrap();
        let group = obs.iter().find(|o| o["tag"] == "Google").unwrap();
        assert_eq!(group["type"], "selector");
        assert_eq!(group["default"], "Proxy");
        assert_eq!(group["outbounds"], json!(["Proxy", "A", "B", "direct"]));
        // A direct rule-set's group starts on Direct.
        assert_eq!(obs.iter().find(|o| o["tag"] == "Domestic").unwrap()["default"], "direct");
        // Groups sit ahead of the nodes.
        let at = |tag: &str| obs.iter().position(|o| o["tag"] == tag).unwrap();
        assert!(at("Google") < at("A"));
        let rules = out.config["route"]["rules"].as_array().unwrap();
        let outbound = |rs: &str| rules.iter().find(|r| r["rule_set"][0] == rs).unwrap()["outbound"].clone();
        assert_eq!(outbound("rp-g"), "Google");
        assert_eq!(outbound("rp-d"), "Domestic");
        let ads = rules.iter().find(|r| r["rule_set"][0] == "rp-r").unwrap();
        assert_eq!(ads["action"], "reject");
        assert!(ads.get("outbound").is_none());
        assert_eq!(outbound("rp-n"), "Proxy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_dns_goes_upstream_under_tun() {
        let mut settings = Settings::default();
        settings.dns_direct = "local".into();
        let mut c = build_config(&BuildInput {
            settings: &settings,
            groups: vec![],
            version: CoreVersion { major: 1, minor: 14, patch: 4 },
            rule_set_dir: None,
            providers: &[],
        })
        .config;
        use_upstream_for_local_dns(&mut c, "192.168.1.1".parse().unwrap());
        let servers = c["dns"]["servers"].as_array().unwrap();
        assert!(servers.iter().all(|s| s["type"] != "local"));
        let direct = servers.iter().find(|s| s["tag"] == "dns-direct").unwrap();
        assert_eq!(direct["server"], "192.168.1.1");
        // Local rule-sets keep their type.
        assert!(c["route"]["rule_set"].as_array().is_none_or(|r| r.iter().all(|x| x["type"] != "udp")));
    }

    #[test]
    fn legacy_detour_before_1_14() {
        let settings = Settings::default();
        let out = build_config(&BuildInput {
            settings: &settings,
            groups: vec![],
            version: CoreVersion { major: 1, minor: 12, patch: 0 },
            rule_set_dir: None,
            providers: &[],
        });
        assert!(out.config.get("http_clients").is_none());
        assert_eq!(out.config["route"]["rule_set"][0]["download_detour"], "Proxy");
        assert_eq!(out.config["outbounds"][0]["outbounds"], json!(["direct"]));
    }

    #[test]
    fn local_rule_sets_skip_missing_files() {
        let dir = std::env::temp_dir().join(format!("bp-rs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(rule_set_file(&dir, "geosite-cn"), b"x").unwrap();
        let mut settings = Settings::default();
        settings.custom_rules = "GEOSITE,netflix,proxy".into();
        let out = build_config(&BuildInput {
            settings: &settings,
            groups: vec![],
            version: CoreVersion { major: 1, minor: 14, patch: 4 },
            rule_set_dir: Some(&dir),
            providers: &[],
        });
        let sets = out.config["route"]["rule_set"].as_array().unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0]["type"], "local");
        assert_eq!(out.rule_sets.len(), 4);
        assert_eq!(out.missing_rule_sets.len(), 3);
        assert!(out.config.get("http_clients").is_none());
        let rules = out.config["route"]["rules"].to_string();
        assert!(!rules.contains("geoip-cn") && !rules.contains("netflix") && rules.contains("geosite-cn"));

        let provider = RuleProvider {
            id: "abc".into(),
            name: "Ads".into(),
            target: "reject".into(),
            stored: Some(RuleFormat::SingBoxSource),
            ..Default::default()
        };
        std::fs::write(provider_file(&dir, &provider).unwrap(), b"{}").unwrap();
        let out = build_config(&BuildInput {
            settings: &settings,
            groups: vec![],
            version: CoreVersion { major: 1, minor: 14, patch: 4 },
            rule_set_dir: Some(&dir),
            providers: std::slice::from_ref(&provider),
        });
        let rules = out.config["route"]["rules"].as_array().unwrap();
        let r = rules.iter().find(|r| r["rule_set"][0] == "rp-abc").unwrap();
        assert_eq!(r["action"], "reject");
        // Providers come before the built-in China rules.
        let pos = |tag: &str| rules.iter().position(|r| r["rule_set"][0] == tag).unwrap();
        assert!(pos("rp-abc") < pos("geosite-cn"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn auto_group(settings: &Settings) -> Value {
        let a = parse_subscription("trojan://p@a.com:443#HK-1\ntrojan://p@b.com:443#JP-1\ntrojan://p@c.com:443#JP-2").unwrap();
        let out = build_config(&BuildInput {
            settings,
            groups: vec![NodeGroup { name: "Sub", nodes: &a.nodes }],
            version: CoreVersion { major: 1, minor: 14, patch: 4 },
            rule_set_dir: None,
            providers: &[],
        });
        out.config["outbounds"].as_array().unwrap().iter().find(|o| o["tag"] == TAG_AUTO).unwrap().clone()
    }

    #[test]
    fn auto_is_one_selector_whatever_the_mode() {
        // Mode and region are applied through the Clash API, never the config,
        // so switching them cannot restart the core.
        let mut s = Settings::default();
        let base = auto_group(&s);
        assert_eq!(base["type"], "selector");
        assert_eq!(base["outbounds"], json!(["HK-1", "JP-1", "JP-2"]));
        assert_eq!(base["interrupt_exist_connections"], false);
        s.auto_strategy = crate::model::AutoStrategy::Ordered;
        s.auto_region = "JP".into();
        assert_eq!(auto_group(&s), base);
    }

    #[test]
    fn udp_and_quic_rules() {
        let mut s = Settings::default();
        let rules = |s: &Settings| {
            build_config(&BuildInput {
                settings: s,
                groups: vec![],
                version: CoreVersion { major: 1, minor: 14, patch: 4 },
                rule_set_dir: None,
                providers: &[],
            })
            .config["route"]["rules"]
                .clone()
        };
        let default = rules(&s).to_string();
        assert!(!default.contains("quic") && !default.contains("\"network\":\"udp\""));
        s.block_quic = true;
        s.proxy_udp = false;
        let r = rules(&s);
        // Right after sniff + DNS hijack, ahead of every routing rule.
        assert_eq!(r[2], json!({ "protocol": "quic", "action": "reject" }));
        assert_eq!(r[3], json!({ "network": "udp", "outbound": TAG_DIRECT }));
    }
}
