//! OS integration: system proxy, launch at login, elevation, child process lifetime.

use anyhow::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SysProxyState {
    pub enabled: bool,
    pub server: String,
}

/// Linux: polkit could not ask for the password (no authentication agent,
/// as under many tiling window managers, or no polkit at all), so the app
/// asks for it and runs the grant through `sudo`.
pub const NEEDS_PASSWORD: &str = "the administrator password is needed";

/// Linux: `sudo` turned the password down.
pub const WRONG_PASSWORD: &str = "incorrect password";

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{Context, bail};
    use std::os::windows::ffi::OsStrExt;
    use winreg::RegKey;
    use winreg::enums::*;

    const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn notify_wininet() {
        use windows_sys::Win32::Networking::WinInet::{
            INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
        };
        unsafe {
            InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null(), 0);
            InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_REFRESH, std::ptr::null(), 0);
        }
    }

    pub fn set_system_proxy(server: &str, bypass: &str) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(INTERNET_SETTINGS).context("open Internet Settings")?;
        key.set_value("ProxyEnable", &1u32)?;
        key.set_value("ProxyServer", &server.to_string())?;
        key.set_value("ProxyOverride", &bypass.to_string())?;
        notify_wininet();
        Ok(())
    }

    pub fn clear_system_proxy() -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(INTERNET_SETTINGS).context("open Internet Settings")?;
        key.set_value("ProxyEnable", &0u32)?;
        notify_wininet();
        Ok(())
    }

    pub fn system_proxy_state() -> SysProxyState {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(INTERNET_SETTINGS) else {
            return SysProxyState::default();
        };
        SysProxyState {
            enabled: key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 1,
            server: key.get_value::<String, _>("ProxyServer").unwrap_or_default(),
        }
    }

    pub fn set_launch_at_login(name: &str, command: Option<&str>) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(RUN_KEY).context("open Run key")?;
        match command {
            Some(cmd) => key.set_value(name, &cmd.to_string())?,
            None => match key.delete_value(name) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            },
        }
        Ok(())
    }

    pub fn launch_at_login_enabled(name: &str) -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .and_then(|k| k.get_value::<String, _>(name))
            .is_ok()
    }

    pub fn is_elevated() -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut _,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            );
            CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    /// Starts a new elevated instance of the current executable (UAC prompt).
    pub fn relaunch_elevated(args: &str) -> Result<()> {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let exe = std::env::current_exe()?;
        let exe = wide(&exe.to_string_lossy());
        let verb = wide("runas");
        let params = wide(args);
        let r = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                exe.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if (r as isize) <= 32 {
            bail!("elevation was cancelled");
        }
        Ok(())
    }

    /// A job object that terminates every assigned process when the app exits,
    /// even if it crashes.
    pub struct KillOnCloseJob(windows_sys::Win32::Foundation::HANDLE);

    unsafe impl Send for KillOnCloseJob {}
    unsafe impl Sync for KillOnCloseJob {}

    impl KillOnCloseJob {
        pub fn new() -> Option<Self> {
            use windows_sys::Win32::System::JobObjects::*;
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                Some(Self(job))
            }
        }

        pub fn assign(&self, process: std::os::windows::io::RawHandle) {
            use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
            unsafe {
                AssignProcessToJobObject(self.0, process as _);
            }
        }
    }

    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// On Windows TUN needs the app (and so the core) to run elevated.
    pub fn core_tun_privileged(_core: &std::path::Path) -> bool {
        is_elevated()
    }

    pub fn grant_tun_privilege(_core: &std::path::Path) -> Result<()> {
        bail!("restart BoxPilot as administrator")
    }

    /// Kills a core left running by a previous crash, if `pid` is still a sing-box.
    pub fn kill_stale_core(pid: u32) {
        use std::os::windows::process::CommandExt;
        let filter = format!("PID eq {pid}");
        let listed = std::process::Command::new("tasklist")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase())
            .unwrap_or_default();
        if listed.contains("sing-box") {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
    }

    fn schtasks(args: &[&str]) -> std::io::Result<std::process::Output> {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("schtasks")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    /// Creates (or removes) a logon task that starts the app with the highest
    /// privileges, so an elevated start needs no UAC prompt. Requires elevation.
    pub fn set_elevated_login_task(name: &str, exe_and_args: Option<(&str, &str)>) -> Result<()> {
        let Some((exe, args)) = exe_and_args else {
            if elevated_login_task_exists(name) {
                let out = schtasks(&["/Delete", "/TN", name, "/F"])?;
                if !out.status.success() {
                    bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
                }
            }
            return Ok(());
        };
        let name_only = std::env::var("USERNAME").unwrap_or_default();
        let user = match std::env::var("USERDOMAIN") {
            Ok(d) if !d.is_empty() => format!("{d}\\{name_only}"),
            _ => name_only,
        };
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT3S</Delay></LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{exe}</Command><Arguments>{args}</Arguments></Exec>
  </Actions>
</Task>"#,
            user = xml_escape(&user),
            exe = xml_escape(exe),
            args = xml_escape(args),
        );
        let path = std::env::temp_dir().join(format!("{name}-task.xml"));
        let mut bytes = vec![0xFF, 0xFE];
        for u in xml.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&path, bytes)?;
        let out = schtasks(&["/Create", "/TN", name, "/XML", &path.to_string_lossy(), "/F"]);
        let _ = std::fs::remove_file(&path);
        let out = out?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    pub fn elevated_login_task_exists(name: &str) -> bool {
        schtasks(&["/Query", "/TN", name]).map(|o| o.status.success()).unwrap_or(false)
    }

    /// Sets the per-app GPU preference Windows applies to `exe` (the same
    /// setting as Settings > Display > Graphics): 1 = power saving,
    /// 2 = high performance, `None` = let Windows decide.
    pub fn set_gpu_preference(exe: &str, preference: Option<u32>) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(r"Software\Microsoft\DirectX\UserGpuPreferences")
            .context("open UserGpuPreferences")?;
        match preference {
            Some(p) => key.set_value(exe, &format!("GpuPreference={p};"))?,
            None => match key.delete_value(exe) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            },
        }
        Ok(())
    }

    /// The Windows accent for light or dark mode (0xRRGGBB), from the palette
    /// Windows builds around it: Light2 on dark, Dark1 on light, as WinUI does.
    pub fn accent_color(dark: bool) -> Option<u32> {
        let raw = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
            .and_then(|k| k.get_raw_value("AccentPalette"))
            .ok()?;
        // Eight RGBA entries: Light3, Light2, Light1, Accent, Dark1, Dark2, Dark3, unused.
        let c = raw.bytes.chunks_exact(4).nth(if dark { 1 } else { 4 })?;
        Some(u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]))
    }

    /// Settings > Accessibility > Text size, as a factor (1.0 = 100 %).
    pub fn system_text_scale() -> f32 {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Accessibility")
            .and_then(|k| k.get_value::<u32, _>("TextScaleFactor"))
            .map(|p| p.clamp(100, 225) as f32 / 100.0)
            .unwrap_or(1.0)
    }

    /// The Windows build number, e.g. 26100 (0 when unknown).
    pub fn windows_build() -> u32 {
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
            .ok()
            .and_then(|b| b.trim().parse().ok())
            .unwrap_or(0)
    }

    /// The Windows display language as a locale name, e.g. `zh-CN`.
    pub fn ui_locale() -> String {
        use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
        let mut buf = [0u16; 85];
        let n = unsafe {
            LCIDToLocaleName(GetUserDefaultUILanguage() as u32, buf.as_mut_ptr(), buf.len() as i32, 0)
        };
        if n <= 1 {
            return "en-US".into();
        }
        String::from_utf16_lossy(&buf[..(n - 1) as usize])
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use anyhow::{Context, bail};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn run(cmd: &str, args: &[&str]) -> Result<String> {
        let out = Command::new(cmd).args(args).output().with_context(|| format!("run {cmd}"))?;
        if !out.status.success() {
            bail!("{cmd}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn home() -> PathBuf {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
    }

    /// `localhost;127.*;<local>` (Windows style) -> individual host patterns.
    fn bypass_hosts(bypass: &str) -> Vec<String> {
        bypass
            .split([';', ','])
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(|h| if h == "<local>" { "*.local".to_string() } else { h.to_string() })
            .collect()
    }

    fn split_host_port(server: &str) -> (&str, &str) {
        server.rsplit_once(':').unwrap_or((server, "7890"))
    }

    // ------------------------------------------------------------ macOS

    #[cfg(target_os = "macos")]
    fn network_services() -> Result<Vec<String>> {
        let out = run("networksetup", &["-listallnetworkservices"])?;
        Ok(out
            .lines()
            .skip(1) // "An asterisk (*) denotes that a network service is disabled."
            .filter(|l| !l.trim().is_empty() && !l.starts_with('*'))
            .map(|l| l.trim().to_string())
            .collect())
    }

    /// `networksetup` reports most failures on stdout with a zero exit status.
    #[cfg(target_os = "macos")]
    fn networksetup(args: &[String]) -> Result<()> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = run("networksetup", &args)?;
        let lower = out.to_ascii_lowercase();
        if lower.contains("** error") || lower.contains("requires admin") || lower.contains("privilege") {
            bail!("networksetup {}: {}", args.first().copied().unwrap_or_default(), out.trim());
        }
        Ok(())
    }

    /// Runs `networksetup` commands as the user; failures (a service without
    /// proxy support, a refusal) are left to the caller's check of the result.
    #[cfg(target_os = "macos")]
    fn networksetup_all(commands: &[Vec<String>]) {
        for c in commands {
            if let Err(e) = networksetup(c) {
                log::warn!("{e:#}");
            }
        }
    }

    /// Runs `networksetup` commands as root after one administrator prompt.
    /// Standard accounts and managed Macs need it to change network settings.
    #[cfg(target_os = "macos")]
    fn networksetup_admin(commands: &[Vec<String>]) -> Result<()> {
        let quote = |a: &str| format!("'{}'", a.replace('\'', "'\\''"));
        let script = commands
            .iter()
            .map(|c| format!("/usr/sbin/networksetup {}", c.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")))
            .collect::<Vec<_>>()
            .join(" ; ");
        let script = script.replace('\\', "\\\\").replace('"', "\\\"");
        run("osascript", &["-e", &format!("do shell script \"{script}\" with administrator privileges")])
            .map(|_| ())
            .context("the administrator prompt was cancelled")
    }

    /// Every enabled service gets the proxy, so it follows Wi-Fi, Ethernet or a dock.
    #[cfg(target_os = "macos")]
    pub fn set_system_proxy(server: &str, bypass: &str) -> Result<()> {
        let (host, port) = split_host_port(server);
        let mut hosts = bypass_hosts(bypass);
        if hosts.is_empty() {
            hosts.push("Empty".into());
        }
        let mut commands = Vec::new();
        for svc in network_services()? {
            for kind in ["-setwebproxy", "-setsecurewebproxy", "-setsocksfirewallproxy"] {
                commands.push(vec![kind.to_string(), svc.clone(), host.to_string(), port.to_string()]);
            }
            for kind in ["-setwebproxystate", "-setsecurewebproxystate", "-setsocksfirewallproxystate"] {
                commands.push(vec![kind.to_string(), svc.clone(), "on".into()]);
            }
            let mut bypass = vec!["-setproxybypassdomains".to_string(), svc.clone()];
            bypass.extend(hosts.iter().cloned());
            commands.push(bypass);
        }
        let wanted = format!("{host}:{port}");
        let taken = || {
            let state = system_proxy_state();
            (state.enabled && state.server == wanted, state)
        };
        networksetup_all(&commands);
        if !taken().0 {
            // Refused quietly or loudly: one administrator prompt applies it all.
            networksetup_admin(&commands)?;
        }
        let (ok, state) = taken();
        if !ok {
            bail!("macOS did not take the proxy setting (now: {})", if state.enabled { state.server.as_str() } else { "off" });
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn clear_system_proxy() -> Result<()> {
        let mut commands = Vec::new();
        for svc in network_services()? {
            for kind in ["-setwebproxystate", "-setsecurewebproxystate", "-setsocksfirewallproxystate"] {
                commands.push(vec![kind.to_string(), svc.clone(), "off".into()]);
            }
        }
        networksetup_all(&commands);
        if system_proxy_state().enabled {
            networksetup_admin(&commands)?;
        }
        Ok(())
    }

    /// The DNS servers macOS asks, from every resolver `scutil --dns` lists.
    #[cfg(target_os = "macos")]
    pub fn system_dns_servers() -> Vec<std::net::IpAddr> {
        let out = run("scutil", &["--dns"]).unwrap_or_default();
        let mut servers = Vec::new();
        for line in out.lines() {
            // "  nameserver[0] : 192.168.1.1" (IPv6 may carry "%en0").
            let Some((_, ip)) = line.trim().strip_prefix("nameserver[").and_then(|r| r.split_once(" : ")) else {
                continue;
            };
            if let Ok(ip) = ip.trim().split('%').next().unwrap_or_default().parse() {
                if !servers.contains(&ip) {
                    servers.push(ip);
                }
            }
        }
        servers
    }

    /// The DNS servers set on each enabled network service; an empty list
    /// means the ones DHCP hands out.
    #[cfg(target_os = "macos")]
    pub fn dns_settings() -> Result<Vec<(String, Vec<String>)>> {
        let mut out = Vec::new();
        for svc in network_services()? {
            let text = run("networksetup", &["-getdnsservers", &svc]).unwrap_or_default();
            let servers = text
                .lines()
                .map(str::trim)
                .filter(|l| l.parse::<std::net::IpAddr>().is_ok())
                .map(String::from)
                .collect();
            out.push((svc, servers));
        }
        Ok(out)
    }

    /// Sets the DNS servers of each listed service (empty: back to DHCP's).
    #[cfg(target_os = "macos")]
    pub fn set_dns_settings(wanted: &[(String, Vec<String>)]) -> Result<()> {
        let commands: Vec<Vec<String>> = wanted
            .iter()
            .map(|(svc, servers)| {
                let mut c = vec!["-setdnsservers".to_string(), svc.clone()];
                if servers.is_empty() {
                    c.push("Empty".into());
                } else {
                    c.extend(servers.iter().cloned());
                }
                c
            })
            .collect();
        let taken = || {
            let now = dns_settings().unwrap_or_default();
            wanted.iter().all(|w| now.iter().any(|n| n == w))
        };
        networksetup_all(&commands);
        if !taken() {
            networksetup_admin(&commands)?;
        }
        if !taken() {
            bail!("macOS did not take the DNS setting");
        }
        let _ = run("dscacheutil", &["-flushcache"]);
        Ok(())
    }

    /// What the system uses right now (the primary service's settings).
    #[cfg(target_os = "macos")]
    pub fn system_proxy_state() -> SysProxyState {
        let out = run("scutil", &["--proxy"]).unwrap_or_default();
        let field = |k: &str| {
            out.lines()
                .find_map(|l| l.trim().strip_prefix(k).and_then(|v| v.trim().strip_prefix(':')).map(|v| v.trim().to_string()))
                .unwrap_or_default()
        };
        SysProxyState {
            enabled: field("HTTPEnable") == "1",
            server: format!("{}:{}", field("HTTPProxy"), field("HTTPPort")),
        }
    }

    #[cfg(target_os = "macos")]
    fn launch_agent(name: &str) -> PathBuf {
        home().join("Library/LaunchAgents").join(format!("app.boxpilot.{name}.plist"))
    }

    #[cfg(target_os = "macos")]
    pub fn set_launch_at_login(name: &str, command: Option<&str>) -> Result<()> {
        let path = launch_agent(name);
        let Some(command) = command else {
            let _ = std::fs::remove_file(&path);
            return Ok(());
        };
        let args: String = split_command(command)
            .iter()
            .map(|a| format!("    <string>{}</string>\n", xml_escape(a)))
            .collect();
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>app.boxpilot.{name}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
        );
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, plist)?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn launch_at_login_enabled(name: &str) -> bool {
        launch_agent(name).is_file()
    }

    /// TUN on macOS needs root: the core runs setuid root (still killable by
    /// the app, since its real uid stays the user's).
    #[cfg(target_os = "macos")]
    pub fn core_tun_privileged(core: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(core).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o4000 != 0)
    }

    #[cfg(target_os = "macos")]
    pub fn grant_tun_privilege(core: &Path) -> Result<()> {
        let p = core.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "do shell script \"chown root:wheel \" & quoted form of \"{p}\" & \" && chmod u+s \" & quoted form of \"{p}\" with administrator privileges"
        );
        run("osascript", &["-e", &script]).map(|_| ())
    }

    /// Resolves missing `process_path` fields on active macOS connections.
    ///
    /// `sing-box`'s Darwin process searcher uses a fixed `pcblist_n` struct stride
    /// (which drifts on newer XNU kernels such as macOS 27 / Darwin 26) and also
    /// misses local `mixed` (system proxy) TCP sockets whose remote endpoint is
    /// `127.0.0.1:<mixed_port>` rather than the final target. We first walk XNU's
    /// self-describing TLV `pcblist_n` stream (`x_len` / `x_kind`) and match by
    /// 4-tuple or local `(ip, port)`, then fall back to `lsof -nP -F pcn` if any
    /// sockets remain unresolved.
    #[cfg(target_os = "macos")]
    pub fn fill_macos_connection_processes(conns: &mut [crate::clash_api::Connection]) {
        use std::collections::HashMap;
        use std::net::IpAddr;

        let mut pid_paths: HashMap<u32, String> = HashMap::new();
        let mut resolve_pid = |pid: u32| -> Option<String> {
            if pid == 0 {
                return None;
            }
            if let Some(cached) = pid_paths.get(&pid) {
                return (!cached.is_empty()).then(|| cached.clone());
            }
            let path = macos_pid_path(pid).unwrap_or_default();
            pid_paths.insert(pid, path.clone());
            (!path.is_empty()).then_some(path)
        };

        // Tier 1: self-describing TLV walk of net.inet.{tcp,udp}.pcblist_n.
        let need_tcp = conns.iter().any(|c| c.process().is_empty() && c.metadata.network.eq_ignore_ascii_case("tcp"));
        let need_udp = conns.iter().any(|c| c.process().is_empty() && c.metadata.network.eq_ignore_ascii_case("udp"));
        let tcp_entries = if need_tcp {
            sysctl_raw(c"net.inet.tcp.pcblist_n").map(|b| super::darwin_proc::parse_pcblist_tlv(&b)).unwrap_or_default()
        } else {
            Vec::new()
        };
        let udp_entries = if need_udp {
            sysctl_raw(c"net.inet.udp.pcblist_n").map(|b| super::darwin_proc::parse_pcblist_tlv(&b)).unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut unresolved = false;
        for c in conns.iter_mut() {
            if !c.process().is_empty() {
                continue;
            }
            let Ok(src_port) = c.metadata.source_port.parse::<u16>() else { continue };
            let src_ip: Option<IpAddr> = c.metadata.source_ip.parse().ok();
            let dst_ip: Option<IpAddr> = c.metadata.destination_ip.parse().ok();
            let dst_port: Option<u16> = c.metadata.destination_port.parse().ok();
            let entries = if c.metadata.network.eq_ignore_ascii_case("udp") {
                &udp_entries
            } else {
                &tcp_entries
            };
            if let Some(entry) = super::darwin_proc::match_entry(entries, src_ip, src_port, dst_ip, dst_port) {
                let path = (entry.effective_pid != 0 && entry.effective_pid != entry.pid)
                    .then(|| resolve_pid(entry.effective_pid))
                    .flatten()
                    .or_else(|| resolve_pid(entry.pid));
                if let Some(p) = path {
                    c.metadata.process_path = p;
                    continue;
                }
            }
            unresolved = true;
        }

        // Tier 2: `lsof` fallback (kernel-ABI-independent, covers any XNU layout change).
        if unresolved {
            if let Ok(out) = run("/usr/sbin/lsof", &["-nP", "-F", "pcn", "-iTCP", "-iUDP"])
                .or_else(|_| run("lsof", &["-nP", "-F", "pcn", "-iTCP", "-iUDP"]))
            {
                let entries = super::darwin_proc::parse_lsof(&out);
                for c in conns.iter_mut() {
                    if !c.process().is_empty() {
                        continue;
                    }
                    let Ok(src_port) = c.metadata.source_port.parse::<u16>() else { continue };
                    let src_ip: Option<IpAddr> = c.metadata.source_ip.parse().ok();
                    if let Some((pid, cmd)) = super::darwin_proc::match_lsof(&entries, src_ip, src_port) {
                        c.metadata.process_path = resolve_pid(pid).unwrap_or_else(|| cmd.to_string());
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn sysctl_raw(name: &std::ffi::CStr) -> Option<Vec<u8>> {
        let mut len: libc::size_t = 0;
        let rc = unsafe {
            libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0)
        };
        if rc != 0 || len < 24 {
            return None;
        }
        // Leave headroom for sockets opened between the size probe and the read.
        let mut cap = len.saturating_add(len / 4).max(len + 4096);
        let mut buf = vec![0u8; cap];
        let rc = unsafe {
            libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut cap, std::ptr::null_mut(), 0)
        };
        if rc != 0 {
            return None;
        }
        buf.truncate(cap);
        Some(buf)
    }

    #[cfg(target_os = "macos")]
    fn macos_pid_path(pid: u32) -> Option<String> {
        unsafe extern "C" {
            fn proc_pidpath(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
            fn proc_name(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
        }
        let mut buf = [0u8; 4096];
        let n = unsafe { proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n > 0 {
            if let Ok(s) = std::str::from_utf8(&buf[..n as usize]) {
                let s = s.trim_end_matches('\0');
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
        let mut name_buf = [0u8; 256];
        let n = unsafe { proc_name(pid as libc::c_int, name_buf.as_mut_ptr().cast(), name_buf.len() as u32) };
        if n > 0 {
            if let Ok(s) = std::str::from_utf8(&name_buf[..n as usize]) {
                let s = s.trim_end_matches('\0');
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
        None
    }

    // ------------------------------------------------------------ Linux

    #[cfg(not(target_os = "macos"))]
    fn is_kde() -> bool {
        std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"))
    }

    /// A system tool by absolute path. Users' PATHs (Debian's among them)
    /// often leave out the sbin directories, and pkexec looks nothing up.
    #[cfg(not(target_os = "macos"))]
    fn system_tool(name: &str) -> Option<PathBuf> {
        ["/usr/sbin", "/sbin", "/usr/bin", "/bin", "/usr/local/sbin", "/usr/local/bin"]
            .iter()
            .map(|d| Path::new(d).join(name))
            .find(|p| p.is_file())
    }

    #[cfg(not(target_os = "macos"))]
    fn kwriteconfig(args: &[&str]) -> Result<()> {
        run("kwriteconfig6", args).or_else(|_| run("kwriteconfig5", args)).map(|_| ())
    }

    #[cfg(not(target_os = "macos"))]
    fn kreadconfig(args: &[&str]) -> Option<String> {
        run("kreadconfig6", args).or_else(|_| run("kreadconfig5", args)).ok().map(|s| s.trim().to_string())
    }

    #[cfg(not(target_os = "macos"))]
    fn gsettings(args: &[&str]) -> Result<()> {
        run("gsettings", args).map(|_| ())
    }

    /// KDE's proxy (KIO apps, and Chromium-based browsers on KDE).
    #[cfg(not(target_os = "macos"))]
    fn set_kde_proxy(host: &str, port: &str, hosts: &[String]) -> Result<()> {
        let k = |key: &str, value: &str| kwriteconfig(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key, value]);
        k("ProxyType", "1")?;
        k("httpProxy", &format!("http://{host} {port}"))?;
        k("httpsProxy", &format!("http://{host} {port}"))?;
        k("socksProxy", &format!("socks://{host} {port}"))?;
        k("NoProxyFor", &hosts.join(","))?;
        reload_kio();
        Ok(())
    }

    /// Tells running KDE apps to read the proxy again.
    #[cfg(not(target_os = "macos"))]
    fn reload_kio() {
        let _ = run(
            "dbus-send",
            &["--type=signal", "/KIO/Scheduler", "org.kde.KIO.Scheduler.reparseSlaveConfiguration", "string:"],
        );
    }

    /// GNOME's proxy (GTK apps, and Firefox on every desktop).
    #[cfg(not(target_os = "macos"))]
    fn set_gnome_proxy(host: &str, port: &str, hosts: &[String]) -> Result<()> {
        for schema in ["http", "https", "socks"] {
            let schema = format!("org.gnome.system.proxy.{schema}");
            gsettings(&["set", &schema, "host", host])?;
            gsettings(&["set", &schema, "port", port])?;
        }
        let ignore = format!("[{}]", hosts.iter().map(|h| format!("'{h}'")).collect::<Vec<_>>().join(", "));
        gsettings(&["set", "org.gnome.system.proxy", "ignore-hosts", &ignore])?;
        gsettings(&["set", "org.gnome.system.proxy", "mode", "manual"])
    }

    /// Apps read the proxy from different places whatever the desktop, so
    /// both KDE's and GNOME's settings are set where they exist.
    #[cfg(not(target_os = "macos"))]
    pub fn set_system_proxy(server: &str, bypass: &str) -> Result<()> {
        let (host, port) = split_host_port(server);
        let hosts = bypass_hosts(bypass);
        let kde = set_kde_proxy(host, port, &hosts);
        let gnome = set_gnome_proxy(host, port, &hosts);
        match (kde, gnome) {
            (Err(k), Err(g)) => bail!("no desktop proxy settings could be changed (KDE: {k:#}; GNOME: {g:#})"),
            _ => Ok(()),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn clear_system_proxy() -> Result<()> {
        let kde = kwriteconfig(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType", "0"]);
        if kde.is_ok() {
            reload_kio();
        }
        let gnome = gsettings(&["set", "org.gnome.system.proxy", "mode", "none"]);
        match (kde, gnome) {
            (Err(k), Err(g)) => bail!("no desktop proxy settings could be changed (KDE: {k:#}; GNOME: {g:#})"),
            _ => Ok(()),
        }
    }

    /// The desktop's own setting: KDE's on Plasma, GNOME's elsewhere.
    #[cfg(not(target_os = "macos"))]
    pub fn system_proxy_state() -> SysProxyState {
        if is_kde() {
            let get = |key: &str| kreadconfig(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key]).unwrap_or_default();
            // "http://127.0.0.1 7890"
            let http = get("httpProxy");
            let server = http.trim_start_matches("http://").replacen(' ', ":", 1);
            return SysProxyState { enabled: get("ProxyType") == "1", server };
        }
        let get = |schema: &str, key: &str| {
            run("gsettings", &["get", schema, key])
                .map(|s| s.trim().trim_matches('\'').to_string())
                .unwrap_or_default()
        };
        SysProxyState {
            enabled: get("org.gnome.system.proxy", "mode") == "manual",
            server: format!(
                "{}:{}",
                get("org.gnome.system.proxy.http", "host"),
                get("org.gnome.system.proxy.http", "port")
            ),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn autostart_file(name: &str) -> PathBuf {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"));
        config.join("autostart").join(format!("{}.desktop", name.to_ascii_lowercase()))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn set_launch_at_login(name: &str, command: Option<&str>) -> Result<()> {
        let path = autostart_file(name);
        let Some(command) = command else {
            let _ = std::fs::remove_file(&path);
            return Ok(());
        };
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec={command}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
            ),
        )?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn launch_at_login_enabled(name: &str) -> bool {
        autostart_file(name).is_file()
    }

    /// TUN on Linux needs CAP_NET_ADMIN on the core binary, or a setuid-root core.
    #[cfg(not(target_os = "macos"))]
    pub fn core_tun_privileged(core: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        let setuid_root = std::fs::metadata(core).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o4000 != 0);
        (setuid_root || has_net_admin_cap(core)) && !nosuid_mount(core)
    }

    /// Reads the file capabilities (`security.capability`) directly, so no
    /// `getcap` is needed (and none runs on every UI refresh).
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    fn has_net_admin_cap(core: &Path) -> bool {
        use std::os::unix::ffi::OsStrExt as _;
        let Ok(path) = std::ffi::CString::new(core.as_os_str().as_bytes()) else { return false };
        let mut buf = [0u8; 32];
        let n = unsafe {
            libc::getxattr(path.as_ptr(), c"security.capability".as_ptr(), buf.as_mut_ptr().cast(), buf.len())
        };
        n > 0 && caps_allow_tun(&buf[..n as usize])
    }

    /// iOS (the phone app, where the core is embedded and never a file) has no
    /// file capabilities, and Apple's `getxattr` takes other arguments.
    #[cfg(target_os = "ios")]
    fn has_net_admin_cap(_core: &Path) -> bool {
        false
    }

    /// Whether a `vfs_cap_data` (revision 2, or 3 with a namespace root id)
    /// lets the core create TUN interfaces: CAP_NET_ADMIN must be permitted
    /// and raised on exec, since sing-box does not raise capabilities itself.
    #[cfg(not(target_os = "macos"))]
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    pub(crate) fn caps_allow_tun(xattr: &[u8]) -> bool {
        const CAP_NET_ADMIN: u32 = 12;
        const VFS_CAP_FLAGS_EFFECTIVE: u32 = 1;
        if xattr.len() < 12 {
            return false;
        }
        let word = |i: usize| u32::from_le_bytes(xattr[i * 4..i * 4 + 4].try_into().unwrap());
        let (magic_etc, permitted) = (word(0), word(1));
        magic_etc & VFS_CAP_FLAGS_EFFECTIVE != 0 && permitted & (1 << CAP_NET_ADMIN) != 0
    }

    /// The kernel ignores file capabilities and setuid on `nosuid` mounts
    /// (systemd-homed home directories, some encrypted homes).
    #[cfg(not(target_os = "macos"))]
    fn nosuid_mount(core: &Path) -> bool {
        use std::os::unix::ffi::OsStrExt as _;
        let Ok(path) = std::ffi::CString::new(core.as_os_str().as_bytes()) else { return false };
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        unsafe { libc::statvfs(path.as_ptr(), &mut st) == 0 && st.f_flag & libc::ST_NOSUID != 0 }
    }

    /// The command, run as root, that lets `core` create TUN interfaces: file
    /// capabilities where `setcap` is installed; otherwise the core becomes
    /// setuid root, as on macOS, which needs nothing beyond coreutils.
    #[cfg(not(target_os = "macos"))]
    fn grant_command(core: &Path) -> Vec<String> {
        let core = core.to_string_lossy().into_owned();
        match system_tool("setcap") {
            Some(setcap) => vec![
                setcap.to_string_lossy().into_owned(),
                "cap_net_admin,cap_net_raw,cap_net_bind_service+eip".into(),
                core,
            ],
            None => vec![
                "/bin/sh".into(),
                "-c".into(),
                "chown root:root \"$1\" && chmod u+s \"$1\"".into(),
                "sh".into(),
                core,
            ],
        }
    }

    /// The same grant as a line to paste into a terminal.
    #[cfg(not(target_os = "macos"))]
    pub fn grant_tun_privilege_hint(core: &Path) -> String {
        let quote = |a: &str| {
            if a.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+=,".contains(c)) {
                a.to_string()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        };
        let cmd = grant_command(core);
        format!("sudo {}", cmd.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "))
    }

    /// Asks for the administrator password through polkit. Without a polkit
    /// agent to ask (tiling window managers often run none), passwordless
    /// `sudo` is tried; failing that the error is [`NEEDS_PASSWORD`] and the
    /// caller asks for the password itself (see [`grant_tun_privilege_sudo`]).
    #[cfg(not(target_os = "macos"))]
    pub fn grant_tun_privilege(core: &Path) -> Result<()> {
        if nosuid_mount(core) {
            bail!("{} is on a filesystem mounted nosuid, where Linux ignores the TUN permission", core.display());
        }
        let cmd = grant_command(core);
        if let Some(pkexec) = system_tool("pkexec") {
            let out = Command::new(&pkexec).args(&cmd).output().context("run pkexec")?;
            if out.status.success() {
                return Ok(());
            }
            // 126: the dialog was dismissed. 127 covers both "no agent" and
            // "not authorized"; sudo may still work, so it gets a turn.
            if out.status.code() == Some(126) {
                bail!("the administrator prompt was cancelled");
            }
            log::warn!("pkexec: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        let Some(sudo) = system_tool("sudo") else {
            bail!("neither polkit (pkexec) nor sudo could ask for the administrator password");
        };
        let quiet = Command::new(sudo)
            .arg("-n")
            .arg("--")
            .args(&cmd)
            .stdin(std::process::Stdio::null())
            .output();
        if quiet.is_ok_and(|o| o.status.success()) {
            return Ok(());
        }
        bail!(NEEDS_PASSWORD)
    }

    /// Runs the grant through `sudo` with the password the user typed.
    #[cfg(not(target_os = "macos"))]
    pub fn grant_tun_privilege_sudo(core: &Path, password: &str) -> Result<()> {
        use std::io::Write as _;
        use std::process::Stdio;
        let sudo = system_tool("sudo").ok_or_else(|| anyhow::anyhow!("sudo is not installed"))?;
        let mut child = Command::new(sudo)
            // -S reads the password from stdin, -p '' drops the prompt.
            .args(["-S", "-p", "", "--"])
            .args(grant_command(core))
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("run sudo")?;
        if let Some(mut stdin) = child.stdin.take() {
            // Closed straight after, so a wrong password fails instead of waiting.
            let _ = stdin.write_all(format!("{password}\n").as_bytes());
        }
        let out = child.wait_with_output().context("run sudo")?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("incorrect password") || err.contains("Sorry, try again") || err.contains("no password was provided") {
            bail!(WRONG_PASSWORD);
        }
        let err: Vec<&str> = err.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        bail!("sudo: {}", err.join(" "))
    }

    // ------------------------------------------------------------ shared

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    /// Splits `"path with spaces" --flag` into arguments.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn split_command(cmd: &str) -> Vec<String> {
        let cmd = cmd.trim();
        match cmd.strip_prefix('"').and_then(|r| r.split_once('"')) {
            Some((exe, rest)) => std::iter::once(exe.to_string())
                .chain(rest.split_whitespace().map(String::from))
                .collect(),
            None => cmd.split_whitespace().map(String::from).collect(),
        }
    }

    pub fn is_elevated() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    pub fn relaunch_elevated(_args: &str) -> Result<()> {
        bail!("not supported on this platform")
    }

    pub fn set_elevated_login_task(_name: &str, _cmd: Option<(&str, &str)>) -> Result<()> {
        Ok(())
    }

    pub fn elevated_login_task_exists(_name: &str) -> bool {
        false
    }

    pub fn ui_locale() -> String {
        #[cfg(target_os = "macos")]
        if let Ok(out) = run("defaults", &["read", "-g", "AppleLanguages"]) {
            if let Some(first) = out.split('"').nth(1) {
                return first.to_string();
            }
        }
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C"))
            .map(|v| v.split('.').next().unwrap_or(&v).replace('_', "-"))
            .unwrap_or_else(|| "en-US".into())
    }

    pub fn windows_build() -> u32 {
        0
    }

    /// The OS accent for light or dark mode (0xRRGGBB): macOS's accent
    /// colour, GNOME's accent-color, or KDE's AccentColor.
    pub fn accent_color(dark: bool) -> Option<u32> {
        #[cfg(target_os = "macos")]
        {
            // Apple's system colours, (light, dark). No key means Multicolor: blue.
            let index = run("defaults", &["read", "-g", "AppleAccentColor"])
                .ok()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(4);
            let (light, dark_c) = match index {
                -1 => (0x8E8E93, 0x98989D),
                0 => (0xFF3B30, 0xFF453A),
                1 => (0xFF9500, 0xFF9F0A),
                2 => (0xFFCC00, 0xFFD60A),
                3 => (0x34C759, 0x30D158),
                5 => (0xAF52DE, 0xBF5AF2),
                6 => (0xFF2D55, 0xFF375F),
                _ => (0x007AFF, 0x0A84FF),
            };
            Some(if dark { dark_c } else { light })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = dark;
            // GNOME 47+: one of libadwaita's named accents.
            if let Ok(v) = run("gsettings", &["get", "org.gnome.desktop.interface", "accent-color"]) {
                let c = match v.trim().trim_matches('\'') {
                    "blue" => Some(0x3584E4),
                    "teal" => Some(0x2190A4),
                    "green" => Some(0x3A944A),
                    "yellow" => Some(0xC88800),
                    "orange" => Some(0xED5B00),
                    "red" => Some(0xE62D42),
                    "pink" => Some(0xD56199),
                    "purple" => Some(0x9141AC),
                    "slate" => Some(0x6F8396),
                    _ => None,
                };
                if c.is_some() {
                    return c;
                }
            }
            // KDE: [General] AccentColor=r,g,b in kdeglobals.
            let home = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
            let text = std::fs::read_to_string(home.join("kdeglobals")).ok()?;
            let value = text.lines().find_map(|l| l.trim().strip_prefix("AccentColor="))?;
            let mut parts = value.split(',').map(|p| p.trim().parse::<u32>().ok());
            let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
            Some((r & 0xFF) << 16 | (g & 0xFF) << 8 | (b & 0xFF))
        }
    }

    /// GNOME's "Large text" / text-scaling-factor; macOS has no global setting.
    pub fn system_text_scale() -> f32 {
        #[cfg(target_os = "macos")]
        {
            1.0
        }
        #[cfg(not(target_os = "macos"))]
        {
            run("gsettings", &["get", "org.gnome.desktop.interface", "text-scaling-factor"])
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .map(|f| f.clamp(0.5, 3.0))
                .unwrap_or(1.0)
        }
    }

    /// Kills a core left running by a previous crash, if `pid` is still a sing-box.
    pub fn kill_stale_core(pid: u32) {
        #[cfg(target_os = "linux")]
        let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        #[cfg(not(target_os = "linux"))]
        let name = run("ps", &["-p", &pid.to_string(), "-o", "comm="]).unwrap_or_default();
        if name.contains("sing-box") {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }

    pub struct KillOnCloseJob;

    impl KillOnCloseJob {
        pub fn new() -> Option<Self> {
            None
        }
    }
}

pub use imp::*;

#[cfg(any(target_os = "macos", test))]
mod darwin_proc {
    use std::net::IpAddr;

    const XSO_SOCKET: u32 = 0x001;
    const XSO_INPCB: u32 = 0x010;

    // Fixed header field offsets within XNU's `#pragma pack(4)` `xinpcb_n` and `xsocket_n`.
    // Walking records via their self-describing `x_len` / `x_kind` TLV headers makes this
    // immune to total record size changes (e.g. macOS 27 / Darwin 26).
    const INPCB_FOREIGN_PORT: usize = 16;
    const INPCB_LOCAL_PORT: usize = 18;
    const INPCB_VFLAG: usize = 44;
    const INPCB_FOREIGN_ADDR: usize = 48;
    const INPCB_LOCAL_ADDR: usize = 64;
    const INPCB_IPV4_OFF: usize = 12;
    const INPCB_MIN_LEN: usize = 80;

    const XSOCKET_LAST_PID: usize = 68;
    const XSOCKET_EFFECTIVE_PID: usize = 72;
    const XSOCKET_MIN_LEN: usize = 76;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PcbEntry {
        pub local_addr: IpAddr,
        pub local_port: u16,
        pub remote_addr: IpAddr,
        pub remote_port: u16,
        pub pid: u32,
        pub effective_pid: u32,
    }

    fn normalize_ip(ip: IpAddr) -> IpAddr {
        match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
            v4 => v4,
        }
    }

    fn parse_inpcb(rec: &[u8]) -> Option<(IpAddr, u16, IpAddr, u16)> {
        if rec.len() < INPCB_MIN_LEN {
            return None;
        }
        let remote_port = u16::from_be_bytes([rec[INPCB_FOREIGN_PORT], rec[INPCB_FOREIGN_PORT + 1]]);
        let local_port = u16::from_be_bytes([rec[INPCB_LOCAL_PORT], rec[INPCB_LOCAL_PORT + 1]]);
        let vflag = rec[INPCB_VFLAG];
        if vflag & 0x01 != 0 {
            let f = INPCB_FOREIGN_ADDR + INPCB_IPV4_OFF;
            let l = INPCB_LOCAL_ADDR + INPCB_IPV4_OFF;
            let remote = IpAddr::from([rec[f], rec[f + 1], rec[f + 2], rec[f + 3]]);
            let local = IpAddr::from([rec[l], rec[l + 1], rec[l + 2], rec[l + 3]]);
            Some((local, local_port, remote, remote_port))
        } else if vflag & 0x02 != 0 {
            let f: [u8; 16] = rec[INPCB_FOREIGN_ADDR..INPCB_FOREIGN_ADDR + 16].try_into().ok()?;
            let l: [u8; 16] = rec[INPCB_LOCAL_ADDR..INPCB_LOCAL_ADDR + 16].try_into().ok()?;
            Some((normalize_ip(IpAddr::from(l)), local_port, normalize_ip(IpAddr::from(f)), remote_port))
        } else {
            None
        }
    }

    pub fn parse_pcblist_tlv(buf: &[u8]) -> Vec<PcbEntry> {
        if buf.len() < 24 {
            return Vec::new();
        }
        let hdr_len = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if hdr_len < 16 || hdr_len > buf.len() {
            return Vec::new();
        }
        let mut entries = Vec::new();
        let mut pending_inpcb: Option<(IpAddr, u16, IpAddr, u16)> = None;
        let mut pending_pids: Option<(u32, u32)> = None;

        let flush = |entries: &mut Vec<PcbEntry>,
                     inpcb: &mut Option<(IpAddr, u16, IpAddr, u16)>,
                     pids: &mut Option<(u32, u32)>| {
            if let (Some((local_addr, local_port, remote_addr, remote_port)), Some((pid, effective_pid))) =
                (inpcb.take(), pids.take())
            {
                if pid != 0 || effective_pid != 0 {
                    entries.push(PcbEntry {
                        local_addr,
                        local_port,
                        remote_addr,
                        remote_port,
                        pid,
                        effective_pid,
                    });
                }
            } else {
                *inpcb = None;
                *pids = None;
            }
        };

        let mut pos = hdr_len;
        while pos + 8 <= buf.len() {
            let rec_len = u32::from_ne_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]) as usize;
            let rec_kind = u32::from_ne_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]);
            if rec_len < 8 || pos + rec_len > buf.len() {
                break;
            }
            // Trailing `xinpgen` terminator has the same length as the header and sits at EOF.
            if rec_len == hdr_len && pos + rec_len == buf.len() {
                break;
            }
            let rec = &buf[pos..pos + rec_len];
            match rec_kind {
                XSO_INPCB => {
                    if pending_inpcb.is_some() {
                        flush(&mut entries, &mut pending_inpcb, &mut pending_pids);
                    }
                    pending_inpcb = parse_inpcb(rec);
                }
                XSO_SOCKET => {
                    if rec.len() >= XSOCKET_MIN_LEN {
                        let pid = u32::from_ne_bytes([
                            rec[XSOCKET_LAST_PID],
                            rec[XSOCKET_LAST_PID + 1],
                            rec[XSOCKET_LAST_PID + 2],
                            rec[XSOCKET_LAST_PID + 3],
                        ]);
                        let epid = u32::from_ne_bytes([
                            rec[XSOCKET_EFFECTIVE_PID],
                            rec[XSOCKET_EFFECTIVE_PID + 1],
                            rec[XSOCKET_EFFECTIVE_PID + 2],
                            rec[XSOCKET_EFFECTIVE_PID + 3],
                        ]);
                        pending_pids = Some((pid, epid));
                    }
                }
                _ => {}
            }
            pos += rec_len;
        }
        flush(&mut entries, &mut pending_inpcb, &mut pending_pids);
        entries
    }

    pub fn match_entry(
        entries: &[PcbEntry],
        src_ip: Option<IpAddr>,
        src_port: u16,
        dst_ip: Option<IpAddr>,
        dst_port: Option<u16>,
    ) -> Option<&PcbEntry> {
        let src_ip = src_ip.map(normalize_ip);
        let dst_ip = dst_ip.map(normalize_ip);
        let mut local_match: Option<&PcbEntry> = None;
        let mut port_match: Option<&PcbEntry> = None;
        for e in entries {
            if e.local_port != src_port {
                continue;
            }
            if let (Some(si), Some(di), Some(dp)) = (src_ip, dst_ip, dst_port) {
                if e.local_addr == si && e.remote_addr == di && e.remote_port == dp {
                    return Some(e);
                }
            }
            if local_match.is_none() && src_ip.is_some_and(|si| e.local_addr == si) {
                local_match = Some(e);
            }
            if port_match.is_none() {
                port_match = Some(e);
            }
        }
        local_match.or(port_match)
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct LsofEntry {
        pub pid: u32,
        pub cmd: String,
        pub local_ip: Option<IpAddr>,
        pub local_port: u16,
    }

    fn unescape_lsof_cmd(raw: &str) -> String {
        let bytes = raw.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1] == b'x' {
                if let Ok(hex) = std::str::from_utf8(&bytes[i + 2..i + 4]) {
                    if let Ok(val) = u8::from_str_radix(hex, 16) {
                        out.push(val);
                        i += 4;
                        continue;
                    }
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn parse_lsof_endpoint(endpoint: &str) -> Option<(Option<IpAddr>, u16)> {
        let local = endpoint.split("->").next()?.trim();
        let (host, port_s) = local.rsplit_once(':')?;
        let port = port_s.parse::<u16>().ok()?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let ip = if host == "*" {
            None
        } else {
            host.split('%').next().and_then(|s| s.parse::<IpAddr>().ok()).map(normalize_ip)
        };
        Some((ip, port))
    }

    pub fn parse_lsof(output: &str) -> Vec<LsofEntry> {
        let mut entries = Vec::new();
        let mut pid = 0u32;
        let mut cmd = String::new();
        for line in output.lines() {
            let Some(first) = line.as_bytes().first() else { continue };
            let rest = &line[1..];
            match first {
                b'p' => {
                    pid = rest.parse().unwrap_or(0);
                    cmd.clear();
                }
                b'c' => {
                    cmd = unescape_lsof_cmd(rest);
                }
                b'n' if pid != 0 => {
                    if let Some((local_ip, local_port)) = parse_lsof_endpoint(rest) {
                        entries.push(LsofEntry { pid, cmd: cmd.clone(), local_ip, local_port });
                    }
                }
                _ => {}
            }
        }
        entries
    }

    pub fn match_lsof(entries: &[LsofEntry], src_ip: Option<IpAddr>, src_port: u16) -> Option<(u32, &str)> {
        let src_ip = src_ip.map(normalize_ip);
        let mut fallback: Option<(u32, &str)> = None;
        for e in entries {
            if e.local_port != src_port {
                continue;
            }
            if src_ip.is_some() && e.local_ip == src_ip {
                return Some((e.pid, e.cmd.as_str()));
            }
            if fallback.is_none() {
                fallback = Some((e.pid, e.cmd.as_str()));
            }
        }
        fallback
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn push_record(buf: &mut Vec<u8>, kind: u32, len: usize, fill: impl FnOnce(&mut [u8])) {
            let start = buf.len();
            buf.resize(start + len, 0);
            buf[start..start + 4].copy_from_slice(&(len as u32).to_ne_bytes());
            buf[start + 4..start + 8].copy_from_slice(&kind.to_ne_bytes());
            fill(&mut buf[start..start + len]);
        }

        #[test]
        fn walks_tlv_pcblist_even_when_kernel_struct_sizes_grow() {
            let mut buf = vec![0u8; 24];
            buf[0..4].copy_from_slice(&24u32.to_ne_bytes());

            // Simulate a future macOS 27 kernel where xinpcb_n = 120, xsocket_n = 128,
            // and extra xsockbuf/xtcpcb records total 320 bytes (well beyond sing-box's 408+208).
            push_record(&mut buf, XSO_INPCB, 120, |r| {
                r[INPCB_FOREIGN_PORT..INPCB_FOREIGN_PORT + 2].copy_from_slice(&7890u16.to_be_bytes());
                r[INPCB_LOCAL_PORT..INPCB_LOCAL_PORT + 2].copy_from_slice(&54321u16.to_be_bytes());
                r[INPCB_VFLAG] = 0x01;
                r[INPCB_FOREIGN_ADDR + INPCB_IPV4_OFF..INPCB_FOREIGN_ADDR + INPCB_IPV4_OFF + 4]
                    .copy_from_slice(&[127, 0, 0, 1]);
                r[INPCB_LOCAL_ADDR + INPCB_IPV4_OFF..INPCB_LOCAL_ADDR + INPCB_IPV4_OFF + 4]
                    .copy_from_slice(&[127, 0, 0, 1]);
            });
            push_record(&mut buf, XSO_SOCKET, 128, |r| {
                r[XSOCKET_LAST_PID..XSOCKET_LAST_PID + 4].copy_from_slice(&4242u32.to_ne_bytes());
                r[XSOCKET_EFFECTIVE_PID..XSOCKET_EFFECTIVE_PID + 4].copy_from_slice(&4242u32.to_ne_bytes());
            });
            push_record(&mut buf, 0x020, 320, |_| {}); // enlarged XSO_TCPCB
            // Second IPv6 socket (e.g. TUN connection).
            push_record(&mut buf, XSO_INPCB, 120, |r| {
                r[INPCB_FOREIGN_PORT..INPCB_FOREIGN_PORT + 2].copy_from_slice(&443u16.to_be_bytes());
                r[INPCB_LOCAL_PORT..INPCB_LOCAL_PORT + 2].copy_from_slice(&60000u16.to_be_bytes());
                r[INPCB_VFLAG] = 0x02;
                r[INPCB_FOREIGN_ADDR + 15] = 2;
                r[INPCB_LOCAL_ADDR + 15] = 1;
            });
            push_record(&mut buf, XSO_SOCKET, 128, |r| {
                r[XSOCKET_LAST_PID..XSOCKET_LAST_PID + 4].copy_from_slice(&9001u32.to_ne_bytes());
            });
            // Trailing xinpgen terminator.
            let end = buf.len();
            buf.resize(end + 24, 0);
            buf[end..end + 4].copy_from_slice(&24u32.to_ne_bytes());

            let entries = parse_pcblist_tlv(&buf);
            assert_eq!(entries.len(), 2);

            // System proxy (mixed-in): remote in Clash API is 1.1.1.1:443 while local socket remote is 127.0.0.1:7890.
            let m = match_entry(
                &entries,
                Some("127.0.0.1".parse().unwrap()),
                54321,
                Some("1.1.1.1".parse().unwrap()),
                Some(443),
            )
            .expect("should match mixed-in loopback socket by local (ip, port)");
            assert_eq!(m.pid, 4242);

            let m2 = match_entry(&entries, Some("::1".parse().unwrap()), 60000, Some("::2".parse().unwrap()), Some(443))
                .expect("should match IPv6 4-tuple");
            assert_eq!(m2.pid, 9001);
        }

        #[test]
        fn parses_lsof_machine_output_and_unescapes_spaces() {
            let sample = "p1234\ncGoogle\\x20Chrome\nn127.0.0.1:54321->127.0.0.1:7890\np5678\ncSafari\nn[::1]:61234->[::1]:7890\n";
            let entries = parse_lsof(sample);
            assert_eq!(entries.len(), 2);
            assert_eq!(match_lsof(&entries, Some("127.0.0.1".parse().unwrap()), 54321), Some((1234, "Google Chrome")));
            assert_eq!(match_lsof(&entries, Some("::1".parse().unwrap()), 61234), Some((5678, "Safari")));
        }
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::imp::{caps_allow_tun, grant_tun_privilege_hint};

    /// `setcap cap_net_admin,cap_net_raw,cap_net_bind_service+eip`, as read
    /// back with `getfattr -e hex` (revision 3: set in a user namespace).
    const EIP_V3: &str = "0100000300340000003400000000000000000000e8030000";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn reads_file_capabilities() {
        assert!(caps_allow_tun(&hex(EIP_V3)));
        // Revision 2, effective, CAP_NET_ADMIN permitted.
        assert!(caps_allow_tun(&hex("0100000200100000000000000000000000000000")));
        // Permitted but not raised on exec (`cap_net_admin+p`).
        assert!(!caps_allow_tun(&hex("0000000200100000000000000000000000000000")));
        // Raised, but only CAP_NET_RAW.
        assert!(!caps_allow_tun(&hex("0100000200200000000000000000000000000000")));
        assert!(!caps_allow_tun(&[]));
        assert!(!caps_allow_tun(&hex("01000002")));
    }

    #[test]
    fn terminal_hint_quotes_the_path() {
        let hint = grant_tun_privilege_hint(std::path::Path::new("/home/a b/it's/sing-box"));
        assert!(hint.starts_with("sudo "), "{hint}");
        assert!(hint.ends_with(r"'/home/a b/it'\''s/sing-box'"), "{hint}");
        let hint = grant_tun_privilege_hint(std::path::Path::new("/home/me/.config/BoxPilot/core/sing-box"));
        assert!(hint.ends_with(" /home/me/.config/BoxPilot/core/sing-box"), "{hint}");
    }
}
