//! Running the sing-box executable.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

use crate::config::CoreVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const ALL: [LogLevel; 5] = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "TRACE" => LogLevel::Trace,
            "DEBUG" => LogLevel::Debug,
            "INFO" => LogLevel::Info,
            "WARN" | "WARNING" => LogLevel::Warn,
            "ERROR" | "FATAL" | "PANIC" => LogLevel::Error,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub time: String,
    pub level: LogLevel,
    pub message: String,
}

impl LogLine {
    pub fn app(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            level,
            message: message.into(),
        }
    }
}

/// Parses `+0800 2026-10-26 12:00:00 INFO [123 0ms] router: ...`.
pub fn parse_log_line(raw: &str) -> LogLine {
    let tokens: Vec<&str> = raw.split_whitespace().take(5).collect();
    for (i, t) in tokens.iter().enumerate() {
        if let Some(level) = LogLevel::parse(t.split('[').next().unwrap_or(t)) {
            let time = if i > 0 { tokens[i - 1] } else { "" };
            let time = if time.contains(':') {
                time.to_string()
            } else {
                chrono::Local::now().format("%H:%M:%S").to_string()
            };
            let idx = raw.find(t).map(|p| p + t.len()).unwrap_or(0);
            let mut message = raw[idx..].trim();
            // Drop the "[connection-id duration]" prefix.
            if message.starts_with('[') {
                if let Some(end) = message.find(']') {
                    message = message[end + 1..].trim();
                }
            }
            return LogLine {
                time,
                level,
                message: message.to_string(),
            };
        }
    }
    LogLine::app(LogLevel::Info, raw.trim())
}

fn command(core: &Path) -> Command {
    let mut cmd = Command::new(core);
    #[cfg(windows)]
    cmd.creation_flags(crate::sys::CREATE_NO_WINDOW);
    cmd.stdin(Stdio::null());
    cmd
}

pub async fn detect_version(core: &Path) -> Result<CoreVersion> {
    let out = command(core)
        .arg("version")
        .output()
        .await
        .with_context(|| format!("run {}", core.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    CoreVersion::parse(&text).with_context(|| format!("unrecognized version output: {text}"))
}

/// Validates a config; returns the core's error message on failure.
pub async fn check_config(core: &Path, config: &Path, workdir: &Path) -> Result<()> {
    let out = command(core)
        .arg("check")
        .arg("-c")
        .arg(config)
        .arg("-D")
        .arg(workdir)
        .arg("--disable-color")
        .output()
        .await
        .context("run sing-box check")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err
            .lines()
            .map(|l| parse_log_line(l).message)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        bail!("{}", if msg.is_empty() { "configuration check failed".into() } else { msg });
    }
    Ok(())
}

pub fn spawn_core(core: &Path, config: &Path, workdir: &Path) -> Result<Child> {
    command(core)
        .arg("run")
        .arg("-c")
        .arg(config)
        .arg("-D")
        .arg(workdir)
        .arg("--disable-color")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("start {}", core.display()))
}

/// Forwards each output line of the child to `sink`.
pub fn pipe_lines<R>(reader: R, sink: impl Fn(LogLine) + Send + 'static)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                sink(parse_log_line(&line));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sing_box_lines() {
        let l = parse_log_line("+0800 2026-10-26 12:00:01 INFO [3514528287 0ms] inbound/mixed[mixed-in]: inbound connection to www.google.com:443");
        assert_eq!(l.level, LogLevel::Info);
        assert_eq!(l.time, "12:00:01");
        assert_eq!(l.message, "inbound/mixed[mixed-in]: inbound connection to www.google.com:443");
        let l = parse_log_line("FATAL[0000] start service: bind: address already in use");
        assert_eq!(l.level, LogLevel::Error);
        assert_eq!(l.message, "start service: bind: address already in use");
        let l = parse_log_line("ERROR router: rule-set not found");
        assert_eq!(l.level, LogLevel::Error);
    }
}
