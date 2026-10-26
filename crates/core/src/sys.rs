//! OS integration: system proxy, launch at login, elevation, child process lifetime.

use anyhow::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SysProxyState {
    pub enabled: bool,
    pub server: String,
}

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

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn set_system_proxy(_server: &str, _bypass: &str) -> Result<()> {
        anyhow::bail!("system proxy is only implemented on Windows")
    }
    pub fn clear_system_proxy() -> Result<()> {
        Ok(())
    }
    pub fn system_proxy_state() -> SysProxyState {
        SysProxyState::default()
    }
    pub fn set_launch_at_login(_name: &str, _command: Option<&str>) -> Result<()> {
        anyhow::bail!("launch at login is only implemented on Windows")
    }
    pub fn launch_at_login_enabled(_name: &str) -> bool {
        false
    }
    pub fn is_elevated() -> bool {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        unsafe { geteuid() == 0 }
    }
    pub fn relaunch_elevated(_args: &str) -> Result<()> {
        anyhow::bail!("run the application with sudo to use TUN mode")
    }
    pub fn set_elevated_login_task(_name: &str, _cmd: Option<(&str, &str)>) -> Result<()> {
        Ok(())
    }
    pub fn elevated_login_task_exists(_name: &str) -> bool {
        false
    }
    pub fn ui_locale() -> String {
        std::env::var("LANG").unwrap_or_else(|_| "en-US".into())
    }
    pub struct KillOnCloseJob;
    impl KillOnCloseJob {
        pub fn new() -> Option<Self> {
            None
        }
    }
}

pub use imp::*;
