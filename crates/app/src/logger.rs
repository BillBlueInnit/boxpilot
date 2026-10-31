//! Minimal file logger (`<data>/boxpilot.log`, truncated at each launch).

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

struct FileLogger {
    file: Mutex<Option<File>>,
}

impl log::Log for FileLogger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        meta.level() <= log::Level::Info && meta.target().starts_with("boxpilot")
            // Which GPU the renderer picked, for reports of slow drawing.
            || meta.level() <= log::Level::Info && meta.target().starts_with("gpui_pre_wgpu")
            || meta.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}: {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            record.level(),
            record.target(),
            record.args()
        );
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
    }
}

pub fn init(dir: &Path) {
    let file = File::create(dir.join("boxpilot.log")).ok();
    let logger = Box::leak(Box::new(FileLogger {
        file: Mutex::new(file),
    }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    // A panic inside a window callback aborts the process, so write it down first.
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        log::error!("panic on thread {}: {info}", thread.name().unwrap_or("?"));
        log::logger().flush();
        default(info);
    }));
}
