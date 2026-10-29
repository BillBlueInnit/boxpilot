# How to use BoxPilot

**English** · [简体中文](usage.zh-CN.md) · [Русский](usage.ru.md) · [Back to README](../README.md)

This guide walks you from installing BoxPilot to routing apps through your nodes the way you want.

## 1. Install

Get the file for your system from [Releases](https://github.com/BillBlueInnit/boxpilot/releases).

| System | File | How |
| --- | --- | --- |
| Windows 10/11 | `BoxPilot-<version>-windows-x64.zip` | Unzip anywhere and run `boxpilot.exe`. |
| macOS, Apple silicon | `BoxPilot-<version>-macos-arm64.dmg` | Open the .dmg, drag **BoxPilot** onto **Applications**. |
| macOS, Intel | `BoxPilot-<version>-macos-x64.dmg` | Same as above. |
| Linux x64 | `BoxPilot-<version>-linux-x64.tar.gz` | Unpack and run `./boxpilot`. |

**macOS, first launch.** BoxPilot is not notarized by Apple, so macOS asks before opening it the
first time. Right-click BoxPilot in Applications → **Open** → **Open**. On macOS 15 and later, open it
once, then go to **System Settings → Privacy & Security** and click **Open Anyway**. If macOS says the
app "is damaged", run this once in Terminal and open it again:

```bash
xattr -dr com.apple.quarantine /Applications/BoxPilot.app
```

**Linux.** The tray icon needs an AppIndicator tray (on GNOME: the AppIndicator extension).

BoxPilot updates itself: **Settings → Updates** chooses whether new versions install automatically,
only show a reminder, or are never announced.

## 2. Get the sing-box core

BoxPilot drives the [sing-box](https://sing-box.sagernet.org) core but doesn't ship it. On the
**Dashboard** click **Download core** (or **Settings → sing-box core → Download latest**). BoxPilot
fetches the official build for your system from GitHub. If GitHub is slow where you are, set a
**GitHub mirror** prefix in the same section first.

## 3. Add your nodes

### A subscription from your provider

**Subscriptions → Add → Add subscription**, paste the subscription URL and click **Add**. Or copy
the URL and press **Paste** — BoxPilot recognises what's on the clipboard. Share links, base64
lists, Clash / Clash.Meta YAML and sing-box JSON all work.

Each subscription card can be switched off, updated now, edited (update interval, update on start,
User-Agent, include / exclude filters) or deleted. **Select** lets you act on several at once.

### Single nodes (Local nodes)

To add a node you got on its own — `ss://`, `vmess://`, `vless://`, `trojan://`, `hysteria2://`,
`hysteria://`, `tuic://`, `anytls://`, `socks5://`, `http(s)://`, `wireguard://` or `ssh://`:

1. **Subscriptions → Add → Add node** (or copy the link and press **Paste**).
2. Paste one or more links, one per line, and click **Add**.

They are kept together in the **Local nodes** card and show up as their own **Local nodes** group
on the Proxies page. Each row has a share button and a delete button; the switch on the card turns
the whole group on or off.

### Share a node

- On the **Proxies** page, hover a node card and click the share icon next to its name: the share
  link is copied to the clipboard.
- On a **subscription card**, the share button copies the links of all its nodes.
- In **Local nodes**, use a row's share button, or the copy button on the card for all of them.

Nodes that have no link format (for example some sing-box-only types) are skipped and counted in
the message.

## 4. Connect

**Dashboard → Connect.** The status card shows the mode, the node in use and how traffic gets in.

How apps reach BoxPilot (**Dashboard → Quick settings**):

- **System proxy** — the simplest choice. Browsers and most apps follow it. On macOS BoxPilot sets
  it on every network service (Wi-Fi, Ethernet…); if macOS refuses, it asks once for your password.
- **TUN mode** — captures all traffic, including apps that ignore the system proxy. Windows needs
  BoxPilot to run as administrator (it offers to restart that way); macOS asks once for your
  password to allow the core; Linux asks through `pkexec`. On macOS, while TUN is on, the
  system's DNS points at it (so lookups are answered by BoxPilot, not by the router); your own
  DNS settings come back when TUN stops.
- Neither — apps can still use the local proxy at `127.0.0.1:7890` (HTTP and SOCKS5 on one port;
  change it under **Settings → Proxy**).

Modes: **Rule** routes by your rules, **Global** sends everything through the proxy, **Direct**
sends nothing through it. Switching is instant.

## 5. Choose nodes

The **Proxies** page has a tab for every group.

- **Proxy** is the main switch: Auto, a subscription group, or one node.
- **Auto** picks a node for you. Choose how:
  - **Quickest response** — the lowest latency, without hopping for tiny gains.
  - **Stay steady** — keeps a working node and only moves on failure or a big gap.
  - **List order** — the first working node in list order; returns when it recovers.
  - **Most bandwidth** — speed-tests the quickest few now and then and keeps the fastest.
  
  and optionally one **region**. Changing either applies at once without restarting the core.
- **Test latency**, **Real latency** and **Speed test** measure every node shown; the small values on
  each card test that node alone. Nodes are also tested automatically on a timer (**Settings →
  Advanced → Test nodes automatically**).

## 6. Rules

The **Rules** page decides where traffic goes in Rule mode.

- **Add rule-set**: a URL or a local file — sing-box `.srs` / `.json`, Clash rule providers or
  ACL4SSR `.list` files. Pick where matches go (Proxy, Direct, Reject or any group). Order matters:
  use the arrows to move a set up or down.
- **Import ACL4SSR**: reads a subconverter `.ini` (ACL4SSR_Online_Full is one click away) and adds
  all of its rule-sets **switched off**. Turn on the ones you want, or use **Enable all** /
  **Disable all** above the list.
- **Own node group**: the switch under a rule-set gives its matches a group of their own on the
  Proxies page, named after the rule-set (its target then reads **Custom**). In that group choose
  **Manual** and click a node, or one of Auto's four modes with an optional region — for example
  streaming through a fast node in one country while everything else uses Auto.
- **Edit file**: the pencil-on-page button opens the rule-set file in your default text editor.
  For a local file, each save is converted and applied automatically. For a downloaded set, the
  stored copy is opened; saves apply at once and last until its next download. Binary `.srs` files
  can't be edited as text.
- **Built-in rules** bypass mainland China and block ads; **Custom rules** take lines like
  `DOMAIN-SUFFIX,openai.com,proxy`.

## 7. Settings worth knowing

- **General**: language, theme, **Theme colour**, launch at login, close to tray and
  **Lightweight mode** (closing the window frees the UI; the proxy keeps running in the tray).
- **Graphics**: **Translucent window** with **Material** (Mica, Mica Alt, Acrylic on Windows 11) and
  **Transparency**. macOS and KDE Plasma use the system blur.
- **Proxy / TUN / DNS**: ports, LAN access, UDP and QUIC handling, TUN stack, DNS servers, FakeIP.
- **Updates**: how BoxPilot handles new versions, and **Check now**.
- **Advanced → Reset BoxPilot**: clear settings, subscriptions, rule-sets or caches.

## 8. Troubleshooting

- **Nothing loads after connecting** — open **Logs**. Check that the node answers (**Test latency**
  on the Proxies page) and that System proxy or TUN is on.
- **"System proxy: …" error on macOS** — approve the password prompt; if there was none,
  check that your account is an administrator.
- **TUN fails to start** — on Windows restart BoxPilot as administrator; on macOS turn TUN off and
  on again to get the password prompt.
- **A rule-set shows an error** — the row says why; fix the source (or the edited file) and press
  its update button.
- **Start over** — **Settings → Advanced → Reset BoxPilot**.

Where data lives: `%APPDATA%\BoxPilot` (Windows), `~/Library/Application Support/BoxPilot` (macOS),
`$XDG_CONFIG_HOME/BoxPilot` (Linux).
