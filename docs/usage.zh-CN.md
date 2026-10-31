# BoxPilot 使用说明

[English](usage.md) · **简体中文** · [Русский](usage.ru.md) · [返回 README](../README.zh-CN.md)

从安装 BoxPilot 开始，到按你的想法让各个应用经由节点上网。

1. [在你的系统上安装](#1-在你的系统上安装)：
   [Windows](#windows) · [macOS](#macos) · [Ubuntu、Debian](#ubuntudebianlinux-mint) ·
   [Fedora、openSUSE](#fedoraopensuse) · [Arch](#arch-linuxmanjaroendeavouros) · [NixOS](#nixos) ·
   [其他 Linux](#其他-linux-发行版)
2. [获取 sing-box 内核](#2-获取-sing-box-内核)
3. [添加节点](#3-添加节点)
4. [连接](#4-连接)
5. [选择节点](#5-选择节点)
6. [规则](#6-规则)
7. [值得了解的设置](#7-值得了解的设置)
8. [Linux 桌面](#8-linux-桌面)
9. [常见问题](#9-常见问题)
10. [数据位置](#10-数据位置)

## 1. 在你的系统上安装

安装文件在[最新发布页](https://github.com/BillBlueInnit/boxpilot/releases/latest)。

### Windows

Windows 10 或 11，64 位。

1. 下载 `BoxPilot-<版本>-windows-x64.zip`，解压到任意位置，例如 `C:\Apps\BoxPilot`。
2. 运行 `boxpilot.exe`。如果 SmartScreen 提示“Windows 已保护你的电脑”，点击 **更多信息** → **仍要运行**
   （应用没有代码签名）。

- **便携模式：** 首次启动前在 `boxpilot.exe` 旁边建立名为 `data` 的文件夹，设置就会保存在应用旁边，
  而不是 `%APPDATA%\BoxPilot`。
- **TUN 模式**需要管理员权限，BoxPilot 会提示以管理员身份重启。想一直以管理员身份启动，可开启
  **设置 → 常规 → 以管理员身份运行**：开机启动会改用计划任务，无需 UAC 确认。
- **半透明窗口：** Windows 11 22H2 及以上可用 Mica、Mica Alt 或 Acrylic。
- **更新：** BoxPilot 会自行更新（**设置 → 更新**）。
- **卸载：** 先关闭 **开机启动**，从托盘退出，然后删除应用文件夹和 `%APPDATA%\BoxPilot`。

### macOS

Apple 芯片（M1 及以后）或 Intel 的 Mac。

1. 下载 `BoxPilot-<版本>-macos-arm64.dmg`（Apple 芯片）或 `-macos-x64.dmg`（Intel）。
2. 打开它，把 **BoxPilot** 拖到 **应用程序**。

BoxPilot 没有经过 Apple 公证，所以第一次打开需要多一步：

- 在“应用程序”中右键点击 BoxPilot → **打开** → **打开**。
- macOS 15 及以上：先打开一次，然后前往 **系统设置 → 隐私与安全性**，点击 **仍要打开**。
- 如果提示应用“已损坏”，在终端运行一次下面的命令后再打开：

  ```bash
  xattr -dr com.apple.quarantine /Applications/BoxPilot.app
  ```

之后：

- **系统代理**会设置到每个网络服务（Wi-Fi、以太网……）。如果 macOS 拒绝，BoxPilot 会请求一次你的密码。
- **TUN 模式**首次会请求一次密码为内核授权，更新内核后需再授权一次。TUN 开启期间系统 DNS 指向它，
  关闭 TUN 后恢复你原来的 DNS 设置。
- **更新：** BoxPilot 会自行更新（**设置 → 更新**）。
- **卸载：** 先关闭 **开机启动**，退出应用，把 BoxPilot 移到废纸篓，并删除
  `~/Library/Application Support/BoxPilot`。

### Ubuntu、Debian、Linux Mint

Ubuntu 24.04、Debian 13、Linux Mint 22 或更新版本（64 位）。

```bash
sudo apt install ./boxpilot_<版本>-1_amd64.deb
```

安装后 BoxPilot 会出现在应用菜单中。

- **托盘图标：** Ubuntu、KDE Plasma、Cinnamon 和 Xfce 直接可用。Debian 的 GNOME 需要 AppIndicator 扩展：
  `sudo apt install gnome-shell-extension-appindicator`，注销后重新登录，在 **扩展** 中启用。
- **TUN 模式**首次会请求一次密码（见 [Linux 桌面](#tun-权限)）。
- **更新：** 有新版本时 BoxPilot 会提醒你，按同样方式安装新的 `.deb` 即可。
- **卸载：** `sudo apt remove boxpilot`。你的数据保留在 `~/.config/BoxPilot`。

### Fedora、openSUSE

Fedora 40 或更新版本，或 openSUSE Tumbleweed（64 位）。

```bash
sudo dnf install ./boxpilot-<版本>-1.x86_64.rpm      # Fedora
sudo zypper install ./boxpilot-<版本>-1.x86_64.rpm   # openSUSE
```

如果安装程序提示包没有签名，选择继续即可。

- **托盘图标：** GNOME 需要 AppIndicator 扩展：`sudo dnf install gnome-shell-extension-appindicator`
  （openSUSE 用 `zypper install`），注销后重新登录，在 **扩展** 中启用。KDE Plasma 直接可用。
- **TUN 模式**首次会请求一次密码（见 [Linux 桌面](#tun-权限)）。
- **更新：** 有新版本时 BoxPilot 会提醒你，按同样方式安装新的 `.rpm` 即可。
- **卸载：** `sudo dnf remove boxpilot`（openSUSE：`sudo zypper remove boxpilot`）。

### Arch Linux、Manjaro、EndeavourOS

BoxPilot 的每个发布本身就是一个 pacman 仓库。在 `/etc/pacman.conf` 中添加一次：

```ini
[boxpilot]
SigLevel = Optional TrustAll
Server = https://github.com/BillBlueInnit/boxpilot/releases/latest/download
```

也可以用一条命令添加，然后安装：

```bash
printf '\n[boxpilot]\nSigLevel = Optional TrustAll\nServer = https://github.com/BillBlueInnit/boxpilot/releases/latest/download\n' | sudo tee -a /etc/pacman.conf
sudo pacman -Syu boxpilot-bin
```

也可以用 `yay -Syu boxpilot-bin`。这些包没有签名，因此该条目信任 BoxPilot GitHub 发布页提供的文件。

- **更新**会随平常的 `sudo pacman -Syu`（或 `yay`）到来。
- **只安装一次**（不添加仓库，不会自动更新）：

  ```bash
  curl -LO https://github.com/BillBlueInnit/boxpilot/releases/latest/download/boxpilot-bin-x86_64.pkg.tar.zst
  sudo pacman -U ./boxpilot-bin-x86_64.pkg.tar.zst
  ```

- **自己构建：** 在本仓库的克隆中运行 `cd packaging/aur/boxpilot && makepkg -si`。
- **托盘、TUN、模糊：** 见 [Linux 桌面](#8-linux-桌面)。
- **卸载：** `sudo pacman -Rns boxpilot-bin`，然后删除 `/etc/pacman.conf` 中的 `[boxpilot]` 几行。

### NixOS

目前还没有 Nix 包，以下步骤也尚未经过测试，欢迎指正。Linux 压缩包可以通过
[nix-ld](https://github.com/nix-community/nix-ld) 运行，它提供常规的库路径。在 `configuration.nix` 中：

```nix
programs.nix-ld = {
  enable = true;
  libraries = with pkgs; [
    stdenv.cc.cc.lib glib gtk3 gdk-pixbuf libxkbcommon xorg.libxcb xorg.libX11
    wayland vulkan-loader fontconfig freetype libayatana-appindicator
  ];
};
hardware.graphics.enable = true;               # Vulkan 驱动（24.11 之前为 hardware.opengl）
environment.systemPackages = [ pkgs.libcap ];  # setcap，供 TUN 模式使用
```

执行 `sudo nixos-rebuild switch` 后，解压并运行：

```bash
mkdir -p ~/BoxPilot && tar -xzf BoxPilot-<版本>-linux-x64.tar.gz -C ~/BoxPilot
~/BoxPilot/boxpilot
```

- **TUN 模式**通过 polkit 请求密码，或通过 BoxPilot 自己的对话框交给 `sudo`。1.0.3 之后的版本能找到
  NixOS 的 `sudo` 和 `pkexec`。
- **更新：** BoxPilot 会自行更新这份压缩包副本（**设置 → 更新**）。

### 其他 Linux 发行版

任何 64 位、glibc 2.39 及以上、带 GTK 3 和 Vulkan 驱动的发行版。托盘图标还需要
`libayatana-appindicator3`。安装到你的主目录：

```bash
mkdir -p ~/.local/opt/boxpilot ~/.local/bin
tar -xzf BoxPilot-<版本>-linux-x64.tar.gz -C ~/.local/opt/boxpilot
ln -sf ~/.local/opt/boxpilot/boxpilot ~/.local/bin/boxpilot
cp -r ~/.local/opt/boxpilot/share/. ~/.local/share/     # 应用菜单项和图标
```

- **更新：** BoxPilot 会自行更新这份副本（**设置 → 更新**）。
- **卸载：** 删除 `~/.local/opt/boxpilot`、`~/.local/bin/boxpilot`、复制到 `~/.local/share` 中的 BoxPilot
  文件（`applications/boxpilot.desktop`、`icons/hicolor/*/apps/boxpilot.png`、
  `metainfo/io.github.billblueinnit.boxpilot.metainfo.xml`）以及 `~/.config/BoxPilot`。

## 2. 获取 sing-box 内核

BoxPilot 驱动 [sing-box](https://sing-box.sagernet.org) 内核，但不附带它。在 **仪表盘** 点击 **下载内核**
（或 **设置 → sing-box 内核 → 下载最新版**），BoxPilot 会从 GitHub 下载适合你系统的官方版本。

如果 GitHub 访问较慢，可先在同一栏设置 **GitHub 镜像** 前缀。

## 3. 添加节点

### 订阅

**订阅 → 添加 → 添加订阅**，粘贴订阅链接并点击 **添加**。也可以先复制链接，再点 **粘贴**：BoxPilot 会识别
剪贴板中的内容。分享链接、base64 列表、Clash / Clash.Meta YAML 和 sing-box JSON 都可以。

每张订阅卡片都可以停用、立即更新、编辑（更新间隔、启动时更新、User-Agent、包含 / 排除过滤）或删除。
点 **选择** 可以一次操作多个订阅。

### 单个节点

对于单独得到的节点（`ss://`、`vmess://`、`vless://`、`trojan://`、`hysteria2://`、`hysteria://`、
`tuic://`、`anytls://`、`socks5://`、`http(s)://`、`wireguard://`、`ssh://`）：

1. **订阅 → 添加 → 添加节点**，或复制链接后点 **粘贴**。
2. 粘贴一个或多个链接，每行一个，点击 **添加**。

这些节点集中在 **本地节点** 卡片中，并在代理页作为 **本地节点** 分组出现。卡片上的开关可以整体启用或停用该分组。

### 分享节点

| 位置 | 方法 |
| --- | --- |
| 代理页 | 鼠标移到节点卡片上，点击名称旁的分享图标 |
| 订阅卡片 | 分享按钮复制该订阅全部节点的链接 |
| 本地节点 | 每行的分享按钮，或卡片上的复制按钮复制全部 |

链接会复制到剪贴板。没有链接格式的节点（部分仅 sing-box 支持的类型）会被跳过，并在提示中计数。

## 4. 连接

**仪表盘 → 连接。** 状态卡片会显示当前模式、正在使用的节点以及流量的接入方式。

在 **仪表盘 → 快捷设置** 中选择应用如何接入 BoxPilot：

| 方式 | 作用 |
| --- | --- |
| **系统代理** | 最简单。浏览器和大多数应用都会遵循。 |
| **TUN 模式** | 接管全部流量，包括不遵循系统代理的应用。 |
| 都不开启 | 应用仍可使用本地代理 `127.0.0.1:7890`（同一端口提供 HTTP 和 SOCKS5，可在 **设置 → 代理** 中修改）。 |

TUN 需要一次性授权：

| 系统 | 会发生什么 |
| --- | --- |
| Windows | BoxPilot 以管理员身份重启。 |
| macOS | 请求一次密码；更新内核后再请求一次。 |
| Linux | 请求一次密码（polkit，或 BoxPilot 自己的对话框）；更新内核后再请求一次。 |

在 Linux 上，系统代理指 GNOME 和 KDE 的代理设置。没有这类设置的窗口管理器（niri、sway、Hyprland……）上，
许多应用读不到它：请使用 TUN 模式，或让应用直接使用 `127.0.0.1:7890`。

**模式：** **规则** 按你的规则分流，**全局** 让全部流量走代理，**直连** 完全不走代理。切换即时生效。

## 5. 选择节点

**代理** 页为每个分组提供一个标签。

- **节点选择** 是总开关：自动选择、某个订阅分组或单个节点。
- **自动选择** 替你挑选节点，还可以限定一个 **地区**：

  | 方式 | 选择 |
  | --- | --- |
  | 响应最快 | 延迟最低，不会为微小差距频繁切换 |
  | 稳定优先 | 保留可用节点，只在失败或差距很大时切换 |
  | 按列表顺序 | 按列表顺序第一个可用节点，恢复后自动切回 |
  | 带宽优先 | 不时对最快的几个节点测速，保留速度最快的 |

  修改立即生效，无需重启内核。
- **测试延迟**、**真实延迟** 和 **测速** 会测量当前显示的全部节点；点击卡片上的小数值只测该节点。节点也会
  定时自动测试（**设置 → 高级 → 自动测试节点**）。

## 6. 规则

**规则** 页决定规则模式下流量的去向。

- **添加规则集：** 链接或本地文件：sing-box `.srs` / `.json`、Clash 规则提供者或 ACL4SSR `.list` 文件。选择
  匹配流量的去向（代理、直连、拒绝或任意分组）。顺序很重要：用箭头上下移动。
- **导入 ACL4SSR：** 读取 subconverter `.ini`（一键选用 ACL4SSR_Online_Full），添加其中全部规则集，默认
  **处于关闭状态**。启用你需要的，或使用 **全部启用** / **全部停用**。
- **独立节点组：** 规则集下方的开关会让它的匹配流量在代理页拥有独立的分组。在该分组中选择 **手动选择** 后
  点击节点，或选择自动选择的一种方式并可限定地区。例如让流媒体走某个国家的高速节点，其余流量仍用自动选择。
- **编辑文件：** 带铅笔的文件按钮会用文本编辑器打开规则集文件。本地文件每次保存都会自动转换并生效。下载的
  规则集打开的是本地保存的副本，保存后立即生效，直到下次下载更新。二进制 `.srs` 文件无法以文本编辑。
- **内置规则** 可绕过中国大陆并拦截广告。**自定义规则** 按行填写，例如 `DOMAIN-SUFFIX,openai.com,proxy`。

## 7. 值得了解的设置

| 分区 | 内容 |
| --- | --- |
| 常规 | 语言、主题、**主题色**、**开机启动**（可选 **启动时最小化**）、**关闭时最小化到托盘**、**轻量模式**（关闭窗口即释放界面，代理在托盘中继续运行） |
| 图形 | **半透明窗口**、**材质**（Windows）和 **透明度**；**硬件加速** |
| 代理、TUN、DNS | 端口、局域网访问、UDP 与 QUIC 处理、TUN 协议栈、DNS 服务器、FakeIP |
| 更新 | 有新版本时如何处理；**立即检查** |
| 高级 | **重置 BoxPilot**：清除设置、订阅、规则集或缓存 |

## 8. Linux 桌面

### 托盘图标

BoxPilot 的托盘图标是 AppIndicator（StatusNotifier）项目。

| 桌面 | 需要什么 |
| --- | --- |
| Ubuntu、KDE Plasma、Cinnamon、Xfce | 无需额外操作 |
| 其他发行版的 GNOME（Fedora、Debian、Arch……） | AppIndicator 扩展（`gnome-shell-extension-appindicator`） |
| niri、sway、Hyprland 等窗口管理器 | 带托盘的状态栏，例如 Waybar |

安装包会一并装好 AppIndicator 库。压缩包版本需要 `libayatana-appindicator3`。

### TUN 权限

TUN 模式需要内核拥有 `CAP_NET_ADMIN`。第一次使用时，BoxPilot 通过 polkit 请求你的密码，并用 `setcap` 授权
（没有安装 `setcap` 时改为让内核 setuid root）。更新内核后会再请求一次。

窗口管理器上常常没有运行能显示该提示的 polkit 代理。此时 BoxPilot 会显示自己的 **需要管理员权限** 对话框：
输入密码并点击 **授权**，密码只会交给 `sudo` 使用一次，不会被保存。对话框中也给出了可复制到终端执行的同一条命令。

### 半透明窗口与模糊

只有合成器能在窗口后面模糊时，BoxPilot 才会让窗口半透明：

| 合成器 | 半透明模糊窗口 |
| --- | --- |
| KDE Plasma（Wayland） | 支持 |
| niri | 支持 |
| 其他 Wayland 合成器 | 提供 `ext-background-effect` 或 KDE 模糊协议时支持 |
| GNOME、sway、任何 X11 会话 | 不支持，窗口不透明 |

`boxpilot.log` 中的 `compositor blur:` 一行显示 BoxPilot 检测到的结果。

### 平铺式合成器

在 niri、sway、Hyprland、river 和 i3 上，窗口边框和阴影由合成器绘制，BoxPilot 交给合成器处理。
`BOXPILOT_DECORATIONS=client`（BoxPilot 的边框）或 `server`（合成器的边框）可以覆盖这一选择。

在 niri 上，把下面的窗口规则加到 `~/.config/niri/config.kdl`，可以去掉 niri 给 BoxPilot 加的边框和阴影，并让
模糊显示窗口后面的其他窗口，而不只是壁纸（niri 的 “xray”）：

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

### 双显卡笔记本

如果驱动屏幕的显卡（通常是 Intel 或 AMD）没有 Vulkan 驱动，BoxPilot 就会由另一块显卡绘制，每一帧都要跨显卡复制，
滚动会卡顿。请安装驱动：

| 系统 | 软件包 |
| --- | --- |
| Arch | `vulkan-intel`（Intel）或 `vulkan-radeon`（AMD） |
| Debian、Ubuntu、Fedora | `mesa-vulkan-drivers` |

遇到这种情况时 **设置 → 图形 → 硬件加速** 会给出提示，`boxpilot.log` 也会写明正在使用的显卡（`Selected GPU`）。

## 9. 常见问题

- **连接后打不开网页：** 打开 **日志** 查看。确认节点有响应（在代理页点 **测试延迟**），并确认系统代理或 TUN 已开启。
- **Linux 上系统代理不起作用：** 见[连接](#4-连接)一节的说明，请使用 TUN 模式。
- **macOS 上出现“系统代理：……”错误：** 在弹出的密码框中授权。如果没有弹出，请确认你的账户是管理员。
- **TUN 无法启动：**
  - Windows：以管理员身份重启 BoxPilot。
  - macOS：关闭再开启 TUN 以弹出密码授权。
  - Linux：关闭再开启 TUN，在 polkit 提示或 BoxPilot 的对话框中输入密码。出现 “mounted nosuid” 错误说明你的
    主目录所在的文件系统不允许这种授权。
- **Linux 上没有托盘图标：** 见[托盘图标](#托盘图标)。
- **笔记本上滚动卡顿：** 见[双显卡笔记本](#双显卡笔记本)。
- **规则集显示错误：** 该行会说明原因。修正来源（或编辑过的文件）后点它的更新按钮。
- **从头开始：** **设置 → 高级 → 重置 BoxPilot**。

## 10. 数据位置

| 系统 | 设置、订阅、内核、日志 | 开机启动项 |
| --- | --- | --- |
| Windows | `%APPDATA%\BoxPilot`，或 `boxpilot.exe` 旁的 `data` | 名为 `BoxPilot` 的 Run 注册表项或计划任务 |
| macOS | `~/Library/Application Support/BoxPilot` | `~/Library/LaunchAgents/app.boxpilot.BoxPilot.plist` |
| Linux | `~/.config/BoxPilot`（`$XDG_CONFIG_HOME/BoxPilot`） | `~/.config/autostart/boxpilot.desktop` |

日志文件是数据文件夹中的 `boxpilot.log`；**日志** 页上的文件夹按钮可以打开它。
