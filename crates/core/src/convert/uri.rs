//! Share-link (v2rayN / Shadowrocket / NekoBox style) parsing.

use serde_json::{Map, Value, json};

use super::util::*;
use crate::model::Node;

pub fn parse_uri(line: &str) -> Result<Node, String> {
    let line = line.trim();
    let scheme = line
        .split_once("://")
        .map(|(s, _)| s.to_ascii_lowercase())
        .ok_or("not a link")?;
    match scheme.as_str() {
        "vmess" => parse_vmess(line),
        "vless" => parse_vless(line),
        "trojan" => parse_trojan(line),
        "ss" => parse_ss(line),
        "hysteria2" | "hy2" => parse_hysteria2(line),
        "hysteria" | "hy" => parse_hysteria(line),
        "tuic" => parse_tuic(line),
        "socks" | "socks5" | "socks5h" => parse_socks(line),
        "http" | "https" => parse_http(line),
        "anytls" => parse_anytls(line),
        "wireguard" | "wg" => parse_wireguard(line),
        "ssh" => parse_ssh(line),
        "ssr" => Err("ShadowsocksR is not supported by sing-box".into()),
        other => Err(format!("unsupported scheme `{other}`")),
    }
}

fn node(name: &str, protocol: &str, server: &str, port: u16, out: Map<String, Value>) -> Node {
    let name = if name.trim().is_empty() {
        format!("{server}:{port}")
    } else {
        name.trim().to_string()
    };
    let mut out = out;
    out.insert("type".into(), json!(protocol));
    out.insert("server".into(), json!(server));
    out.insert("server_port".into(), json!(port));
    Node {
        name,
        protocol: protocol.into(),
        server: server.into(),
        port,
        outbound: Value::Object(out),
    }
}

fn require_host_port(u: &UriParts) -> Result<(String, u16), String> {
    if u.host.is_empty() {
        return Err("missing server".into());
    }
    let port = u.port_u16().ok_or("missing or invalid port")?;
    Ok((u.host.clone(), port))
}

/// TLS settings from v2ray style query parameters.
fn tls_from_query(u: &UriParts, default_enabled: bool) -> TlsOpts {
    let security = u.q("security").unwrap_or("").to_ascii_lowercase();
    let enabled = match security.as_str() {
        "tls" | "reality" | "xtls" => true,
        "none" => false,
        _ => default_enabled,
    };
    TlsOpts {
        enabled,
        server_name: u.q_any(&["sni", "peer", "servername"]).map(String::from),
        insecure: u.q_bool(&["allowInsecure", "insecure", "skip-cert-verify", "allow_insecure"]),
        alpn: u.q("alpn").map(split_list).unwrap_or_default(),
        fingerprint: u.q_any(&["fp", "fingerprint"]).map(String::from),
        reality_public_key: u.q_any(&["pbk", "publicKey"]).map(String::from),
        reality_short_id: u.q_any(&["sid", "shortId"]).map(String::from),
        disable_sni: false,
    }
}

fn transport_from_query(u: &UriParts) -> TransportOpts {
    TransportOpts {
        network: u.q_any(&["type", "net", "network"]).unwrap_or("tcp").into(),
        host: u.q_any(&["host", "obfsParam"]).map(String::from),
        path: u.q("path").map(String::from),
        service_name: u.q_any(&["serviceName", "service_name"]).map(String::from),
        header_type: u.q("headerType").map(String::from),
        method: None,
        headers: vec![],
        max_early_data: u.q("ed").and_then(|v| v.parse().ok()),
        early_data_header: u.q("eh").map(String::from),
    }
}

fn apply_tls_transport(
    out: &mut Map<String, Value>,
    tls: TlsOpts,
    transport: TransportOpts,
) -> Result<(), String> {
    if let Some(tls) = tls.to_value() {
        out.insert("tls".into(), tls);
    }
    if let Some(t) = transport.to_value()? {
        out.insert("transport".into(), t);
    }
    Ok(())
}

fn parse_vmess(line: &str) -> Result<Node, String> {
    let body = &line["vmess://".len()..];
    let body_no_frag = body.split('#').next().unwrap_or(body);
    if let Some(decoded) = b64_decode(body_no_frag) {
        if let Ok(v) = serde_json::from_str::<Value>(&decoded) {
            return parse_vmess_json(&v);
        }
    }
    // Fallback: vmess://uuid@host:port?type=ws&security=tls#name
    let u = UriParts::parse(line).ok_or("invalid vmess link")?;
    let (server, port) = require_host_port(&u)?;
    let mut out = Map::new();
    out.insert("uuid".into(), json!(u.userinfo_decoded().unwrap_or_default()));
    out.insert(
        "security".into(),
        json!(u.q("encryption").unwrap_or("auto")),
    );
    out.insert(
        "alter_id".into(),
        json!(u.q("aid").and_then(|a| a.parse::<u32>().ok()).unwrap_or(0)),
    );
    apply_tls_transport(&mut out, tls_from_query(&u, false), transport_from_query(&u))?;
    Ok(node(&u.fragment, "vmess", &server, port, out))
}

fn json_str(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn parse_vmess_json(v: &Value) -> Result<Node, String> {
    let server = json_str(v, "add");
    let port: u16 = json_str(v, "port").parse().map_err(|_| "invalid port")?;
    if server.is_empty() {
        return Err("missing server".into());
    }
    let mut out = Map::new();
    out.insert("uuid".into(), json!(json_str(v, "id")));
    let scy = json_str(v, "scy");
    out.insert(
        "security".into(),
        json!(if scy.is_empty() { "auto".into() } else { scy }),
    );
    out.insert(
        "alter_id".into(),
        json!(json_str(v, "aid").parse::<u32>().unwrap_or(0)),
    );
    let tls_flag = json_str(v, "tls").to_ascii_lowercase();
    let host = json_str(v, "host");
    let sni = json_str(v, "sni");
    let tls = TlsOpts {
        enabled: tls_flag == "tls" || tls_flag == "reality",
        server_name: Some(if sni.is_empty() { host.clone() } else { sni })
            .filter(|s| !s.is_empty()),
        insecure: truthy(&json_str(v, "allowInsecure")) || truthy(&json_str(v, "skip-cert-verify")),
        alpn: split_list(&json_str(v, "alpn")),
        fingerprint: Some(json_str(v, "fp")).filter(|s| !s.is_empty()),
        reality_public_key: Some(json_str(v, "pbk")).filter(|s| !s.is_empty()),
        reality_short_id: Some(json_str(v, "sid")).filter(|s| !s.is_empty()),
        disable_sni: false,
    };
    let net = json_str(v, "net");
    let path = json_str(v, "path");
    let transport = TransportOpts {
        network: net.clone(),
        host: Some(host).filter(|s| !s.is_empty()),
        path: Some(path.clone()).filter(|s| !s.is_empty()),
        service_name: (net == "grpc").then_some(path).filter(|s| !s.is_empty()),
        header_type: Some(json_str(v, "type")).filter(|s| !s.is_empty()),
        ..Default::default()
    };
    apply_tls_transport(&mut out, tls, transport)?;
    Ok(node(&json_str(v, "ps"), "vmess", &server, port, out))
}

fn parse_vless(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid vless link")?;
    let (server, port) = require_host_port(&u)?;
    let mut out = Map::new();
    out.insert("uuid".into(), json!(u.userinfo_decoded().unwrap_or_default()));
    if let Some(flow) = u.q("flow") {
        out.insert("flow".into(), json!(flow));
    }
    out.insert(
        "packet_encoding".into(),
        json!(u.q("packetEncoding").unwrap_or("xudp")),
    );
    apply_tls_transport(&mut out, tls_from_query(&u, false), transport_from_query(&u))?;
    Ok(node(&u.fragment, "vless", &server, port, out))
}

fn parse_trojan(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid trojan link")?;
    let (server, port) = require_host_port(&u)?;
    let mut out = Map::new();
    out.insert(
        "password".into(),
        json!(u.userinfo_decoded().unwrap_or_default()),
    );
    apply_tls_transport(&mut out, tls_from_query(&u, true), transport_from_query(&u))?;
    Ok(node(&u.fragment, "trojan", &server, port, out))
}

fn parse_ss(line: &str) -> Result<Node, String> {
    let body = &line["ss://".len()..];
    let (main, fragment) = match body.split_once('#') {
        Some((m, f)) => (m, pct_decode(f.trim())),
        None => (body, String::new()),
    };
    // Legacy: ss://BASE64(method:password@host:port)
    let normalized = if !main.contains('@') {
        let b64 = main.split(['?', '/']).next().unwrap_or(main);
        let decoded = b64_decode(b64).ok_or("invalid ss link")?;
        let tail = &main[b64.len()..];
        format!("ss://{decoded}{tail}")
    } else {
        format!("ss://{main}")
    };
    let u = UriParts::parse(&normalized).ok_or("invalid ss link")?;
    let (server, port) = require_host_port(&u)?;
    let userinfo = u.userinfo.clone().ok_or("missing ss credentials")?;
    let creds = if userinfo.contains(':') {
        pct_decode(&userinfo)
    } else {
        b64_decode(&pct_decode(&userinfo)).ok_or("invalid ss credentials")?
    };
    let (method, password) = creds.split_once(':').ok_or("invalid ss credentials")?;
    let mut out = Map::new();
    out.insert("method".into(), json!(method));
    out.insert("password".into(), json!(password));
    if let Some(plugin) = u.q("plugin") {
        let (name, opts) = plugin.split_once(';').unwrap_or((plugin, ""));
        let name = match name {
            "simple-obfs" | "obfs-local" | "obfs" => "obfs-local",
            "v2ray-plugin" => "v2ray-plugin",
            other => return Err(format!("unsupported ss plugin `{other}`")),
        };
        out.insert("plugin".into(), json!(name));
        put(&mut out, "plugin_opts", opts);
    }
    if u.q_bool(&["uot", "udp-over-tcp"]) {
        out.insert("udp_over_tcp".into(), json!(true));
    }
    Ok(node(&fragment, "shadowsocks", &server, port, out))
}

fn parse_hysteria2(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid hysteria2 link")?;
    let (server, port) = require_host_port(&u)?;
    let mut out = Map::new();
    put(
        &mut out,
        "password",
        u.userinfo_decoded().or_else(|| u.q("auth").map(String::from)).unwrap_or_default(),
    );
    // Port hopping: either `host:443,20000-30000` or `mport=20000-30000`.
    let mut hops = Vec::new();
    if u.port.contains([',', '-']) {
        hops.extend(port_ranges(&u.port));
    }
    if let Some(mport) = u.q_any(&["mport", "ports"]) {
        hops.extend(port_ranges(mport));
    }
    if !hops.is_empty() {
        out.insert("server_ports".into(), json!(hops));
        if let Some(i) = u.q_any(&["hop_interval", "hopInterval"]) {
            let i = if i.chars().all(|c| c.is_ascii_digit()) {
                format!("{i}s")
            } else {
                i.to_string()
            };
            out.insert("hop_interval".into(), json!(i));
        }
    }
    if let Some(obfs) = u.q("obfs").filter(|o| *o != "none") {
        out.insert(
            "obfs".into(),
            json!({ "type": obfs, "password": u.q("obfs-password").unwrap_or("") }),
        );
    }
    if let Some(up) = u.q_any(&["upmbps", "up"]).and_then(parse_mbps) {
        out.insert("up_mbps".into(), json!(up));
    }
    if let Some(down) = u.q_any(&["downmbps", "down"]).and_then(parse_mbps) {
        out.insert("down_mbps".into(), json!(down));
    }
    let tls = TlsOpts {
        enabled: true,
        server_name: u.q_any(&["sni", "peer"]).map(String::from),
        insecure: u.q_bool(&["insecure", "allowInsecure"]),
        alpn: u.q("alpn").map(split_list).unwrap_or_default(),
        ..Default::default()
    };
    out.insert("tls".into(), tls.to_value().unwrap());
    Ok(node(&u.fragment, "hysteria2", &server, port, out))
}

fn parse_hysteria(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid hysteria link")?;
    let (server, port) = require_host_port(&u)?;
    let mut out = Map::new();
    if let Some(auth) = u.q("auth") {
        out.insert("auth_str".into(), json!(auth));
    }
    out.insert(
        "up_mbps".into(),
        json!(u.q_any(&["upmbps", "up"]).and_then(parse_mbps).unwrap_or(10)),
    );
    out.insert(
        "down_mbps".into(),
        json!(u.q_any(&["downmbps", "down"]).and_then(parse_mbps).unwrap_or(50)),
    );
    if let Some(obfs) = u.q("obfsParam").or(u.q("obfs").filter(|o| *o != "xplus")) {
        out.insert("obfs".into(), json!(obfs));
    }
    if u.port.contains([',', '-']) {
        out.insert("server_ports".into(), json!(port_ranges(&u.port)));
    }
    let tls = TlsOpts {
        enabled: true,
        server_name: u.q_any(&["peer", "sni"]).map(String::from),
        insecure: u.q_bool(&["insecure", "allowInsecure"]),
        alpn: u.q("alpn").map(split_list).unwrap_or_default(),
        ..Default::default()
    };
    out.insert("tls".into(), tls.to_value().unwrap());
    Ok(node(&u.fragment, "hysteria", &server, port, out))
}

fn parse_tuic(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid tuic link")?;
    let (server, port) = require_host_port(&u)?;
    let creds = u.userinfo_decoded().unwrap_or_default();
    let (uuid, password) = creds.split_once(':').unwrap_or((&creds, ""));
    let mut out = Map::new();
    out.insert("uuid".into(), json!(uuid));
    put(
        &mut out,
        "password",
        if password.is_empty() { u.q("password").unwrap_or("") } else { password },
    );
    if let Some(cc) = u.q_any(&["congestion_control", "congestion-control", "congestion"]) {
        out.insert("congestion_control".into(), json!(cc));
    }
    if let Some(mode) = u.q_any(&["udp_relay_mode", "udp-relay-mode"]) {
        out.insert("udp_relay_mode".into(), json!(mode));
    }
    if u.q_bool(&["reduce_rtt", "zero_rtt_handshake"]) {
        out.insert("zero_rtt_handshake".into(), json!(true));
    }
    let tls = TlsOpts {
        enabled: true,
        server_name: u.q_any(&["sni", "peer"]).map(String::from),
        insecure: u.q_bool(&["allow_insecure", "insecure", "allowInsecure"]),
        alpn: u.q("alpn").map(split_list).unwrap_or_default(),
        disable_sni: u.q_bool(&["disable_sni"]),
        ..Default::default()
    };
    out.insert("tls".into(), tls.to_value().unwrap());
    Ok(node(&u.fragment, "tuic", &server, port, out))
}

fn credentials(u: &UriParts) -> (String, String) {
    let Some(raw) = u.userinfo.as_deref() else {
        return Default::default();
    };
    let decoded = if raw.contains(':') {
        pct_decode(raw)
    } else {
        b64_decode(&pct_decode(raw)).unwrap_or_else(|| pct_decode(raw))
    };
    match decoded.split_once(':') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (decoded, String::new()),
    }
}

fn parse_socks(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid socks link")?;
    let (server, port) = require_host_port(&u)?;
    let (user, pass) = credentials(&u);
    let mut out = Map::new();
    out.insert("version".into(), json!("5"));
    put(&mut out, "username", user);
    put(&mut out, "password", pass);
    Ok(node(&u.fragment, "socks", &server, port, out))
}

fn parse_http(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid http link")?;
    let (server, port) = require_host_port(&u)?;
    let (user, pass) = credentials(&u);
    let mut out = Map::new();
    put(&mut out, "username", user);
    put(&mut out, "password", pass);
    if u.scheme == "https" {
        let tls = TlsOpts {
            enabled: true,
            server_name: u.q("sni").map(String::from),
            insecure: u.q_bool(&["insecure", "allowInsecure", "skip-cert-verify"]),
            ..Default::default()
        };
        out.insert("tls".into(), tls.to_value().unwrap());
    }
    Ok(node(&u.fragment, "http", &server, port, out))
}

fn parse_anytls(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid anytls link")?;
    let (server, port) = require_host_port(&u)?;
    let mut out = Map::new();
    out.insert(
        "password".into(),
        json!(u.userinfo_decoded().unwrap_or_default()),
    );
    let mut tls = tls_from_query(&u, true);
    tls.enabled = true;
    out.insert("tls".into(), tls.to_value().unwrap());
    Ok(node(&u.fragment, "anytls", &server, port, out))
}

fn parse_wireguard(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid wireguard link")?;
    let (server, port) = require_host_port(&u)?;
    let private_key = u
        .userinfo_decoded()
        .filter(|k| !k.is_empty())
        .or_else(|| u.q_any(&["privatekey", "private_key", "secretKey"]).map(String::from))
        .ok_or("missing private key")?;
    let public_key = u
        .q_any(&["publickey", "public_key", "peer_public_key", "publicKey"])
        .ok_or("missing peer public key")?;
    let address: Vec<String> = u
        .q_any(&["address", "ip", "local_address"])
        .map(split_list)
        .unwrap_or_default()
        .into_iter()
        .map(|a| {
            if a.contains('/') {
                a
            } else if a.contains(':') {
                format!("{a}/128")
            } else {
                format!("{a}/32")
            }
        })
        .collect();
    let reserved: Vec<u8> = u
        .q("reserved")
        .map(|r| r.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default();
    Ok(wireguard_node(
        &u.fragment,
        &server,
        port,
        WireguardParams {
            private_key,
            public_key: public_key.into(),
            pre_shared_key: u.q_any(&["presharedkey", "pre_shared_key"]).map(String::from),
            address,
            reserved,
            mtu: u.q("mtu").and_then(|m| m.parse().ok()),
        },
    ))
}

pub struct WireguardParams {
    pub private_key: String,
    pub public_key: String,
    pub pre_shared_key: Option<String>,
    pub address: Vec<String>,
    pub reserved: Vec<u8>,
    pub mtu: Option<u32>,
}

pub fn wireguard_node(name: &str, server: &str, port: u16, p: WireguardParams) -> Node {
    let mut peer = Map::new();
    peer.insert("address".into(), json!(server));
    peer.insert("port".into(), json!(port));
    peer.insert("public_key".into(), json!(p.public_key));
    if let Some(psk) = p.pre_shared_key.filter(|k| !k.is_empty()) {
        peer.insert("pre_shared_key".into(), json!(psk));
    }
    peer.insert("allowed_ips".into(), json!(["0.0.0.0/0", "::/0"]));
    if p.reserved.len() == 3 {
        peer.insert("reserved".into(), json!(p.reserved));
    }
    let mut out = Map::new();
    out.insert("type".into(), json!("wireguard"));
    out.insert(
        "address".into(),
        json!(if p.address.is_empty() { vec!["172.16.0.2/32".to_string()] } else { p.address }),
    );
    out.insert("private_key".into(), json!(p.private_key));
    if let Some(mtu) = p.mtu {
        out.insert("mtu".into(), json!(mtu));
    }
    out.insert("peers".into(), json!([Value::Object(peer)]));
    let name = if name.trim().is_empty() {
        format!("{server}:{port}")
    } else {
        name.trim().to_string()
    };
    Node {
        name,
        protocol: "wireguard".into(),
        server: server.into(),
        port,
        outbound: Value::Object(out),
    }
}

fn parse_ssh(line: &str) -> Result<Node, String> {
    let u = UriParts::parse(line).ok_or("invalid ssh link")?;
    let (server, port) = require_host_port(&u)?;
    let (user, pass) = credentials(&u);
    let mut out = Map::new();
    put(&mut out, "user", user);
    put(&mut out, "password", pass);
    Ok(node(&u.fragment, "ssh", &server, port, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vless_reality() {
        let n = parse_uri("vless://11111111-2222-3333-4444-555555555555@example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.apple.com&fp=chrome&pbk=PUBKEY&sid=ab12&type=tcp&headerType=none#%F0%9F%87%BA%F0%9F%87%B8%20US").unwrap();
        assert_eq!(n.name, "🇺🇸 US");
        assert_eq!(n.outbound["flow"], "xtls-rprx-vision");
        assert_eq!(n.outbound["tls"]["reality"]["public_key"], "PUBKEY");
        assert_eq!(n.outbound["tls"]["utls"]["fingerprint"], "chrome");
        assert_eq!(n.outbound["tls"]["server_name"], "www.apple.com");
        assert!(n.outbound.get("transport").is_none());
    }

    #[test]
    fn vmess_base64_ws() {
        let j = r#"{"v":"2","ps":"HK 01","add":"hk.example.com","port":"8443","id":"uuid-x","aid":"0","scy":"auto","net":"ws","type":"none","host":"cdn.example.com","path":"/ray?ed=2048","tls":"tls","sni":""}"#;
        let link = format!("vmess://{}", b64_encode(j));
        let n = parse_uri(&link).unwrap();
        assert_eq!(n.name, "HK 01");
        assert_eq!(n.port, 8443);
        assert_eq!(n.outbound["transport"]["type"], "ws");
        assert_eq!(n.outbound["transport"]["path"], "/ray");
        assert_eq!(n.outbound["transport"]["max_early_data"], 2048);
        assert_eq!(n.outbound["transport"]["headers"]["Host"], "cdn.example.com");
        assert_eq!(n.outbound["tls"]["server_name"], "cdn.example.com");
    }

    #[test]
    fn ss_sip002_and_legacy() {
        let n = parse_uri("ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@1.2.3.4:8388#Test").unwrap();
        assert_eq!(n.outbound["method"], "aes-256-gcm");
        assert_eq!(n.outbound["password"], "password");
        let legacy = format!("ss://{}#Old", b64_encode("chacha20-ietf-poly1305:pa:ss@5.6.7.8:443"));
        let n = parse_uri(&legacy).unwrap();
        assert_eq!(n.server, "5.6.7.8");
        assert_eq!(n.outbound["password"], "pa:ss");
        let n = parse_uri("ss://2022-blake3-aes-128-gcm:a2V5%3D@h.com:443/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dbing.com#P").unwrap();
        assert_eq!(n.outbound["password"], "a2V5=");
        assert_eq!(n.outbound["plugin"], "obfs-local");
        assert_eq!(n.outbound["plugin_opts"], "obfs=http;obfs-host=bing.com");
    }

    #[test]
    fn hysteria2_port_hopping() {
        let n = parse_uri("hy2://pass@[2001:db8::1]:443,20000-30000/?sni=a.com&insecure=1&obfs=salamander&obfs-password=x#H").unwrap();
        assert_eq!(n.server, "2001:db8::1");
        assert_eq!(n.port, 443);
        assert_eq!(n.outbound["server_ports"], json!(["443:443", "20000:30000"]));
        assert_eq!(n.outbound["obfs"]["type"], "salamander");
        assert_eq!(n.outbound["tls"]["insecure"], true);
    }

    #[test]
    fn trojan_defaults_tls() {
        let n = parse_uri("trojan://p%40ss@t.com:443?type=grpc&serviceName=svc#T").unwrap();
        assert_eq!(n.outbound["password"], "p@ss");
        assert_eq!(n.outbound["tls"]["enabled"], true);
        assert_eq!(n.outbound["transport"]["service_name"], "svc");
    }

    #[test]
    fn tuic_and_wireguard() {
        let n = parse_uri("tuic://uuid:pw@t.com:443?congestion_control=bbr&alpn=h3&sni=t.com#Tu").unwrap();
        assert_eq!(n.outbound["uuid"], "uuid");
        assert_eq!(n.outbound["password"], "pw");
        let n = parse_uri("wireguard://cHJpdg%3D@1.1.1.1:51820?publickey=cHVi&address=10.0.0.2,fd00::2&reserved=1,2,3#W").unwrap();
        assert!(n.is_endpoint());
        assert_eq!(n.outbound["private_key"], "cHJpdg=");
        assert_eq!(n.outbound["address"], json!(["10.0.0.2/32", "fd00::2/128"]));
        assert_eq!(n.outbound["peers"][0]["reserved"], json!([1, 2, 3]));
    }

    #[test]
    fn unsupported_transport_is_error() {
        assert!(parse_uri("vless://u@h.com:443?type=xhttp#X").is_err());
        assert!(parse_uri("ssr://abc").is_err());
    }
}
