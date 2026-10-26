//! Live smoke test of the engine against a real core, isolated from the user's setup.
//! `cargo run -p boxpilot-core --example engine_smoke -- <sing-box.exe> <data-dir> [bypass_cn]`
use std::time::Duration;

use boxpilot_core::model::{ProxyMode, Subscription};
use boxpilot_core::store::Paths;
use boxpilot_core::{CoreStatus, Engine};

fn main() {
    let mut args = std::env::args().skip(1);
    let core = args.next().expect("core path");
    let dir = args.next().expect("data dir");
    let bypass_cn = args.next().is_some();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let (engine, mut events) = Engine::with_paths(rt.handle().clone(), Paths::at(dir.into())).unwrap();
    rt.spawn(async move { while events.recv().await.is_some() {} });
    engine.update_settings(|s| {
        s.core_path = core.clone();
        s.mixed_port = 17890;
        s.clash_api_port = 19090;
        s.system_proxy = false;
        s.auto_start_core = false;
        s.bypass_cn = bypass_cn;
    });
    let content = "socks5://127.0.0.1:1#Dead A\ntrojan://pw@127.0.0.1:2#Dead B";
    engine
        .add_subscription(Subscription { name: "Local test".into(), ..Default::default() }, Some(content.into()))
        .unwrap();
    rt.block_on(async {
        engine.refresh_core_version().await;
        println!("core version: {:?}", engine.core_version());
        let t = std::time::Instant::now();
        let r = engine.start_async().await;
        println!("start: {:?} in {:?} -> {:?}", r.as_ref().err(), t.elapsed(), engine.status());
        if !engine.status().is_running() {
            for l in engine.logs().iter().rev().take(10) { println!("  log: {:?} {}", l.level, l.message); }
            return;
        }
        let api = engine.api();
        let groups = engine.groups();
        for g in &groups {
            println!("group {} ({}) now={} members={:?}", g.name, g.kind, g.now, g.members.iter().map(|m| &m.name).collect::<Vec<_>>());
        }
        engine.select_proxy("Proxy", "Dead B");
        tokio::time::sleep(Duration::from_millis(500)).await;
        println!("after select: {:?}", engine.groups().iter().find(|g| g.name == "Proxy").map(|g| g.now.clone()));
        engine.test_delays(vec!["Dead A".into(), "direct".into()]);
        tokio::time::sleep(Duration::from_secs(6)).await;
        println!("delay Dead A: {:?}, direct: {:?}", engine.delay("Dead A"), engine.delay("direct"));
        engine.set_mode(ProxyMode::Global);
        tokio::time::sleep(Duration::from_millis(500)).await;
        let cfg: serde_json::Value = reqwest::Client::builder().no_proxy().build().unwrap()
            .get("http://127.0.0.1:19090/configs").bearer_auth(engine.settings().clash_api_secret)
            .send().await.unwrap().json().await.unwrap();
        println!("mode via API: {}", cfg["mode"]);
        // A request through the mixed port in Direct mode reaches the internet.
        engine.set_mode(ProxyMode::Direct);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let client = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:17890").unwrap()).build().unwrap();
        let r = client.get("https://www.gstatic.com/generate_204").timeout(Duration::from_secs(10)).send().await;
        println!("direct via mixed port: {:?}", r.map(|r| r.status()));
        println!("connections: {:?}", api.connections().await.map(|c| c.connections.map(|v| v.len())));
        // Settings change that affects the config triggers a reload.
        engine.update_settings(|s| s.log_level = "debug".into());
        tokio::time::sleep(Duration::from_secs(3)).await;
        println!("after reload: {:?}", engine.status());
        let mut slot_status = engine.status();
        engine.stop();
        for _ in 0..50 { tokio::time::sleep(Duration::from_millis(100)).await; slot_status = engine.status(); if slot_status == CoreStatus::Stopped { break; } }
        println!("after stop: {:?}; api alive: {}", slot_status, api.version().await.is_ok());
    });
}
