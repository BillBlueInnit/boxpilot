//! Writes sample configurations for validating against a real core:
//! `cargo run -p boxpilot-core --example gen_config -- <out-dir>`
use boxpilot_core::config::{BuildInput, CoreVersion, NodeGroup, build_config};
use boxpilot_core::convert::parse_subscription;
use boxpilot_core::model::{Settings, TunStack};

const LINKS: &str = "\
vless://11111111-2222-3333-4444-555555555555@example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.apple.com&fp=chrome&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=ab12&type=tcp#Reality
vless://11111111-2222-3333-4444-555555555555@example.com:443?security=tls&type=ws&host=cdn.com&path=%2Fws%3Fed%3D2048#VLESS WS
vless://11111111-2222-3333-4444-555555555555@example.com:443?security=tls&type=grpc&serviceName=svc#VLESS gRPC
vless://11111111-2222-3333-4444-555555555555@example.com:443?security=tls&type=httpupgrade&host=a.com&path=/up#VLESS HU
trojan://pass@t.com:443?sni=t.com#Trojan
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@1.2.3.4:8388#SS
ss://2022-blake3-aes-128-gcm:YWJjZGVmZ2hpamtsbW5vcA%3D%3D@h.com:443/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dbing.com#SS2022 obfs
hy2://pass@h.com:443,20000-30000/?sni=h.com&insecure=1&obfs=salamander&obfs-password=x#Hy2
hysteria://h.com:443?auth=x&upmbps=10&downmbps=50&peer=h.com#Hy1
tuic://11111111-2222-3333-4444-555555555555:pw@t.com:443?congestion_control=bbr&alpn=h3&sni=t.com#TUIC
socks5://u:p@1.1.1.1:1080#Socks
https://u:p@p.com:443#HTTPS
anytls://pw@a.com:443?sni=a.com#AnyTLS
wireguard://yAnz5TF%2BlXXJte14tji3zlMNq%2BhdD5XbJ0YiprL1ccU%3D@162.159.192.1:2408?publickey=bmXOC%2BF1FxEMF9dyiK2H5%2F1SUtzH0JuVo51h2wPfgyo%3D&address=172.16.0.2%2F32,2606:4700:110:8a36::2%2F128&reserved=1,2,3&mtu=1280#WARP
";

const CLASH: &str = r#"
proxies:
  - { name: "VMess WS", type: vmess, server: v.com, port: 443, uuid: 11111111-2222-3333-4444-555555555555, alterId: 0, cipher: auto, tls: true, servername: v.com, network: ws, ws-opts: { path: /ws, headers: { Host: cdn.v.com } } }
  - { name: "VMess H2", type: vmess, server: v.com, port: 443, uuid: 11111111-2222-3333-4444-555555555555, alterId: 0, cipher: auto, tls: true, network: h2, h2-opts: { host: [v.com], path: /h2 } }
  - { name: "VMess HTTP", type: vmess, server: v.com, port: 80, uuid: 11111111-2222-3333-4444-555555555555, alterId: 0, cipher: auto, network: http, http-opts: { method: GET, path: [/], headers: { Host: [a.com] } } }
  - { name: "SS v2ray-plugin", type: ss, server: s.com, port: 443, cipher: aes-128-gcm, password: pw, plugin: v2ray-plugin, plugin-opts: { mode: websocket, tls: true, host: s.com, path: /ws } }
  - { name: "Hy2 clash", type: hysteria2, server: h.com, port: 443, password: x, up: "50 Mbps", down: "200 Mbps", sni: h.com }
"#;

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("out dir"));
    std::fs::create_dir_all(&out).unwrap();
    let a = parse_subscription(LINKS).unwrap();
    assert!(a.skipped.is_empty(), "{:?}", a.skipped);
    let b = parse_subscription(CLASH).unwrap();
    assert!(b.skipped.is_empty(), "{:?}", b.skipped);
    let mut variants: Vec<(&str, Settings)> = vec![("default", Settings::default())];
    let mut s = Settings::default();
    s.tun_enabled = true;
    s.tun_stack = TunStack::System;
    s.fake_ip = true;
    s.ipv6 = true;
    s.block_ads = true;
    s.allow_lan = true;
    s.dns_remote = "tls://dns.google".into();
    s.dns_direct = "223.5.5.5".into();
    s.custom_rules = "DOMAIN-SUFFIX,openai.com,proxy\nGEOSITE,netflix,Sub B\nIP-CIDR,10.0.0.0/8,direct\nPROCESS-NAME,steam.exe,direct\nDST-PORT,22|2222,reject\nRULE-SET,https://example.com/x.srs,proxy".into();
    variants.push(("everything", s));
    let mut s = Settings::default();
    s.bypass_cn = false;
    s.ruleset_via_proxy = false;
    variants.push(("minimal", s));
    for (name, settings) in &variants {
        for version in [CoreVersion { major: 1, minor: 14, patch: 4 }, CoreVersion { major: 1, minor: 12, patch: 0 }] {
            let built = build_config(&BuildInput {
                settings,
                groups: vec![
                    NodeGroup { name: "Sub A", nodes: &a.nodes },
                    NodeGroup { name: "Sub B", nodes: &b.nodes },
                ],
                version,
                rule_set_dir: None,
                providers: &[],
            });
            assert!(built.warnings.is_empty(), "{:?}", built.warnings);
            let path = out.join(format!("{name}-{}.{}.json", version.major, version.minor));
            std::fs::write(&path, serde_json::to_string_pretty(&built.config).unwrap()).unwrap();
            println!("{}", path.display());
        }
    }
    let empty = build_config(&BuildInput { settings: &Settings::default(), groups: vec![], version: CoreVersion { major: 1, minor: 14, patch: 4 }, rule_set_dir: None, providers: &[] });
    let path = out.join("empty-1.14.json");
    std::fs::write(&path, serde_json::to_string_pretty(&empty.config).unwrap()).unwrap();
    println!("{}", path.display());
}
