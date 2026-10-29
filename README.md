# BoxPilot

**English** · [简体中文](README.zh-CN.md) · [Русский](README.ru.md) · [How to use](docs/usage.md)

A clean desktop proxy client for Windows, macOS and Linux, written in Rust with
[GPUI](https://gpui.rs) (via gpui-kit). It drives the [sing-box](https://sing-box.sagernet.org) core
and has a built-in subscription converter.

## Getting started

Download BoxPilot for your platform from
[Releases](https://github.com/BillBlueInnit/boxpilot/releases):

- **Windows**: `BoxPilot-<version>-windows-x64.zip` — unzip and run `boxpilot.exe`.
- **macOS**: `BoxPilot-<version>-macos-arm64.dmg` (Apple silicon) or `-macos-x64.dmg` (Intel) — open it
  and drag BoxPilot into Applications. The app is not notarized: the first time, right-click it and
  choose **Open** (or allow it under System Settings → Privacy & Security).
- **Linux**: `BoxPilot-<version>-linux-x64.tar.gz` — unpack and run `boxpilot`.

Or build it yourself:

```
cargo run --release -p boxpilot
```

Then:

1. **Settings → sing-box core → Download latest.** The core is not bundled; BoxPilot fetches the
   official release for your platform from GitHub (through the proxy when one is running, or a
   mirror prefix you set).
2. **Subscriptions → Add**, paste your provider's URL (or **Paste** to import from the clipboard).
3. **Dashboard → Connect.**

The step-by-step guide is in [docs/usage.md](docs/usage.md). Data lives in the folder listed under
[Platforms](#platforms); on Windows, a `data` folder next to `boxpilot.exe` makes it portable.

## Features

- **Subscription converter**: v2ray/Shadowrocket share links (plain or base64), Clash / Clash.Meta
  YAML and sing-box JSON. Supports VLESS (REALITY, Vision), VMess, Trojan, Shadowsocks (incl. 2022,
  obfs and v2ray-plugin), Hysteria 2 (port hopping), Hysteria, TUIC, AnyTLS, WireGuard, SOCKS, HTTP(S)
  and SSH, over ws / grpc / h2 / httpupgrade / quic transports. Unsupported entries (SSR, xhttp…)
  are reported in the log instead of breaking the import.
- **Subscription updates**: per-subscription interval, update on startup, catch-up of updates missed
  while the app was closed, quicker retry after failures, proxy → direct fallback, custom User-Agent,
  include/exclude regex filters (re-applied offline), traffic/expiry from `subscription-userinfo`,
  provider title and interval hints. The core reloads once after a batch of updates.
- **Proxy**: Rule / Global / Direct modes (switched live), selector + Auto groups per
  subscription, latency tests, system proxy (restored on exit and after crashes), TUN mode with a
  "restart as administrator" flow, LAN sharing, UDP through the proxy (or direct) and an optional
  QUIC block. Config changes are batched into one core reload, Auto switching never cuts open
  connections, and a crashed core is restarted automatically.
- **Local nodes and sharing**: add single nodes from share links (ss, vmess, vless, trojan, hysteria2,
  tuic, anytls, socks5, http(s), wireguard, ssh — or Clash / sing-box node lists) into a **Local
  nodes** group, and copy any node — from a subscription or local — back out as a share link.
- **Auto group**: quickest response, stay steady (switches only on failure or a big gap), list
  order (failover down your list and back), or most bandwidth (periodic speed tests of the
  quickest nodes) — each optionally limited to one region.
- **Speed test**: download speed and real latency (a fresh HTTPS request through the node) per node
  or for all shown nodes, through a separate local probe so your selection is untouched.
- **Rules**: import rule-sets from a URL or a local file — sing-box `.srs` / `.json`, Clash rule
  providers (`payload:` YAML, domain / ipcidr / classical) and ACL4SSR `.list` files, which are
  converted to sing-box rule-sets. Each set routes to Proxy, Direct, Reject or any group, in the
  order you arrange, and refreshes on its own interval. **Import ACL4SSR** reads a subconverter
  `.ini` (e.g. ACL4SSR_Online_Full) and creates every `ruleset=` entry with the right action,
  switched off until you pick the ones you want (Enable all / Disable all). A rule-set can get a
  **node group of its own** on the Proxies page — chosen by hand or picked automatically with the
  same modes and region filter as Auto — and its file can be opened in your text editor; saves
  apply by themselves. Built-in China bypass and ad blocking, plus custom rules (`TYPE,VALUE,TARGET`).
- **DNS**: DoH/DoT/DoQ/UDP, FakeIP, IPv6. Rule-sets are cached by the app and handed to sing-box as
  local files, so a blocked GitHub or a dead node never stops the core from starting.
- **App**: English, 简体中文, 繁體中文 and Русский (follows the system language, switchable
  live); run as administrator (login start through a highest-privilege scheduled task, so no UAC
  prompt); tray icon with mode/system-proxy/TUN toggles, close to tray, single instance, launch at
  login (optionally minimized), live traffic chart, connections view, log viewer, light/dark themes,
  a theme colour, and a translucent window (Mica, Mica Alt or Acrylic on Windows 11, the system blur
  on macOS and KDE Plasma).
- **Proxies page**: a country code on every node — from the node name (flag, place, code), else a
  GeoIP lookup of the server, else (anycast/CDN or private addresses) the exit IP checked through
  that node. Lightweight mode unloads the UI while the app sits in the tray.
- **App updates**: checks GitHub Releases once a day; install automatically, just remind, or never
  notify (Settings → Updates). Updates replace the executable in place and apply on restart.
- **Motion**: every transition is a damped spring (GPUI's analytic solver), so movement eases and
  settles like a physical object, keeps its velocity when interrupted and respects "reduce motion".

## Platforms

BoxPilot runs on every platform GPUI supports — Windows 10/11, macOS (Apple silicon and Intel)
and Linux (X11 and Wayland) — with the same features. Where the operating systems differ, each
one gets its native mechanism:

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Install | zip | `.dmg` with BoxPilot.app (Dock, Launchpad) | tar.gz |
| Window | borderless, native frame, snap layouts on Maximize, optional Mica | native traffic lights | client-side decorations (or the WM's own) |
| Caption buttons | Segoe Fluent / MDL2 glyphs, WinUI colours | AppKit traffic lights | Adwaita (GNOME) or Breeze (KDE) style, `button-layout` order and side |
| System proxy | WinINet (registry) | `networksetup`, every network service | KDE `kioslaverc` and GNOME `gsettings` (both, for KDE, GTK and Firefox) |
| TUN permission | run BoxPilot as administrator (login via scheduled task, no UAC prompt) | core made setuid root after one admin prompt; DNS points at TUN while it runs | `CAP_NET_ADMIN` on the core via `pkexec setcap`, or a setuid core where `setcap` isn't installed |
| Launch at login | Run key / scheduled task | LaunchAgent | XDG autostart |
| Close to tray | hides the window | hides the app (Dock icon brings it back) | closes the window (tray reopens it) |
| Core cleanup if the app crashes | job object | stale-pid cleanup at next start | parent-death signal + stale-pid cleanup |
| Data folder | `%APPDATA%\BoxPilot` | `~/Library/Application Support/BoxPilot` | `$XDG_CONFIG_HOME/BoxPilot` |

Linux needs an AppIndicator-capable tray (GNOME: the AppIndicator extension) for the tray icon.

`.github/workflows/ci.yml` builds and tests natively on Windows, macOS (arm64 and x64) and Linux:
unit tests, every generated config through `sing-box check`, a live engine test against the real
core (start, groups, selection, latency, mode switch, traffic, reload, stop) and a GUI start-up
check (`boxpilot --smoke-test`).

`.github/workflows/release.yml` publishes a release when a version tag is pushed. The tag must
match the workspace version in `Cargo.toml`:

```
git tag v1.0.0
git push origin v1.0.0
```

It attaches `BoxPilot-<version>-<platform>.zip|tar.gz` for Windows x64, macOS arm64/x64 and
Linux x64 — the names the in-app updater looks for — plus a `.dmg` for each macOS build.

## Layout

| Path | What it does |
| --- | --- |
| `crates/core` | No UI. Converter (`convert/`), config generator (`config.rs`), engine with the process lifecycle, scheduler and updates (`engine.rs`), Clash API client, core downloader and Windows integration (`sys.rs`). |
| `crates/core` (`rules.rs`) | Rule-set conversion (Clash / ACL4SSR → sing-box) and ACL4SSR `.ini` parsing. |
| `crates/app` | GPUI front end: pages in `src/ui/`, springs in `ui/anim.rs`, translations in `i18n.rs`, tray, single instance, window handling. |

`cargo test -p boxpilot-core` runs the converter and config tests. To validate generated configs
against a real core:

```
cargo run -p boxpilot-core --example gen_config -- out
sing-box check -c out/everything-1.14.json
```

Generated configs target sing-box 1.12+ (1.14 recommended): rule actions, endpoints for WireGuard,
the new DNS server format and `http_clients`.

## License

MIT — see [LICENSE](LICENSE).
