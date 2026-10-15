use std::{collections::BTreeSet, io};
use why_core::{settings::TrackedApp, windows::WindowInfo};

#[cfg(target_os = "linux")]
mod kwin;
#[cfg(target_os = "linux")]
pub use kwin::Backend;
#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use win32::Backend;

pub trait WindowBackend {
	fn snapshot(&mut self) -> io::Result<Vec<WindowInfo>>;
	fn set_blocked(&mut self, apps: BTreeSet<String>, active: bool) -> io::Result<()>;
	fn close_app(&mut self, app_id: &str) -> io::Result<()>;
	fn focus_goal(&mut self, app_id: &str, pid: u32);
}
