//! Downloads the sing-box core from the official GitHub releases.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::Deserialize;

const REPO: &str = "SagerNet/sing-box";

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub prerelease: bool,
    pub asset_name: String,
    pub download_url: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn platform_suffix() -> Result<String> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "linux" => "linux",
        "macos" => "darwin",
        other => bail!("unsupported OS {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        other => bail!("unsupported architecture {other}"),
    };
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    Ok(format!("{os}-{arch}.{ext}"))
}

/// Finds the newest release (optionally including pre-releases) with a build for this platform.
pub async fn latest_release(client: &reqwest::Client, include_prerelease: bool) -> Result<Release> {
    let suffix = platform_suffix()?;
    let releases: Vec<GhRelease> = client
        .get(format!("https://api.github.com/repos/{REPO}/releases?per_page=15"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("query GitHub releases")?
        .error_for_status()
        .context("query GitHub releases")?
        .json()
        .await?;
    for r in releases {
        if r.draft || (r.prerelease && !include_prerelease) {
            continue;
        }
        let version = r.tag_name.trim_start_matches('v').to_string();
        let wanted = format!("sing-box-{version}-{suffix}");
        if let Some(a) = r.assets.into_iter().find(|a| a.name == wanted) {
            return Ok(Release {
                version,
                prerelease: r.prerelease,
                asset_name: a.name,
                download_url: a.browser_download_url,
                size: a.size,
            });
        }
    }
    bail!("no sing-box release found for {suffix}")
}

/// Downloads `release` and reports `(downloaded, total)` while doing so.
pub async fn download(
    client: &reqwest::Client,
    release: &Release,
    mirror_prefix: &str,
    mut progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>> {
    let url = if mirror_prefix.trim().is_empty() {
        release.download_url.clone()
    } else {
        format!("{}{}", mirror_prefix.trim(), release.download_url)
    };
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(600))
        .send()
        .await
        .with_context(|| format!("download {url}"))?
        .error_for_status()?;
    let total = resp.content_length().unwrap_or(release.size);
    let mut data = Vec::with_capacity(total as usize);
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        data.extend_from_slice(&chunk?);
        progress(data.len() as u64, total);
    }
    if release.size > 0 && data.len() as u64 != release.size {
        bail!("download incomplete ({} of {} bytes)", data.len(), release.size);
    }
    Ok(data)
}

/// Extracts the release archive (`.zip` on Windows, `.tar.gz` elsewhere).
pub fn install_archive(name: &str, archive: &[u8], dest: &Path) -> Result<()> {
    if name.ends_with(".tar.gz") {
        install_tar_gz(archive, dest)
    } else {
        install_zip(archive, dest)
    }
}

/// Writes one extracted file, replacing the old one and keeping it executable.
fn place_file(dest: &Path, name: &str, data: &[u8]) -> Result<()> {
    let target = dest.join(name);
    let tmp = dest.join(format!("{name}.new"));
    std::fs::write(&tmp, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    if target.exists() {
        std::fs::remove_file(&target)
            .with_context(|| format!("replace {} (is it still running?)", target.display()))?;
    }
    std::fs::rename(&tmp, &target)?;
    Ok(())
}

fn install_tar_gz(archive: &[u8], dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut found_exe = false;
    for entry in tar.entries().context("open archive")? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let Some(name) = entry.path()?.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if name.eq_ignore_ascii_case("LICENSE") {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        place_file(dest, &name, &buf)?;
        found_exe |= name.starts_with("sing-box");
    }
    if !found_exe {
        bail!("archive does not contain sing-box");
    }
    Ok(())
}

fn install_zip(archive: &[u8], dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).context("open archive")?;
    let mut found_exe = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        else {
            continue;
        };
        if name.eq_ignore_ascii_case("LICENSE") {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        place_file(dest, &name, &buf)?;
        found_exe |= name.starts_with("sing-box");
    }
    if !found_exe {
        bail!("archive does not contain sing-box");
    }
    Ok(())
}
