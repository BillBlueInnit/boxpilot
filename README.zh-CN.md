# BoxPilot

[English](README.md) · **简体中文** · [Русский](README.ru.md) · [使用说明](docs/usage.zh-CN.md)

一款简洁的桌面代理客户端，支持 Windows、macOS 和 Linux，使用 Rust 与
[GPUI](https://gpui.rs)（通过 gpui-kit）编写。它驱动 [sing-box](https://sing-box.sagernet.org) 内核，
并内置订阅转换器。

## 快速开始

从 [Releases](https://github.com/BillBlueInnit/boxpilot/releases) 下载对应平台的版本：

- **Windows**：`BoxPilot-<版本>-windows-x64.zip`，解压后运行 `boxpilot.exe`。
- **macOS**：`BoxPilot-<版本>-macos-arm64.dmg`（Apple 芯片）或 `-macos-x64.dmg`（Intel），打开后把
  BoxPilot 拖进“应用程序”。应用未经公证：第一次打开时请右键点击它并选择**打开**（或在“系统设置 →
  隐私与安全性”中允许）。
- **Linux**：`BoxPilot-<版本>-linux-x64.tar.gz`，解压后运行 `boxpilot`。

也可以自行编译：

```
cargo run --release -p boxpilot
```

然后：

1. **设置 → sing-box 内核 → 下载最新版。** 内核不随应用附带；BoxPilot 会从 GitHub 下载适合你平台的
   官方版本（代理运行时经由代理下载，也可以设置镜像前缀）。
2. **订阅 → 添加**，粘贴服务商提供的订阅链接（或点**粘贴**从剪贴板导入）。
3. **仪表盘 → 连接。**

详细的分步说明见 [docs/usage.zh-CN.md](docs/usage.zh-CN.md)。数据保存位置见下方[平台](#平台)表格；
在 Windows 上，在 `boxpilot.exe` 旁边建立 `data` 文件夹即为便携模式。

## 功能

- **订阅转换**：v2ray / Shadowrocket 分享链接（明文或 base64）、Clash / Clash.Meta YAML 和 sing-box JSON。
  支持 VLESS（REALITY、Vision）、VMess、Trojan、Shadowsocks（含 2022、obfs 和 v2ray-plugin）、
  Hysteria 2（端口跳跃）、Hysteria、TUIC、AnyTLS、WireGuard、SOCKS、HTTP(S) 和 SSH，传输层支持
  ws / grpc / h2 / httpupgrade / quic。不支持的条目（SSR、xhttp 等）会记录在日志中，不会中断导入。
- **订阅更新**：每个订阅独立的更新间隔、启动时更新、补上应用关闭期间错过的更新、失败后更快重试、
  代理 → 直连回退、自定义 User-Agent、包含 / 排除正则过滤（离线重新应用）、从 `subscription-userinfo`
  读取流量与到期时间、服务商标题与间隔提示。一批更新完成后内核只重载一次。
- **代理**：规则 / 全局 / 直连模式（实时切换）、每个订阅的选择组和自动组、延迟测试、系统代理（退出
  及崩溃后自动恢复）、TUN 模式及“以管理员身份重启”流程、局域网共享、UDP 走代理（或直连）以及可选的
  QUIC 阻断。配置改动合并为一次内核重载，自动切换不会中断已有连接，内核崩溃会自动重启。
- **本地节点与分享**：通过分享链接单独添加节点（ss、vmess、vless、trojan、hysteria2、tuic、anytls、
  socks5、http(s)、wireguard、ssh，也支持 Clash / sing-box 节点列表），它们集中在**本地节点**分组中；
  任何节点（订阅节点或本地节点）都可以复制为分享链接。
- **自动组**：响应最快、稳定优先（只在失败或差距很大时切换）、按列表顺序（沿列表故障转移并自动回切）、
  带宽优先（定期对最快的几个节点测速），每种方式都可以限定在一个地区。
- **测速**：对单个节点或当前显示的全部节点测试下载速度和真实延迟（经节点发起一次全新的 HTTPS 请求），
  通过独立的本地探针进行，不会改变你的节点选择。
- **规则**：从链接或本地文件导入规则集，包括 sing-box `.srs` / `.json`、Clash 规则提供者（`payload:`
  YAML，domain / ipcidr / classical）以及 ACL4SSR `.list` 文件，并转换为 sing-box 规则集。每个规则集
  可以发往代理、直连、拒绝或任意分组，按你排列的顺序生效，并按各自的间隔更新。**导入 ACL4SSR**
  会读取 subconverter `.ini`（例如 ACL4SSR_Online_Full），为每条 `ruleset=` 建立规则集并设置正确的
  处理方式；导入后默认关闭，由你挑选启用（可一键全部启用 / 全部停用）。规则集可以在代理页拥有
  **独立的节点组**：手动选择，或用与自动组相同的方式和地区筛选自动选择；规则集文件可以用系统默认
  文本编辑器打开，保存后自动生效。另有内置的绕过中国大陆与广告拦截，以及自定义规则（`类型,值,目标`）。
- **DNS**：DoH / DoT / DoQ / UDP、FakeIP、IPv6。规则集由应用缓存并以本地文件交给 sing-box，因此
  GitHub 被墙或节点失效都不会导致内核无法启动。
- **应用**：English、简体中文、繁體中文和 Русский（跟随系统语言，可实时切换）；以管理员身份运行
  （开机启动通过最高权限计划任务，不弹 UAC）；托盘图标可切换模式、系统代理和 TUN；关闭到托盘、单实例、
  开机启动（可最小化）、实时流量图、连接列表、日志查看、浅色 / 深色主题、主题色，以及半透明窗口
  （Windows 11 上为 Mica、Mica Alt 或 Acrylic，macOS 和 KDE Plasma 上为系统模糊）。
- **代理页**：每个节点都显示国家 / 地区代码：先从节点名称（旗帜、地名、代码）识别，其次对服务器做
  GeoIP 查询，若为任播 / CDN 或内网地址，则经该节点检测出口 IP。轻量模式会在应用驻留托盘时卸载界面。
- **应用更新**：每天检查一次 GitHub Releases；可自动安装、仅提醒或从不通知（设置 → 更新）。更新会
  原地替换可执行文件，重启后生效。
- **动效**：所有过渡都是阻尼弹簧（GPUI 的解析解算器），运动会像实物一样缓动并稳定下来，被打断时保留
  速度，并遵循“减少动态效果”设置。

## 平台

BoxPilot 可在 GPUI 支持的所有平台上运行：Windows 10/11、macOS（Apple 芯片与 Intel）和 Linux（X11 与
Wayland），功能一致。各系统不同之处，分别采用其原生机制：

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 安装 | zip | 含 BoxPilot.app 的 `.dmg`（程序坞、启动台） | tar.gz |
| 窗口 | 无边框、原生边框、最大化按钮的贴靠布局、可选 Mica | 原生红绿灯按钮 | 客户端装饰（或窗口管理器自带） |
| 标题栏按钮 | Segoe Fluent / MDL2 图标、WinUI 配色 | AppKit 红绿灯 | Adwaita（GNOME）或 Breeze（KDE）风格，遵循 `button-layout` 顺序与位置 |
| 系统代理 | WinINet（注册表） | `networksetup`，覆盖每个网络服务 | GNOME `gsettings`、KDE `kioslaverc` |
| TUN 权限 | 以管理员身份运行 BoxPilot（开机启动通过计划任务，不弹 UAC） | 一次管理员授权后内核设为 setuid root | 通过 `pkexec setcap` 为内核授予 `CAP_NET_ADMIN` |
| 开机启动 | Run 注册表项 / 计划任务 | LaunchAgent | XDG autostart |
| 关闭到托盘 | 隐藏窗口 | 隐藏应用（点击程序坞图标恢复） | 关闭窗口（从托盘重新打开） |
| 应用崩溃时清理内核 | 作业对象 | 下次启动时清理残留进程 | 父进程退出信号 + 清理残留进程 |
| 数据文件夹 | `%APPDATA%\BoxPilot` | `~/Library/Application Support/BoxPilot` | `$XDG_CONFIG_HOME/BoxPilot` |

Linux 上的托盘图标需要支持 AppIndicator 的托盘（GNOME 需安装 AppIndicator 扩展）。

`.github/workflows/ci.yml` 在 Windows、macOS（arm64 与 x64）和 Linux 上原生构建并测试：单元测试、
所有生成的配置通过 `sing-box check`、针对真实内核的引擎实测（启动、分组、选择、延迟、模式切换、
流量、重载、停止），以及界面启动检查（`boxpilot --smoke-test`）。

`.github/workflows/release.yml` 在推送版本标签时发布 Release。标签必须与 `Cargo.toml` 中的工作区版本一致：

```
git tag v1.0.0
git push origin v1.0.0
```

它会为 Windows x64、macOS arm64/x64 和 Linux x64 附上 `BoxPilot-<版本>-<平台>.zip|tar.gz`（应用内
更新会查找这些文件名），并为每个 macOS 版本附上 `.dmg`。

## 目录结构

| 路径 | 作用 |
| --- | --- |
| `crates/core` | 不含界面。转换器（`convert/`）、配置生成（`config.rs`）、负责进程生命周期、调度与更新的引擎（`engine.rs`）、Clash API 客户端、内核下载器以及系统集成（`sys.rs`）。 |
| `crates/core`（`rules.rs`） | 规则集转换（Clash / ACL4SSR → sing-box）与 ACL4SSR `.ini` 解析。 |
| `crates/app` | GPUI 前端：页面在 `src/ui/`，弹簧动效在 `ui/anim.rs`，翻译在 `i18n.rs`，以及托盘、单实例和窗口处理。 |

`cargo test -p boxpilot-core` 运行转换器与配置测试。用真实内核校验生成的配置：

```
cargo run -p boxpilot-core --example gen_config -- out
sing-box check -c out/everything-1.14.json
```

生成的配置面向 sing-box 1.12+（推荐 1.14）：规则动作、WireGuard 端点、新的 DNS 服务器格式以及 `http_clients`。

## 许可证

MIT，详见 [LICENSE](LICENSE)。
