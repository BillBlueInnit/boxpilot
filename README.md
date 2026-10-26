# BoxPilot

A clean desktop proxy client for Windows, written in Rust with [GPUI](https://gpui.rs) (via gpui-kit).
It drives the [sing-box](https://sing-box.sagernet.org) core and has a built-in subscription converter.

## Getting started

```
cargo run --release -p boxpilot
```

1. **Settings → sing-box core → Download latest.** The core is not bundled; BoxPilot fetches the
   official release for your platform from GitHub (through the proxy when one is running, or a
   mirror prefix you set).
2. **Subscriptions → Add**, paste your provider's URL (or **Paste** to import from the clipboard).
3. **Dashboard → Connect.**

Data lives in `%APPDATA%\BoxPilot`. Create a `data` folder next to `boxpilot.exe` for portable mode.

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
- **Proxy**: Rule / Global / Direct modes (switched live), selector + auto (urltest) groups per
  subscription, latency tests, system proxy (restored on exit and after crashes), TUN mode with a
  "restart as administrator" flow, LAN sharing.
- **Rules**: import rule-sets from a URL or a local file — sing-box `.srs` / `.json`, Clash rule
  providers (`payload:` YAML, domain / ipcidr / classical) and ACL4SSR `.list` files, which are
  converted to sing-box rule-sets. Each set routes to Proxy, Direct, Reject or any group, in the
  order you arrange, and refreshes on its own interval. **Import ACL4SSR** reads a subconverter
  `.ini` (e.g. ACL4SSR_Online_Full) and creates every `ruleset=` entry with the right action.
  Built-in China bypass and ad blocking, plus custom rules (`TYPE,VALUE,TARGET`).
- **DNS**: DoH/DoT/DoQ/UDP, FakeIP, IPv6. Rule-sets are cached by the app and handed to sing-box as
  local files, so a blocked GitHub or a dead node never stops the core from starting.
- **App**: English, 简体中文, 繁體中文 and Русский (follows the Windows display language, switchable
  live); run as administrator (login start through a highest-privilege scheduled task, so no UAC
  prompt); tray icon with mode/system-proxy/TUN toggles, close to tray, single instance, launch at
  login (optionally minimized), live traffic chart, connections view, log viewer, light/dark themes.
- **Motion**: every transition is a damped spring (GPUI's analytic solver), so movement eases and
  settles like a physical object, keeps its velocity when interrupted and respects "reduce motion".

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
