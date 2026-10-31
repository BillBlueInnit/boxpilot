//! Checks GitHub Releases for a newer BoxPilot and swaps the executable in place.
//!
//! Release assets are named `BoxPilot-<version>-<platform>.<zip|tar.gz>` (see
//! `.github/workflows/release.yml`) and carry the `boxpilot` binary.

use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::Value;

pub const REPO: &str = "BillBlueInnit/boxpilot";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// Without the leading `v`.
    pub version: String,
    /// The release page on GitHub.
    pub page: String,
    pub notes: String,
    /// The download for this platform, if the release has one.
    pub asset: Option<Asset>,
}

impl Release {
    pub fn is_newer(&self) -> bool {
        is_newer(&self.version, CURRENT)
    }
}

/// Asset name suffix for the platform this binary was built for.
pub fn platform_suffix() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64.zip",
        ("windows", "aarch64") => "windows-arm64.zip",
        ("macos", "aarch64") => "macos-arm64.tar.gz",
        ("macos", "x86_64") => "macos-x64.tar.gz",
        ("linux", "x86_64") => "linux-x64.tar.gz",
        ("linux", "aarch64") => "linux-arm64.tar.gz",
        _ => return None,
    })
}

/// Linux packages (deb, rpm, AUR) put the binary under `/usr`, which the
/// package manager owns: those installs hear about updates but leave the
/// install to `apt`, `dnf` or `yay`.
pub fn package_managed() -> bool {
    cfg!(target_os = "linux")
        && std::env::current_exe().is_ok_and(|p| (p.starts_with("/usr") && !p.starts_with("/usr/local")) || p.starts_with("/opt"))
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    // "1.2.3-beta" compares as 1.2.3.
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Reads a `GET /repos/{REPO}/releases/latest` response.
pub fn parse_release(body: &Value) -> Option<Release> {
    let tag = body.get("tag_name")?.as_str()?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let suffix = if package_managed() { None } else { platform_suffix() };
    let asset = body.get("assets").and_then(Value::as_array).and_then(|assets| {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            if !suffix.is_some_and(|s| name.ends_with(s)) {
                return None;
            }
            Some(Asset {
                name: name.to_string(),
                url: a.get("browser_download_url")?.as_str()?.to_string(),
                size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
            })
        })
    });
    Some(Release {
        version,
        page: body.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        notes: body.get("body").and_then(Value::as_str).unwrap_or_default().to_string(),
        asset,
    })
}

/// The latest published release, or `None` when the repository has none yet.
pub async fn latest(client: &reqwest::Client) -> Result<Option<Release>> {
    let resp = client
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("accept", "application/vnd.github+json")
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let body: Value = resp.error_for_status()?.json().await?;
    Ok(parse_release(&body))
}

const BINARY: &str = if cfg!(windows) { "boxpilot.exe" } else { "boxpilot" };

/// Pulls the app binary out of a release archive.
pub fn extract_binary(archive: &[u8], name: &str) -> Result<Vec<u8>> {
    let is_ours = |path: &str| path.rsplit(['/', '\\']).next() == Some(BINARY);
    let mut out = Vec::new();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).context("open zip")?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            if f.is_file() && is_ours(f.name()) {
                f.read_to_end(&mut out)?;
                return Ok(out);
            }
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for entry in tar.entries().context("open archive")? {
            let mut entry = entry?;
            if entry.header().entry_type().is_file() && is_ours(&entry.path()?.to_string_lossy()) {
                entry.read_to_end(&mut out)?;
                return Ok(out);
            }
        }
    }
    bail!("{BINARY} not found in {name}")
}

fn current_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locate the running executable")?;
    // After an in-place update Windows may report the renamed original.
    Ok(if exe.extension().is_some_and(|e| e == "old") { exe.with_extension(std::env::consts::EXE_EXTENSION) } else { exe })
}

/// The executable a restart should launch (the updated one after an install).
pub fn restart_target() -> Option<PathBuf> {
    current_exe().ok()
}

/// Replaces the running executable with `binary`. The new version runs from
/// the next start; the old file is removed then (see [`cleanup`]).
pub fn install(binary: &[u8]) -> Result<PathBuf> {
    if binary.len() < 1024 {
        bail!("downloaded binary is too small");
    }
    let exe = current_exe()?;
    let new = exe.with_extension("new");
    std::fs::write(&new, binary).with_context(|| format!("write {}", new.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(windows)]
    {
        // A running .exe can't be overwritten, but it can be renamed.
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).with_context(|| format!("move {} aside", exe.display()))?;
        if let Err(e) = std::fs::rename(&new, &exe) {
            let _ = std::fs::rename(&old, &exe);
            let _ = std::fs::remove_file(&new);
            return Err(anyhow::Error::new(e).context("put the new version in place"));
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(&new, &exe).with_context(|| format!("replace {}", exe.display()))?;
    // Inside BoxPilot.app the bundle's ad-hoc signature covers the binary;
    // sign it again so macOS keeps opening the app.
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
        let _ = std::process::Command::new("codesign").args(["--force", "--deep", "--sign", "-"]).arg(bundle).output();
    }
    Ok(exe)
}

/// Removes what the previous update left behind.
pub fn cleanup() {
    if let Ok(exe) = current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
        let _ = std::fs::remove_file(exe.with_extension("new"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn picks_this_platforms_asset() {
        let suffix = platform_suffix().unwrap();
        let body = serde_json::json!({
            "tag_name": "v9.9.9",
            "html_url": "https://github.com/x/y/releases/tag/v9.9.9",
            "body": "notes",
            "assets": [
                { "name": "BoxPilot-9.9.9-other.tar.xz", "browser_download_url": "https://a", "size": 1 },
                { "name": format!("BoxPilot-9.9.9-{suffix}"), "browser_download_url": "https://b", "size": 2 }
            ]
        });
        let r = parse_release(&body).unwrap();
        assert_eq!(r.version, "9.9.9");
        assert!(r.is_newer());
        assert_eq!(r.asset.unwrap().url, "https://b");
    }
}
