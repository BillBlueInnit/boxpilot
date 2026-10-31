# BoxPilot

**English** · [简体中文](README.zh-CN.md) · [Русский](README.ru.md)

A clean desktop proxy client for Windows, macOS and Linux. It drives the
[sing-box](https://sing-box.sagernet.org) core, converts subscriptions itself, and is written in Rust
with [GPUI](https://gpui.rs) (via gpui-kit).

[![CI](https://github.com/BillBlueInnit/boxpilot/actions/workflows/ci.yml/badge.svg)](https://github.com/BillBlueInnit/boxpilot/actions/workflows/ci.yml)

**[Download](https://github.com/BillBlueInnit/boxpilot/releases/latest)** ·
**[User guide](docs/usage.md)** ·
[Build from source](#build-from-source)

- [Install](#install)
- [Quick start](#quick-start)
- [Features](#features)
- [Platform support](#platform-support)
- [Build from source](#build-from-source)
- [Project layout](#project-layout)

## Install

Files are on the [latest release](https://github.com/BillBlueInnit/boxpilot/releases/latest). Each
system has a section in the user guide covering first launch, updates and removal.

| System | Install | Guide |
| --- | --- | --- |
| Windows 10 / 11 | Unzip `BoxPilot-<version>-windows-x64.zip` and run `boxpilot.exe` | [Windows](docs/usage.md#windows) |
| macOS (Apple silicon / Intel) | Open `BoxPilot-<version>-macos-arm64.dmg` / `-macos-x64.dmg`, drag BoxPilot to Applications | [macOS](docs/usage.md#macos) |
| Ubuntu, Debian, Linux Mint | `sudo apt install ./boxpilot_<version>-1_amd64.deb` | [Ubuntu, Debian](docs/usage.md#ubuntu-debian-linux-mint) |
| Fedora, openSUSE | `sudo dnf install ./boxpilot-<version>-1.x86_64.rpm` (openSUSE: `zypper`) | [Fedora, openSUSE](docs/usage.md#fedora-opensuse) |
| Arch, Manjaro, EndeavourOS | `sudo pacman -Syu boxpilot-bin` from BoxPilot's repository, see below | [Arch](docs/usage.md#arch-linux-manjaro-endeavouros) |
| NixOS | The Linux archive through `nix-ld` | [NixOS](docs/usage.md#nixos) |
| Any other Linux (x64) | Unpack `BoxPilot-<version>-linux-x64.tar.gz` and run `./boxpilot` | [Other Linux](docs/usage.md#other-linux-distributions) |

The Linux packages need glibc 2.39 or newer: Ubuntu 24.04, Debian 13, Fedora 40, openSUSE
Tumbleweed or later.

**Arch Linux.** Every release doubles as a pacman repository. Add it once, then install; later
versions come with your usual `pacman -Syu` (or `yay`):

```bash
printf '\n[boxpilot]\nSigLevel = Optional TrustAll\nServer = https://github.com/BillBlueInnit/boxpilot/releases/latest/download\n' | sudo tee -a /etc/pacman.conf
sudo pacman -Syu boxpilot-bin
```

The packages are not signed, so that entry trusts what this project's GitHub releases serve.

## Quick start

1. **Get the core.** Dashboard → **Download core**. BoxPilot fetches the official sing-box build for
   your system (set a **GitHub mirror** under Settings → sing-box core if GitHub is slow for you).
2. **Add your nodes.** Subscriptions → **Add**, paste your provider's URL, or press **Paste** to
   import from the clipboard.
3. **Connect.** Dashboard → **Connect**, then turn on **System proxy** or **TUN mode** under Quick
   settings.

The [user guide](docs/usage.md) walks through everything step by step.

## Features

**Subscriptions and nodes**

- Formats: v2ray / Shadowrocket share links (plain or base64), Clash / Clash.Meta YAML, sing-box JSON.
- Protocols: VLESS (REALITY, Vision), VMess, Trojan, Shadowsocks (2022, obfs, v2ray-plugin),
  Hysteria 2 (port hopping), Hysteria, TUIC, AnyTLS, WireGuard, SOCKS, HTTP(S) and SSH, over ws, grpc,
  h2, httpupgrade and quic. Unsupported entries (SSR, xhttp…) are logged and skipped.
- Updates: an interval per subscription, update on start, catch-up after the app was closed, quick
  retry after a failure, proxy → direct fallback, custom User-Agent, include / exclude filters,
  traffic and expiry from `subscription-userinfo`. A batch of updates reloads the core once.
- Local nodes: add single nodes from share links; copy any node back out as a share link.

**Routing**

- Rule, Global and Direct modes, switched live.
- A selector and an **Auto** group per subscription. Auto picks the quickest node, a steady one,
  the first working one in list order, or the one with the most bandwidth, optionally within one
  region. Switching never cuts open connections.
- Rule-sets from a URL or a file: sing-box `.srs` / `.json`, Clash rule providers and ACL4SSR
  `.list`. Each goes to Proxy, Direct, Reject or any group, in your order, on its own refresh interval.
- **Import ACL4SSR** reads a subconverter `.ini` (for example ACL4SSR_Online_Full) and creates all
  of its rule-sets, switched off until you pick the ones you want.
- A rule-set can have its own node group, chosen by hand or automatically, and its file opens in
  your text editor; saves apply by themselves.
- Built-in mainland China bypass and ad blocking, plus custom rules (`TYPE,VALUE,TARGET`).
- DNS over HTTPS, TLS, QUIC or UDP, FakeIP and IPv6. Rule-sets are cached locally, so a blocked
  GitHub or a dead node never keeps the core from starting.

**Connecting**

- System proxy (restored on exit and after a crash), TUN mode, LAN sharing, UDP through the proxy
  or direct, and an optional QUIC block.
- Latency, real latency (a fresh HTTPS request through the node) and download speed tests, through
  a separate probe so your selection stays as it is.
- A country code on every node: from its name, else a GeoIP lookup, else its exit IP.
- Settings changes are batched into one core reload; a crashed core restarts by itself.

**App**

- English, 简体中文, 繁體中文 and Русский, following the system language and switchable live.
- Tray menu with mode, system proxy and TUN switches; close to tray; **Lightweight mode** frees the
  interface while the proxy keeps running.
- Launch at login (optionally minimized), live traffic chart, connections view, log viewer.
- Light and dark themes, a theme colour and a translucent, blurred window where the system offers
  one (see [Platform support](#platform-support)).
- Updates from GitHub Releases: install automatically, just remind, or never notify.
- Every transition is a damped spring that respects "reduce motion".

## Platform support

The same features everywhere GPUI runs. Where systems differ, each uses its own mechanism:

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Package | zip | `.dmg` with BoxPilot.app | `.deb`, `.rpm`, Arch repository, tar.gz |
| Window frame | native, snap layouts | native traffic lights | drawn by BoxPilot (Adwaita or Breeze buttons), or by the compositor on tiling ones |
| Translucent window | Mica, Mica Alt, Acrylic (Windows 11 22H2+) | system vibrancy | blur where the compositor offers it (niri, KDE Plasma) |
| System proxy | WinINet | `networksetup`, every network service | GNOME `gsettings` and KDE `kioslaverc` |
| TUN permission | run as administrator | core made setuid root after one password prompt | core granted `CAP_NET_ADMIN` after one password prompt |
| Launch at login | Run key / scheduled task | LaunchAgent | XDG autostart |
| Core cleanup after a crash | job object | at next start | parent-death signal, and at next start |
| Data folder | `%APPDATA%\BoxPilot` | `~/Library/Application Support/BoxPilot` | `~/.config/BoxPilot` |

Linux desktop details (tray icons, TUN without a polkit agent, blur, tiling compositors, laptops
with two GPUs) are in the [user guide](docs/usage.md#8-linux-desktops).

## Build from source

You need stable Rust ([rustup](https://rustup.rs)). On Linux you also need the development
packages GPUI builds against. On Debian or Ubuntu, CI installs:

```bash
sudo apt install build-essential pkg-config clang libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev \
  libpipewire-0.3-dev libgtk-3-dev libayatana-appindicator3-dev libxdo-dev
```

On Arch, the [`boxpilot` recipe](packaging/aur/boxpilot/PKGBUILD) lists them.

Run it:

```bash
cargo run --release -p boxpilot
```

**Tests.** `cargo test --workspace` runs the unit tests. To check generated configs against a real
core:

```bash
cargo run -p boxpilot-core --example gen_config -- out
sing-box check -c out/everything-1.14.json
```

Generated configs target sing-box 1.12 or newer (1.14 recommended).

**Linux packages.** With `cargo install cargo-deb cargo-generate-rpm`,
`packaging/linux/build.sh [tar] [deb] [rpm] [arch]` writes the archive, `.deb`, `.rpm` and, on Arch,
the pacman package and repository to `dist/`. Packages built on a rolling distribution need its
glibc; the released ones are built on Ubuntu 24.04. Recipes for the AUR are in
[`packaging/aur`](packaging/aur).

**A second copy for development.** `BOXPILOT_INSTANCE=dev` gives a copy its own single-instance
lock, and `XDG_CONFIG_HOME` (Linux) its own data folder, so it runs next to your everyday BoxPilot.

**CI and releases.**

- [`ci.yml`](.github/workflows/ci.yml) builds and tests on Windows, macOS (arm64, x64) and Linux:
  unit tests, every generated config through `sing-box check`, a live engine test with the real core,
  a GUI start-up check, and the Linux packages installed on Ubuntu and Arch.
- [`live.yml`](.github/workflows/live.yml) checks the system proxy and TUN on real machines.
- [`release.yml`](.github/workflows/release.yml) publishes a release for a version tag. Set the
  version in `Cargo.toml`, then:

  ```bash
  git tag v1.2.3
  git push origin v1.2.3
  ```

  It attaches the Windows zip, the macOS `.dmg` and archives, and the Linux archive, `.deb`, `.rpm`
  and Arch repository. With an `AUR_SSH_PRIVATE_KEY` secret it also updates the AUR.

## Project layout

| Path | What's there |
| --- | --- |
| `crates/core` | Everything without a UI: subscription converter (`convert/`), config generator (`config.rs`), rule-set conversion (`rules.rs`), the engine with process lifecycle, scheduling and updates (`engine.rs`), Clash API client, core downloader, OS integration (`sys.rs`). |
| `crates/app` | The GPUI app: pages (`src/ui/`), springs (`ui/anim.rs`), translations (`i18n.rs`), tray, single instance, windows. |
| `packaging/` | Linux desktop entry and package scripts, AUR recipes, macOS `Info.plist`. |
| `docs/` | The user guide in three languages. |

## License

MIT, see [LICENSE](LICENSE).
