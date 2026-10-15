#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Tray;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Tray;

const ICON: &[u8; 32 * 32 * 4] = include_bytes!("../../../../assets/tray-icon.rgba");
