//! On-disk persistence: settings, subscriptions and converted nodes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};

use crate::model::{Node, Settings, Subscription};

pub const APP_NAME: &str = "BoxPilot";

/// `%APPDATA%` on Windows, `~/Library/Application Support` on macOS and
/// `$XDG_CONFIG_HOME` (or `~/.config`) on Linux.
fn config_home() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home)
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home().join(".config"))
    }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub core_dir: PathBuf,
    pub run_dir: PathBuf,
    pub subs_dir: PathBuf,
}

impl Paths {
    /// Portable mode when a `data` folder sits next to the executable,
    /// otherwise `%APPDATA%\BoxPilot`.
    pub fn detect() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let portable = exe_dir.as_ref().map(|d| d.join("data")).filter(|d| d.is_dir());
        let root = portable.unwrap_or_else(|| config_home().join(APP_NAME));
        Self::at(root)
    }

    pub fn at(root: PathBuf) -> Self {
        Self {
            core_dir: root.join("core"),
            run_dir: root.join("run"),
            subs_dir: root.join("subs"),
            root,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        for d in [&self.root, &self.core_dir, &self.run_dir, &self.subs_dir] {
            std::fs::create_dir_all(d).with_context(|| format!("create {}", d.display()))?;
        }
        Ok(())
    }

    pub fn default_core_path(&self) -> PathBuf {
        self.core_dir.join(if cfg!(windows) { "sing-box.exe" } else { "sing-box" })
    }

    pub fn config_file(&self) -> PathBuf {
        self.run_dir.join("config.json")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn rules_file(&self) -> PathBuf {
        self.root.join("rule_providers.json")
    }

    /// Converted rule-sets and the cached geosite / geoip files.
    pub fn rules_dir(&self) -> PathBuf {
        self.run_dir.join("rules")
    }

    pub fn subs_file(&self) -> PathBuf {
        self.root.join("subscriptions.json")
    }

    fn nodes_file(&self, id: &str) -> PathBuf {
        self.subs_dir.join(format!("{id}.json"))
    }

    fn raw_file(&self, id: &str) -> PathBuf {
        self.subs_dir.join(format!("{id}.raw"))
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

#[derive(Clone)]
pub struct Store {
    pub paths: Paths,
    /// Set by a reset: nothing may write the deleted files back.
    frozen: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Store {
    pub fn new(paths: Paths) -> Result<Self> {
        paths.ensure()?;
        Ok(Self { paths, frozen: Default::default() })
    }

    /// Stops all further saves (the app is about to restart after a reset).
    pub fn freeze(&self) {
        self.frozen.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn frozen(&self) -> bool {
        self.frozen.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn load_settings(&self) -> Settings {
        match read_json::<Settings>(&self.paths.settings_file()) {
            Ok(Some(s)) => s,
            Ok(None) => Settings::default(),
            Err(e) => {
                log::error!("settings unreadable, using defaults: {e:#}");
                let _ = std::fs::copy(
                    self.paths.settings_file(),
                    self.paths.root.join("settings.broken.json"),
                );
                Settings::default()
            }
        }
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        if self.frozen() {
            return Ok(());
        }
        write_json(&self.paths.settings_file(), s)
    }

    pub fn load_rule_providers(&self) -> Vec<crate::rules::RuleProvider> {
        read_json(&self.paths.rules_file())
            .unwrap_or_else(|e| {
                log::error!("rule providers unreadable: {e:#}");
                None
            })
            .unwrap_or_default()
    }

    pub fn save_rule_providers(&self, providers: &[crate::rules::RuleProvider]) -> Result<()> {
        if self.frozen() {
            return Ok(());
        }
        write_json(&self.paths.rules_file(), providers)
    }

    pub fn load_subscriptions(&self) -> Vec<Subscription> {
        read_json(&self.paths.subs_file())
            .unwrap_or_else(|e| {
                log::error!("subscriptions unreadable: {e:#}");
                None
            })
            .unwrap_or_default()
    }

    pub fn save_subscriptions(&self, subs: &[Subscription]) -> Result<()> {
        if self.frozen() {
            return Ok(());
        }
        write_json(&self.paths.subs_file(), subs)
    }

    pub fn load_nodes(&self, id: &str) -> Vec<Node> {
        read_json(&self.paths.nodes_file(id))
            .unwrap_or_else(|e| {
                log::error!("nodes for {id} unreadable: {e:#}");
                None
            })
            .unwrap_or_default()
    }

    pub fn save_nodes(&self, id: &str, nodes: &[Node]) -> Result<()> {
        if self.frozen() {
            return Ok(());
        }
        write_json(&self.paths.nodes_file(id), nodes)
    }

    pub fn delete_nodes(&self, id: &str) {
        let _ = std::fs::remove_file(self.paths.nodes_file(id));
        let _ = std::fs::remove_file(self.paths.raw_file(id));
    }

    /// The last downloaded subscription body, kept so filters can be re-applied offline.
    pub fn save_raw(&self, id: &str, content: &str) -> Result<()> {
        if self.frozen() {
            return Ok(());
        }
        write_atomic(&self.paths.raw_file(id), content.as_bytes())
    }

    pub fn load_raw(&self, id: &str) -> Option<String> {
        std::fs::read_to_string(self.paths.raw_file(id)).ok()
    }
}
