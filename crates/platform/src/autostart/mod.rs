use std::{io, path::Path};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

/// Called only by the persistent settings save, never by the demo or startup.
pub fn set_enabled(enabled: bool) -> io::Result<()> {
	#[cfg(target_os = "linux")]
	let path = crate::settings::settings_path()
		.map_err(io::Error::other)?
		.parent()
		.and_then(Path::parent)
		.ok_or_else(|| io::Error::other("missing configuration directory"))?
		.join("autostart/org.local.WhyHere.desktop");
	let executable = std::env::current_exe()?;
	#[cfg(target_os = "linux")]
	return linux::configure(&path, enabled, &executable);
	#[cfg(windows)]
	windows::configure(windows::RUN_KEY, enabled, &executable)
}

fn executable_text(path: &Path) -> io::Result<&str> {
	path.to_str()
		.filter(|text| path.is_absolute() && !text.chars().any(char::is_control))
		.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid executable path"))
}
