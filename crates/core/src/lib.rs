#![forbid(unsafe_code)]

pub mod engine;
pub mod i18n;
pub mod ipc;
pub mod settings;
pub mod windows;

pub const APP_ID: &str = "org.local.WhyHere";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
