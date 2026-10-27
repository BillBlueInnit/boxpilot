use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Routing mode, mapped onto sing-box `clash_mode`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum ProxyMode {
    #[default]
    Rule,
    Global,
    Direct,
}

impl ProxyMode {
    pub const ALL: [ProxyMode; 3] = [ProxyMode::Rule, ProxyMode::Global, ProxyMode::Direct];

    pub fn as_str(&self) -> &'static str {
        match self {
            ProxyMode::Rule => "Rule",
            ProxyMode::Global => "Global",
            ProxyMode::Direct => "Direct",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str().eq_ignore_ascii_case(s))
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum TunStack {
    System,
    Gvisor,
    #[default]
    Mixed,
}

impl TunStack {
    pub const ALL: [TunStack; 3] = [TunStack::Mixed, TunStack::System, TunStack::Gvisor];

    pub fn as_str(&self) -> &'static str {
        match self {
            TunStack::System => "system",
            TunStack::Gvisor => "gvisor",
            TunStack::Mixed => "mixed",
        }
    }
}

/// UI language. `System` follows the Windows display language.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Language {
    #[default]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
    #[serde(rename = "zh-TW")]
    TraditionalChinese,
    #[serde(rename = "ru")]
    Russian,
}

impl Language {
    pub const ALL: [Language; 5] = [
        Language::System,
        Language::English,
        Language::SimplifiedChinese,
        Language::TraditionalChinese,
        Language::Russian,
    ];

    /// The language's own name.
    pub fn native_name(&self) -> &'static str {
        match self {
            Language::System => "System",
            Language::English => "English",
            Language::SimplifiedChinese => "简体中文",
            Language::TraditionalChinese => "繁體中文",
            Language::Russian => "Русский",
        }
    }

    /// Maps a BCP-47 locale such as `zh-TW` or `ru-RU` to a supported language.
    pub fn from_locale(locale: &str) -> Language {
        let l = locale.to_ascii_lowercase().replace('_', "-");
        if l.starts_with("zh") {
            if l.contains("hant") || l.ends_with("-tw") || l.ends_with("-hk") || l.ends_with("-mo") {
                Language::TraditionalChinese
            } else {
                Language::SimplifiedChinese
            }
        } else if l.starts_with("ru") {
            Language::Russian
        } else {
            Language::English
        }
    }
}

/// Which graphics processor renders the UI. Applied at start-up.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum GpuPreference {
    /// The operating system's choice.
    #[default]
    Auto,
    /// The integrated GPU, to save battery.
    PowerSaving,
    /// The discrete GPU.
    HighPerformance,
    /// No GPU: render on the CPU (Linux / Mesa only).
    Software,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

/// What to do when a newer BoxPilot is published.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    /// Download and install in the background; it runs from the next start.
    Auto,
    /// Only tell the user.
    #[default]
    Notify,
    /// Never check.
    Off,
}

/// How a subscription download should be routed.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateVia {
    /// Through the local proxy when the core is running, falling back to direct.
    #[default]
    Auto,
    Direct,
    Proxy,
}

impl UpdateVia {
    pub const ALL: [UpdateVia; 3] = [UpdateVia::Auto, UpdateVia::Direct, UpdateVia::Proxy];

    pub fn label(&self) -> &'static str {
        match self {
            UpdateVia::Auto => "Auto (proxy, then direct)",
            UpdateVia::Direct => "Direct",
            UpdateVia::Proxy => "Proxy",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    // Inbound
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub system_proxy: bool,
    pub sysproxy_bypass: String,
    pub tun_enabled: bool,
    pub tun_stack: TunStack,
    pub tun_strict_route: bool,
    pub tun_mtu: u32,
    pub mode: ProxyMode,

    // DNS
    pub dns_remote: String,
    pub dns_direct: String,
    pub fake_ip: bool,
    pub ipv6: bool,

    // Rules
    pub bypass_cn: bool,
    pub block_ads: bool,
    pub ruleset_via_proxy: bool,
    pub geosite_url: String,
    pub geoip_url: String,
    pub custom_rules: String,

    // Core
    pub core_path: String,
    pub log_level: String,
    /// Keep core log lines in memory for the Logs page.
    pub record_logs: bool,
    pub clash_api_port: u16,
    /// Local port the exit-location check goes through; picked on first run.
    pub probe_port: u16,
    pub clash_api_secret: String,
    pub urltest_url: String,
    pub urltest_interval_min: u32,
    /// Prefix prepended to GitHub download URLs, e.g. `https://ghfast.top/`.
    pub github_mirror: String,
    pub core_prerelease: bool,

    // Subscription defaults
    pub sub_user_agent: String,
    pub sub_default_interval_min: u32,
    pub sub_timeout_secs: u64,
    pub sub_reload_after_update: bool,

    // Application
    pub launch_at_login: bool,
    /// Relaunch elevated on start (needed for TUN); login start uses a scheduled task.
    pub run_as_admin: bool,
    pub language: Language,
    pub start_minimized: bool,
    pub auto_start_core: bool,
    pub close_to_tray: bool,
    /// Closing to the tray destroys the window to free UI memory; the proxy keeps running.
    pub lightweight: bool,
    pub theme: ThemePref,
    /// Windows 11 Mica backdrop behind the sidebar.
    pub mica: bool,
    pub gpu: GpuPreference,
    pub update_mode: UpdateMode,
    /// Unix time of the last update check.
    pub last_update_check: i64,
    /// Interface size in percent; 0 follows the OS text-size setting.
    pub ui_scale: u32,
    /// Window position and size from the last session (logical pixels).
    pub window: Option<SavedWindow>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SavedWindow {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

pub const DEFAULT_BYPASS: &str = "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*;<local>";

impl Default for Settings {
    fn default() -> Self {
        Self {
            mixed_port: 7890,
            allow_lan: false,
            system_proxy: false,
            sysproxy_bypass: DEFAULT_BYPASS.into(),
            tun_enabled: false,
            tun_stack: TunStack::Mixed,
            tun_strict_route: true,
            tun_mtu: 9000,
            mode: ProxyMode::Rule,
            dns_remote: "https://1.1.1.1/dns-query".into(),
            dns_direct: "https://223.5.5.5/dns-query".into(),
            fake_ip: false,
            ipv6: false,
            bypass_cn: true,
            block_ads: false,
            ruleset_via_proxy: true,
            geosite_url:
                "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-{name}.srs"
                    .into(),
            geoip_url:
                "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-{name}.srs"
                    .into(),
            custom_rules: String::new(),
            core_path: String::new(),
            log_level: "info".into(),
            record_logs: true,
            clash_api_port: 9090,
            probe_port: 0,
            clash_api_secret: String::new(),
            urltest_url: "https://www.gstatic.com/generate_204".into(),
            urltest_interval_min: 10,
            github_mirror: String::new(),
            core_prerelease: false,
            sub_user_agent: "clash.meta".into(),
            sub_default_interval_min: 720,
            sub_timeout_secs: 30,
            sub_reload_after_update: true,
            launch_at_login: false,
            run_as_admin: false,
            language: Language::System,
            start_minimized: false,
            auto_start_core: true,
            close_to_tray: true,
            lightweight: true,
            theme: ThemePref::System,
            mica: true,
            gpu: GpuPreference::Auto,
            update_mode: UpdateMode::Notify,
            last_update_check: 0,
            ui_scale: 0,
            window: None,
        }
    }
}

/// Traffic quota reported by the `subscription-userinfo` header.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SubInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    /// Unix seconds, 0 when the provider reports no expiry.
    pub expire: i64,
}

impl SubInfo {
    pub fn used(&self) -> u64 {
        self.upload + self.download
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    /// Remote URL. Empty for local (pasted) subscriptions.
    pub url: String,
    pub enabled: bool,
    pub auto_update: bool,
    pub interval_min: u32,
    pub update_on_start: bool,
    pub update_via: UpdateVia,
    /// Overrides the global user agent when non-empty.
    pub user_agent: String,
    /// Regex; when non-empty only matching node names are kept.
    pub include: String,
    /// Regex; matching node names are dropped.
    pub exclude: String,

    pub last_update: Option<i64>,
    pub last_attempt: Option<i64>,
    pub last_error: Option<String>,
    pub node_count: usize,
    pub info: Option<SubInfo>,
    pub web_page: Option<String>,
}

impl Default for Subscription {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            enabled: true,
            auto_update: true,
            interval_min: 720,
            update_on_start: true,
            update_via: UpdateVia::Auto,
            user_agent: String::new(),
            include: String::new(),
            exclude: String::new(),
            last_update: None,
            last_attempt: None,
            last_error: None,
            node_count: 0,
            info: None,
            web_page: None,
        }
    }
}

impl Subscription {
    pub fn is_remote(&self) -> bool {
        !self.url.trim().is_empty()
    }

    /// When the scheduler should next refresh this subscription, in unix seconds.
    /// Failed attempts are retried sooner than the regular interval.
    pub fn next_due(&self) -> Option<i64> {
        if !self.enabled || !self.auto_update || !self.is_remote() || self.interval_min == 0 {
            return None;
        }
        let interval = self.interval_min as i64 * 60;
        let Some(last_update) = self.last_update else {
            // Never fetched successfully: retry with a short backoff.
            return Some(self.last_attempt.map(|t| t + interval.min(600)).unwrap_or(0));
        };
        let regular = last_update + interval;
        match (self.last_error.is_some(), self.last_attempt) {
            (true, Some(attempt)) if attempt > last_update => {
                Some(regular.min(attempt + interval.min(900)))
            }
            _ => Some(regular),
        }
    }
}

/// One proxy server converted to a sing-box outbound (or endpoint).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    /// sing-box outbound type, e.g. `vless`.
    pub protocol: String,
    pub server: String,
    pub port: u16,
    /// The outbound object without `tag`.
    pub outbound: Value,
}

impl Node {
    /// WireGuard is an endpoint since sing-box 1.11.
    pub fn is_endpoint(&self) -> bool {
        self.protocol == "wireguard"
    }
}

pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn new_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..10)
        .map(|_| char::from_digit(rng.random_range(0..36), 36).unwrap())
        .collect()
}

pub fn random_secret() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..24)
        .map(|_| {
            let n = rng.random_range(0..62u32);
            match n {
                0..=9 => (b'0' + n as u8) as char,
                10..=35 => (b'a' + (n - 10) as u8) as char,
                _ => (b'A' + (n - 36) as u8) as char,
            }
        })
        .collect()
}
