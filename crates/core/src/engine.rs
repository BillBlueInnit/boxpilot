//! The engine owns all application state and runs every background job on a
//! Tokio runtime. The UI reads state through getters and is told about changes
//! through [`Event`]s.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use serde_json::Value;
use tokio::process::Child;
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::clash_api::{ClashApi, Traffic};
use crate::config::{BuildInput, BuiltConfig, CoreVersion, NodeGroup, RuleSetSpec, build_config, rule_set_file};
use crate::convert::{filter_nodes, parse_subscription};
use crate::core_download::{self, Release};
use crate::fetch::{self, Route};
use crate::model::*;
use crate::process::{self, LogLevel, LogLine};
use crate::store::{APP_NAME, Paths, Store};
use crate::rules::{self, RuleProvider};
use crate::sys;

pub const TUN_NEEDS_ADMIN: &str = "TUN mode requires administrator rights";
const MAX_LOGS: usize = 5000;
const DELAY_TIMEOUT_MS: u32 = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed(String),
}

impl CoreStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, CoreStatus::Running)
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, CoreStatus::Starting | CoreStatus::Stopping)
    }

    pub fn label(&self) -> &'static str {
        match self {
            CoreStatus::Stopped => "Stopped",
            CoreStatus::Starting => "Starting…",
            CoreStatus::Running => "Running",
            CoreStatus::Stopping => "Stopping…",
            CoreStatus::Failed(_) => "Error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreDownloadState {
    Idle,
    Checking,
    Available(String),
    UpToDate(String),
    Downloading { done: u64, total: u64 },
    Installing,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct ProxyMember {
    pub name: String,
    pub protocol: String,
    pub is_group: bool,
}

#[derive(Debug, Clone)]
pub struct ProxyGroup {
    pub name: String,
    /// `Selector` or `URLTest`.
    pub kind: String,
    pub now: String,
    pub members: Vec<ProxyMember>,
}

impl ProxyGroup {
    pub fn selectable(&self) -> bool {
        self.kind.eq_ignore_ascii_case("selector")
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Status,
    Log(LogLine),
    Traffic(Traffic),
    Subscriptions,
    Settings,
    CoreInfo,
    CoreDownload,
    Proxies,
    Delays,
    Notice(NoticeKind, Msg),
    RuleProviders,
}

/// A user-facing message: a translation key with positional arguments
/// (`{0}`, `{1}`, …). The UI owns the translations.
#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub key: &'static str,
    pub args: Vec<String>,
}

impl Msg {
    pub fn new(key: &'static str) -> Self {
        Self { key, args: Vec::new() }
    }

    pub fn arg(mut self, a: impl ToString) -> Self {
        self.args.push(a.to_string());
        self
    }
}

/// Prefix of the error returned when no core is installed.
pub const CORE_MISSING: &str = "sing-box core is not installed";

struct Running {
    kill: oneshot::Sender<oneshot::Sender<()>>,
    generation: u64,
}

#[derive(Default, Clone)]
struct BuildMeta {
    groups: Vec<String>,
    types: HashMap<String, String>,
}

struct State {
    settings: Settings,
    subs: Vec<Subscription>,
    nodes: HashMap<String, Vec<Node>>,
    status: CoreStatus,
    core_version: Option<CoreVersion>,
    latest_release: Option<Release>,
    download: CoreDownloadState,
    sysproxy_applied: bool,
    updating: HashSet<String>,
    delays: HashMap<String, Option<u32>>,
    testing: HashSet<String>,
    groups: Vec<ProxyGroup>,
    meta: BuildMeta,
    logs: VecDeque<LogLine>,
    traffic: Traffic,
    providers: Vec<RuleProvider>,
}

pub struct Engine {
    rt: Handle,
    store: Store,
    state: Mutex<State>,
    tx: mpsc::UnboundedSender<Event>,
    proc: tokio::sync::Mutex<Option<Running>>,
    generation: AtomicU64,
    /// Generation of the currently running core, 0 when stopped.
    active: AtomicU64,
    #[allow(dead_code)]
    job: Option<sys::KillOnCloseJob>,
    wake_scheduler: Notify,
}

impl Engine {
    pub fn new(rt: Handle) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        Self::with_paths(rt, Paths::detect())
    }

    pub fn with_paths(rt: Handle, paths: Paths) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        let store = Store::new(paths)?;
        let mut settings = store.load_settings();
        if settings.clash_api_secret.is_empty() {
            settings.clash_api_secret = random_secret();
            store.save_settings(&settings)?;
        }
        let subs = store.load_subscriptions();
        let nodes = subs
            .iter()
            .map(|s| (s.id.clone(), store.load_nodes(&s.id)))
            .collect();
        let providers = store.load_rule_providers();
        let (tx, rx) = mpsc::unbounded_channel();
        let engine = Arc::new(Self {
            rt,
            store,
            state: Mutex::new(State {
                settings,
                subs,
                nodes,
                status: CoreStatus::Stopped,
                core_version: None,
                latest_release: None,
                download: CoreDownloadState::Idle,
                sysproxy_applied: false,
                updating: HashSet::new(),
                delays: HashMap::new(),
                testing: HashSet::new(),
                groups: Vec::new(),
                meta: BuildMeta::default(),
                logs: VecDeque::new(),
                traffic: Traffic::default(),
                providers,
            }),
            tx,
            proc: tokio::sync::Mutex::new(None),
            generation: AtomicU64::new(0),
            active: AtomicU64::new(0),
            job: sys::KillOnCloseJob::new(),
            wake_scheduler: Notify::new(),
        });
        Ok((engine, rx))
    }

    // ---------------------------------------------------------------- state

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
    }

    fn notice(&self, kind: NoticeKind, msg: Msg) {
        self.emit(Event::Notice(kind, msg));
    }

    pub fn runtime(&self) -> &Handle {
        &self.rt
    }

    pub fn paths(&self) -> &Paths {
        &self.store.paths
    }

    pub fn settings(&self) -> Settings {
        self.with(|s| s.settings.clone())
    }

    pub fn subscriptions(&self) -> Vec<Subscription> {
        self.with(|s| s.subs.clone())
    }

    pub fn subscription(&self, id: &str) -> Option<Subscription> {
        self.with(|s| s.subs.iter().find(|x| x.id == id).cloned())
    }

    pub fn nodes(&self, id: &str) -> Vec<Node> {
        self.with(|s| s.nodes.get(id).cloned().unwrap_or_default())
    }

    pub fn status(&self) -> CoreStatus {
        self.with(|s| s.status.clone())
    }

    pub fn core_version(&self) -> Option<CoreVersion> {
        self.with(|s| s.core_version)
    }

    pub fn core_path(&self) -> PathBuf {
        let custom = self.with(|s| s.settings.core_path.trim().to_string());
        if custom.is_empty() {
            self.store.paths.default_core_path()
        } else {
            PathBuf::from(custom)
        }
    }

    pub fn core_installed(&self) -> bool {
        self.core_path().is_file()
    }

    /// Whether TUN can be used: an elevated app on Windows; on macOS / Linux
    /// either root or a core binary granted the privilege.
    pub fn tun_permitted(&self) -> bool {
        sys::is_elevated() || sys::core_tun_privileged(&self.core_path())
    }

    /// macOS / Linux: asks for an administrator password and grants the core
    /// binary the right to create TUN interfaces (needed again after a core update).
    pub async fn grant_tun_privilege(&self) -> Result<()> {
        let core = self.core_path();
        if !core.is_file() {
            bail!("{CORE_MISSING}");
        }
        tokio::task::spawn_blocking(move || sys::grant_tun_privilege(&core)).await??;
        self.log_app(LogLevel::Info, "granted TUN permission to the core");
        Ok(())
    }

    fn pid_file(&self) -> PathBuf {
        self.store.paths.run_dir.join("core.pid")
    }

    pub fn download_state(&self) -> CoreDownloadState {
        self.with(|s| s.download.clone())
    }

    pub fn is_updating(&self, id: &str) -> bool {
        self.with(|s| s.updating.contains(id))
    }

    pub fn any_updating(&self) -> bool {
        self.with(|s| !s.updating.is_empty())
    }

    pub fn logs(&self) -> Vec<LogLine> {
        self.with(|s| s.logs.iter().cloned().collect())
    }

    pub fn clear_logs(&self) {
        self.with(|s| s.logs.clear());
    }

    pub fn groups(&self) -> Vec<ProxyGroup> {
        self.with(|s| s.groups.clone())
    }

    pub fn delay(&self, name: &str) -> Option<Option<u32>> {
        self.with(|s| s.delays.get(name).copied())
    }

    pub fn is_testing(&self, name: &str) -> bool {
        self.with(|s| s.testing.contains(name))
    }

    pub fn any_testing(&self) -> bool {
        self.with(|s| !s.testing.is_empty())
    }

    pub fn traffic(&self) -> Traffic {
        self.with(|s| s.traffic)
    }

    pub fn total_nodes(&self) -> usize {
        self.with(|s| {
            s.subs
                .iter()
                .filter(|x| x.enabled)
                .map(|x| s.nodes.get(&x.id).map_or(0, Vec::len))
                .sum()
        })
    }

    pub fn api(&self) -> ClashApi {
        self.with(|s| ClashApi::new(s.settings.clash_api_port, &s.settings.clash_api_secret))
    }

    fn set_status(&self, status: CoreStatus) {
        self.with(|s| {
            if !status.is_running() {
                s.traffic = Traffic::default();
            }
            s.status = status;
        });
        self.emit(Event::Status);
    }

    fn push_log(&self, line: LogLine) {
        self.with(|s| {
            if s.logs.len() >= MAX_LOGS {
                s.logs.pop_front();
            }
            s.logs.push_back(line.clone());
        });
        self.emit(Event::Log(line));
    }

    fn log_app(&self, level: LogLevel, msg: impl Into<String>) {
        let msg = msg.into();
        match level {
            LogLevel::Error => log::error!("{msg}"),
            LogLevel::Warn => log::warn!("{msg}"),
            _ => log::info!("{msg}"),
        }
        self.push_log(LogLine::app(level, format!("[app] {msg}")));
    }

    fn last_core_error(&self) -> String {
        self.with(|s| {
            s.logs
                .iter()
                .rev()
                .take(30)
                .find(|l| l.level >= LogLevel::Error && !l.message.starts_with("[app]"))
                .map(|l| l.message.clone())
                .unwrap_or_default()
        })
    }

    fn save_subs(&self) {
        let subs = self.subscriptions();
        if let Err(e) = self.store.save_subscriptions(&subs) {
            self.notice(NoticeKind::Error, Msg::new("notice.save_failed").arg(format!("{e:#}")));
        }
        self.emit(Event::Subscriptions);
    }

    fn mixed_proxy_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.with(|s| s.settings.mixed_port))
    }

    // ------------------------------------------------------------ lifecycle

    /// Runs once after the UI is up: cleans stale state, auto-starts the core,
    /// refreshes subscriptions flagged "update on start" and starts the scheduler.
    pub fn startup(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move {
            let s = this.settings();
            // A core from a crashed session would hold the ports; stop it.
            if let Some(pid) = std::fs::read_to_string(this.pid_file()).ok().and_then(|p| p.trim().parse().ok()) {
                sys::kill_stale_core(pid);
                let _ = std::fs::remove_file(this.pid_file());
            }
            // A marker means we set the system proxy and did not get to restore it
            // (crash or power loss). Only then is it safe to clear it.
            let marker = this.sysproxy_marker();
            if let Ok(server) = std::fs::read_to_string(&marker) {
                let state = sys::system_proxy_state();
                if state.enabled && state.server == server.trim() {
                    let _ = sys::clear_system_proxy();
                }
                let _ = std::fs::remove_file(&marker);
            }
            if s.launch_at_login {
                this.apply_launch_at_login(true);
            }
            this.refresh_core_version().await;

            let had_nodes = this.total_nodes() > 0;
            if s.auto_start_core && had_nodes && this.core_installed() {
                let _ = this.start_async().await;
            }
            let ids: Vec<String> = this.with(|st| {
                st.subs
                    .iter()
                    .filter(|x| x.enabled && x.is_remote() && (x.update_on_start || x.last_update.is_none()))
                    .map(|x| x.id.clone())
                    .collect()
            });
            if !ids.is_empty() {
                this.log_app(LogLevel::Info, format!("updating {} subscription(s) on startup", ids.len()));
                this.update_many(ids, true).await;
            }
            if s.auto_start_core
                && !this.status().is_running()
                && !had_nodes
                && this.total_nodes() > 0
                && this.core_installed()
            {
                let _ = this.start_async().await;
            }
            this.scheduler().await;
        });
    }

    /// Stops the core and restores the system proxy. Blocks; call on quit.
    pub fn shutdown_blocking(&self) {
        let fut = async {
            let mut slot = self.proc.lock().await;
            self.stop_locked(&mut slot, false).await;
        };
        if Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| self.rt.block_on(fut));
        } else {
            self.rt.block_on(fut);
        }
    }

    pub fn start(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move {
            let _ = this.start_async().await;
        });
    }

    pub fn stop(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move {
            let mut slot = this.proc.lock().await;
            this.stop_locked(&mut slot, false).await;
        });
    }

    pub fn restart(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move {
            let _ = this.restart_async().await;
        });
    }

    pub async fn start_async(self: &Arc<Self>) -> Result<()> {
        let mut slot = self.proc.lock().await;
        self.start_locked(&mut slot).await
    }

    pub async fn restart_async(self: &Arc<Self>) -> Result<()> {
        let mut slot = self.proc.lock().await;
        if slot.is_some() {
            self.log_app(LogLevel::Info, "reloading core");
            self.stop_locked(&mut slot, true).await;
        }
        self.start_locked(&mut slot).await
    }

    async fn start_locked(self: &Arc<Self>, slot: &mut Option<Running>) -> Result<()> {
        if slot.is_some() {
            return Ok(());
        }
        self.set_status(CoreStatus::Starting);
        match self.launch(slot).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let msg = format!("{e:#}");
                self.log_app(LogLevel::Error, format!("start failed: {msg}"));
                if self.with(|s| s.sysproxy_applied) {
                    self.clear_sysproxy();
                }
                self.set_status(CoreStatus::Failed(msg.clone()));
                Err(anyhow!(msg))
            }
        }
    }

    async fn launch(self: &Arc<Self>, slot: &mut Option<Running>) -> Result<()> {
        let core = self.core_path();
        if !core.is_file() {
            bail!("{CORE_MISSING}");
        }
        let version = match self.core_version() {
            Some(v) => v,
            None => {
                let v = process::detect_version(&core).await?;
                self.with(|s| s.core_version = Some(v));
                self.emit(Event::CoreInfo);
                v
            }
        };
        if version < CoreVersion::MIN_SUPPORTED {
            bail!(
                "sing-box {version} is too old; {} or newer is required",
                CoreVersion::MIN_SUPPORTED
            );
        }
        let settings = self.settings();
        if settings.tun_enabled && !self.tun_permitted() {
            bail!(TUN_NEEDS_ADMIN);
        }
        let mut built = self.render_config(&settings, version);
        if !built.missing_rule_sets.is_empty() {
            let n = self.download_rule_sets(&built.missing_rule_sets, &[Route::Direct]).await;
            if n > 0 {
                built = self.render_config(&settings, version);
            }
            if !built.missing_rule_sets.is_empty() {
                let tags: Vec<&str> = built.missing_rule_sets.iter().map(|r| r.tag.as_str()).collect();
                self.log_app(
                    LogLevel::Warn,
                    format!("starting without rule-sets {} (will retry through the proxy)", tags.join(", ")),
                );
            }
        }
        for w in &built.warnings {
            self.log_app(LogLevel::Warn, format!("custom rules: {w}"));
        }
        let run_dir = self.store.paths.run_dir.clone();
        let cfg_path = self.store.paths.config_file();
        crate::store::write_atomic(&cfg_path, &serde_json::to_vec_pretty(&built.config)?)?;
        process::check_config(&core, &cfg_path, &run_dir)
            .await
            .context("invalid configuration")?;

        let mut child = process::spawn_core(&core, &cfg_path, &run_dir)?;
        if let Some(pid) = child.id() {
            let _ = std::fs::write(self.pid_file(), pid.to_string());
        }
        #[cfg(windows)]
        if let (Some(job), Some(h)) = (&self.job, child.raw_handle()) {
            job.assign(h);
        }
        for pipe in [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child.stderr.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let this = Arc::downgrade(self);
            process::pipe_lines(pipe, move |line| {
                if let Some(e) = this.upgrade() {
                    e.push_log(line);
                }
            });
        }

        let api = self.api();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = child.try_wait()? {
                tokio::time::sleep(Duration::from_millis(200)).await;
                bail!("sing-box exited ({status}) {}", self.last_core_error());
            }
            if api.version().await.is_ok() {
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill().await;
                bail!("sing-box did not become ready in time. {}", self.last_core_error());
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (kill_tx, kill_rx) = oneshot::channel();
        *slot = Some(Running { kill: kill_tx, generation });
        self.active.store(generation, Ordering::SeqCst);
        self.rt.spawn(self.clone().watch(child, kill_rx, generation));

        if let Err(e) = api.set_mode(settings.mode.as_str()).await {
            self.log_app(LogLevel::Warn, format!("could not set mode: {e:#}"));
        }
        self.set_status(CoreStatus::Running);
        self.log_app(
            LogLevel::Info,
            format!("sing-box {version} running with {} nodes", built.node_count),
        );
        self.apply_sysproxy();
        self.spawn_traffic(generation);
        self.spawn_rule_set_maintenance(generation, built.rule_sets, !built.missing_rule_sets.is_empty());
        let _ = self.refresh_proxies_async().await;
        Ok(())
    }

    fn rules_dir(&self) -> PathBuf {
        self.store.paths.rules_dir()
    }

    /// Downloads rule-sets into the local cache, trying each route in order.
    /// Returns how many were saved.
    async fn download_rule_sets(&self, specs: &[RuleSetSpec], routes: &[Route]) -> usize {
        let dir = self.rules_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mirror = self.settings().github_mirror.trim().to_string();
        let results: Vec<bool> = futures_util::stream::iter(specs.iter().cloned())
            .map(|spec| {
                let dir = dir.clone();
                let mirror = mirror.clone();
                async move {
                    let url = if !mirror.is_empty()
                        && (spec.url.starts_with("https://raw.githubusercontent.com/")
                            || spec.url.starts_with("https://github.com/"))
                    {
                        format!("{mirror}{}", spec.url)
                    } else {
                        spec.url.clone()
                    };
                    for route in routes {
                        let Ok(client) = fetch::client(route, Duration::from_secs(20), "BoxPilot") else {
                            continue;
                        };
                        let resp = match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
                            Ok(r) => r,
                            Err(e) => {
                                log::warn!("rule-set {} via {}: {e}", spec.tag, route.label());
                                continue;
                            }
                        };
                        match resp.bytes().await {
                            Ok(b) if !b.is_empty() => {
                                let path = rule_set_file(&dir, &spec.tag);
                                if crate::store::write_atomic(&path, &b).is_ok() {
                                    return true;
                                }
                            }
                            _ => {}
                        }
                    }
                    false
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;
        let n = results.iter().filter(|ok| **ok).count();
        if n > 0 {
            self.log_app(LogLevel::Info, format!("downloaded {n} rule-set(s)"));
        }
        n
    }

    /// While the core runs: fetch rule-sets that were missing at start (through the
    /// proxy, then reload) and refresh cached ones older than three days. sing-box
    /// reloads local rule-set files by itself when they change.
    fn spawn_rule_set_maintenance(self: &Arc<Self>, generation: u64, wanted: Vec<RuleSetSpec>, had_missing: bool) {
        let this = self.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let alive = |this: &Arc<Self>| this.active.load(Ordering::SeqCst) == generation;
            if !alive(&this) {
                return;
            }
            let dir = this.rules_dir();
            let proxy = Route::Proxy(this.mixed_proxy_url());
            let routes = if this.settings().ruleset_via_proxy {
                vec![proxy, Route::Direct]
            } else {
                vec![Route::Direct, proxy]
            };
            if had_missing {
                let missing: Vec<RuleSetSpec> = wanted
                    .iter()
                    .filter(|r| !rule_set_file(&dir, &r.tag).is_file())
                    .cloned()
                    .collect();
                if this.download_rule_sets(&missing, &routes).await > 0 && alive(&this) {
                    let _ = this.restart_async().await;
                    return;
                }
            }
            let stale_after = Duration::from_secs(3 * 86400);
            let stale: Vec<RuleSetSpec> = wanted
                .into_iter()
                .filter(|r| {
                    std::fs::metadata(rule_set_file(&dir, &r.tag))
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > stale_after)
                })
                .collect();
            if !stale.is_empty() && alive(&this) {
                this.download_rule_sets(&stale, &routes).await;
            }
        });
    }

    async fn watch(
        self: Arc<Self>,
        mut child: Child,
        mut kill_rx: oneshot::Receiver<oneshot::Sender<()>>,
        generation: u64,
    ) {
        tokio::select! {
            status = child.wait() => {
                // Release a pending stop request before taking the lock.
                drop(kill_rx);
                let mut slot = self.proc.lock().await;
                if slot.as_ref().map(|r| r.generation) == Some(generation) {
                    *slot = None;
                    self.active.store(0, Ordering::SeqCst);
                    if self.with(|s| s.sysproxy_applied) {
                        self.clear_sysproxy();
                    }
                    let code = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let msg = format!("sing-box exited unexpectedly ({code}). {}", self.last_core_error());
                    self.log_app(LogLevel::Error, &msg);
                    self.set_status(CoreStatus::Failed(msg.clone()));
                    self.notice(NoticeKind::Error, Msg::new("notice.core_exited").arg(msg));
                }
            }
            req = &mut kill_rx => {
                let _ = child.kill().await;
                let _ = std::fs::remove_file(self.pid_file());
                if let Ok(done) = req {
                    let _ = done.send(());
                }
            }
        }
    }

    async fn stop_locked(&self, slot: &mut Option<Running>, keep_sysproxy: bool) {
        if let Some(r) = slot.take() {
            self.set_status(CoreStatus::Stopping);
            self.active.store(0, Ordering::SeqCst);
            if !keep_sysproxy {
                self.clear_sysproxy();
            }
            let (tx, rx) = oneshot::channel();
            if r.kill.send(tx).is_ok() {
                let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
            }
            self.with(|s| s.groups.clear());
            self.emit(Event::Proxies);
        }
        self.set_status(CoreStatus::Stopped);
    }

    fn spawn_traffic(self: &Arc<Self>, generation: u64) {
        let this = self.clone();
        self.rt.spawn(async move {
            let api = this.api();
            while this.active.load(Ordering::SeqCst) == generation {
                let _ = api
                    .stream_traffic(|t| {
                        if this.active.load(Ordering::SeqCst) != generation {
                            return false;
                        }
                        this.with(|s| s.traffic = t);
                        this.emit(Event::Traffic(t));
                        true
                    })
                    .await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }

    fn render_config(&self, settings: &Settings, version: CoreVersion) -> BuiltConfig {
        let rules_dir = self.rules_dir();
        let built = self.with(|st| {
            let groups = st
                .subs
                .iter()
                .filter(|s| s.enabled)
                .map(|s| NodeGroup {
                    name: if s.name.is_empty() { "Subscription" } else { &s.name },
                    nodes: st.nodes.get(&s.id).map(Vec::as_slice).unwrap_or(&[]),
                })
                .collect();
            build_config(&BuildInput {
                settings,
                groups,
                version,
                rule_set_dir: Some(&rules_dir),
                providers: &st.providers,
            })
        });
        let mut meta = BuildMeta::default();
        for ob in built
            .config
            .get("outbounds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(built.config.get("endpoints").and_then(Value::as_array).into_iter().flatten())
        {
            let tag = ob["tag"].as_str().unwrap_or_default().to_string();
            let ty = ob["type"].as_str().unwrap_or_default().to_string();
            if ty == "selector" || ty == "urltest" {
                meta.groups.push(tag.clone());
            }
            meta.types.insert(tag, ty);
        }
        self.with(|s| s.meta = meta);
        built
    }

    /// The configuration that would be used right now (for "export config").
    pub fn preview_config(&self) -> Value {
        let settings = self.settings();
        let version = self.core_version().unwrap_or(CoreVersion { major: 1, minor: 14, patch: 0 });
        self.render_config(&settings, version).config
    }

    // --------------------------------------------------------- system proxy

    fn apply_sysproxy(&self) {
        let (enabled, port, bypass, running) = self.with(|s| {
            (
                s.settings.system_proxy,
                s.settings.mixed_port,
                s.settings.sysproxy_bypass.clone(),
                s.status.is_running(),
            )
        });
        if enabled && running {
            let server = format!("127.0.0.1:{port}");
            match sys::set_system_proxy(&server, &bypass) {
                Ok(()) => {
                    let _ = std::fs::write(self.sysproxy_marker(), &server);
                    self.with(|s| s.sysproxy_applied = true)
                }
                Err(e) => self.notice(NoticeKind::Error, Msg::new("notice.sysproxy_failed").arg(format!("{e:#}"))),
            }
        } else {
            self.clear_sysproxy();
        }
    }

    fn clear_sysproxy(&self) {
        if self.with(|s| std::mem::take(&mut s.sysproxy_applied)) {
            if let Err(e) = sys::clear_system_proxy() {
                self.notice(NoticeKind::Error, Msg::new("notice.sysproxy_failed").arg(format!("{e:#}")));
            }
            let _ = std::fs::remove_file(self.sysproxy_marker());
        }
    }

    fn sysproxy_marker(&self) -> PathBuf {
        self.store.paths.run_dir.join("sysproxy.applied")
    }

    /// Registers the login item. With "run as administrator" and an elevated
    /// process, a highest-privilege scheduled task avoids a UAC prompt at login;
    /// otherwise the Run key is used (the app then asks for elevation itself).
    fn apply_launch_at_login(&self, enabled: bool) {
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
        let use_task = enabled && self.settings().run_as_admin && sys::is_elevated();
        let result = (|| -> Result<()> {
            if use_task {
                sys::set_elevated_login_task(APP_NAME, Some((&exe, "--autostart")))?;
                sys::set_launch_at_login(APP_NAME, None)?;
            } else {
                let cmd = format!("\"{exe}\" --autostart");
                sys::set_launch_at_login(APP_NAME, enabled.then_some(cmd.as_str()))?;
                if sys::elevated_login_task_exists(APP_NAME) {
                    if let Err(e) = sys::set_elevated_login_task(APP_NAME, None) {
                        log::warn!("could not remove the elevated login task: {e:#}");
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.notice(NoticeKind::Error, Msg::new("notice.login_failed").arg(format!("{e:#}")));
        }
    }

    // ------------------------------------------------------------- settings

    /// Applies a settings change, persisting it and reloading only what is needed.
    pub fn update_settings(self: &Arc<Self>, f: impl FnOnce(&mut Settings)) {
        let (old, new) = self.with(|s| {
            let old = s.settings.clone();
            f(&mut s.settings);
            (old, s.settings.clone())
        });
        if old == new {
            return;
        }
        if let Err(e) = self.store.save_settings(&new) {
            self.notice(NoticeKind::Error, Msg::new("notice.save_failed").arg(format!("{e:#}")));
        }
        self.emit(Event::Settings);
        let this = self.clone();
        self.rt.spawn(async move {
            if old.launch_at_login != new.launch_at_login || old.run_as_admin != new.run_as_admin {
                this.apply_launch_at_login(new.launch_at_login);
            }
            if old.core_path != new.core_path {
                this.with(|s| s.core_version = None);
                this.refresh_core_version().await;
            }
            if !this.status().is_running() {
                return;
            }
            if old.mode != new.mode {
                if let Err(e) = this.api().set_mode(new.mode.as_str()).await {
                    this.notice(NoticeKind::Error, Msg::new("notice.mode_failed").arg(format!("{e:#}")));
                }
            }
            let version = this.core_version().unwrap_or(CoreVersion::MIN_SUPPORTED);
            let mut old_cmp = old.clone();
            old_cmp.mode = new.mode;
            let config_changed = this.render_config(&old_cmp, version).config
                != this.render_config(&new, version).config;
            if config_changed {
                let _ = this.restart_async().await;
            } else if old.system_proxy != new.system_proxy
                || old.sysproxy_bypass != new.sysproxy_bypass
            {
                this.apply_sysproxy();
            }
        });
    }

    pub fn set_mode(self: &Arc<Self>, mode: ProxyMode) {
        self.update_settings(|s| s.mode = mode);
    }

    // -------------------------------------------------------- subscriptions

    /// Adds a subscription. `local_content` makes it a local (pasted) one.
    pub fn add_subscription(
        self: &Arc<Self>,
        mut sub: Subscription,
        local_content: Option<String>,
    ) -> Result<String> {
        sub.id = new_id();
        if let Some(content) = &local_content {
            let nodes = self.process_content(&sub, content)?;
            sub.node_count = nodes.len();
            sub.last_update = Some(now_unix());
            self.store.save_raw(&sub.id, content)?;
            self.store.save_nodes(&sub.id, &nodes)?;
            self.with(|s| s.nodes.insert(sub.id.clone(), nodes));
        }
        let id = sub.id.clone();
        let remote = sub.is_remote();
        self.with(|s| s.subs.push(sub));
        self.save_subs();
        if remote {
            self.update_subscription(&id);
        } else {
            let this = self.clone();
            self.rt.spawn(async move { this.after_nodes_changed().await });
        }
        self.wake_scheduler.notify_one();
        Ok(id)
    }

    /// Saves edits to an existing subscription.
    pub fn save_subscription(self: &Arc<Self>, sub: Subscription, local_content: Option<String>) -> Result<()> {
        let old = self.subscription(&sub.id).ok_or_else(|| anyhow!("subscription not found"))?;
        let mut sub = sub;
        let mut nodes_changed = false;
        if let Some(content) = &local_content {
            let nodes = self.process_content(&sub, content)?;
            sub.node_count = nodes.len();
            sub.last_update = Some(now_unix());
            self.store.save_raw(&sub.id, content)?;
            self.store.save_nodes(&sub.id, &nodes)?;
            self.with(|s| s.nodes.insert(sub.id.clone(), nodes));
            nodes_changed = true;
        }
        let refetch = sub.is_remote()
            && (old.url != sub.url || (old.user_agent != sub.user_agent) || old.last_update.is_none());
        let refilter = !refetch && (old.include != sub.include || old.exclude != sub.exclude);
        let id = sub.id.clone();
        self.with(|s| {
            if let Some(slot) = s.subs.iter_mut().find(|x| x.id == id) {
                *slot = sub.clone();
            }
        });
        if refilter {
            match self.store.load_raw(&id) {
                Some(raw) => {
                    let nodes = self.process_content(&sub, &raw)?;
                    self.with(|s| {
                        if let Some(x) = s.subs.iter_mut().find(|x| x.id == id) {
                            x.node_count = nodes.len();
                        }
                        s.nodes.insert(id.clone(), nodes.clone());
                    });
                    self.store.save_nodes(&id, &nodes)?;
                    nodes_changed = true;
                }
                None if sub.is_remote() => {
                    self.save_subs();
                    self.update_subscription(&id);
                    return Ok(());
                }
                None => {}
            }
        }
        self.save_subs();
        self.wake_scheduler.notify_one();
        if refetch {
            self.update_subscription(&id);
        } else if nodes_changed || old.enabled != sub.enabled || old.name != sub.name {
            let this = self.clone();
            self.rt.spawn(async move { this.after_nodes_changed().await });
        }
        Ok(())
    }

    pub fn set_subscription_enabled(self: &Arc<Self>, id: &str, enabled: bool) {
        if let Some(mut sub) = self.subscription(id) {
            sub.enabled = enabled;
            let _ = self.save_subscription(sub, None);
        }
    }

    pub fn remove_subscription(self: &Arc<Self>, id: &str) {
        self.with(|s| {
            s.subs.retain(|x| x.id != id);
            s.nodes.remove(id);
        });
        self.store.delete_nodes(id);
        self.save_subs();
        let this = self.clone();
        self.rt.spawn(async move { this.after_nodes_changed().await });
    }

    pub fn move_subscription(&self, id: &str, delta: isize) {
        self.with(|s| {
            if let Some(i) = s.subs.iter().position(|x| x.id == id) {
                let j = (i as isize + delta).clamp(0, s.subs.len() as isize - 1) as usize;
                let item = s.subs.remove(i);
                s.subs.insert(j, item);
            }
        });
        self.save_subs();
    }

    async fn after_nodes_changed(self: &Arc<Self>) {
        if self.status().is_running() {
            let _ = self.restart_async().await;
        } else if self.settings().auto_start_core
            && self.total_nodes() > 0
            && self.core_installed()
            && matches!(self.status(), CoreStatus::Stopped)
            && self.with(|s| s.subs.len() == 1)
        {
            // First subscription added: get the user online right away.
            let _ = self.start_async().await;
        }
    }

    fn process_content(&self, sub: &Subscription, content: &str) -> Result<Vec<Node>> {
        let parsed = parse_subscription(content).map_err(|e| anyhow!(e))?;
        for s in parsed.skipped.iter().take(20) {
            self.log_app(LogLevel::Warn, format!("{}: skipped {s}", sub.name));
        }
        let total = parsed.nodes.len();
        let nodes = filter_nodes(parsed.nodes, &sub.include, &sub.exclude).map_err(|e| anyhow!(e))?;
        if nodes.is_empty() {
            if total > 0 {
                bail!("all {total} nodes were removed by the filters");
            }
            bail!(
                "no supported nodes found ({} format{})",
                parsed.format.label(),
                if parsed.skipped.is_empty() {
                    String::new()
                } else {
                    format!(", {} skipped", parsed.skipped.len())
                }
            );
        }
        Ok(nodes)
    }

    pub fn update_subscription(self: &Arc<Self>, id: &str) {
        let this = self.clone();
        let id = id.to_string();
        self.rt.spawn(async move {
            this.update_many(vec![id], false).await;
        });
    }

    pub fn update_all(self: &Arc<Self>) {
        let ids: Vec<String> = self.with(|s| {
            s.subs
                .iter()
                .filter(|x| x.enabled && x.is_remote())
                .map(|x| x.id.clone())
                .collect()
        });
        if ids.is_empty() {
            self.notice(NoticeKind::Info, Msg::new("notice.no_remote_subs"));
            return;
        }
        let this = self.clone();
        self.rt.spawn(async move {
            this.update_many(ids, false).await;
        });
    }

    /// Updates several subscriptions concurrently, then reloads the core once.
    async fn update_many(self: &Arc<Self>, ids: Vec<String>, quiet: bool) {
        let results: Vec<bool> = futures_util::stream::iter(ids)
            .map(|id| {
                let this = self.clone();
                async move { this.update_one(&id, quiet).await }
            })
            .buffer_unordered(4)
            .collect()
            .await;
        if results.iter().any(|c| *c) && self.settings().sub_reload_after_update {
            self.after_nodes_changed().await;
        }
    }

    /// Returns whether the node list changed.
    async fn update_one(self: &Arc<Self>, id: &str, quiet: bool) -> bool {
        let Some(sub) = self.subscription(id) else { return false };
        if !sub.is_remote() || !self.with(|s| s.updating.insert(id.to_string())) {
            return false;
        }
        self.emit(Event::Subscriptions);
        let settings = self.settings();
        let running = self.status().is_running();
        let proxy = Route::Proxy(self.mixed_proxy_url());
        let routes = match (sub.update_via, running) {
            (UpdateVia::Direct, _) | (_, false) => vec![Route::Direct],
            (UpdateVia::Auto, true) => vec![proxy, Route::Direct],
            (UpdateVia::Proxy, true) => vec![proxy],
        };
        let ua = if sub.user_agent.trim().is_empty() {
            settings.sub_user_agent.clone()
        } else {
            sub.user_agent.clone()
        };
        let timeout = Duration::from_secs(settings.sub_timeout_secs.max(5));
        let result = async {
            let fetched = fetch::fetch_subscription(&sub.url, &ua, timeout, &routes, 2).await?;
            let nodes = self.process_content(&sub, &fetched.body)?;
            Ok::<_, anyhow::Error>((fetched, nodes))
        }
        .await;

        let now = now_unix();
        let name = if sub.name.is_empty() { sub.url.clone() } else { sub.name.clone() };
        let changed = match result {
            Ok((fetched, nodes)) => {
                let _ = self.store.save_raw(id, &fetched.body);
                let _ = self.store.save_nodes(id, &nodes);
                let count = nodes.len();
                let changed = self.with(|s| {
                    let changed = s.nodes.get(id) != Some(&nodes);
                    s.nodes.insert(id.to_string(), nodes);
                    if let Some(x) = s.subs.iter_mut().find(|x| x.id == id) {
                        x.last_update = Some(now);
                        x.last_attempt = Some(now);
                        x.last_error = None;
                        x.node_count = count;
                        x.info = fetched.info.clone().or(x.info.take());
                        if fetched.web_page.is_some() {
                            x.web_page = fetched.web_page.clone();
                        }
                        if x.name.trim().is_empty() {
                            x.name = fetched.title.clone().unwrap_or_else(|| host_of(&x.url));
                        }
                        if let Some(h) = fetched.update_interval_hours {
                            // Respect the provider's hint only when the user kept the default.
                            if x.interval_min == settings.sub_default_interval_min && h > 0 {
                                x.interval_min = h * 60;
                            }
                        }
                    }
                    changed
                });
                self.log_app(
                    LogLevel::Info,
                    format!("updated {name} via {}: {count} nodes", fetched.route),
                );
                if !quiet {
                    self.notice(NoticeKind::Success, Msg::new("notice.sub_updated").arg(&name).arg(count));
                }
                changed
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.with(|s| {
                    if let Some(x) = s.subs.iter_mut().find(|x| x.id == id) {
                        x.last_attempt = Some(now);
                        x.last_error = Some(msg.clone());
                    }
                });
                self.log_app(LogLevel::Error, format!("update {name} failed: {msg}"));
                self.notice(NoticeKind::Error, Msg::new("notice.sub_failed").arg(&name).arg(&msg));
                false
            }
        };
        self.with(|s| s.updating.remove(id));
        self.save_subs();
        changed
    }

    async fn scheduler(self: &Arc<Self>) {
        loop {
            let now = now_unix();
            let (due, next): (Vec<String>, Option<i64>) = self.with(|s| {
                let mut due = Vec::new();
                let mut next: Option<i64> = None;
                for sub in &s.subs {
                    let Some(at) = sub.next_due() else { continue };
                    if at <= now {
                        if !s.updating.contains(&sub.id) {
                            due.push(sub.id.clone());
                        }
                    } else {
                        next = Some(next.map_or(at, |n| n.min(at)));
                    }
                }
                (due, next)
            });
            let (due_rules, next_rules): (Vec<String>, Option<i64>) = self.with(|s| {
                let mut due = Vec::new();
                let mut next: Option<i64> = None;
                for p in &s.providers {
                    let Some(at) = p.next_due() else { continue };
                    if at <= now {
                        if !s.updating.contains(&p.id) {
                            due.push(p.id.clone());
                        }
                    } else {
                        next = Some(next.map_or(at, |n| n.min(at)));
                    }
                }
                (due, next)
            });
            if !due.is_empty() {
                self.log_app(LogLevel::Info, format!("scheduled update of {} subscription(s)", due.len()));
                self.update_many(due, true).await;
                continue;
            }
            if !due_rules.is_empty() {
                self.update_providers(due_rules, true).await;
                continue;
            }
            let next = match (next, next_rules) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let wait = next.map_or(300, |n| (n - now).clamp(5, 300)) as u64;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                _ = self.wake_scheduler.notified() => {}
            }
        }
    }

    // --------------------------------------------------------- rule providers

    pub fn rule_providers(&self) -> Vec<RuleProvider> {
        self.with(|s| s.providers.clone())
    }

    fn save_providers(&self) {
        let providers = self.rule_providers();
        if let Err(e) = self.store.save_rule_providers(&providers) {
            self.notice(NoticeKind::Error, Msg::new("notice.save_failed").arg(format!("{e:#}")));
        }
        self.emit(Event::RuleProviders);
    }

    /// Adds or replaces a rule provider and fetches it.
    pub fn save_rule_provider(self: &Arc<Self>, mut p: RuleProvider) -> String {
        let is_new = p.id.is_empty();
        if is_new {
            p.id = new_id();
        }
        let id = p.id.clone();
        let refetch = self.with(|s| match s.providers.iter_mut().find(|x| x.id == id) {
            Some(slot) => {
                let refetch = slot.source != p.source || slot.format != p.format || slot.stored.is_none();
                *slot = p;
                refetch
            }
            None => {
                s.providers.push(p);
                true
            }
        });
        self.save_providers();
        let this = self.clone();
        let id2 = id.clone();
        self.rt.spawn(async move {
            if refetch {
                this.update_providers(vec![id2], false).await;
            } else {
                this.reload_if_config_changed().await;
            }
        });
        self.wake_scheduler.notify_one();
        id
    }

    pub fn set_rule_provider_enabled(self: &Arc<Self>, id: &str, enabled: bool) {
        if let Some(mut p) = self.with(|s| s.providers.iter().find(|x| x.id == id).cloned()) {
            p.enabled = enabled;
            self.save_rule_provider(p);
        }
    }

    pub fn remove_rule_provider(self: &Arc<Self>, id: &str) {
        let removed = self.with(|s| {
            let i = s.providers.iter().position(|x| x.id == id)?;
            Some(s.providers.remove(i))
        });
        if let Some(p) = removed {
            if let Some(f) = crate::config::provider_file(&self.rules_dir(), &p) {
                let _ = std::fs::remove_file(f);
            }
        }
        self.save_providers();
        let this = self.clone();
        self.rt.spawn(async move { this.reload_if_config_changed().await });
    }

    /// Moves a provider up (negative) or down; earlier providers take priority.
    pub fn move_rule_provider(self: &Arc<Self>, id: &str, delta: isize) {
        self.with(|s| {
            if let Some(i) = s.providers.iter().position(|x| x.id == id) {
                let j = (i as isize + delta).clamp(0, s.providers.len() as isize - 1) as usize;
                let item = s.providers.remove(i);
                s.providers.insert(j, item);
            }
        });
        self.save_providers();
        let this = self.clone();
        self.rt.spawn(async move { this.reload_if_config_changed().await });
    }

    pub fn update_rule_provider(self: &Arc<Self>, id: &str) {
        let this = self.clone();
        let id = id.to_string();
        self.rt.spawn(async move {
            this.update_providers(vec![id], false).await;
        });
    }

    pub fn update_all_rule_providers(self: &Arc<Self>) {
        let ids: Vec<String> = self.with(|s| s.providers.iter().filter(|p| p.enabled).map(|p| p.id.clone()).collect());
        let this = self.clone();
        self.rt.spawn(async move {
            let total = ids.len();
            let ok = this.update_providers(ids, true).await;
            let kind = if ok == total { NoticeKind::Success } else { NoticeKind::Warning };
            this.notice(kind, Msg::new("notice.rules_batch").arg(ok).arg(total - ok));
        });
    }

    /// Imports the `ruleset=` lines of an ACL4SSR / subconverter `.ini` (from a
    /// URL or pasted text). Returns how many rule-sets were added.
    pub async fn import_acl_ini(self: &Arc<Self>, source: &str) -> Result<usize> {
        let source = source.trim();
        let text = if source.starts_with("http://") || source.starts_with("https://") {
            let data = self.download(source).await?;
            String::from_utf8_lossy(&data).into_owned()
        } else {
            source.to_string()
        };
        let settings = self.settings();
        let sets = rules::parse_acl_ini(&text, &settings.geosite_url, &settings.geoip_url);
        if sets.is_empty() {
            bail!("no `ruleset=` lines found");
        }
        let ids: Vec<String> = self.with(|s| {
            let mut ids = Vec::new();
            for r in &sets {
                if s.providers.iter().any(|p| p.source == r.source && p.target == r.target) {
                    continue;
                }
                let p = RuleProvider {
                    id: new_id(),
                    name: r.name.clone(),
                    source: r.source.clone(),
                    format: r.format,
                    target: r.target.clone(),
                    ..Default::default()
                };
                ids.push(p.id.clone());
                s.providers.push(p);
            }
            ids
        });
        self.save_providers();
        let n = ids.len();
        let this = self.clone();
        self.rt.spawn(async move {
            let ok = this.update_providers(ids.clone(), true).await;
            let failed = ids.len() - ok;
            let kind = if failed == 0 { NoticeKind::Success } else { NoticeKind::Warning };
            this.notice(kind, Msg::new("notice.rules_batch").arg(ok).arg(failed));
        });
        Ok(n)
    }

    /// Downloads a URL through the proxy when running, falling back to direct.
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let routes = if self.status().is_running() {
            vec![Route::Proxy(self.mixed_proxy_url()), Route::Direct]
        } else {
            vec![Route::Direct]
        };
        let mirror = self.settings().github_mirror.trim().to_string();
        let url = if !mirror.is_empty()
            && (url.starts_with("https://raw.githubusercontent.com/") || url.starts_with("https://github.com/"))
        {
            format!("{mirror}{url}")
        } else {
            url.to_string()
        };
        let mut last = anyhow!("no route");
        for route in &routes {
            let client = fetch::client(route, Duration::from_secs(30), "BoxPilot")?;
            match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
                Ok(resp) => return Ok(resp.bytes().await?.to_vec()),
                Err(e) => last = e.into(),
            }
        }
        Err(last)
    }

    async fn fetch_provider(&self, p: &RuleProvider) -> Result<rules::Converted> {
        let data = if p.is_remote() {
            self.download(p.source.trim()).await?
        } else {
            std::fs::read(p.source.trim()).with_context(|| format!("read {}", p.source.trim()))?
        };
        rules::convert_rules(&p.source, p.format, &data).map_err(|e| anyhow!(e))
    }

    /// Returns how many providers were updated successfully.
    async fn update_providers(self: &Arc<Self>, ids: Vec<String>, quiet: bool) -> usize {
        let mut ok = 0;
        let before = self.current_config_json();
        let dir = self.rules_dir();
        let _ = std::fs::create_dir_all(&dir);
        for id in ids {
            let Some(p) = self.with(|s| s.providers.iter().find(|x| x.id == id).cloned()) else { continue };
            if !self.with(|s| s.updating.insert(id.clone())) {
                continue;
            }
            self.emit(Event::RuleProviders);
            let now = now_unix();
            let result = self.fetch_provider(&p).await;
            let name = p.name.clone();
            match result {
                Ok(conv) => {
                    let stored = RuleProvider { stored: Some(conv.format), ..p.clone() };
                    // Remove a file of the other format left by an earlier conversion.
                    if let Some(old) = crate::config::provider_file(&dir, &p) {
                        let _ = std::fs::remove_file(old);
                    }
                    let path = crate::config::provider_file(&dir, &stored).unwrap();
                    let write = crate::store::write_atomic(&path, &conv.bytes);
                    self.with(|s| {
                        if let Some(x) = s.providers.iter_mut().find(|x| x.id == id) {
                            x.last_attempt = Some(now);
                            match &write {
                                Ok(()) => {
                                    x.stored = Some(conv.format);
                                    x.last_update = Some(now);
                                    x.last_error = None;
                                    x.rule_count = conv.rule_count;
                                }
                                Err(e) => x.last_error = Some(format!("{e:#}")),
                            }
                        }
                    });
                    if conv.skipped > 0 {
                        self.log_app(LogLevel::Warn, format!("{name}: skipped {} rules sing-box cannot use", conv.skipped));
                    }
                    self.log_app(LogLevel::Info, format!("rule-set {name}: {} rules", conv.rule_count));
                    if write.is_ok() {
                        ok += 1;
                    }
                    if !quiet {
                        self.notice(NoticeKind::Success, Msg::new("notice.rules_updated").arg(&name).arg(conv.rule_count));
                    }
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    self.with(|s| {
                        if let Some(x) = s.providers.iter_mut().find(|x| x.id == id) {
                            x.last_attempt = Some(now);
                            x.last_error = Some(msg.clone());
                        }
                    });
                    self.log_app(LogLevel::Error, format!("rule-set {name}: {msg}"));
                    if !quiet {
                        self.notice(NoticeKind::Error, Msg::new("notice.rules_failed").arg(&name).arg(&msg));
                    }
                }
            }
            self.with(|s| s.updating.remove(&id));
            self.save_providers();
        }
        // A changed file of an existing local rule-set is picked up by sing-box
        // itself; only a different configuration needs a reload.
        if self.current_config_json() != before {
            self.reload_if_running().await;
        }
        ok
    }

    fn current_config_json(&self) -> Value {
        let settings = self.settings();
        let version = self.core_version().unwrap_or(CoreVersion::MIN_SUPPORTED);
        self.render_config(&settings, version).config
    }

    async fn reload_if_config_changed(self: &Arc<Self>) {
        // Rendering is cheap; compare against the config the core runs with.
        let running = std::fs::read(self.store.paths.config_file())
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        if running.as_ref() != Some(&self.current_config_json()) {
            self.reload_if_running().await;
        }
    }

    async fn reload_if_running(self: &Arc<Self>) {
        if self.status().is_running() {
            let _ = self.restart_async().await;
        }
    }

    // -------------------------------------------------------------- proxies

    pub fn refresh_proxies(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move {
            let _ = this.refresh_proxies_async().await;
        });
    }

    async fn refresh_proxies_async(&self) -> Result<()> {
        if !self.status().is_running() {
            return Ok(());
        }
        let proxies = self.api().proxies().await?;
        self.with(|s| {
            let meta = &s.meta;
            let groups = meta
                .groups
                .iter()
                .filter_map(|g| proxies.get(g))
                .map(|p| ProxyGroup {
                    name: p.name.clone(),
                    kind: p.kind.clone(),
                    now: p.now.clone().unwrap_or_default(),
                    members: p
                        .all
                        .iter()
                        .flatten()
                        .map(|m| ProxyMember {
                            name: m.clone(),
                            protocol: meta
                                .types
                                .get(m)
                                .cloned()
                                .or_else(|| proxies.get(m).map(|x| x.kind.to_ascii_lowercase()))
                                .unwrap_or_default(),
                            is_group: meta.groups.contains(m),
                        })
                        .collect(),
                })
                .collect();
            s.groups = groups;
            for (name, p) in &proxies {
                if let Some(d) = p.last_delay().filter(|d| *d > 0) {
                    s.delays.entry(name.clone()).or_insert(Some(d));
                }
            }
        });
        self.emit(Event::Proxies);
        Ok(())
    }

    pub fn select_proxy(self: &Arc<Self>, group: &str, name: &str) {
        let this = self.clone();
        let (group, name) = (group.to_string(), name.to_string());
        // Optimistic update so the UI reacts instantly.
        self.with(|s| {
            if let Some(g) = s.groups.iter_mut().find(|g| g.name == group) {
                g.now = name.clone();
            }
        });
        self.emit(Event::Proxies);
        self.rt.spawn(async move {
            if let Err(e) = this.api().select(&group, &name).await {
                this.notice(NoticeKind::Error, Msg::new("notice.select_failed").arg(&name).arg(format!("{e:#}")));
            }
            let _ = this.refresh_proxies_async().await;
        });
    }

    /// Measures latency of the given outbounds concurrently.
    pub fn test_delays(self: &Arc<Self>, names: Vec<String>) {
        if !self.status().is_running() {
            self.notice(NoticeKind::Warning, Msg::new("notice.start_core_first"));
            return;
        }
        let names: Vec<String> = self.with(|s| {
            names
                .into_iter()
                .filter(|n| s.testing.insert(n.clone()))
                .collect()
        });
        self.emit(Event::Delays);
        let this = self.clone();
        self.rt.spawn(async move {
            let api = this.api();
            let url = this.settings().urltest_url;
            futures_util::stream::iter(names)
                .for_each_concurrent(32, |name| {
                    let (api, url, this) = (api.clone(), url.clone(), this.clone());
                    async move {
                        let d = api.delay(&name, &url, DELAY_TIMEOUT_MS).await.ok();
                        this.with(|s| {
                            s.testing.remove(&name);
                            s.delays.insert(name, d);
                        });
                        this.emit(Event::Delays);
                    }
                })
                .await;
            let _ = this.refresh_proxies_async().await;
        });
    }

    // ----------------------------------------------------------------- core

    pub async fn refresh_core_version(&self) {
        let core = self.core_path();
        let v = if core.is_file() {
            process::detect_version(&core).await.ok()
        } else {
            None
        };
        self.with(|s| s.core_version = v);
        self.emit(Event::CoreInfo);
    }

    fn set_download(&self, d: CoreDownloadState) {
        self.with(|s| s.download = d);
        self.emit(Event::CoreDownload);
    }

    fn download_routes(&self) -> Vec<Route> {
        if self.status().is_running() {
            vec![Route::Proxy(self.mixed_proxy_url()), Route::Direct]
        } else {
            vec![Route::Direct]
        }
    }

    async fn find_release(&self) -> Result<(Release, Route)> {
        let prerelease = self.settings().core_prerelease;
        let mut last = anyhow!("no route");
        for route in self.download_routes() {
            let client = fetch::client(&route, Duration::from_secs(20), "BoxPilot")?;
            match core_download::latest_release(&client, prerelease).await {
                Ok(r) => return Ok((r, route)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Looks up the newest sing-box release on GitHub.
    pub fn check_core_update(self: &Arc<Self>) {
        let this = self.clone();
        self.set_download(CoreDownloadState::Checking);
        self.rt.spawn(async move {
            match this.find_release().await {
                Ok((r, _)) => {
                    let installed = this.core_version().map(|v| v.to_string());
                    let state = if installed.as_deref() == Some(r.version.as_str()) {
                        CoreDownloadState::UpToDate(r.version.clone())
                    } else {
                        CoreDownloadState::Available(r.version.clone())
                    };
                    this.with(|s| s.latest_release = Some(r));
                    this.set_download(state);
                }
                Err(e) => this.set_download(CoreDownloadState::Failed(format!("{e:#}"))),
            }
        });
    }

    /// Downloads and installs the newest core (restarting the running one).
    pub fn install_core(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = this.install_core_async().await {
                let msg = format!("{e:#}");
                this.log_app(LogLevel::Error, format!("core download failed: {msg}"));
                this.set_download(CoreDownloadState::Failed(msg.clone()));
                this.notice(NoticeKind::Error, Msg::new("notice.core_download_failed").arg(&msg));
            }
        });
    }

    async fn install_core_async(self: &Arc<Self>) -> Result<()> {
        self.set_download(CoreDownloadState::Checking);
        let (release, route) = self.find_release().await?;
        let mirror = self.settings().github_mirror;
        let client = fetch::client(&route, Duration::from_secs(600), "BoxPilot")?;
        self.log_app(
            LogLevel::Info,
            format!("downloading {} via {}", release.asset_name, route.label()),
        );
        let mut last_emit = Instant::now();
        let data = core_download::download(&client, &release, &mirror, |done, total| {
            if last_emit.elapsed() > Duration::from_millis(100) || done == total {
                last_emit = Instant::now();
                self.set_download(CoreDownloadState::Downloading { done, total });
            }
        })
        .await?;
        self.set_download(CoreDownloadState::Installing);
        let was_running = self.status().is_running();
        let mut slot = self.proc.lock().await;
        if was_running {
            self.stop_locked(&mut slot, true).await;
        }
        let dest = self.paths().core_dir.clone();
        let installed = {
            let dest = dest.clone();
            let name = release.asset_name.clone();
            tokio::task::spawn_blocking(move || core_download::install_archive(&name, &data, &dest)).await?
        };
        if installed.is_ok() && !self.settings().core_path.is_empty() {
            // Use the downloaded core instead of the custom path.
            self.with(|s| s.settings.core_path.clear());
            let _ = self.store.save_settings(&self.settings());
            self.emit(Event::Settings);
        }
        self.with(|s| s.core_version = None);
        self.refresh_core_version().await;
        let version = self.core_version().map(|v| v.to_string()).unwrap_or(release.version.clone());
        if was_running {
            let _ = self.start_locked(&mut slot).await;
        }
        drop(slot);
        installed?;
        self.set_download(CoreDownloadState::UpToDate(version.clone()));
        self.notice(NoticeKind::Success, Msg::new("notice.core_installed").arg(&version));
        if !was_running && self.settings().auto_start_core && self.total_nodes() > 0 {
            let _ = self.start_async().await;
        }
        Ok(())
    }
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split(['/', '?', ':']).next())
        .unwrap_or(url)
        .to_string()
}
