# BoxPilot user guide

**English** · [简体中文](usage.zh-CN.md) · [Русский](usage.ru.md) · [Back to README](../README.md)

From installing BoxPilot to routing your apps through your nodes the way you want.

1. [Install on your system](#1-install-on-your-system):
   [Windows](#windows) · [macOS](#macos) · [Ubuntu, Debian](#ubuntu-debian-linux-mint) ·
   [Fedora, openSUSE](#fedora-opensuse) · [Arch](#arch-linux-manjaro-endeavouros) · [NixOS](#nixos) ·
   [Other Linux](#other-linux-distributions)
2. [Get the sing-box core](#2-get-the-sing-box-core)
3. [Add your nodes](#3-add-your-nodes)
4. [Connect](#4-connect)
5. [Choose nodes](#5-choose-nodes)
6. [Rules](#6-rules)
7. [Settings worth knowing](#7-settings-worth-knowing)
8. [Linux desktops](#8-linux-desktops)
9. [Troubleshooting](#9-troubleshooting)
10. [Where your data lives](#10-where-your-data-lives)

## 1. Install on your system

Download files from the [latest release](https://github.com/BillBlueInnit/boxpilot/releases/latest).

### Windows

Windows 10 or 11, 64-bit.

1. Download `BoxPilot-<version>-windows-x64.zip` and unzip it anywhere, for example `C:\Apps\BoxPilot`.
2. Run `boxpilot.exe`. If SmartScreen says "Windows protected your PC", click **More info** →
   **Run anyway** (the app isn't code-signed).

- **Portable:** create a folder named `data` next to `boxpilot.exe` before the first start, and
  settings stay beside the app instead of in `%APPDATA%\BoxPilot`.
- **TUN mode** needs administrator rights; BoxPilot offers to restart that way. To always start
  elevated, turn on **Settings → General → Run as administrator**: launch at login then uses a
  scheduled task, without a UAC prompt.
- **Translucent window:** Mica, Mica Alt or Acrylic on Windows 11 22H2 or newer.
- **Updates:** BoxPilot updates itself (**Settings → Updates**).
- **Remove:** turn off **Launch at login**, quit from the tray, then delete the folder and
  `%APPDATA%\BoxPilot`.

### macOS

macOS on Apple silicon (M1 and later) or Intel.

1. Download `BoxPilot-<version>-macos-arm64.dmg` (Apple silicon) or `-macos-x64.dmg` (Intel).
2. Open it and drag **BoxPilot** onto **Applications**.

BoxPilot isn't notarized by Apple, so the first launch needs one extra step:

- Right-click BoxPilot in Applications → **Open** → **Open**.
- On macOS 15 or newer: open it once, then go to **System Settings → Privacy & Security** and click
  **Open Anyway**.
- If macOS says the app "is damaged", run this once in Terminal and open it again:

  ```bash
  xattr -dr com.apple.quarantine /Applications/BoxPilot.app
  ```

Then:

- **System proxy** is set on every network service (Wi-Fi, Ethernet…). If macOS refuses, BoxPilot
  asks once for your password.
- **TUN mode** asks once for your password to allow the core, and again after a core update. While
  TUN is on, the system's DNS points at it; your own DNS settings come back when TUN stops.
- **Updates:** BoxPilot updates itself (**Settings → Updates**).
- **Remove:** turn off **Launch at login**, quit, move BoxPilot to the Trash and delete
  `~/Library/Application Support/BoxPilot`.

### Ubuntu, Debian, Linux Mint

Ubuntu 24.04, Debian 13, Linux Mint 22 or newer (64-bit).

```bash
sudo apt install ./boxpilot_<version>-1_amd64.deb
```

BoxPilot appears in your app menu.

- **Tray icon:** Ubuntu, KDE Plasma, Cinnamon and Xfce show it as is. Debian's GNOME needs the
  AppIndicator extension: `sudo apt install gnome-shell-extension-appindicator`, log out and in, and
  enable it in **Extensions**.
- **TUN mode** asks once for your password (see [Linux desktops](#tun-permission)).
- **Updates:** BoxPilot tells you when a new version is out; install the new `.deb` the same way.
- **Remove:** `sudo apt remove boxpilot`. Your data stays in `~/.config/BoxPilot`.

### Fedora, openSUSE

Fedora 40 or newer, or openSUSE Tumbleweed (64-bit).

```bash
sudo dnf install ./boxpilot-<version>-1.x86_64.rpm      # Fedora
sudo zypper install ./boxpilot-<version>-1.x86_64.rpm   # openSUSE
```

If the installer warns that the package isn't signed, accept to continue.

- **Tray icon:** GNOME needs the AppIndicator extension:
  `sudo dnf install gnome-shell-extension-appindicator` (openSUSE: `zypper install`), log out and
  in, and enable it in **Extensions**. KDE Plasma shows it as is.
- **TUN mode** asks once for your password (see [Linux desktops](#tun-permission)).
- **Updates:** BoxPilot tells you when a new version is out; install the new `.rpm` the same way.
- **Remove:** `sudo dnf remove boxpilot` (openSUSE: `sudo zypper remove boxpilot`).

### Arch Linux, Manjaro, EndeavourOS

Every BoxPilot release doubles as a pacman repository. Add it to `/etc/pacman.conf` once:

```ini
[boxpilot]
SigLevel = Optional TrustAll
Server = https://github.com/BillBlueInnit/boxpilot/releases/latest/download
```

The same in one command, followed by the install:

```bash
printf '\n[boxpilot]\nSigLevel = Optional TrustAll\nServer = https://github.com/BillBlueInnit/boxpilot/releases/latest/download\n' | sudo tee -a /etc/pacman.conf
sudo pacman -Syu boxpilot-bin
```

`yay -Syu boxpilot-bin` works too. The packages are not signed, so the entry trusts what BoxPilot's
GitHub releases serve.

- **Updates** come with your usual `sudo pacman -Syu` (or `yay`).
- **One-off install** without the repository (no updates):

  ```bash
  curl -LO https://github.com/BillBlueInnit/boxpilot/releases/latest/download/boxpilot-bin-x86_64.pkg.tar.zst
  sudo pacman -U ./boxpilot-bin-x86_64.pkg.tar.zst
  ```

- **Build it yourself:** in a clone of this repository, `cd packaging/aur/boxpilot && makepkg -si`.
- **Tray, TUN, blur:** see [Linux desktops](#8-linux-desktops).
- **Remove:** `sudo pacman -Rns boxpilot-bin`, then delete the `[boxpilot]` lines from
  `/etc/pacman.conf`.

### NixOS

There is no Nix package yet, and these steps are untested; corrections are welcome. The Linux
archive runs through [nix-ld](https://github.com/nix-community/nix-ld), which gives it the usual
library paths. In `configuration.nix`:

```nix
programs.nix-ld = {
  enable = true;
  libraries = with pkgs; [
    stdenv.cc.cc.lib glib gtk3 gdk-pixbuf libxkbcommon xorg.libxcb xorg.libX11
    wayland vulkan-loader fontconfig freetype libayatana-appindicator
  ];
};
hardware.graphics.enable = true;               # Vulkan drivers (hardware.opengl before 24.11)
environment.systemPackages = [ pkgs.libcap ];  # setcap, for TUN mode
```

After `sudo nixos-rebuild switch`, unpack the archive and run it:

```bash
mkdir -p ~/BoxPilot && tar -xzf BoxPilot-<version>-linux-x64.tar.gz -C ~/BoxPilot
~/BoxPilot/boxpilot
```

- **TUN mode** asks for your password through polkit, or through BoxPilot's own dialog and `sudo`.
  Releases after 1.0.3 find NixOS's `sudo` and `pkexec`.
- **Updates:** BoxPilot updates the archive copy itself (**Settings → Updates**).

### Other Linux distributions

Any 64-bit distribution with glibc 2.39 or newer, GTK 3 and a Vulkan driver. The tray icon also
needs `libayatana-appindicator3`. Install into your home folder:

```bash
mkdir -p ~/.local/opt/boxpilot ~/.local/bin
tar -xzf BoxPilot-<version>-linux-x64.tar.gz -C ~/.local/opt/boxpilot
ln -sf ~/.local/opt/boxpilot/boxpilot ~/.local/bin/boxpilot
cp -r ~/.local/opt/boxpilot/share/. ~/.local/share/     # app menu entry and icons
```

- **Updates:** BoxPilot updates this copy itself (**Settings → Updates**).
- **Remove:** delete `~/.local/opt/boxpilot`, `~/.local/bin/boxpilot`, the BoxPilot files copied
  into `~/.local/share` (`applications/boxpilot.desktop`, `icons/hicolor/*/apps/boxpilot.png`,
  `metainfo/io.github.billblueinnit.boxpilot.metainfo.xml`) and `~/.config/BoxPilot`.

## 2. Get the sing-box core

BoxPilot drives the [sing-box](https://sing-box.sagernet.org) core but doesn't ship it. On the
**Dashboard** click **Download core** (or **Settings → sing-box core → Download latest**), and
BoxPilot fetches the official build for your system from GitHub.

If GitHub is slow where you are, set a **GitHub mirror** prefix in the same section first.

## 3. Add your nodes

### A subscription

**Subscriptions → Add → Add subscription**, paste the URL and click **Add**. Or copy the URL and
press **Paste**: BoxPilot recognises what's on the clipboard. Share links, base64 lists, Clash /
Clash.Meta YAML and sing-box JSON all work.

Each subscription card can be switched off, updated now, edited (update interval, update on start,
User-Agent, include / exclude filters) or deleted. **Select** acts on several at once.

### Single nodes

For a node you got on its own (`ss://`, `vmess://`, `vless://`, `trojan://`, `hysteria2://`,
`hysteria://`, `tuic://`, `anytls://`, `socks5://`, `http(s)://`, `wireguard://`, `ssh://`):

1. **Subscriptions → Add → Add node**, or copy the link and press **Paste**.
2. Paste one or more links, one per line, and click **Add**.

They are kept in the **Local nodes** card and appear as a **Local nodes** group on the Proxies page.
The switch on the card turns the whole group on or off.

### Share a node

| Where | How |
| --- | --- |
| Proxies page | Hover a node card and click the share icon next to its name |
| Subscription card | The share button copies the links of all its nodes |
| Local nodes | A row's share button, or the copy button on the card for all of them |

The link goes to the clipboard. Nodes with no link format (some sing-box-only types) are skipped
and counted in the message.

## 4. Connect

**Dashboard → Connect.** The status card shows the mode, the node in use and how traffic gets in.

Under **Dashboard → Quick settings**, choose how apps reach BoxPilot:

| Option | What it does |
| --- | --- |
| **System proxy** | The simplest. Browsers and most apps follow it. |
| **TUN mode** | Captures all traffic, including apps that ignore the system proxy. |
| Neither | Apps can still use the local proxy at `127.0.0.1:7890` (HTTP and SOCKS5 on one port, set under **Settings → Proxy**). |

TUN needs one-time permission:

| System | What happens |
| --- | --- |
| Windows | BoxPilot restarts as administrator. |
| macOS | One password prompt; again after a core update. |
| Linux | One password prompt (polkit, or BoxPilot's own dialog); again after a core update. |

On Linux, the system proxy is GNOME's and KDE's setting. Window managers without one (niri, sway,
Hyprland…) leave many apps unaware of it there: use TUN mode, or point the app at `127.0.0.1:7890`.

**Modes:** **Rule** routes by your rules, **Global** sends everything through the proxy, **Direct**
sends nothing through it. Switching is instant.

## 5. Choose nodes

The **Proxies** page has a tab for every group.

- **Proxy** is the main switch: Auto, a subscription group, or one node.
- **Auto** picks a node for you, optionally within one **region**:

  | Mode | Picks |
  | --- | --- |
  | Quickest response | The lowest latency, without hopping for tiny gains |
  | Stay steady | Keeps a working node; moves only on failure or a big gap |
  | List order | The first working node in list order; returns when it recovers |
  | Most bandwidth | Speed-tests the quickest few now and then and keeps the fastest |

  Changes apply at once, without restarting the core.
- **Test latency**, **Real latency** and **Speed test** measure every node shown; the small values on
  a card test that node alone. Nodes are also tested on a timer (**Settings → Advanced → Test nodes
  automatically**).

## 6. Rules

The **Rules** page decides where traffic goes in Rule mode.

- **Add rule-set:** a URL or a local file: sing-box `.srs` / `.json`, Clash rule providers or
  ACL4SSR `.list` files. Choose where matches go (Proxy, Direct, Reject or any group). Order matters:
  the arrows move a set up or down.
- **Import ACL4SSR:** reads a subconverter `.ini` (ACL4SSR_Online_Full is one click away) and adds
  all of its rule-sets **switched off**. Turn on the ones you want, or use **Enable all** /
  **Disable all**.
- **Own node group:** the switch under a rule-set gives its matches their own group on the Proxies
  page. There, choose **Manual** and click a node, or one of Auto's modes with an optional region.
  For example: streaming through a fast node in one country, everything else through Auto.
- **Edit file:** the pencil-on-page button opens the rule-set file in your text editor. A local
  file is converted and applied on every save. For a downloaded set, the stored copy opens; saves
  apply at once and last until its next download. Binary `.srs` files can't be edited as text.
- **Built-in rules** bypass mainland China and block ads. **Custom rules** take lines like
  `DOMAIN-SUFFIX,openai.com,proxy`.

## 7. Settings worth knowing

| Section | What's there |
| --- | --- |
| General | Language, theme, **Theme colour**, **Launch at login** (optionally **Start minimized**), **Close to tray**, **Lightweight mode** (closing the window frees the interface; the proxy keeps running in the tray) |
| Graphics | **Translucent window**, **Material** (Windows) and **Transparency**; **Hardware acceleration** |
| Proxy, TUN, DNS | Ports, LAN access, UDP and QUIC handling, TUN stack, DNS servers, FakeIP |
| Updates | What happens when a new version is out; **Check now** |
| Advanced | **Reset BoxPilot**: clear settings, subscriptions, rule-sets or caches |

## 8. Linux desktops

### Tray icon

BoxPilot's tray icon is an AppIndicator (StatusNotifier) item.

| Desktop | What you need |
| --- | --- |
| Ubuntu, KDE Plasma, Cinnamon, Xfce | Nothing |
| GNOME elsewhere (Fedora, Debian, Arch…) | The AppIndicator extension (`gnome-shell-extension-appindicator`) |
| niri, sway, Hyprland, other window managers | A bar with a tray, such as Waybar |

The packages bring the AppIndicator library along. The archive needs `libayatana-appindicator3`.

### TUN permission

TUN mode needs the core to have `CAP_NET_ADMIN`. The first time, BoxPilot asks for your password
through polkit and grants it with `setcap` (or makes the core setuid root where `setcap` isn't
installed). After a core update it asks again.

Window managers often run no polkit agent to show that prompt. BoxPilot then shows its own
**Administrator rights required** dialog: enter your password and click **Grant**, and it is passed to
`sudo` once and not stored. The dialog also shows the same command to copy into a terminal.

### Translucent window and blur

BoxPilot only turns the window translucent where the compositor can also blur behind it:

| Compositor | Translucent, blurred window |
| --- | --- |
| KDE Plasma (Wayland) | Yes |
| niri | Yes |
| Other Wayland compositors | Yes if they offer `ext-background-effect` or KDE's blur |
| GNOME, sway, any X11 session | No, opaque |

The `compositor blur:` line in `boxpilot.log` shows what BoxPilot found.

### Tiling compositors

On niri, sway, Hyprland, river and i3 the compositor draws window borders and shadows, and
BoxPilot leaves that to it. `BOXPILOT_DECORATIONS=client` (BoxPilot's frame) or `server` (the
compositor's) overrides the choice.

On niri, this window rule in `~/.config/niri/config.kdl` drops niri's border and shadow around
BoxPilot and makes its blur show the windows behind it, not only the wallpaper (niri's "xray"):

```kdl
window-rule {
    match app-id="^BoxPilot$"
    shadow { off; }
    border { off; }
    draw-border-with-background false
    background-effect {
        xray false
    }
}
```

### Laptops with two GPUs

If the GPU driving your screen (usually Intel or AMD) has no Vulkan driver, BoxPilot is drawn on
the other GPU and every frame is copied across, which makes scrolling stutter. Install the driver:

| System | Package |
| --- | --- |
| Arch | `vulkan-intel` (Intel) or `vulkan-radeon` (AMD) |
| Debian, Ubuntu, Fedora | `mesa-vulkan-drivers` |

**Settings → Graphics → Hardware acceleration** warns when this applies, and `boxpilot.log` names the
GPU in use (`Selected GPU`).

## 9. Troubleshooting

- **Nothing loads after connecting:** open **Logs**. Check that the node answers (**Test latency**
  on the Proxies page) and that System proxy or TUN is on.
- **The system proxy has no effect on Linux:** see the note under [Connect](#4-connect); use TUN mode.
- **"System proxy: …" error on macOS:** approve the password prompt. If there was none, check that
  your account is an administrator.
- **TUN fails to start:**
  - Windows: restart BoxPilot as administrator.
  - macOS: turn TUN off and on again to get the password prompt.
  - Linux: turn TUN off and on again, and enter your password in the polkit prompt or BoxPilot's
    dialog. A "mounted nosuid" error means your home folder doesn't allow the permission.
- **No tray icon on Linux:** see [Tray icon](#tray-icon).
- **Scrolling stutters on a laptop:** see [Laptops with two GPUs](#laptops-with-two-gpus).
- **A rule-set shows an error:** the row says why. Fix the source (or the edited file) and press
  its update button.
- **Start over:** **Settings → Advanced → Reset BoxPilot**.

## 10. Where your data lives

| System | Settings, subscriptions, core, log | Launch at login |
| --- | --- | --- |
| Windows | `%APPDATA%\BoxPilot`, or `data` next to `boxpilot.exe` | Run key or scheduled task named `BoxPilot` |
| macOS | `~/Library/Application Support/BoxPilot` | `~/Library/LaunchAgents/app.boxpilot.BoxPilot.plist` |
| Linux | `~/.config/BoxPilot` (`$XDG_CONFIG_HOME/BoxPilot`) | `~/.config/autostart/boxpilot.desktop` |

The log is `boxpilot.log` in the data folder; the folder button on the **Logs** page opens it.
