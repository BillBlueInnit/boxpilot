//! Live test of how traffic reaches BoxPilot — the system proxy or TUN —
//! against a real core and a local node. CI runs it on each platform:
//! `cargo run -p boxpilot-core --example platform_live -- <sing-box> <work-dir> sysproxy|tun`
//!
//! The "node" is a second sing-box with a SOCKS inbound that logs every
//! connection, so the test can tell an ordinary request went through BoxPilot.
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

use boxpilot_core::engine::Event;
use boxpilot_core::model::{ProxyMode, Subscription};
use boxpilot_core::store::Paths;
use boxpilot_core::{Engine, sys};

const NODE_PORT: u16 = 21080;
const URL: &str = "https://www.google.com/generate_204";
const HOST: &str = "www.google.com";

fn start_node(core: &str, dir: &Path) -> Child {
    let log = dir.join("node.log");
    let config = serde_json::json!({
        "log": { "level": "info", "output": log.to_string_lossy() },
        "inbounds": [{ "type": "socks", "listen": "127.0.0.1", "listen_port": NODE_PORT }],
        "outbounds": [{ "type": "direct" }],
        // A real node is a remote server: its own traffic must not come back
        // through BoxPilot's TUN, so it leaves by the physical interface, and
        // it has a DNS server of its own rather than this machine's.
        "dns": { "servers": [{ "type": "udp", "tag": "node-dns", "server": "8.8.8.8" }] },
        "route": { "auto_detect_interface": true, "default_domain_resolver": "node-dns" },
    });
    let path = dir.join("node.json");
    std::fs::write(&path, config.to_string()).unwrap();
    Command::new(core).arg("run").arg("-c").arg(&path).spawn().expect("start the node")
}

fn node_connections(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("node.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("inbound connection to"))
        .count()
}

/// An ordinary client of the platform: it follows the system proxy, or is
/// captured by TUN without knowing about BoxPilot.
fn client(mode: &str) -> Command {
    let mut c = if mode == "sysproxy" && cfg!(target_os = "macos") {
        // Python's urllib reads the macOS proxy settings (SystemConfiguration).
        let mut c = Command::new("python3");
        c.args([
            "-c",
            &format!(
                "import urllib.request as u; print('proxies', u.getproxies()); print('status', u.urlopen('{URL}', timeout=20).status)"
            ),
        ]);
        c
    } else if mode == "sysproxy" && cfg!(target_os = "linux") {
        // GIO's proxy resolver reads the desktop settings, as GTK apps do.
        let mut c = Command::new("python3");
        c.args([
            "-c",
            &format!(
                "import urllib.request as u; from gi.repository import Gio; \
                 p = Gio.ProxyResolver.get_default().lookup('{URL}', None)[0]; print('proxies', p); \
                 h = u.ProxyHandler({{}} if p == 'direct://' else {{'https': p, 'http': p}}); \
                 print('status', u.build_opener(h).open('{URL}', timeout=20).status)"
            ),
        ]);
        c
    } else if mode == "sysproxy" && cfg!(windows) {
        // Windows PowerShell's web client follows the WinINet proxy.
        let mut c = Command::new("powershell");
        c.args([
            "-NoProfile",
            "-Command",
            &format!("'status ' + (Invoke-WebRequest -UseBasicParsing -TimeoutSec 20 '{URL}').StatusCode"),
        ]);
        c
    } else {
        let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
        c.args(["-sS", "-m", "20", "-o", if cfg!(windows) { "NUL" } else { "/dev/null" }, "-w", "status %{http_code}\n", URL]);
        c
    };
    c.env_remove("http_proxy").env_remove("https_proxy").env_remove("HTTP_PROXY").env_remove("HTTPS_PROXY").env_remove("ALL_PROXY");
    c
}

fn main() {
    let mut args = std::env::args().skip(1);
    let core = args.next().expect("core path");
    let dir = std::path::PathBuf::from(args.next().expect("work dir"));
    let mode = args.next().expect("sysproxy | tun");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // TUN takes DNS over on macOS; it must all be put back afterwards.
    #[cfg(target_os = "macos")]
    let dns_before = sys::dns_settings().unwrap_or_default();
    let mut node = start_node(&core, &dir);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (engine, mut events) = Engine::with_paths(rt.handle().clone(), Paths::at(dir.join("data"))).unwrap();
    rt.spawn(async move {
        while let Some(e) = events.recv().await {
            if let Event::Notice(kind, msg) = e {
                println!("notice {kind:?}: {} {:?}", msg.key, msg.args);
            }
        }
    });
    engine.update_settings(|s| {
        s.core_path = core.clone();
        s.mixed_port = 17890;
        s.clash_api_port = 19090;
        s.auto_start_core = false;
        // The defaults people run with: Rule mode and the China bypass.
        s.ruleset_via_proxy = false;
        s.ipv6 = false;
        s.log_level = "debug".into();
        s.mode = ProxyMode::Rule;
        s.system_proxy = mode == "sysproxy";
        s.tun_enabled = mode == "tun";
    });
    engine
        .add_subscription(
            Subscription { name: "Live".into(), ..Default::default() },
            Some(format!("socks5://127.0.0.1:{NODE_PORT}#Node")),
        )
        .unwrap();

    let ok = rt.block_on(async {
        engine.refresh_core_version().await;
        println!("core {:?}, tun ready {}", engine.core_version(), engine.tun_permitted());
        let r = engine.start_async().await;
        println!("start: {:?} -> {:?}", r.as_ref().err(), engine.status());
        if !engine.status().is_running() {
            for l in engine.logs().iter().rev().take(15).rev() {
                println!("  core: {}", l.message);
            }
            return false;
        }
        engine.select_proxy("Proxy", "Node");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let state = sys::system_proxy_state();
        println!("system proxy now: enabled={} server={}", state.enabled, state.server);

        let before = node_connections(&dir);
        let out = tokio::task::spawn_blocking(move || client(&mode).output()).await.unwrap();
        let (status, text) = match out {
            Ok(o) => {
                let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                (text.contains("status 204"), text)
            }
            Err(e) => (false, format!("client failed to run: {e}")),
        };
        println!("client: {}", text.trim());
        tokio::time::sleep(Duration::from_secs(1)).await;
        let through_node = node_connections(&dir) - before;
        println!("node saw {through_node} new connection(s)");
        let logs = engine.logs();
        let dns_seen = logs.iter().any(|l| l.message.contains("dns") && l.message.contains(HOST));
        println!("DNS for {HOST} reached the core: {dns_seen}");
        for l in logs.iter().filter(|l| l.message.contains(HOST)).take(12) {
            println!("  core: {}", l.message);
        }
        let text: String = logs.iter().map(|l| format!("{}\n", l.message)).collect();
        let _ = std::fs::write(dir.join("core.log"), text);
        engine.stop();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let after = sys::system_proxy_state();
        println!("after stop: system proxy enabled={} server={}", after.enabled, after.server);
        #[cfg(target_os = "macos")]
        let dns_restored = {
            let now = sys::dns_settings().unwrap_or_default();
            println!("DNS before {dns_before:?}, after stop {now:?}");
            now == dns_before
        };
        #[cfg(not(target_os = "macos"))]
        let dns_restored = true;
        status && through_node > 0 && dns_restored && !(after.enabled && after.server == "127.0.0.1:17890")
    });
    let _ = node.kill();
    if ok {
        println!("platform live test passed");
    } else {
        let node_log = std::fs::read_to_string(dir.join("node.log")).unwrap_or_default();
        println!("--- node log (last lines) ---");
        for l in node_log.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev() {
            println!("{l}");
        }
        panic!("platform live test failed");
    }
}
