//! The engine owns all application state and runs every background job on a
//! Tokio runtime. The UI reads state through getters and is told about changes
//! through [`Event`]s.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::process::Child;
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::clash_api::{ClashApi, Traffic};
use crate::config::{BuildInput, BuiltConfig, CoreVersion, NodeGroup, RuleSetSpec, build_config, rule_set_file};
use crate::convert::{filter_nodes, parse_subscription};
use crate::core_download::{self, Release};
use crate::fetch::{self, Route};
use crate::geo;
use crate::model::*;
use crate::process::{self, LogLevel, LogLine};
use crate::store::{APP_NAME, Paths, Store};
use crate::rules::{self, RuleProvider};
use crate::speed;
use crate::sys;
use crate::update;

pub const TUN_NEEDS_ADMIN: &str = "TUN mode requires administrator rights";
const MAX_LOGS: usize = 2000;
/// Config changes arriving within this window share one core reload.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(1200);
const DELAY_TIMEOUT_MS: u32 = 5000;
/// Exit checks are slow-ish; each node's answer is reused this long.
const EXIT_REUSE: Duration = Duration::from_secs(6 * 3600);

/// The node traffic leaves through: Proxy's choice, followed through any
/// groups. `None` for direct, reject or an unresolved group.
fn current_node(groups: &[ProxyGroup]) -> Option<String> {
    let mut now = groups.iter().find(|g| g.name == crate::config::TAG_PROXY)?.now.clone();
    for _ in 0..8 {
        match groups.iter().find(|g| g.name == now) {
            Some(g) if !g.now.is_empty() => now = g.now.clone(),
            Some(_) => return None,
            None => break,
        }
    }
    // Still a group after that many hops: a loop.
    if groups.iter().any(|g| g.name == now) {
        return None;
    }
    let builtin = now.is_empty() || now == crate::config::TAG_DIRECT || now == "block" || now == "reject";
    (!builtin).then_some(now)
}

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
    /// New lines in [`Engine::logs`].
    Log,
    Traffic(Traffic),
    Subscriptions,
    Settings,
    CoreInfo,
    CoreDownload,
    /// [`Engine::update_info`] changed.
    Update,
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

/// Runs sing-box somewhere other than a child process. The mobile apps embed
/// libbox in their VPN service: the engine hands over the generated config and
/// still drives the core only through the Clash API, as on desktop.
pub trait CoreHost: Send + Sync + 'static {
    /// Version of the embedded core.
    fn version(&self) -> CoreVersion;
    /// Starts the core with this configuration (JSON text). Resolves once the
    /// platform accepted it; readiness is then checked through the Clash API.
    fn start(&self, config: String) -> BoxFuture<'static, Result<()>>;
    fn stop(&self) -> BoxFuture<'static, ()>;
    /// Whether the embedded core was built with this outbound type. Mobile builds
    /// leave out the heaviest ones (naive brings a whole browser network stack).
    fn supports(&self, _protocol: &str) -> bool {
        true
    }
    /// Whether the core may start without the user asking, i.e. the platform's
    /// VPN permission is already granted. Asking for it is the user's call, so
    /// "connect on launch" never brings up a permission prompt.
    fn can_autostart(&self) -> bool {
        true
    }
}

/// What a reset clears. The downloaded sing-box core is always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetScope {
    /// Settings (and the login start they set up).
    pub settings: bool,
    /// Subscriptions with their nodes.
    pub subscriptions: bool,
    /// Imported rule-sets.
    pub rule_sets: bool,
    /// Generated config, core cache (selections, fake-IP), downloaded rule-sets, GeoIP data.
    pub caches: bool,
}

impl ResetScope {
    pub const ALL: ResetScope = ResetScope { settings: true, subscriptions: true, rule_sets: true, caches: true };

    pub fn is_empty(&self) -> bool {
        !(self.settings || self.subscriptions || self.rule_sets || self.caches)
    }
}

/// The app-update state shown in Settings.
#[derive(Debug, Clone, Default)]
pub struct UpdateInfo {
    pub checking: bool,
    pub installing: bool,
    /// A newer release, once found.
    pub latest: Option<update::Release>,
    /// Version swapped in, waiting for a restart.
    pub installed: Option<String>,
    pub error: Option<String>,
}

/// A group BoxPilot picks the node for.
struct Picker {
    tag: String,
    strategy: AutoStrategy,
    region: String,
}

#[derive(Default, Clone)]
struct BuildMeta {
    groups: Vec<String>,
    types: HashMap<String, String>,
    /// Nodes left out because the embedded core lacks their protocol.
    unsupported: usize,
    /// Rule-set groups: group tag -> provider id.
    rule_groups: HashMap<String, String>,
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
    /// The core's latest error line, kept even while recording is off.
    last_error: Option<Arc<str>>,
    traffic: Traffic,
    providers: Vec<RuleProvider>,
    /// Country of each proxy outbound, by tag.
    countries: HashMap<String, geo::Country>,
    /// Exit checks are slow-ish; each node's answer is reused for a while.
    exit_checked: HashMap<String, (geo::Country, Instant)>,
    update: UpdateInfo,
    speeds: HashMap<String, speed::SpeedResult>,
    /// Latest real-connection latency per node (`None` = failed).
    real_delays: HashMap<String, Option<u32>>,
    real_testing: HashSet<String>,
    speed_testing: HashSet<String>,
    /// Recent unexpected core exits, for the automatic restart's limit.
    crashes: Vec<Instant>,
    /// The node traffic currently leaves through, once its exit was checked.
    exit_watched: Option<String>,
    /// Rule-sets open in a text editor, watched for saves.
    editing: HashSet<String>,
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
    /// Wakes the Auto-group driver (core started, strategy changed).
    wake_auto: Notify,
    /// One country refresh at a time.
    geo_lock: tokio::sync::Mutex<()>,
    /// The probe selector points at one node at a time (exit and speed checks).
    probe_lock: tokio::sync::Mutex<()>,
    /// Latest reload request; an older pending one gives way to it.
    reload_ticket: AtomicU64,
    /// Embedded core (mobile); `None` runs the sing-box executable.
    host: Option<Arc<dyn CoreHost>>,
    /// Told by the host when the embedded core stops on its own.
    host_exit: Mutex<Option<oneshot::Sender<String>>>,
}

impl Engine {
    pub fn new(rt: Handle) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        Self::with_paths(rt, Paths::detect())
    }

    pub fn with_paths(rt: Handle, paths: Paths) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        Self::build(rt, paths, None)
    }

    /// An engine whose core runs inside the platform (see [`CoreHost`]).
    pub fn with_host(
        rt: Handle,
        paths: Paths,
        host: Arc<dyn CoreHost>,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        Self::build(rt, paths, Some(host))
    }

    fn build(
        rt: Handle,
        paths: Paths,
        host: Option<Arc<dyn CoreHost>>,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Event>)> {
        let store = Store::new(paths)?;
        let mut settings = store.load_settings();
        if settings.clash_api_secret.is_empty() {
            settings.clash_api_secret = random_secret();
            store.save_settings(&settings)?;
        }
        if settings.probe_port == 0 {
            // Any free port, kept from then on so the config stays stable.
            if let Ok(port) = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()) {
                settings.probe_port = port;
                store.save_settings(&settings)?;
            }
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
                last_error: None,
                traffic: Traffic::default(),
                providers,
                countries: HashMap::new(),
                exit_checked: HashMap::new(),
                update: UpdateInfo::default(),
                speeds: HashMap::new(),
                real_delays: HashMap::new(),
                real_testing: HashSet::new(),
                speed_testing: HashSet::new(),
                crashes: Vec::new(),
                exit_watched: None,
                editing: HashSet::new(),
            }),
            tx,
            proc: tokio::sync::Mutex::new(None),
            geo_lock: tokio::sync::Mutex::new(()),
            probe_lock: tokio::sync::Mutex::new(()),
            reload_ticket: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            active: AtomicU64::new(0),
            job: if host.is_none() { sys::KillOnCloseJob::new() } else { None },
            host,
            host_exit: Mutex::new(None),
            wake_scheduler: Notify::new(),
            wake_auto: Notify::new(),
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

    /// The subscriptions, without the Local nodes group.
    pub fn subscriptions(&self) -> Vec<Subscription> {
        self.with(|s| s.subs.iter().filter(|x| !x.is_local_nodes()).cloned().collect())
    }

    /// The Local nodes group, once a node was added.
    pub fn local_group(&self) -> Option<Subscription> {
        self.subscription(LOCAL_NODES_ID)
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
        self.host.is_some() || self.core_path().is_file()
    }

    /// Whether sing-box is embedded (mobile) rather than a downloaded executable.
    pub fn embedded_core(&self) -> bool {
        self.host.is_some()
    }

    /// The embedded core stopped by itself (VPN revoked, service killed).
    pub fn core_exited(&self, reason: impl Into<String>) {
        if let Some(tx) = self.host_exit.lock().unwrap().take() {
            let _ = tx.send(reason.into());
        }
    }

    /// A log line written by the embedded core.
    pub fn push_core_log(&self, raw: &str) {
        self.push_log(process::parse_log_line(raw));
    }

    fn push_core_line(&self, line: LogLine) {
        self.push_log(line);
    }

    /// Whether TUN can be used: an elevated app on Windows; on macOS / Linux
    /// either root or a core binary granted the privilege. The platform's VPN
    /// consent covers an embedded core.
    pub fn tun_permitted(&self) -> bool {
        self.host.is_some() || sys::is_elevated() || sys::core_tun_privileged(&self.core_path())
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
        let kept = self.with(|s| {
            if line.level >= LogLevel::Error && !line.message.starts_with("[app]") {
                s.last_error = Some(line.message.clone());
            }
            if !s.settings.record_logs {
                return false;
            }
            if s.logs.len() >= MAX_LOGS {
                s.logs.pop_front();
            }
            s.logs.push_back(line);
            true
        });
        if kept {
            self.emit(Event::Log);
        }
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
        self.with(|s| s.last_error.as_deref().unwrap_or_default().to_string())
    }

    fn save_subs(&self) {
        let subs = self.with(|s| s.subs.clone());
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
        // App self-updates replace the executable; the mobile apps update through their stores.
        if self.host.is_none() {
            self.rt.spawn(self.clone().update_loop());
        }
        self.rt.spawn(self.clone().auto_loop());
        let this = self.clone();
        self.rt.spawn(async move {
            let s = this.settings();
            if this.host.is_none() {
                this.clean_up_after_crash();
            }
            if this.host.is_none() && s.launch_at_login {
                this.apply_launch_at_login(true);
            }
            this.refresh_core_version().await;

            let had_nodes = this.total_nodes() > 0;
            let autostart = s.auto_start_core && this.host.as_ref().is_none_or(|h| h.can_autostart());
            if autostart && had_nodes && this.core_installed() {
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
            if autostart
                && !this.status().is_running()
                && !had_nodes
                && this.total_nodes() > 0
                && this.core_installed()
            {
                let _ = this.start_async().await;
            }
            if !this.status().is_running() {
                // Otherwise the core start already did it.
                this.refresh_countries();
            }
            this.scheduler().await;
        });
    }

    /// Desktop: what a crashed session left behind.
    fn clean_up_after_crash(&self) {
        // A core from a crashed session would hold the ports; stop it.
        if let Some(pid) = std::fs::read_to_string(self.pid_file()).ok().and_then(|p| p.trim().parse().ok()) {
            sys::kill_stale_core(pid);
            let _ = std::fs::remove_file(self.pid_file());
        }
        // A marker means we set the system proxy and did not get to restore it
        // (crash or power loss). Only then is it safe to clear it.
        let marker = self.sysproxy_marker();
        if let Ok(server) = std::fs::read_to_string(&marker) {
            let state = sys::system_proxy_state();
            if state.enabled && state.server == server.trim() {
                let _ = sys::clear_system_proxy();
            }
            let _ = std::fs::remove_file(&marker);
        }
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

    /// Back to a fresh install for the chosen parts: stops the core (restoring
    /// the system proxy), removes login start when settings go, and deletes
    /// the files. Nothing is saved afterwards; the caller restarts the app.
    pub fn reset_blocking(&self, scope: ResetScope) {
        self.store.freeze();
        self.shutdown_blocking();
        let paths = &self.store.paths;
        let remove = |path: PathBuf| {
            let result = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            if let Err(e) = result.or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
                log::warn!("reset: could not remove {}: {e}", path.display());
            }
        };
        if scope.settings && self.host.is_none() {
            if let Err(e) = sys::set_launch_at_login(APP_NAME, None) {
                log::warn!("reset: could not remove login start: {e:#}");
            }
            if sys::elevated_login_task_exists(APP_NAME) {
                if let Err(e) = sys::set_elevated_login_task(APP_NAME, None) {
                    log::warn!("reset: could not remove the elevated login task: {e:#}");
                }
            }
        }
        if scope.settings {
            remove(paths.settings_file());
            remove(paths.root.join("settings.broken.json"));
        }
        if scope.subscriptions {
            remove(paths.subs_file());
            remove(paths.subs_dir.clone());
        }
        if scope.rule_sets {
            remove(paths.rules_file());
            remove(paths.rules_dir());
        }
        if scope.caches {
            remove(paths.run_dir.clone());
            remove(paths.root.join("geoip"));
        }
        log::info!("reset: {scope:?}");
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
        let settings = self.settings();
        let (version, built, generation) = match self.host.clone() {
            Some(host) => {
                let version = host.version();
                self.with(|s| s.core_version = Some(version));
                let built = self.prepare_config(&settings, version).await?;
                // Errors from an earlier run must not explain this one.
                self.with(|s| s.last_error = None);
                let (exit_tx, mut exit_rx) = oneshot::channel();
                *self.host_exit.lock().unwrap() = Some(exit_tx);
                host.start(serde_json::to_string(&built.config)?)
                    .await
                    .context("could not start the core")?;
                let ready = self.wait_ready(|| exit_rx.try_recv().ok()).await;
                if let Err(e) = ready {
                    host.stop().await;
                    return Err(e);
                }
                let (generation, kill_rx) = self.next_run(slot);
                self.rt.spawn(self.clone().watch_hosted(host, exit_rx, kill_rx, generation));
                (version, built, generation)
            }
            None => {
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
                if settings.tun_enabled && !self.tun_permitted() {
                    bail!(TUN_NEEDS_ADMIN);
                }
                let built = self.prepare_config(&settings, version).await?;
                let run_dir = self.store.paths.run_dir.clone();
                let cfg_path = self.store.paths.config_file();
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
                // Errors from an earlier run must not explain this one.
                self.with(|s| s.last_error = None);
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

                let ready = self
                    .wait_ready(|| match child.try_wait() {
                        Ok(Some(status)) => Some(status.to_string()),
                        Ok(None) => None,
                        Err(e) => Some(e.to_string()),
                    })
                    .await;
                if let Err(e) = ready {
                    let _ = child.kill().await;
                    return Err(e);
                }
                let (generation, kill_rx) = self.next_run(slot);
                self.rt.spawn(self.clone().watch(child, kill_rx, generation));
                (version, built, generation)
            }
        };

        if let Err(e) = self.api().set_mode(settings.mode.as_str()).await {
            self.log_app(LogLevel::Warn, format!("could not set mode: {e:#}"));
        }
        self.set_status(CoreStatus::Running);
        self.log_app(
            LogLevel::Info,
            format!("sing-box {version} running with {} nodes", built.node_count),
        );
        self.apply_sysproxy();
        self.spawn_traffic(generation);
        if self.host.is_some() {
            self.spawn_core_logs(generation);
        }
        self.wake_auto.notify_one();
        self.spawn_rule_set_maintenance(generation, built.rule_sets, !built.missing_rule_sets.is_empty());
        let _ = self.refresh_proxies_async().await;
        // Exit checks go through the core, so they run once it is up.
        self.refresh_countries();
        Ok(())
    }

    /// Renders the configuration (fetching missing rule-sets first) and writes it to disk.
    async fn prepare_config(&self, settings: &Settings, version: CoreVersion) -> Result<BuiltConfig> {
        let mut built = self.render_config(settings, version);
        if !built.missing_rule_sets.is_empty() {
            let n = self.download_rule_sets(&built.missing_rule_sets, &[Route::Direct]).await;
            if n > 0 {
                built = self.render_config(settings, version);
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
        let unsupported = self.with(|s| s.meta.unsupported);
        if unsupported > 0 {
            self.log_app(LogLevel::Warn, format!("left out {unsupported} node(s) whose protocol this core build does not include"));
        }
        let cfg_path = self.store.paths.config_file();
        crate::store::write_atomic(&cfg_path, &serde_json::to_vec_pretty(&built.config)?)?;
        Ok(built)
    }

    /// Waits for the Clash API to answer; `exited` reports a core that died meanwhile.
    async fn wait_ready(&self, mut exited: impl FnMut() -> Option<String>) -> Result<()> {
        let api = self.api();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = exited() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                bail!("sing-box exited ({status}) {}", self.last_core_error());
            }
            if api.version().await.is_ok() {
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!("sing-box did not become ready in time. {}", self.last_core_error());
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    /// Records a newly running core in `slot`: its generation and stop channel.
    fn next_run(&self, slot: &mut Option<Running>) -> (u64, oneshot::Receiver<oneshot::Sender<()>>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (kill_tx, kill_rx) = oneshot::channel();
        *slot = Some(Running { kill: kill_tx, generation });
        self.active.store(generation, Ordering::SeqCst);
        (generation, kill_rx)
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
                    this.request_reload();
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
                let code = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                self.exited_unexpectedly(generation, code).await;
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

    async fn watch_hosted(
        self: Arc<Self>,
        host: Arc<dyn CoreHost>,
        mut exit_rx: oneshot::Receiver<String>,
        mut kill_rx: oneshot::Receiver<oneshot::Sender<()>>,
        generation: u64,
    ) {
        tokio::select! {
            reason = &mut exit_rx => {
                drop(kill_rx);
                let reason = reason.unwrap_or_else(|_| "stopped".into());
                self.exited_unexpectedly(generation, reason).await;
            }
            req = &mut kill_rx => {
                // Our own stop: the host's exit report is expected, not a crash.
                self.host_exit.lock().unwrap().take();
                host.stop().await;
                if let Ok(done) = req {
                    let _ = done.send(());
                }
            }
        }
    }

    async fn exited_unexpectedly(self: &Arc<Self>, generation: u64, code: String) {
        let mut slot = self.proc.lock().await;
        if slot.as_ref().map(|r| r.generation) != Some(generation) {
            return;
        }
        *slot = None;
        self.active.store(0, Ordering::SeqCst);
        if self.with(|s| s.sysproxy_applied) {
            self.clear_sysproxy();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let msg = format!("sing-box exited unexpectedly ({code}). {}", self.last_core_error());
        self.log_app(LogLevel::Error, &msg);
        self.set_status(CoreStatus::Failed(msg.clone()));
        self.notice(NoticeKind::Error, Msg::new("notice.core_exited").arg(msg));
        // Bring it back, but at most 3 times in 10 minutes. A revoked VPN is the
        // user's (or another VPN app's) decision, so an embedded core stays down.
        if self.host.is_some() {
            return;
        }
        let recent = self.with(|s| {
            s.crashes.retain(|t| t.elapsed() < Duration::from_secs(600));
            s.crashes.push(Instant::now());
            s.crashes.len()
        });
        if recent <= 3 {
            let this = self.clone();
            self.rt.spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if matches!(this.status(), CoreStatus::Failed(_)) {
                    this.log_app(LogLevel::Info, "restarting the core after it exited");
                    // Not awaited here: the start future spawns this watcher.
                    this.start();
                }
            });
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

    /// The embedded core's log, read through the Clash API. With recording off only
    /// errors are read, so a failure can still be explained.
    fn spawn_core_logs(self: &Arc<Self>, generation: u64) {
        let this = self.clone();
        self.rt.spawn(async move {
            let api = this.api();
            while this.active.load(Ordering::SeqCst) == generation {
                let s = this.settings();
                let level = if !s.record_logs {
                    "error"
                } else {
                    match s.log_level.as_str() {
                        "trace" | "debug" => "debug",
                        "warn" | "warning" => "warning",
                        "error" | "fatal" | "panic" => "error",
                        _ => "info",
                    }
                };
                let _ = api
                    .stream_logs(level, |entry| {
                        if this.active.load(Ordering::SeqCst) != generation {
                            return false;
                        }
                        let level = match entry.level.as_str() {
                            "debug" => LogLevel::Debug,
                            "warning" => LogLevel::Warn,
                            "error" => LogLevel::Error,
                            _ => LogLevel::Info,
                        };
                        // Drop the "[connection-id duration]" prefix, as for the desktop's stdout.
                        let mut msg = entry.payload.trim();
                        if msg.starts_with('[') {
                            if let Some(end) = msg.find(']') {
                                msg = msg[end + 1..].trim_start();
                            }
                        }
                        this.push_core_line(LogLine::app(level, msg));
                        // A changed level or recording switch takes effect on the next stream.
                        this.settings().record_logs == s.record_logs && this.settings().log_level == s.log_level
                    })
                    .await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }

    fn render_config(&self, settings: &Settings, version: CoreVersion) -> BuiltConfig {
        // An embedded core always runs as the platform's VPN.
        let mobile;
        let settings = if self.host.is_some() {
            mobile = Settings { tun_enabled: true, ..settings.clone() };
            &mobile
        } else {
            settings
        };
        let rules_dir = self.rules_dir();
        let mut skipped = 0usize;
        let built = self.with(|st| {
            let supported: HashMap<&str, Vec<Node>> = match &self.host {
                Some(host) => st
                    .nodes
                    .iter()
                    .map(|(id, nodes)| {
                        let kept: Vec<Node> = nodes.iter().filter(|n| host.supports(&n.protocol)).cloned().collect();
                        skipped += nodes.len() - kept.len();
                        (id.as_str(), kept)
                    })
                    .collect(),
                None => HashMap::new(),
            };
            let groups = st
                .subs
                .iter()
                .filter(|s| s.enabled)
                .map(|s| NodeGroup {
                    name: if s.name.is_empty() { "Subscription" } else { &s.name },
                    nodes: match &self.host {
                        Some(_) => supported.get(s.id.as_str()).map(Vec::as_slice).unwrap_or(&[]),
                        None => st.nodes.get(&s.id).map(Vec::as_slice).unwrap_or(&[]),
                    },
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
        let mut meta = BuildMeta {
            unsupported: skipped,
            rule_groups: built.provider_groups.iter().map(|(id, tag)| (tag.clone(), id.clone())).collect(),
            ..BuildMeta::default()
        };
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
            if (ty == "selector" || ty == "urltest") && tag != crate::config::TAG_PROBE {
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
        // Mobile routes traffic through its VPN; there is no system proxy to set.
        if self.host.is_some() {
            return;
        }
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
        if self.host.is_some() {
            return;
        }
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
        if old.auto_strategy != new.auto_strategy
            || old.auto_region != new.auto_region
            || old.auto_test_nodes != new.auto_test_nodes
            || old.urltest_interval_min != new.urltest_interval_min
        {
            // Applied through the Clash API right away; the core keeps running.
            self.wake_auto.notify_one();
        }
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
                this.request_reload();
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
        // Local nodes stay the last group.
        self.with(|s| {
            let at = s.subs.iter().position(Subscription::is_local_nodes).unwrap_or(s.subs.len());
            s.subs.insert(at, sub);
        });
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

    /// Adds nodes from share links (or any subscription format) to the Local
    /// nodes group. Returns how many were added and the entries skipped.
    pub fn add_local_nodes(self: &Arc<Self>, text: &str) -> Result<(usize, Vec<String>)> {
        let parsed = parse_subscription(text).map_err(|e| anyhow!(e))?;
        if parsed.nodes.is_empty() {
            bail!("{}", parsed.skipped.first().cloned().unwrap_or_else(|| "no supported nodes found".into()));
        }
        let added = parsed.nodes.len();
        let nodes = self.with(|s| {
            if !s.subs.iter().any(Subscription::is_local_nodes) {
                s.subs.push(Subscription {
                    id: LOCAL_NODES_ID.into(),
                    name: LOCAL_NODES_NAME.into(),
                    auto_update: false,
                    update_on_start: false,
                    ..Default::default()
                });
            }
            let list = s.nodes.entry(LOCAL_NODES_ID.into()).or_default();
            list.extend(parsed.nodes);
            list.clone()
        });
        self.save_local_nodes(&nodes)?;
        Ok((added, parsed.skipped))
    }

    /// Removes one node from the Local nodes group; the group goes with its last node.
    pub fn remove_local_node(self: &Arc<Self>, index: usize) -> Result<()> {
        let nodes = self.with(|s| {
            let list = s.nodes.get_mut(LOCAL_NODES_ID)?;
            (index < list.len()).then(|| {
                list.remove(index);
                list.clone()
            })
        });
        match nodes {
            Some(nodes) if nodes.is_empty() => {
                self.remove_subscription(LOCAL_NODES_ID);
                Ok(())
            }
            Some(nodes) => self.save_local_nodes(&nodes),
            None => Ok(()),
        }
    }

    fn save_local_nodes(self: &Arc<Self>, nodes: &[Node]) -> Result<()> {
        self.store.save_nodes(LOCAL_NODES_ID, nodes)?;
        self.with(|s| {
            if let Some(sub) = s.subs.iter_mut().find(|x| x.is_local_nodes()) {
                sub.node_count = nodes.len();
                sub.last_update = Some(now_unix());
            }
        });
        self.save_subs();
        let this = self.clone();
        self.rt.spawn(async move { this.after_nodes_changed().await });
        Ok(())
    }

    /// The share link of the node behind an outbound tag of the running config.
    pub fn share_link_for(&self, tag: &str) -> Result<String> {
        let config = self.preview_config();
        let ob = config
            .get("outbounds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(config.get("endpoints").and_then(Value::as_array).into_iter().flatten())
            .find(|o| o.get("tag").and_then(Value::as_str) == Some(tag))
            .ok_or_else(|| anyhow!("node not found"))?;
        crate::convert::outbound_share_link(tag, ob).map_err(|e| anyhow!(e))
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

    /// Removes several subscriptions with a single core reload.
    pub fn remove_subscriptions(self: &Arc<Self>, ids: &[String]) {
        self.with(|s| {
            s.subs.retain(|x| !ids.contains(&x.id));
            for id in ids {
                s.nodes.remove(id);
            }
        });
        for id in ids {
            self.store.delete_nodes(id);
        }
        self.save_subs();
        let this = self.clone();
        self.rt.spawn(async move { this.after_nodes_changed().await });
    }

    /// Enables or disables several subscriptions with a single core reload.
    pub fn set_subscriptions_enabled(self: &Arc<Self>, ids: &[String], enabled: bool) {
        let changed = self.with(|s| {
            let mut changed = false;
            for x in s.subs.iter_mut().filter(|x| ids.contains(&x.id) && x.enabled != enabled) {
                x.enabled = enabled;
                changed = true;
            }
            changed
        });
        if changed {
            self.save_subs();
            self.wake_scheduler.notify_one();
            let this = self.clone();
            self.rt.spawn(async move { this.after_nodes_changed().await });
        }
    }

    /// Refreshes the given remote subscriptions together (one reload at the end).
    pub fn update_subscriptions(self: &Arc<Self>, ids: &[String]) {
        let ids: Vec<String> = self.with(|s| {
            s.subs
                .iter()
                .filter(|x| ids.contains(&x.id) && x.is_remote())
                .map(|x| x.id.clone())
                .collect()
        });
        if ids.is_empty() {
            return;
        }
        let this = self.clone();
        self.rt.spawn(async move { this.update_many(ids, false).await });
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
            // The reload refreshes countries once the core is back.
            self.request_reload();
            return;
        }
        self.refresh_countries();
        if self.settings().auto_start_core
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
                this.request_reload();
            }
        });
        self.wake_scheduler.notify_one();
        id
    }

    /// The rule-set behind a group tag on the Proxies page, if it is one.
    pub fn rule_group(&self, tag: &str) -> Option<RuleProvider> {
        self.with(|s| {
            let id = s.meta.rule_groups.get(tag)?;
            s.providers.iter().find(|p| p.id == *id).cloned()
        })
    }

    /// The text file to edit for a rule-set: a local source as itself, a
    /// downloaded one as the stored copy. `None` for binary (.srs) rule-sets
    /// and ones not downloaded yet. The flag tells a local source.
    pub fn rule_provider_text_file(&self, id: &str) -> Option<(PathBuf, bool)> {
        let p = self.with(|s| s.providers.iter().find(|x| x.id == id).cloned())?;
        if !p.is_remote() {
            let path = PathBuf::from(p.source.trim());
            let binary = p.format == rules::RuleFormat::SingBoxBinary
                || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("srs"));
            return (!binary && path.is_file()).then_some((path, true));
        }
        if p.stored != Some(rules::RuleFormat::SingBoxSource) {
            return None;
        }
        crate::config::provider_file(&self.rules_dir(), &p).filter(|f| f.is_file()).map(|f| (f, false))
    }

    /// Applies each save of a rule-set file open in an editor, for a couple of
    /// hours. A local source is converted again; the core reloads a stored
    /// copy by itself.
    pub fn watch_rule_provider_edits(self: &Arc<Self>, id: &str, path: PathBuf, local_source: bool) {
        if !self.with(|s| s.editing.insert(id.to_string())) {
            return;
        }
        let (this, id) = (self.clone(), id.to_string());
        self.rt.spawn(async move {
            let modified = || std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let mut last = modified();
            let until = Instant::now() + Duration::from_secs(2 * 3600);
            while Instant::now() < until && this.with(|s| s.providers.iter().any(|p| p.id == id)) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let now = modified();
                if now == last {
                    continue;
                }
                last = now;
                if local_source {
                    this.update_providers(vec![id.clone()], false).await;
                } else {
                    // A broken save shows on the rule-set's row.
                    let checked = std::fs::read(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|b| rules::convert_rules("", rules::RuleFormat::SingBoxSource, &b));
                    this.with(|s| {
                        if let Some(p) = s.providers.iter_mut().find(|p| p.id == id) {
                            match &checked {
                                Ok(c) => {
                                    p.rule_count = c.rule_count;
                                    p.last_error = None;
                                }
                                Err(e) => p.last_error = Some(e.clone()),
                            }
                            p.last_update = Some(now_unix());
                        }
                    });
                    this.save_providers();
                }
                this.log_app(LogLevel::Info, format!("rule-set file saved: {}", path.display()));
            }
            this.with(|s| s.editing.remove(&id));
        });
    }

    /// Gives a rule-set's matches a node group of their own (a config change).
    pub fn set_rule_provider_group(self: &Arc<Self>, id: &str, own: bool) {
        if let Some(mut p) = self.with(|s| s.providers.iter().find(|x| x.id == id).cloned()) {
            p.own_group = own;
            self.save_rule_provider(p);
        }
    }

    /// How a rule-set's group picks its node. Applied through the Clash API,
    /// like Auto, so the core keeps running.
    pub fn set_rule_group_pick(self: &Arc<Self>, id: &str, pick: Option<AutoStrategy>, region: &str) {
        let changed = self.with(|s| {
            let p = s.providers.iter_mut().find(|x| x.id == id)?;
            let changed = p.pick != pick || p.region != region;
            p.pick = pick;
            p.region = region.to_string();
            Some(changed)
        });
        if changed == Some(true) {
            self.save_providers();
            self.wake_auto.notify_one();
            let this = self.clone();
            self.rt.spawn(async move {
                let _ = this.refresh_proxies_async().await;
            });
        }
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
        self.request_reload();
    }

    /// Removes several rule-sets with a single core reload.
    pub fn remove_rule_providers(self: &Arc<Self>, ids: &[String]) {
        let removed: Vec<RuleProvider> = self.with(|s| {
            let (gone, kept) = std::mem::take(&mut s.providers).into_iter().partition(|p| ids.contains(&p.id));
            s.providers = kept;
            gone
        });
        for p in &removed {
            if let Some(f) = crate::config::provider_file(&self.rules_dir(), p) {
                let _ = std::fs::remove_file(f);
            }
        }
        self.save_providers();
        self.request_reload();
    }

    /// Enables or disables several rule-sets with a single core reload.
    pub fn set_rule_providers_enabled(self: &Arc<Self>, ids: &[String], enabled: bool) {
        // Turning on a rule-set that was never downloaded fetches it first.
        let missing: Vec<String> = self.with(|s| {
            let mut missing = Vec::new();
            for p in s.providers.iter_mut().filter(|p| ids.contains(&p.id)) {
                p.enabled = enabled;
                if enabled && p.stored.is_none() {
                    missing.push(p.id.clone());
                }
            }
            missing
        });
        self.save_providers();
        self.wake_scheduler.notify_one();
        if missing.is_empty() {
            self.request_reload();
        } else {
            let this = self.clone();
            self.rt.spawn(async move {
                this.update_providers(missing, true).await;
                this.request_reload();
            });
        }
    }

    /// Refreshes the given rule-sets and reports one summary.
    pub fn update_rule_providers(self: &Arc<Self>, ids: &[String]) {
        let ids = ids.to_vec();
        let this = self.clone();
        self.rt.spawn(async move {
            let total = ids.len();
            let ok = this.update_providers(ids, true).await;
            let kind = if ok == total { NoticeKind::Success } else { NoticeKind::Warning };
            this.notice(kind, Msg::new("notice.rules_batch").arg(ok).arg(total - ok));
        });
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
        self.request_reload();
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
                // A template brings dozens of rule-sets; the user turns on the ones they want.
                let p = RuleProvider {
                    id: new_id(),
                    name: r.name.clone(),
                    source: r.source.clone(),
                    format: r.format,
                    target: r.target.clone(),
                    enabled: false,
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
        self.download_with(url, Duration::from_secs(30)).await
    }

    /// Proxy first while the core runs, then direct.
    fn routes(&self) -> Vec<Route> {
        if self.status().is_running() {
            vec![Route::Proxy(self.mixed_proxy_url()), Route::Direct]
        } else {
            vec![Route::Direct]
        }
    }

    async fn download_with(&self, url: &str, timeout: Duration) -> Result<Vec<u8>> {
        let routes = self.routes();
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
            let client = fetch::client(route, timeout, "BoxPilot")?;
            match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
                Ok(resp) => return Ok(resp.bytes().await?.to_vec()),
                Err(e) => last = e.into(),
            }
        }
        Err(last)
    }

    // -------------------------------------------------------------- speed test

    pub fn speed(&self, tag: &str) -> Option<speed::SpeedResult> {
        self.with(|s| s.speeds.get(tag).copied())
    }

    pub fn is_speed_testing(&self, tag: &str) -> bool {
        self.with(|s| s.speed_testing.contains(tag))
    }

    pub fn real_delay(&self, tag: &str) -> Option<Option<u32>> {
        self.with(|s| s.real_delays.get(tag).copied())
    }

    pub fn is_real_testing(&self, tag: &str) -> bool {
        self.with(|s| s.real_testing.contains(tag))
    }

    pub fn any_real_testing(&self) -> bool {
        self.with(|s| !s.real_testing.is_empty())
    }

    /// Real-connection latency through each node: a fresh HTTPS request, one
    /// node at a time via the probe selector (no data beyond the handshake).
    pub fn test_real_latency(self: &Arc<Self>, tags: Vec<String>) {
        if !self.status().is_running() {
            self.notice(NoticeKind::Warning, Msg::new("notice.start_core_first"));
            return;
        }
        let queued: Vec<String> = self.with(|s| tags.into_iter().filter(|t| s.real_testing.insert(t.clone())).collect());
        if queued.is_empty() {
            return;
        }
        self.emit(Event::Proxies);
        let this = self.clone();
        self.rt.spawn(async move {
            let port = this.with(|s| s.settings.probe_port);
            for tag in queued {
                let d = if port == 0 || !this.status().is_running() {
                    None
                } else {
                    let _probe = this.probe_lock.lock().await;
                    match this.api().select(crate::config::TAG_PROBE, &tag).await {
                        Ok(()) => speed::latency(&format!("http://127.0.0.1:{port}")).await,
                        Err(_) => None,
                    }
                };
                this.with(|s| {
                    s.real_testing.remove(&tag);
                    s.real_delays.insert(tag.clone(), d);
                });
                this.emit(Event::Proxies);
            }
        });
    }

    pub fn any_speed_testing(&self) -> bool {
        self.with(|s| !s.speed_testing.is_empty())
    }

    /// Download speed and real-connection latency through each node, one node
    /// at a time via the probe selector (the user's selection is untouched).
    pub fn speed_test(self: &Arc<Self>, tags: Vec<String>) {
        let queued: Vec<String> = self.with(|s| tags.into_iter().filter(|t| s.speed_testing.insert(t.clone())).collect());
        if queued.is_empty() {
            return;
        }
        self.emit(Event::Proxies);
        let this = self.clone();
        self.rt.spawn(async move {
            for tag in queued {
                let result = this.speed_test_one(&tag, speed::DOWNLOAD_BYTES).await;
                this.with(|s| {
                    s.speed_testing.remove(&tag);
                    s.speeds.insert(tag.clone(), result);
                    s.real_delays.insert(tag.clone(), result.latency_ms);
                });
                this.emit(Event::Proxies);
            }
        });
    }

    async fn speed_test_one(&self, tag: &str, bytes: u64) -> speed::SpeedResult {
        let port = self.with(|s| s.settings.probe_port);
        if port == 0 || !self.status().is_running() {
            return speed::SpeedResult::default();
        }
        let _probe = self.probe_lock.lock().await;
        if self.api().select(crate::config::TAG_PROBE, tag).await.is_err() {
            return speed::SpeedResult::default();
        }
        speed::measure(&format!("http://127.0.0.1:{port}"), bytes).await
    }

    // -------------------------------------------------------------- auto group

    /// Countries among the nodes, most nodes first, for the region filter.
    pub fn country_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = self.with(|s| {
            let mut m: HashMap<String, usize> = HashMap::new();
            for c in s.countries.values() {
                *m.entry(c.code.clone()).or_default() += 1;
            }
            m.into_iter().collect()
        });
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        counts
    }

    /// Keeps Auto, and every rule-set group set to pick, on a good node. On
    /// the test interval it measures every node (or only the candidates when
    /// automatic tests are off); between rounds it checks the nodes in use
    /// every two minutes; and it re-picks at once when a mode or region
    /// changes or the core starts. Picks go through the Clash API, so none of
    /// this restarts the core.
    async fn auto_loop(self: Arc<Self>) {
        let mut last_test: Option<Instant> = None;
        let mut last_bandwidth: HashMap<String, Instant> = HashMap::new();
        loop {
            let s = self.settings();
            let mut wait = Duration::from_secs(600);
            if self.status().is_running() {
                let members = self.auto_members().await;
                if !members.is_empty() {
                    let pickers = self.pickers(&s);
                    let candidates = || {
                        let mut all: Vec<String> = Vec::new();
                        for p in &pickers {
                            for c in self.auto_candidates(&members, &p.region) {
                                if !all.contains(&c) {
                                    all.push(c);
                                }
                            }
                        }
                        all
                    };
                    let every = Duration::from_secs(u64::from(s.urltest_interval_min.max(1)) * 60);
                    if last_test.is_none_or(|t| t.elapsed() >= every) {
                        let names = if s.auto_test_nodes { members.clone() } else { candidates() };
                        self.measure_delays(names).await;
                        last_test = Some(Instant::now());
                    } else {
                        let mut alive = true;
                        for p in &pickers {
                            alive &= self.auto_current_alive(&p.tag, &members).await;
                        }
                        if !alive {
                            self.measure_delays(candidates()).await;
                        }
                    }
                    for p in &pickers {
                        let last = last_bandwidth.get(&p.tag).copied();
                        if let Some(at) = self.pick_auto(p, &members, last).await {
                            last_bandwidth.insert(p.tag.clone(), at);
                        }
                    }
                    let _ = self.refresh_proxies_async().await;
                    let left = every.saturating_sub(last_test.map_or(Duration::ZERO, |t| t.elapsed()));
                    wait = left.clamp(Duration::from_secs(5), Duration::from_secs(120));
                }
            } else {
                last_test = None;
            }
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = self.wake_auto.notified() => {}
            }
        }
    }

    /// The groups BoxPilot picks for: Auto, then each rule-set group set to pick.
    fn pickers(&self, s: &Settings) -> Vec<Picker> {
        let mut out = vec![Picker {
            tag: crate::config::TAG_AUTO.into(),
            strategy: s.auto_strategy,
            region: s.auto_region.clone(),
        }];
        self.with(|st| {
            for (tag, id) in &st.meta.rule_groups {
                if let Some(p) = st.providers.iter().find(|p| p.id == *id)
                    && let Some(strategy) = p.pick
                {
                    out.push(Picker { tag: tag.clone(), strategy, region: p.region.clone() });
                }
            }
        });
        out
    }

    /// Every node Auto can use (its selector holds them all).
    async fn auto_members(&self) -> Vec<String> {
        self.api()
            .proxies()
            .await
            .ok()
            .and_then(|p| p.get(crate::config::TAG_AUTO).and_then(|g| g.all.clone()))
            .unwrap_or_default()
    }

    /// The members in the chosen region, in list order; all of them when the
    /// region has none (yet).
    fn auto_candidates(&self, members: &[String], region: &str) -> Vec<String> {
        let region = region.trim();
        if region.is_empty() {
            return members.to_vec();
        }
        let picked: Vec<String> = self.with(|st| {
            members
                .iter()
                .filter(|m| st.countries.get(*m).is_some_and(|c| c.code.eq_ignore_ascii_case(region)))
                .cloned()
                .collect()
        });
        if picked.is_empty() { members.to_vec() } else { picked }
    }

    /// Whether the group's node still answers. A group on something other
    /// than a node (a rule-set group starting on Proxy) needs a pick.
    async fn auto_current_alive(&self, tag: &str, members: &[String]) -> bool {
        let api = self.api();
        let Some(now) = api.proxies().await.ok().and_then(|p| p.get(tag).and_then(|g| g.now.clone())) else {
            return false;
        };
        if !members.contains(&now) {
            return false;
        }
        let d = api.delay(&now, &self.settings().urltest_url, DELAY_TIMEOUT_MS).await.ok();
        self.with(|s| s.delays.insert(now, d));
        self.emit(Event::Delays);
        d.is_some()
    }

    /// Chooses a group's node from the latest measurements, per mode.
    /// Returns when it ran speed tests (the bandwidth mode's clock).
    async fn pick_auto(self: &Arc<Self>, picker: &Picker, members: &[String], last_bandwidth: Option<Instant>) -> Option<Instant> {
        let api = self.api();
        let candidates = self.auto_candidates(members, &picker.region);
        let now = api.proxies().await.ok().and_then(|p| p.get(&picker.tag).and_then(|g| g.now.clone()));
        let delay = |name: &str| self.with(|st| st.delays.get(name).copied().flatten());
        // Reachable candidates, in list order.
        let alive: Vec<(String, u32)> = candidates.iter().filter_map(|m| delay(m).map(|d| (m.clone(), d))).collect();
        let Some(best) = alive.iter().min_by_key(|(_, d)| *d).cloned() else {
            // Nothing answered: leave the current node rather than guess.
            return None;
        };
        let mut tested = None;
        let current = now.clone().filter(|n| candidates.contains(n)).and_then(|n| delay(&n).map(|d| (n, d)));
        // Moving costs nothing for open connections (they stay), but only a
        // clear gain is worth reshuffling new ones.
        let keep_within = |tolerance: u32| current.clone().filter(|(_, d)| *d <= best.1 + tolerance).map(|(n, _)| n);
        let pick = match picker.strategy {
            AutoStrategy::Quickest => keep_within(50).unwrap_or_else(|| best.0.clone()),
            AutoStrategy::Steady => keep_within(300).unwrap_or_else(|| best.0.clone()),
            AutoStrategy::Ordered => alive[0].0.clone(),
            AutoStrategy::Bandwidth => {
                // Speed tests cost data (about 25 MB a round): every 6 hours,
                // or when the node in use is gone or outside the region.
                let stale = last_bandwidth.is_none_or(|t| t.elapsed() >= Duration::from_secs(6 * 3600));
                match current.clone() {
                    Some((n, _)) if !stale => n,
                    _ => {
                        let mut ranked = alive.clone();
                        ranked.sort_by_key(|(_, d)| *d);
                        let mut fastest: Option<(String, u64)> = None;
                        for (m, _) in ranked.iter().take(3) {
                            let r = self.speed_test_one(m, speed::AUTO_BYTES).await;
                            self.with(|st| {
                                st.speeds.insert(m.clone(), r);
                                st.real_delays.insert(m.clone(), r.latency_ms);
                            });
                            self.emit(Event::Proxies);
                            if let Some(bps) = r.bytes_per_sec.filter(|b| fastest.as_ref().is_none_or(|(_, top)| b > top)) {
                                fastest = Some((m.clone(), bps));
                            }
                        }
                        tested = Some(Instant::now());
                        fastest.map(|(m, _)| m).unwrap_or_else(|| best.0.clone())
                    }
                }
            }
        };
        if now.as_deref() != Some(pick.as_str()) && api.select(&picker.tag, &pick).await.is_ok() {
            self.log_app(LogLevel::Info, format!("{} ({:?}) → {pick}", picker.tag, picker.strategy));
        }
        tested
    }

    // -------------------------------------------------------------- updates

    pub fn update_info(&self) -> UpdateInfo {
        self.with(|s| s.update.clone())
    }

    /// Asks GitHub for the latest release. A manual check also reports
    /// "up to date" and failures.
    pub fn check_for_update(self: &Arc<Self>, manual: bool) {
        let this = self.clone();
        self.rt.spawn(async move { this.check_update_async(manual).await });
    }

    async fn check_update_async(self: &Arc<Self>, manual: bool) {
        if self.with(|s| std::mem::replace(&mut s.update.checking, true)) {
            return;
        }
        self.emit(Event::Update);
        let mut result = Err(anyhow!("no route"));
        for route in self.routes() {
            let Ok(client) = fetch::client(&route, Duration::from_secs(20), "BoxPilot") else { continue };
            result = update::latest(&client).await;
            if result.is_ok() {
                break;
            }
        }
        let latest = match result {
            Ok(latest) => latest,
            Err(e) => {
                self.with(|s| {
                    s.update.checking = false;
                    s.update.error = Some(format!("{e:#}"));
                });
                self.emit(Event::Update);
                if manual {
                    self.notice(NoticeKind::Warning, Msg::new("notice.update_failed").arg(format!("{e:#}")));
                }
                return;
            }
        };
        self.update_settings(|s| s.last_update_check = now_unix());
        let newer = latest.filter(update::Release::is_newer);
        self.with(|s| {
            s.update.checking = false;
            s.update.error = None;
            s.update.latest = newer.clone();
        });
        self.emit(Event::Update);
        let Some(release) = newer else {
            if manual {
                self.notice(NoticeKind::Success, Msg::new("notice.update_latest"));
            }
            return;
        };
        if self.with(|s| s.update.installed.as_deref() == Some(release.version.as_str())) {
            return;
        }
        if !manual && release.asset.is_some() && self.settings().update_mode == UpdateMode::Auto {
            self.install_update_async().await;
        } else {
            self.notice(NoticeKind::Info, Msg::new("notice.update_available").arg(&release.version));
        }
    }

    /// Downloads the latest release for this platform and swaps it in.
    pub fn install_update(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move { this.install_update_async().await });
    }

    async fn install_update_async(self: &Arc<Self>) {
        let Some(release) = self.with(|s| s.update.latest.clone()) else { return };
        let Some(asset) = release.asset.clone() else { return };
        if self.with(|s| std::mem::replace(&mut s.update.installing, true)) {
            return;
        }
        self.emit(Event::Update);
        self.log_app(LogLevel::Info, format!("downloading BoxPilot {}", release.version));
        let result: Result<PathBuf> = async {
            let data = self.download_with(&asset.url, Duration::from_secs(600)).await?;
            let name = asset.name.clone();
            tokio::task::spawn_blocking(move || update::install(&update::extract_binary(&data, &name)?)).await?
        }
        .await;
        self.with(|s| {
            s.update.installing = false;
            match &result {
                Ok(_) => {
                    s.update.installed = Some(release.version.clone());
                    s.update.error = None;
                }
                Err(e) => s.update.error = Some(format!("{e:#}")),
            }
        });
        self.emit(Event::Update);
        match result {
            Ok(path) => {
                self.log_app(LogLevel::Info, format!("BoxPilot {} installed at {}", release.version, path.display()));
                self.notice(NoticeKind::Success, Msg::new("notice.update_installed").arg(&release.version));
            }
            Err(e) => {
                self.log_app(LogLevel::Warn, format!("update failed: {e:#}"));
                self.notice(NoticeKind::Error, Msg::new("notice.update_failed").arg(format!("{e:#}")));
            }
        }
    }

    /// Checks once a day unless updates are off.
    async fn update_loop(self: Arc<Self>) {
        update::cleanup();
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            let s = self.settings();
            if s.update_mode != UpdateMode::Off && now_unix() - s.last_update_check >= 86400 {
                self.check_update_async(false).await;
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }

    // -------------------------------------------------------------- countries

    /// Where a proxy (by outbound tag) is, when known.
    pub fn country(&self, tag: &str) -> Option<geo::Country> {
        self.with(|s| s.countries.get(tag).cloned())
    }

    fn geoip_file(&self) -> PathBuf {
        self.store.paths.root.join("geoip").join("country.mmdb")
    }

    /// Works out every node's country: hints in the name right away, then a
    /// GeoIP lookup of the server address for the rest.
    pub fn refresh_countries(self: &Arc<Self>) {
        let this = self.clone();
        self.rt.spawn(async move { this.refresh_countries_async().await });
    }

    async fn refresh_countries_async(self: &Arc<Self>) {
        let _one_at_a_time = self.geo_lock.lock().await;
        let mut named = HashMap::new();
        let mut pending = Vec::new();
        for (tag, host) in geo::servers(&self.current_config_json()) {
            match geo::from_name(&tag) {
                Some(code) => {
                    named.insert(tag, geo::Country { code: code.to_string(), origin: geo::Origin::Name });
                }
                None => pending.push((tag, host)),
            }
        }
        // Earlier GeoIP answers stay visible while new ones load. A measured exit
        // outranks both the name and the server's GeoIP: relays and transit nodes
        // are entered in one country and leave from another.
        self.with(|s| {
            let old = std::mem::replace(&mut s.countries, named);
            for (tag, _) in &pending {
                if let Some(c) = old.get(tag) {
                    s.countries.insert(tag.clone(), c.clone());
                }
            }
            for (tag, (c, at)) in &s.exit_checked {
                if at.elapsed() < EXIT_REUSE {
                    s.countries.insert(tag.clone(), c.clone());
                }
            }
        });
        self.emit(Event::Proxies);
        if pending.is_empty() {
            return;
        }

        let db = self.geoip_file();
        if let Err(e) = self.ensure_geoip_db(&db).await {
            self.log_app(LogLevel::Warn, format!("GeoIP database unavailable: {e:#}"));
            return;
        }
        let routes = self.routes();
        let hosts: HashSet<String> = pending.iter().map(|(_, h)| h.clone()).collect();
        let resolved: HashMap<String, IpAddr> = futures_util::stream::iter(hosts)
            .map(|host| {
                let routes = routes.clone();
                async move {
                    let ip = geo::resolve(&host, &routes).await;
                    (host, ip)
                }
            })
            .buffer_unordered(8)
            .filter_map(|(host, ip)| async move { ip.map(|ip| (host, ip)) })
            .collect()
            .await;
        let ips: Vec<IpAddr> = resolved.values().copied().collect::<HashSet<_>>().into_iter().collect();
        let codes = match tokio::task::spawn_blocking(move || geo::lookup(&db, &ips)).await {
            Ok(Ok(codes)) => codes,
            Ok(Err(e)) => {
                self.log_app(LogLevel::Warn, format!("GeoIP lookup failed: {e:#}"));
                return;
            }
            Err(_) => return,
        };
        // Nodes whose server address can't place them get an exit check instead.
        let unsure: Vec<String> = self.with(|s| {
            let mut unsure = Vec::new();
            for (tag, host) in &pending {
                let ip = resolved.get(host);
                match (ip, ip.and_then(|ip| codes.get(ip))) {
                    _ if s.exit_checked.get(tag).is_some_and(|(_, at)| at.elapsed() < EXIT_REUSE) => {}
                    (Some(ip), Some(code)) if !geo::needs_exit_check(ip) => {
                        s.countries.insert(tag.clone(), geo::Country { code: code.clone(), origin: geo::Origin::Ip(*ip) });
                    }
                    _ => unsure.push(tag.clone()),
                }
            }
            unsure
        });
        self.emit(Event::Proxies);
        self.check_exits(unsure).await;
        // A region filter may now match different nodes.
        let regional = !self.settings().auto_region.is_empty()
            || self.with(|s| s.providers.iter().any(|p| p.own_group && p.pick.is_some() && !p.region.is_empty()));
        if regional {
            self.wake_auto.notify_one();
        }
    }

    /// Asks an IP service through each node (via the hidden probe selector)
    /// where its traffic leaves. Needs the core running; answers are reused for 6 h.
    async fn check_exits(self: &Arc<Self>, tags: Vec<String>) {
        let port = self.with(|s| s.settings.probe_port);
        if tags.is_empty() || port == 0 || !self.status().is_running() {
            return;
        }
        let probe_url = format!("http://127.0.0.1:{port}");
        let api = self.api();
        for tag in tags {
            let cached = self.with(|s| s.exit_checked.get(&tag).filter(|(_, at)| at.elapsed() < EXIT_REUSE).map(|(c, _)| c.clone()));
            let country = match cached {
                Some(c) => Some(c),
                None => {
                    if !self.status().is_running() {
                        return;
                    }
                    let _probe = self.probe_lock.lock().await;
                    if api.select(crate::config::TAG_PROBE, &tag).await.is_err() {
                        continue;
                    }
                    geo::exit_country(&probe_url).await.map(|(ip, code)| geo::Country { code, origin: geo::Origin::Exit(ip) })
                }
            };
            if let Some(c) = country {
                self.with(|s| {
                    s.exit_checked.insert(tag.clone(), (c.clone(), Instant::now()));
                    s.countries.insert(tag, c);
                });
                self.emit(Event::Proxies);
            }
        }
    }

    /// Downloads the GeoIP database when missing or older than 30 days; a
    /// stale copy is still used when the download fails.
    async fn ensure_geoip_db(&self, path: &Path) -> Result<()> {
        let age = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| a < Duration::from_secs(30 * 86400)) {
            return Ok(());
        }
        let mut last = anyhow!("no source");
        for url in geo::DB_URLS {
            match self.download_with(url, Duration::from_secs(300)).await {
                Ok(bytes) if geo::is_database(&bytes) => {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    let tmp = path.with_extension("tmp");
                    std::fs::write(&tmp, &bytes)?;
                    std::fs::rename(&tmp, path)?;
                    self.log_app(LogLevel::Info, "GeoIP database updated");
                    return Ok(());
                }
                Ok(_) => last = anyhow!("{url}: not a GeoIP database"),
                Err(e) => last = e.context(url.to_string()),
            }
        }
        if age.is_some() { Ok(()) } else { Err(last) }
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
            self.request_reload();
        }
        ok
    }

    fn current_config_json(&self) -> Value {
        let settings = self.settings();
        let version = self.core_version().unwrap_or(CoreVersion::MIN_SUPPORTED);
        self.render_config(&settings, version).config
    }

    /// Reloads the core once changes settle: a burst (toggling several
    /// subscriptions, updates finishing together) costs one reload, and none
    /// when the config ends up as it was. Every reload drops all connections.
    fn request_reload(self: &Arc<Self>) {
        let ticket = self.reload_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        let this = self.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(RELOAD_DEBOUNCE).await;
            if this.reload_ticket.load(Ordering::SeqCst) == ticket {
                this.reload_if_config_changed().await;
            }
        });
    }

    async fn reload_if_config_changed(self: &Arc<Self>) {
        // The mode is switched live through the API, so the start-up default
        // in the file may differ without the core needing a reload.
        let comparable = |mut v: Value| {
            if let Some(api) = v.pointer_mut("/experimental/clash_api").and_then(Value::as_object_mut) {
                api.remove("default_mode");
            }
            v
        };
        // Rendering is cheap; compare against the config the core runs with.
        let running = std::fs::read(self.store.paths.config_file())
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(comparable);
        if running != Some(comparable(self.current_config_json())) {
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

    async fn refresh_proxies_async(self: &Arc<Self>) -> Result<()> {
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
                    // Auto (and a rule-set group set to pick) is a selector for
                    // sing-box, but BoxPilot picks for it.
                    kind: if p.name == crate::config::TAG_AUTO
                        || meta.rule_groups.get(&p.name).is_some_and(|id| s.providers.iter().any(|x| x.id == *id && x.pick.is_some()))
                    {
                        "URLTest".to_string()
                    } else {
                        p.kind.clone()
                    },
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
        self.watch_current_exit();
        Ok(())
    }

    /// Checks where the node in use really leaves, once per node it switches to
    /// (cached for hours), so the country shown for it is the measured one.
    fn watch_current_exit(self: &Arc<Self>) {
        let tag = self.with(|s| {
            let leaf = current_node(&s.groups)?;
            if s.exit_watched.as_deref() == Some(leaf.as_str()) {
                return None;
            }
            s.exit_watched = Some(leaf.clone());
            Some(leaf)
        });
        if let Some(tag) = tag {
            let this = self.clone();
            self.rt.spawn(async move { this.check_exits(vec![tag]).await });
        }
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
        let this = self.clone();
        self.rt.spawn(async move {
            this.measure_delays(names).await;
            let _ = this.refresh_proxies_async().await;
        });
    }

    /// URL-test latency through the core, 32 at a time; the badges show
    /// "testing" and fill in as results arrive.
    async fn measure_delays(&self, names: Vec<String>) {
        let names: Vec<String> = self.with(|s| names.into_iter().filter(|n| s.testing.insert(n.clone())).collect());
        if names.is_empty() {
            return;
        }
        self.emit(Event::Delays);
        let api = self.api();
        let url = self.settings().urltest_url;
        futures_util::stream::iter(names)
            .for_each_concurrent(32, |name| {
                let (api, url) = (api.clone(), url.clone());
                async move {
                    let d = api.delay(&name, &url, DELAY_TIMEOUT_MS).await.ok();
                    self.with(|s| {
                        s.testing.remove(&name);
                        s.delays.insert(name, d);
                    });
                    self.emit(Event::Delays);
                }
            })
            .await;
    }

    // ----------------------------------------------------------------- core

    pub async fn refresh_core_version(&self) {
        let core = self.core_path();
        let v = if let Some(host) = &self.host {
            Some(host.version())
        } else if core.is_file() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, now: &str) -> ProxyGroup {
        ProxyGroup { name: name.into(), kind: "Selector".into(), now: now.into(), members: Vec::new() }
    }

    #[test]
    fn current_node_follows_groups() {
        let groups = [group("Proxy", "Auto"), group("Auto", "HK 01")];
        assert_eq!(current_node(&groups).as_deref(), Some("HK 01"));
        assert_eq!(current_node(&[group("Proxy", "JP 02")]).as_deref(), Some("JP 02"));
    }

    #[test]
    fn current_node_skips_builtins_and_unresolved_groups() {
        assert_eq!(current_node(&[group("Proxy", "direct")]), None);
        assert_eq!(current_node(&[group("Proxy", "Auto"), group("Auto", "")]), None);
        assert_eq!(current_node(&[]), None);
        // A loop stops instead of spinning, and is not a node.
        assert_eq!(current_node(&[group("Proxy", "A"), group("A", "Proxy")]), None);
    }
}
