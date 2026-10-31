# BoxPilot

[English](README.md) · **简体中文** · [Русский](README.ru.md)

一款简洁的桌面代理客户端，支持 Windows、macOS 和 Linux。它驱动 [sing-box](https://sing-box.sagernet.org)
内核，自带订阅转换，使用 Rust 与 [GPUI](https://gpui.rs)（通过 gpui-kit）编写。

[![CI](https://github.com/BillBlueInnit/boxpilot/actions/workflows/ci.yml/badge.svg)](https://github.com/BillBlueInnit/boxpilot/actions/workflows/ci.yml)

**[下载](https://github.com/BillBlueInnit/boxpilot/releases/latest)** ·
**[使用说明](docs/usage.zh-CN.md)** ·
[从源码构建](#从源码构建)

- [安装](#安装)
- [快速开始](#快速开始)
- [功能](#功能)
- [平台支持](#平台支持)
- [从源码构建](#从源码构建)
- [目录结构](#目录结构)

## 安装

安装文件在[最新发布页](https://github.com/BillBlueInnit/boxpilot/releases/latest)。使用说明中每个系统都有
单独一节，介绍首次打开、更新和卸载。

| 系统 | 安装 | 说明 |
| --- | --- | --- |
| Windows 10 / 11 | 解压 `BoxPilot-<版本>-windows-x64.zip`，运行 `boxpilot.exe` | [Windows](docs/usage.zh-CN.md#windows) |
| macOS（Apple 芯片 / Intel） | 打开 `BoxPilot-<版本>-macos-arm64.dmg` / `-macos-x64.dmg`，把 BoxPilot 拖进“应用程序” | [macOS](docs/usage.zh-CN.md#macos) |
| Ubuntu、Debian、Linux Mint | `sudo apt install ./boxpilot_<版本>-1_amd64.deb` | [Ubuntu、Debian](docs/usage.zh-CN.md#ubuntudebianlinux-mint) |
| Fedora、openSUSE | `sudo dnf install ./boxpilot-<版本>-1.x86_64.rpm`（openSUSE 用 `zypper`） | [Fedora、openSUSE](docs/usage.zh-CN.md#fedoraopensuse) |
| Arch、Manjaro、EndeavourOS | 从 BoxPilot 的仓库 `sudo pacman -Syu boxpilot-bin`，见下文 | [Arch](docs/usage.zh-CN.md#arch-linuxmanjaroendeavouros) |
| NixOS | 通过 `nix-ld` 运行 Linux 压缩包 | [NixOS](docs/usage.zh-CN.md#nixos) |
| 其他 Linux（x64） | 解压 `BoxPilot-<版本>-linux-x64.tar.gz`，运行 `./boxpilot` | [其他 Linux](docs/usage.zh-CN.md#其他-linux-发行版) |

Linux 安装包需要 glibc 2.39 及以上：Ubuntu 24.04、Debian 13、Fedora 40、openSUSE Tumbleweed 或更新版本。

**Arch Linux。** 每个发布本身就是一个 pacman 仓库。添加一次后安装，之后的新版本会随平常的
`pacman -Syu`（或 `yay`）更新：

```bash
printf '\n[boxpilot]\nSigLevel = Optional TrustAll\nServer = https://github.com/BillBlueInnit/boxpilot/releases/latest/download\n' | sudo tee -a /etc/pacman.conf
sudo pacman -Syu boxpilot-bin
```

这些包没有签名，因此该仓库条目信任本项目 GitHub 发布页提供的文件。

## 快速开始

1. **获取内核。** 仪表盘 → **下载内核**。BoxPilot 会下载适合你系统的 sing-box 官方版本（GitHub 访问较慢时，
   可在 设置 → sing-box 内核 中设置 **GitHub 镜像**）。
2. **添加节点。** 订阅 → **添加**，粘贴服务商提供的订阅链接，或点 **粘贴** 从剪贴板导入。
3. **连接。** 仪表盘 → **连接**，然后在快捷设置中开启 **系统代理** 或 **TUN 模式**。

完整的分步说明见[使用说明](docs/usage.zh-CN.md)。

## 功能

**订阅与节点**

- 格式：v2ray / Shadowrocket 分享链接（明文或 base64）、Clash / Clash.Meta YAML、sing-box JSON。
- 协议：VLESS（REALITY、Vision）、VMess、Trojan、Shadowsocks（2022、obfs、v2ray-plugin）、
  Hysteria 2（端口跳跃）、Hysteria、TUIC、AnyTLS、WireGuard、SOCKS、HTTP(S) 和 SSH，传输层支持 ws、grpc、
  h2、httpupgrade 和 quic。不支持的条目（SSR、xhttp 等）会记录在日志中并跳过。
- 更新：每个订阅独立的更新间隔、启动时更新、补上应用关闭期间错过的更新、失败后快速重试、代理 → 直连回退、
  自定义 User-Agent、包含 / 排除过滤、从 `subscription-userinfo` 读取流量与到期时间。一批更新只重载一次内核。
- 本地节点：通过分享链接单独添加节点；任何节点都可以复制为分享链接。

**分流**

- 规则、全局、直连三种模式，实时切换。
- 每个订阅都有一个选择组和一个**自动选择**组。自动选择可以挑响应最快的、稳定的、按列表顺序第一个可用的、
  或带宽最大的节点，还可以限定在一个地区。切换不会中断已有连接。
- 规则集可来自链接或文件：sing-box `.srs` / `.json`、Clash 规则提供者和 ACL4SSR `.list`。每个规则集可以发往
  代理、直连、拒绝或任意分组，按你排列的顺序生效，并按各自的间隔更新。
- **导入 ACL4SSR** 读取 subconverter `.ini`（例如 ACL4SSR_Online_Full），创建其中全部规则集，默认关闭，由你挑选启用。
- 规则集可以拥有独立的节点组，手动或自动选择节点；规则集文件可在文本编辑器中打开，保存后自动生效。
- 内置绕过中国大陆与广告拦截，以及自定义规则（`类型,值,目标`）。
- DNS：HTTPS、TLS、QUIC 或 UDP，FakeIP 与 IPv6。规则集缓存在本地，GitHub 被墙或节点失效都不会让内核无法启动。

**连接**

- 系统代理（退出和崩溃后自动恢复）、TUN 模式、局域网共享、UDP 走代理或直连，以及可选的 QUIC 阻断。
- 延迟、真实延迟（经节点发起一次全新的 HTTPS 请求）和下载测速，通过独立的探针进行，不会改变你的选择。
- 每个节点都显示国家 / 地区代码：先看节点名称，其次 GeoIP 查询，最后检测出口 IP。
- 设置改动合并为一次内核重载；内核崩溃会自动重启。

**应用**

- English、简体中文、繁體中文和 Русский，跟随系统语言，可实时切换。
- 托盘菜单可切换模式、系统代理和 TUN；关闭到托盘；**轻量模式**在代理继续运行时释放界面。
- 开机启动（可最小化）、实时流量图、连接列表、日志查看。
- 浅色 / 深色主题、主题色，以及系统支持时的半透明模糊窗口（见[平台支持](#平台支持)）。
- 从 GitHub Releases 更新：自动安装、仅提醒或从不通知。
- 所有过渡都是阻尼弹簧动效，并遵循“减少动态效果”设置。

## 平台支持

GPUI 支持的平台上功能一致。各系统不同之处，分别采用其原生机制：

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 安装包 | zip | 含 BoxPilot.app 的 `.dmg` | `.deb`、`.rpm`、Arch 仓库、tar.gz |
| 窗口边框 | 原生边框、贴靠布局 | 原生红绿灯按钮 | BoxPilot 自绘（Adwaita 或 Breeze 风格按钮），平铺式合成器上由合成器绘制 |
| 半透明窗口 | Mica、Mica Alt、Acrylic（Windows 11 22H2+） | 系统材质模糊 | 合成器提供模糊时可用（niri、KDE Plasma） |
| 系统代理 | WinINet | `networksetup`，覆盖每个网络服务 | GNOME `gsettings` 与 KDE `kioslaverc` |
| TUN 权限 | 以管理员身份运行 | 一次密码授权后内核设为 setuid root | 一次密码授权后为内核授予 `CAP_NET_ADMIN` |
| 开机启动 | Run 注册表项 / 计划任务 | LaunchAgent | XDG autostart |
| 应用崩溃后清理内核 | 作业对象 | 下次启动时 | 父进程退出信号，以及下次启动时 |
| 数据文件夹 | `%APPDATA%\BoxPilot` | `~/Library/Application Support/BoxPilot` | `~/.config/BoxPilot` |

Linux 桌面的细节（托盘图标、没有 polkit 代理时的 TUN、模糊、平铺式合成器、双显卡笔记本）见
[使用说明](docs/usage.zh-CN.md#8-linux-桌面)。

## 从源码构建

需要 stable 版 Rust（[rustup](https://rustup.rs)）。在 Linux 上还需要 GPUI 依赖的开发包。Debian 或 Ubuntu
上 CI 安装的是：

```bash
sudo apt install build-essential pkg-config clang libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev \
  libpipewire-0.3-dev libgtk-3-dev libayatana-appindicator3-dev libxdo-dev
```

Arch 上所需的包列在 [`boxpilot` 打包脚本](packaging/aur/boxpilot/PKGBUILD)中。

运行：

```bash
cargo run --release -p boxpilot
```

**测试。** `cargo test --workspace` 运行单元测试。用真实内核校验生成的配置：

```bash
cargo run -p boxpilot-core --example gen_config -- out
sing-box check -c out/everything-1.14.json
```

生成的配置面向 sing-box 1.12 及以上（推荐 1.14）。

**Linux 安装包。** 先 `cargo install cargo-deb cargo-generate-rpm`，再运行
`packaging/linux/build.sh [tar] [deb] [rpm] [arch]`，会在 `dist/` 中生成压缩包、`.deb`、`.rpm`，在 Arch 上还有
pacman 包和仓库。在滚动更新的发行版上构建的包需要该发行版的 glibc；发布用的包在 Ubuntu 24.04 上构建。
AUR 打包脚本位于 [`packaging/aur`](packaging/aur)。

**开发用的第二个副本。** `BOXPILOT_INSTANCE=dev` 让副本使用独立的单实例锁，配合 `XDG_CONFIG_HOME`（Linux）
使用独立的数据文件夹，就能和日常使用的 BoxPilot 同时运行。

**CI 与发布。**

- [`ci.yml`](.github/workflows/ci.yml) 在 Windows、macOS（arm64、x64）和 Linux 上构建并测试：单元测试、所有
  生成的配置通过 `sing-box check`、针对真实内核的引擎实测、界面启动检查，以及在 Ubuntu 和 Arch 上安装 Linux 包。
- [`live.yml`](.github/workflows/live.yml) 在真实机器上检查系统代理与 TUN。
- [`release.yml`](.github/workflows/release.yml) 在推送版本标签时发布。先修改 `Cargo.toml` 中的版本，然后：

  ```bash
  git tag v1.2.3
  git push origin v1.2.3
  ```

  它会附上 Windows zip、macOS `.dmg` 与压缩包，以及 Linux 压缩包、`.deb`、`.rpm` 和 Arch 仓库。设置
  `AUR_SSH_PRIVATE_KEY` 密钥后还会更新 AUR。

## 目录结构

| 路径 | 内容 |
| --- | --- |
| `crates/core` | 不含界面的部分：订阅转换（`convert/`）、配置生成（`config.rs`）、规则集转换（`rules.rs`）、负责进程生命周期、调度与更新的引擎（`engine.rs`）、Clash API 客户端、内核下载器、系统集成（`sys.rs`）。 |
| `crates/app` | GPUI 应用：页面（`src/ui/`）、弹簧动效（`ui/anim.rs`）、翻译（`i18n.rs`）、托盘、单实例、窗口。 |
| `packaging/` | Linux 桌面文件与打包脚本、AUR 打包脚本、macOS `Info.plist`。 |
| `docs/` | 三种语言的使用说明。 |

## 许可证

MIT，详见 [LICENSE](LICENSE)。
