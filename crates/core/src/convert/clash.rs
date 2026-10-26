//! Clash / Clash.Meta (mihomo) `proxies:` conversion.

use serde_json::{Map, Value, json};
use serde_yaml_ng::Value as Yaml;

use super::uri::{WireguardParams, wireguard_node};
use super::util::*;
use crate::model::Node;

pub fn parse_clash(content: &str) -> Result<(Vec<Node>, Vec<String>), String> {
    let doc: Yaml = serde_yaml_ng::from_str(content).map_err(|e| format!("invalid YAML: {e}"))?;
    let proxies = doc
        .get("proxies")
        .and_then(Yaml::as_sequence)
        .ok_or("no `proxies` list in Clash config")?;
    let mut nodes = Vec::new();
    let mut skipped = Vec::new();
    for p in proxies {
        let name = ystr(p, "name").unwrap_or_default();
        match convert_proxy(p) {
            Ok(n) => nodes.push(n),
            Err(e) => skipped.push(format!("{name}: {e}")),
        }
    }
    Ok((nodes, skipped))
}

fn ystr(v: &Yaml, key: &str) -> Option<String> {
    match v.get(key)? {
        Yaml::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Yaml::Number(n) => Some(n.to_string()),
        Yaml::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn ybool(v: &Yaml, key: &str) -> bool {
    match v.get(key) {
        Some(Yaml::Bool(b)) => *b,
        Some(Yaml::String(s)) => truthy(s),
        Some(Yaml::Number(n)) => n.as_i64() == Some(1),
        _ => false,
    }
}

fn ystr_list(v: &Yaml, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Yaml::Sequence(seq)) => seq
            .iter()
            .filter_map(|x| match x {
                Yaml::String(s) => Some(s.clone()),
                Yaml::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        Some(Yaml::String(s)) => split_list(s),
        _ => vec![],
    }
}

fn yaml_to_json(v: &Yaml) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn convert_proxy(p: &Yaml) -> Result<Node, String> {
    let ty = ystr(p, "type").ok_or("missing type")?.to_ascii_lowercase();
    let name = ystr(p, "name").ok_or("missing name")?;
    let server = ystr(p, "server").ok_or("missing server")?;
    let port_raw = ystr(p, "port").unwrap_or_default();
    let port: u16 = port_raw
        .split([',', '-'])
        .next()
        .and_then(|s| s.trim().parse().ok())
        .ok_or("invalid port")?;

    let mut out = Map::new();
    let protocol = match ty.as_str() {
        "ss" => {
            out.insert("method".into(), json!(ystr(p, "cipher").ok_or("missing cipher")?));
            out.insert("password".into(), json!(ystr(p, "password").unwrap_or_default()));
            if ybool(p, "udp-over-tcp") {
                out.insert("udp_over_tcp".into(), json!(true));
            }
            if let Some(plugin) = ystr(p, "plugin") {
                let opts = p.get("plugin-opts").cloned().unwrap_or(Yaml::Null);
                let (name, opts) = ss_plugin(&plugin, &opts)?;
                out.insert("plugin".into(), json!(name));
                put(&mut out, "plugin_opts", opts);
            }
            "shadowsocks"
        }
        "ssr" => return Err("ShadowsocksR is not supported by sing-box".into()),
        "vmess" => {
            out.insert("uuid".into(), json!(ystr(p, "uuid").ok_or("missing uuid")?));
            out.insert(
                "security".into(),
                json!(ystr(p, "cipher").unwrap_or_else(|| "auto".into())),
            );
            out.insert(
                "alter_id".into(),
                json!(ystr(p, "alterId").and_then(|a| a.parse::<u32>().ok()).unwrap_or(0)),
            );
            if ybool(p, "global-padding") {
                out.insert("global_padding".into(), json!(true));
            }
            if ybool(p, "authenticated-length") {
                out.insert("authenticated_length".into(), json!(true));
            }
            if let Some(pe) = ystr(p, "packet-encoding") {
                out.insert("packet_encoding".into(), json!(pe));
            }
            apply_tls(&mut out, p, false);
            apply_transport(&mut out, p)?;
            "vmess"
        }
        "vless" => {
            out.insert("uuid".into(), json!(ystr(p, "uuid").ok_or("missing uuid")?));
            if let Some(flow) = ystr(p, "flow") {
                out.insert("flow".into(), json!(flow));
            }
            out.insert(
                "packet_encoding".into(),
                json!(ystr(p, "packet-encoding").unwrap_or_else(|| "xudp".into())),
            );
            apply_tls(&mut out, p, false);
            apply_transport(&mut out, p)?;
            "vless"
        }
        "trojan" => {
            out.insert("password".into(), json!(ystr(p, "password").unwrap_or_default()));
            apply_tls(&mut out, p, true);
            apply_transport(&mut out, p)?;
            "trojan"
        }
        "hysteria2" | "hy2" => {
            put(&mut out, "password", ystr(p, "password").or(ystr(p, "auth")).unwrap_or_default());
            let ports = ystr(p, "ports").or_else(|| {
                port_raw.contains([',', '-']).then(|| port_raw.clone())
            });
            if let Some(ports) = ports {
                out.insert("server_ports".into(), json!(port_ranges(&ports)));
                if let Some(hop) = ystr(p, "hop-interval") {
                    let hop = if hop.chars().all(|c| c.is_ascii_digit()) { format!("{hop}s") } else { hop };
                    out.insert("hop_interval".into(), json!(hop));
                }
            }
            if let Some(obfs) = ystr(p, "obfs").filter(|o| o != "none") {
                out.insert(
                    "obfs".into(),
                    json!({ "type": obfs, "password": ystr(p, "obfs-password").unwrap_or_default() }),
                );
            }
            if let Some(up) = ystr(p, "up").and_then(|v| parse_mbps(&v)) {
                out.insert("up_mbps".into(), json!(up));
            }
            if let Some(down) = ystr(p, "down").and_then(|v| parse_mbps(&v)) {
                out.insert("down_mbps".into(), json!(down));
            }
            apply_tls(&mut out, p, true);
            "hysteria2"
        }
        "hysteria" => {
            if let Some(auth) = ystr(p, "auth-str").or(ystr(p, "auth_str")) {
                out.insert("auth_str".into(), json!(auth));
            }
            out.insert(
                "up_mbps".into(),
                json!(ystr(p, "up").and_then(|v| parse_mbps(&v)).unwrap_or(10)),
            );
            out.insert(
                "down_mbps".into(),
                json!(ystr(p, "down").and_then(|v| parse_mbps(&v)).unwrap_or(50)),
            );
            if let Some(obfs) = ystr(p, "obfs") {
                out.insert("obfs".into(), json!(obfs));
            }
            if let Some(ports) = ystr(p, "ports") {
                out.insert("server_ports".into(), json!(port_ranges(&ports)));
            }
            for (from, to) in [("recv-window-conn", "recv_window_conn"), ("recv-window", "recv_window")] {
                if let Some(n) = ystr(p, from).and_then(|v| v.parse::<u64>().ok()) {
                    out.insert(to.into(), json!(n));
                }
            }
            if ybool(p, "disable_mtu_discovery") || ybool(p, "disable-mtu-discovery") {
                out.insert("disable_mtu_discovery".into(), json!(true));
            }
            apply_tls(&mut out, p, true);
            "hysteria"
        }
        "tuic" => {
            if ystr(p, "token").is_some() && ystr(p, "uuid").is_none() {
                return Err("TUIC v4 (token) is not supported by sing-box".into());
            }
            out.insert("uuid".into(), json!(ystr(p, "uuid").ok_or("missing uuid")?));
            put(&mut out, "password", ystr(p, "password").unwrap_or_default());
            if let Some(cc) = ystr(p, "congestion-controller") {
                out.insert("congestion_control".into(), json!(cc));
            }
            if let Some(mode) = ystr(p, "udp-relay-mode") {
                out.insert("udp_relay_mode".into(), json!(mode));
            }
            if ybool(p, "reduce-rtt") {
                out.insert("zero_rtt_handshake".into(), json!(true));
            }
            if let Some(hb) = ystr(p, "heartbeat-interval").and_then(|v| v.parse::<u64>().ok()) {
                out.insert("heartbeat".into(), json!(format!("{hb}ms")));
            }
            apply_tls(&mut out, p, true);
            if ybool(p, "disable-sni") {
                if let Some(tls) = out.get_mut("tls").and_then(Value::as_object_mut) {
                    tls.insert("disable_sni".into(), json!(true));
                }
            }
            "tuic"
        }
        "wireguard" | "wg" => {
            let (peer_server, peer_port, public_key, psk, reserved_v) =
                match p.get("peers").and_then(Yaml::as_sequence).and_then(|s| s.first()) {
                    Some(peer) => (
                        ystr(peer, "server").unwrap_or(server.clone()),
                        ystr(peer, "port").and_then(|x| x.parse().ok()).unwrap_or(port),
                        ystr(peer, "public-key"),
                        ystr(peer, "pre-shared-key"),
                        peer.get("reserved").cloned(),
                    ),
                    None => (
                        server.clone(),
                        port,
                        ystr(p, "public-key"),
                        ystr(p, "pre-shared-key"),
                        p.get("reserved").cloned(),
                    ),
                };
            let mut address = Vec::new();
            if let Some(ip) = ystr(p, "ip") {
                address.push(if ip.contains('/') { ip } else { format!("{ip}/32") });
            }
            if let Some(ip6) = ystr(p, "ipv6") {
                address.push(if ip6.contains('/') { ip6 } else { format!("{ip6}/128") });
            }
            let reserved = match reserved_v {
                Some(Yaml::Sequence(seq)) => seq
                    .iter()
                    .filter_map(|x| x.as_u64().map(|n| n as u8))
                    .collect(),
                Some(Yaml::String(s)) => {
                    // Either "1,2,3" or a base64 client id.
                    let nums: Vec<u8> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if nums.len() == 3 {
                        nums
                    } else {
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD
                            .decode(s.trim())
                            .unwrap_or_default()
                    }
                }
                _ => vec![],
            };
            return Ok(wireguard_node(
                &name,
                &peer_server,
                peer_port,
                WireguardParams {
                    private_key: ystr(p, "private-key").ok_or("missing private-key")?,
                    public_key: public_key.ok_or("missing public-key")?,
                    pre_shared_key: psk,
                    address,
                    reserved,
                    mtu: ystr(p, "mtu").and_then(|m| m.parse().ok()),
                },
            ));
        }
        "socks5" | "socks" => {
            out.insert("version".into(), json!("5"));
            put(&mut out, "username", ystr(p, "username").unwrap_or_default());
            put(&mut out, "password", ystr(p, "password").unwrap_or_default());
            "socks"
        }
        "http" => {
            put(&mut out, "username", ystr(p, "username").unwrap_or_default());
            put(&mut out, "password", ystr(p, "password").unwrap_or_default());
            apply_tls(&mut out, p, false);
            "http"
        }
        "anytls" => {
            out.insert("password".into(), json!(ystr(p, "password").unwrap_or_default()));
            apply_tls(&mut out, p, true);
            "anytls"
        }
        "ssh" => {
            put(&mut out, "user", ystr(p, "username").unwrap_or_default());
            put(&mut out, "password", ystr(p, "password").unwrap_or_default());
            put(&mut out, "private_key", ystr(p, "private-key").unwrap_or_default());
            let host_keys = ystr_list(p, "host-key");
            if !host_keys.is_empty() {
                out.insert("host_key".into(), json!(host_keys));
            }
            "ssh"
        }
        other => return Err(format!("unsupported type `{other}`")),
    };
    out.insert("type".into(), json!(protocol));
    out.insert("server".into(), json!(server));
    out.insert("server_port".into(), json!(port));
    Ok(Node {
        name,
        protocol: protocol.into(),
        server,
        port,
        outbound: Value::Object(out),
    })
}

fn ss_plugin(plugin: &str, opts: &Yaml) -> Result<(&'static str, String), String> {
    match plugin {
        "obfs" | "obfs-local" | "simple-obfs" => {
            let mode = ystr(opts, "mode").unwrap_or_else(|| "http".into());
            let mut s = format!("obfs={mode}");
            if let Some(host) = ystr(opts, "host") {
                s.push_str(&format!(";obfs-host={host}"));
            }
            Ok(("obfs-local", s))
        }
        "v2ray-plugin" => {
            let mut parts = vec![format!(
                "mode={}",
                ystr(opts, "mode").unwrap_or_else(|| "websocket".into())
            )];
            if ybool(opts, "tls") {
                parts.push("tls".into());
            }
            if let Some(host) = ystr(opts, "host") {
                parts.push(format!("host={host}"));
            }
            if let Some(path) = ystr(opts, "path") {
                parts.push(format!("path={path}"));
            }
            if ybool(opts, "mux") {
                parts.push("mux=4".into());
            }
            Ok(("v2ray-plugin", parts.join(";")))
        }
        other => Err(format!("unsupported ss plugin `{other}`")),
    }
}

fn apply_tls(out: &mut Map<String, Value>, p: &Yaml, default_enabled: bool) {
    let enabled = match p.get("tls") {
        Some(_) => ybool(p, "tls"),
        None => default_enabled,
    } || p.get("reality-opts").is_some();
    let reality = p.get("reality-opts");
    let tls = TlsOpts {
        enabled,
        server_name: ystr(p, "servername").or(ystr(p, "sni")).or(ystr(p, "peer")),
        insecure: ybool(p, "skip-cert-verify"),
        alpn: ystr_list(p, "alpn"),
        fingerprint: ystr(p, "client-fingerprint"),
        reality_public_key: reality.and_then(|r| ystr(r, "public-key")),
        reality_short_id: reality.and_then(|r| ystr(r, "short-id")),
        disable_sni: false,
    };
    if let Some(v) = tls.to_value() {
        out.insert("tls".into(), v);
    }
}

fn apply_transport(out: &mut Map<String, Value>, p: &Yaml) -> Result<(), String> {
    let network = ystr(p, "network").unwrap_or_else(|| "tcp".into());
    let mut t = TransportOpts {
        network: network.clone(),
        ..Default::default()
    };
    match network.as_str() {
        "ws" => {
            if let Some(o) = p.get("ws-opts") {
                t.path = ystr(o, "path");
                if let Some(Yaml::Mapping(h)) = o.get("headers") {
                    for (k, v) in h {
                        if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                            if k.eq_ignore_ascii_case("host") {
                                t.host = Some(v.to_string());
                            } else {
                                t.headers.push((k.to_string(), v.to_string()));
                            }
                        }
                    }
                }
                t.max_early_data = ystr(o, "max-early-data").and_then(|v| v.parse().ok());
                t.early_data_header = ystr(o, "early-data-header-name");
                if ybool(o, "v2ray-http-upgrade") {
                    t.network = "httpupgrade".into();
                }
            } else {
                t.path = ystr(p, "ws-path");
                if let Some(Yaml::Mapping(h)) = p.get("ws-headers") {
                    t.host = h
                        .iter()
                        .find(|(k, _)| k.as_str().is_some_and(|k| k.eq_ignore_ascii_case("host")))
                        .and_then(|(_, v)| v.as_str().map(String::from));
                }
            }
        }
        "grpc" => {
            t.service_name = p.get("grpc-opts").and_then(|o| ystr(o, "grpc-service-name"));
        }
        "h2" => {
            if let Some(o) = p.get("h2-opts") {
                let hosts = ystr_list(o, "host");
                t.host = (!hosts.is_empty()).then(|| hosts.join(","));
                t.path = ystr(o, "path");
            }
        }
        "http" => {
            // Clash `network: http` is HTTP/1.1 header obfuscation over TCP.
            t.network = "tcp".into();
            t.header_type = Some("http".into());
            if let Some(o) = p.get("http-opts") {
                t.method = ystr(o, "method");
                t.path = ystr_list(o, "path").into_iter().next();
                if let Some(Yaml::Mapping(h)) = o.get("headers") {
                    for (k, v) in h {
                        if k.as_str().is_some_and(|k| k.eq_ignore_ascii_case("host")) {
                            let hosts = match v {
                                Yaml::Sequence(_) => serde_json::from_value::<Vec<String>>(yaml_to_json(v)).unwrap_or_default(),
                                Yaml::String(s) => vec![s.clone()],
                                _ => vec![],
                            };
                            t.host = (!hosts.is_empty()).then(|| hosts.join(","));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(v) = t.to_value()? {
        out.insert("transport".into(), v);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
port: 7890
proxies:
  - { name: "SS 1", type: ss, server: 1.2.3.4, port: 8388, cipher: aes-128-gcm, password: "pw", plugin: obfs, plugin-opts: { mode: tls, host: bing.com } }
  - name: VMess WS
    type: vmess
    server: v.example.com
    port: 443
    uuid: 0000
    alterId: 0
    cipher: auto
    tls: true
    servername: sni.example.com
    network: ws
    ws-opts:
      path: /path
      headers:
        Host: cdn.example.com
  - name: Reality
    type: vless
    server: r.example.com
    port: 443
    uuid: 1111
    flow: xtls-rprx-vision
    network: tcp
    tls: true
    servername: www.microsoft.com
    client-fingerprint: chrome
    reality-opts: { public-key: PK, short-id: "0a" }
  - { name: Hy2, type: hysteria2, server: h.com, port: 443, ports: "20000-30000", password: x, up: "50 Mbps", down: 200, sni: h.com, skip-cert-verify: true }
  - { name: Tuic, type: tuic, server: t.com, port: 443, uuid: u, password: p, alpn: [h3], congestion-controller: bbr }
  - { name: WG, type: wireguard, server: 9.9.9.9, port: 51820, ip: 172.16.0.2, private-key: priv, public-key: pub, reserved: [1, 2, 3], udp: true }
  - { name: Old, type: ssr, server: s.com, port: 1, cipher: none, password: p, protocol: origin, obfs: plain }
  - { name: Grpc, type: trojan, server: g.com, port: 443, password: pw, network: grpc, grpc-opts: { grpc-service-name: svc } }
proxy-groups: []
"#;

    #[test]
    fn converts_sample() {
        let (nodes, skipped) = parse_clash(SAMPLE).unwrap();
        assert_eq!(nodes.len(), 7, "{skipped:?}");
        assert_eq!(skipped.len(), 1);
        let ss = &nodes[0].outbound;
        assert_eq!(ss["plugin"], "obfs-local");
        assert_eq!(ss["plugin_opts"], "obfs=tls;obfs-host=bing.com");
        let vm = &nodes[1].outbound;
        assert_eq!(vm["transport"]["headers"]["Host"], "cdn.example.com");
        assert_eq!(vm["tls"]["server_name"], "sni.example.com");
        let re = &nodes[2].outbound;
        assert_eq!(re["tls"]["reality"]["short_id"], "0a");
        let hy = &nodes[3].outbound;
        assert_eq!(hy["server_ports"], json!(["20000:30000"]));
        assert_eq!(hy["up_mbps"], 50);
        assert_eq!(hy["tls"]["insecure"], true);
        assert!(nodes[5].is_endpoint());
        assert_eq!(nodes[6].outbound["transport"]["service_name"], "svc");
    }
}
