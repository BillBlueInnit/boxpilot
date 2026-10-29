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
const URL: &str = "https://www.gstatic.com/generate_204";

fn start_node(core: &str, dir: &Path) -> Child {
    let log = dir.join("node.log");
    let config = serde_json::json!({
        "log": { "level": "info", "output": log.to_string_lossy() },
        "inbounds": [{ "type": "socks", "listen": "127.0.0.1", "listen_port": NODE_PORT }],
        "outbounds": [{ "type": "direct" }],
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
        s.bypass_cn = false;
        s.ruleset_via_proxy = false;
        s.ipv6 = false;
        s.log_level = "debug".into();
        s.mode = ProxyMode::Global;
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
        for l in engine.logs().iter().filter(|l| l.message.contains("gstatic") || l.message.contains("dns")).take(12) {
            println!("  core: {}", l.message);
        }
        engine.stop();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let after = sys::system_proxy_state();
        println!("after stop: system proxy enabled={} server={}", after.enabled, after.server);
        status && through_node > 0 && !(after.enabled && after.server == "127.0.0.1:17890")
    });
    let _ = node.kill();
    if ok {
        println!("platform live test passed");
    } else {
        println!("--- node log ---\n{}", std::fs::read_to_string(dir.join("node.log")).unwrap_or_default());
        panic!("platform live test failed");
    }
}
