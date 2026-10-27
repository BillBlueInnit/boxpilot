//! BoxPilot core: subscription conversion, sing-box configuration and process
//! management. Contains no UI code.

pub mod clash_api;
pub mod config;
pub mod convert;
pub mod core_download;
pub mod engine;
pub mod fetch;
pub mod geo;
pub mod model;
pub mod process;
pub mod rules;
pub mod store;
pub mod speed;
pub mod sys;
pub mod update;

pub use engine::{CoreDownloadState, CoreStatus, Engine, Event, Msg, NoticeKind, ProxyGroup, ProxyMember};
