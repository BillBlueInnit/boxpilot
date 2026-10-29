//! Share-link export: the reverse of [`super::uri`], so any node (from a link,
//! Clash YAML or sing-box JSON) can be handed to another client.

use base64::Engine;
use base64::engine::general_purpose;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

use crate::model::Node;

/// Everything but RFC 3986 unreserved characters.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, COMPONENT).to_string()
}

fn host(server: &str) -> String {
    if server.contains(':') { format!("[{server}]") } else { server.to_string() }
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A string or a list of strings, comma joined.
fn list(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","),
        _ => String::new(),
    }
}

/// Ordered query parameters; empty values are left out.
#[derive(Default)]
struct Query(Vec<(&'static str, String)>);

impl Query {
    fn add(&mut self, key: &'static str, value: impl Into<String>) {
        let value = value.into();
        if !value.is_empty() {
            self.0.push((key, value));
        }
    }

    fn render(&self) -> String {
        if self.0.is_empty() {
            return String::new();
        }
        let pairs: Vec<String> = self.0.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect();
        format!("?{}", pairs.join("&"))
    }
}

struct Tls {
    enabled: bool,
    sni: String,
    insecure: bool,
    alpn: String,
    fp: String,
    pbk: String,
    sid: String,
}

fn tls(out: &Value) -> Tls {
    let t = out.get("tls").cloned().unwrap_or(Value::Null);
    let reality = t.get("reality").filter(|r| flag(r, "enabled"));
    Tls {
        enabled: flag(&t, "enabled"),
        sni: s(&t, "server_name").into(),
        insecure: flag(&t, "insecure"),
        alpn: list(&t, "alpn"),
        fp: t.get("utls").filter(|u| flag(u, "enabled")).map(|u| s(u, "fingerprint").to_string()).unwrap_or_default(),
        pbk: reality.map(|r| s(r, "public_key").to_string()).unwrap_or_default(),
        sid: reality.map(|r| s(r, "short_id").to_string()).unwrap_or_default(),
    }
}

/// The SNI to write into a v2ray style link. Those clients fall back to the
/// transport host rather than the server, so an unset name is spelled out.
fn v2ray_sni(t: &Tls, server: &str) -> String {
    if t.sni.is_empty() { server.to_string() } else { t.sni.clone() }
}

/// v2ray style `security=…&sni=…` parameters.
fn tls_query(q: &mut Query, t: &Tls, server: &str) {
    q.add("security", if !t.enabled { "none" } else if t.pbk.is_empty() { "tls" } else { "reality" });
    if t.enabled {
        q.add("sni", v2ray_sni(t, server));
        q.add("alpn", t.alpn.clone());
        q.add("fp", t.fp.clone());
        q.add("pbk", t.pbk.clone());
        q.add("sid", t.sid.clone());
        if t.insecure {
            q.add("allowInsecure", "1");
        }
    }
}

/// A transport in v2ray link terms.
#[derive(Default)]
struct Transport {
    /// `tcp`, `ws`, `grpc`, `http`, `httpupgrade` or `quic`.
    net: String,
    host: String,
    /// The path, with v2ray style `?ed=` early data folded in for WebSocket.
    path: String,
    service_name: String,
    /// `http` for plain-TCP HTTP obfuscation.
    header_type: String,
}

fn transport(out: &Value, tls_on: bool) -> Transport {
    let Some(t) = out.get("transport") else {
        return Transport { net: "tcp".into(), ..Default::default() };
    };
    match s(t, "type") {
        "ws" => {
            let mut path = s(t, "path").to_string();
            let early = t.get("max_early_data").and_then(Value::as_u64).unwrap_or(0);
            if early > 0 && s(t, "early_data_header_name") == "Sec-WebSocket-Protocol" {
                path = format!("{path}{}ed={early}", if path.contains('?') { '&' } else { '?' });
            }
            let host = t.get("headers").map(|h| s(h, "Host").to_string()).unwrap_or_default();
            Transport { net: "ws".into(), host, path, ..Default::default() }
        }
        "grpc" => Transport { net: "grpc".into(), service_name: s(t, "service_name").into(), ..Default::default() },
        // Without TLS sing-box's HTTP transport is what v2ray calls TCP with an HTTP header.
        "http" if !tls_on => Transport {
            net: "tcp".into(),
            host: list(t, "host"),
            path: s(t, "path").into(),
            header_type: "http".into(),
            ..Default::default()
        },
        "http" => Transport { net: "http".into(), host: list(t, "host"), path: s(t, "path").into(), ..Default::default() },
        "httpupgrade" => Transport {
            net: "httpupgrade".into(),
            host: s(t, "host").into(),
            path: s(t, "path").into(),
            ..Default::default()
        },
        "quic" => Transport { net: "quic".into(), ..Default::default() },
        _ => Transport { net: "tcp".into(), ..Default::default() },
    }
}

fn transport_query(q: &mut Query, t: &Transport) {
    q.add("type", t.net.clone());
    q.add("headerType", t.header_type.clone());
    q.add("host", t.host.clone());
    q.add("path", t.path.clone());
    q.add("serviceName", t.service_name.clone());
}

/// A share link for `node`, or why it has none.
pub fn share_link(node: &Node) -> Result<String, String> {
    let out = &node.outbound;
    let name = enc(&node.name);
    let server = host(&node.server);
    let port = node.port;
    let link = match node.protocol.as_str() {
        "shadowsocks" => {
            let method = s(out, "method");
            let password = s(out, "password");
            // SIP002: 2022 ciphers keep plain credentials, the rest are base64url.
            let user = if method.starts_with("2022-") {
                format!("{}:{}", enc(method), enc(password))
            } else {
                general_purpose::URL_SAFE_NO_PAD.encode(format!("{method}:{password}"))
            };
            let mut q = Query::default();
            let plugin = s(out, "plugin");
            if !plugin.is_empty() {
                let opts = s(out, "plugin_opts");
                q.add("plugin", if opts.is_empty() { plugin.to_string() } else { format!("{plugin};{opts}") });
            }
            if flag(out, "udp_over_tcp") {
                q.add("uot", "1");
            }
            let q = q.render();
            let slash = if q.is_empty() { "" } else { "/" };
            format!("ss://{user}@{server}:{port}{slash}{q}#{name}")
        }
        "vmess" => {
            let t = tls(out);
            let tr = transport(out, t.enabled);
            let path = if tr.net == "grpc" { tr.service_name.clone() } else { tr.path.clone() };
            let v = json!({
                "v": "2",
                "ps": node.name,
                "add": node.server,
                "port": port.to_string(),
                "id": s(out, "uuid"),
                "aid": out.get("alter_id").and_then(Value::as_u64).unwrap_or(0).to_string(),
                "scy": match s(out, "security") { "" => "auto", x => x },
                "net": tr.net,
                "type": if tr.header_type.is_empty() { "none" } else { tr.header_type.as_str() },
                "host": tr.host,
                "path": path,
                "tls": if t.enabled { "tls" } else { "" },
                "sni": if t.enabled { v2ray_sni(&t, &node.server) } else { String::new() },
                "alpn": t.alpn,
                "fp": t.fp,
                "allowInsecure": if t.insecure { "1" } else { "" },
            });
            format!("vmess://{}", general_purpose::STANDARD.encode(v.to_string()))
        }
        "vless" => {
            let t = tls(out);
            let mut q = Query::default();
            q.add("encryption", "none");
            q.add("flow", s(out, "flow"));
            match s(out, "packet_encoding") {
                "" | "xudp" => {}
                other => q.add("packetEncoding", other),
            }
            tls_query(&mut q, &t, &node.server);
            transport_query(&mut q, &transport(out, t.enabled));
            format!("vless://{}@{server}:{port}{}#{name}", enc(s(out, "uuid")), q.render())
        }
        "trojan" => {
            let t = tls(out);
            let mut q = Query::default();
            tls_query(&mut q, &t, &node.server);
            transport_query(&mut q, &transport(out, t.enabled));
            format!("trojan://{}@{server}:{port}{}#{name}", enc(s(out, "password")), q.render())
        }
        "hysteria2" => {
            let t = tls(out);
            let mut q = Query::default();
            q.add("sni", t.sni.clone());
            q.add("alpn", t.alpn.clone());
            if t.insecure {
                q.add("insecure", "1");
            }
            if let Some(obfs) = out.get("obfs") {
                q.add("obfs", s(obfs, "type"));
                q.add("obfs-password", s(obfs, "password"));
            }
            let hops: Vec<String> = out
                .get("server_ports")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|r| match r.split_once(':') {
                    Some((a, b)) if a == b => a.to_string(),
                    Some((a, b)) => format!("{a}-{b}"),
                    None => r.to_string(),
                })
                .collect();
            q.add("mport", hops.join(","));
            q.add("hop_interval", s(out, "hop_interval"));
            if let Some(up) = out.get("up_mbps").and_then(Value::as_u64) {
                q.add("upmbps", up.to_string());
            }
            if let Some(down) = out.get("down_mbps").and_then(Value::as_u64) {
                q.add("downmbps", down.to_string());
            }
            format!("hysteria2://{}@{server}:{port}/{}#{name}", enc(s(out, "password")), q.render())
        }
        "hysteria" => {
            let t = tls(out);
            let mut q = Query::default();
            q.add("auth", s(out, "auth_str"));
            q.add("peer", t.sni.clone());
            q.add("alpn", t.alpn.clone());
            if t.insecure {
                q.add("insecure", "1");
            }
            q.add("obfsParam", s(out, "obfs"));
            for (key, field) in [("upmbps", "up_mbps"), ("downmbps", "down_mbps")] {
                if let Some(v) = out.get(field).and_then(Value::as_u64) {
                    q.add(key, v.to_string());
                }
            }
            format!("hysteria://{server}:{port}{}#{name}", q.render())
        }
        "tuic" => {
            let t = tls(out);
            let mut q = Query::default();
            q.add("congestion_control", s(out, "congestion_control"));
            q.add("udp_relay_mode", s(out, "udp_relay_mode"));
            q.add("sni", t.sni.clone());
            q.add("alpn", t.alpn.clone());
            if t.insecure {
                q.add("allow_insecure", "1");
            }
            if out.get("tls").is_some_and(|t| flag(t, "disable_sni")) {
                q.add("disable_sni", "1");
            }
            if flag(out, "zero_rtt_handshake") {
                q.add("reduce_rtt", "1");
            }
            let user = format!("{}:{}", enc(s(out, "uuid")), enc(s(out, "password")));
            format!("tuic://{user}@{server}:{port}{}#{name}", q.render())
        }
        "anytls" => {
            let t = tls(out);
            let mut q = Query::default();
            q.add("sni", t.sni.clone());
            q.add("alpn", t.alpn.clone());
            q.add("fp", t.fp.clone());
            if t.insecure {
                q.add("insecure", "1");
            }
            format!("anytls://{}@{server}:{port}{}#{name}", enc(s(out, "password")), q.render())
        }
        "socks" => {
            let user = s(out, "username");
            let creds = if user.is_empty() {
                String::new()
            } else {
                format!("{}:{}@", enc(user), enc(s(out, "password")))
            };
            format!("socks5://{creds}{server}:{port}#{name}")
        }
        "http" => {
            let user = s(out, "username");
            let creds = if user.is_empty() {
                String::new()
            } else {
                format!("{}:{}@", enc(user), enc(s(out, "password")))
            };
            let t = tls(out);
            let mut q = Query::default();
            let scheme = if t.enabled {
                q.add("sni", t.sni.clone());
                if t.insecure {
                    q.add("insecure", "1");
                }
                "https"
            } else {
                "http"
            };
            format!("{scheme}://{creds}{server}:{port}{}#{name}", q.render())
        }
        "wireguard" => {
            let peer = out.get("peers").and_then(|p| p.get(0)).cloned().unwrap_or(Value::Null);
            let mut q = Query::default();
            q.add("publickey", s(&peer, "public_key"));
            q.add("presharedkey", s(&peer, "pre_shared_key"));
            q.add("address", list(out, "address"));
            let reserved: Vec<String> = peer
                .get("reserved")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .map(|b| b.to_string())
                .collect();
            q.add("reserved", reserved.join(","));
            if let Some(mtu) = out.get("mtu").and_then(Value::as_u64) {
                q.add("mtu", mtu.to_string());
            }
            format!("wireguard://{}@{server}:{port}{}#{name}", enc(s(out, "private_key")), q.render())
        }
        "ssh" if s(out, "private_key").is_empty() && s(out, "private_key_path").is_empty() => {
            format!("ssh://{}:{}@{server}:{port}#{name}", enc(s(out, "user")), enc(s(out, "password")))
        }
        other => return Err(format!("{other} nodes have no share link")),
    };
    Ok(link)
}

/// The share link of a finished outbound (or endpoint) from a config.
pub fn outbound_share_link(name: &str, ob: &Value) -> Result<String, String> {
    let peer = ob.get("peers").and_then(|p| p.get(0));
    let server = ob.get("server").or_else(|| peer.and_then(|p| p.get("address")));
    let port = ob.get("server_port").or_else(|| peer.and_then(|p| p.get("port")));
    let mut outbound = ob.clone();
    if let Some(o) = outbound.as_object_mut() {
        o.remove("tag");
    }
    share_link(&Node {
        name: name.to_string(),
        protocol: s(ob, "type").to_string(),
        server: server.and_then(Value::as_str).unwrap_or_default().to_string(),
        port: port.and_then(Value::as_u64).unwrap_or(0) as u16,
        outbound,
    })
}

/// Links for every node that has one, one per line, plus the names left out.
pub fn share_links(nodes: &[Node]) -> (String, Vec<String>) {
    let mut lines = Vec::new();
    let mut skipped = Vec::new();
    for n in nodes {
        match share_link(n) {
            Ok(l) => lines.push(l),
            Err(_) => skipped.push(n.name.clone()),
        }
    }
    (lines.join("\n"), skipped)
}

#[cfg(test)]
mod tests {
    use super::super::parse_subscription;
    use super::super::uri::parse_uri;
    use super::*;

    /// An unset TLS server name means the server address.
    fn normalized(n: &Node) -> Value {
        let mut out = n.outbound.clone();
        if let Some(tls) = out.get_mut("tls").and_then(Value::as_object_mut) {
            tls.entry("server_name").or_insert_with(|| json!(n.server));
        }
        out
    }

    /// Exporting then importing gives back the same node.
    fn round_trip(link: &str) {
        let a = parse_uri(link).unwrap_or_else(|e| panic!("{link}: {e}"));
        let shared = share_link(&a).unwrap();
        let b = parse_uri(&shared).unwrap_or_else(|e| panic!("{shared}: {e}"));
        assert_eq!(a.name, b.name, "{shared}");
        assert_eq!((a.server.as_str(), a.port), (b.server.as_str(), b.port), "{shared}");
        assert_eq!(normalized(&a), normalized(&b), "\n{link}\n{shared}");
    }

    #[test]
    fn links_round_trip() {
        for link in [
            "vless://11111111-2222-3333-4444-555555555555@example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.apple.com&fp=chrome&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=ab12&type=tcp#Reality",
            "vless://11111111-2222-3333-4444-555555555555@example.com:443?security=tls&type=ws&host=cdn.com&path=%2Fws%3Fed%3D2048#VLESS WS",
            "vless://11111111-2222-3333-4444-555555555555@example.com:443?security=tls&type=grpc&serviceName=svc&alpn=h2,http/1.1#VLESS gRPC",
            "vless://11111111-2222-3333-4444-555555555555@example.com:443?security=tls&type=httpupgrade&host=a.com&path=/up#VLESS HU",
            "vless://11111111-2222-3333-4444-555555555555@[2001:db8::1]:8443?security=none&type=tcp&headerType=http&host=a.com&path=/#V6 HTTP",
            "trojan://p%40ss@t.com:443?sni=t.com&allowInsecure=1#Trojan 中文",
            "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@1.2.3.4:8388#SS",
            "ss://2022-blake3-aes-128-gcm:YWJjZGVmZ2hpamtsbW5vcA%3D%3D@h.com:443/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dbing.com#SS2022 obfs",
            "hy2://pass@h.com:443,20000-30000/?sni=h.com&insecure=1&obfs=salamander&obfs-password=x&upmbps=50#Hy2",
            "hysteria://h.com:443?auth=x&upmbps=10&downmbps=50&peer=h.com&obfsParam=o#Hy1",
            "tuic://11111111-2222-3333-4444-555555555555:pw@t.com:443?congestion_control=bbr&alpn=h3&sni=t.com&reduce_rtt=1#TUIC",
            "socks5://u:p@1.1.1.1:1080#Socks",
            "socks5://1.1.1.1:1080#Open",
            "https://u:p@p.com:443?sni=p.com#HTTPS",
            "http://p.com:8080#HTTP",
            "anytls://pw@a.com:443?sni=a.com&fp=chrome#AnyTLS",
            "wireguard://yAnz5TF%2BlXXJte14tji3zlMNq%2BhdD5XbJ0YiprL1ccU%3D@162.159.192.1:2408?publickey=bmXOC%2BF1FxEMF9dyiK2H5%2F1SUtzH0JuVo51h2wPfgyo%3D&address=172.16.0.2%2F32,2606:4700:110:8a36::2%2F128&reserved=1,2,3&mtu=1280#WARP",
            "ssh://root:pw@s.com:22#SSH",
        ] {
            round_trip(link);
        }
    }

    #[test]
    fn vmess_round_trips() {
        let j = r#"{"v":"2","ps":"HK 01","add":"hk.example.com","port":"8443","id":"uuid-x","aid":"0","scy":"auto","net":"ws","type":"none","host":"cdn.example.com","path":"/ray?ed=2048","tls":"tls","sni":"","alpn":"h2"}"#;
        round_trip(&format!("vmess://{}", general_purpose::STANDARD.encode(j)));
        let j = r#"{"v":"2","ps":"gRPC","add":"g.com","port":443,"id":"u","aid":0,"net":"grpc","path":"svc","tls":"tls","sni":"g.com","fp":"chrome"}"#;
        round_trip(&format!("vmess://{}", general_purpose::STANDARD.encode(j)));
    }

    #[test]
    fn clash_nodes_export() {
        let yaml = r#"
proxies:
  - { name: "VMess H2", type: vmess, server: v.com, port: 443, uuid: 11111111-2222-3333-4444-555555555555, alterId: 0, cipher: auto, tls: true, network: h2, h2-opts: { host: [v.com], path: /h2 } }
  - { name: "SS v2ray-plugin", type: ss, server: s.com, port: 443, cipher: aes-128-gcm, password: pw, plugin: v2ray-plugin, plugin-opts: { mode: websocket, tls: true, host: s.com, path: /ws } }
"#;
        let parsed = parse_subscription(yaml).unwrap();
        for node in &parsed.nodes {
            let link = share_link(node).unwrap();
            let back = parse_uri(&link).unwrap();
            assert_eq!(normalized(node), normalized(&back), "{link}");
        }
    }

    #[test]
    fn unsupported_protocols_are_named() {
        let node = Node {
            name: "Tor".into(),
            protocol: "tor".into(),
            server: String::new(),
            port: 0,
            outbound: json!({ "type": "tor" }),
        };
        assert!(share_link(&node).is_err());
        let (text, skipped) = share_links(&[node]);
        assert!(text.is_empty());
        assert_eq!(skipped, ["Tor"]);
    }
}
