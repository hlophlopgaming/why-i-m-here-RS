use super::*;
use std::{
	fs,
	os::{
		fd::{AsRawFd, FromRawFd, OwnedFd},
		unix::fs::MetadataExt,
	},
};
pub(super) struct Process {
	fd: OwnedFd,
	pub parent: u32,
	pub started: u64,
	pub parent_started: Option<u64>,
	pub safe: bool,
}
fn stat(value: &str) -> io::Result<(u32, u64)> {
	let fields: Vec<_> = value
		.rsplit_once(')')
		.ok_or_else(|| io::Error::other("invalid proc stat"))?
		.1
		.split_whitespace()
		.collect();
	let parent = fields.get(1).and_then(|s| s.parse().ok());
	let started = fields.get(19).and_then(|s| s.parse().ok());
	parent
		.zip(started)
		.ok_or_else(|| io::Error::other("invalid proc process identity"))
}
impl Process {
	pub fn open(pid: u32) -> io::Result<Self> {
		// SAFETY: pidfd_open pins one process; the returned descriptor is owned below.
		let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
		if raw < 0 {
			return Err(io::Error::last_os_error());
		}
		// SAFETY: Successful pidfd_open returned a new, uniquely owned descriptor.
		let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
		let path = format!("/proc/{pid}");
		let (parent, started) = stat(&fs::read_to_string(format!("{path}/stat"))?)?;
		let name = fs::read_link(format!("{path}/exe"))
			.ok()
			.and_then(|path| path.file_name().map(|s| s.to_string_lossy().into_owned()));
		// SAFETY: geteuid has no arguments or side effects.
		let same_user = fs::metadata(&path)?.uid() == unsafe { libc::geteuid() };
		let parent_started = fs::read_to_string(format!("/proc/{parent}/stat"))
			.ok()
			.and_then(|s| stat(&s).ok())
			.map(|(_, time)| time);
		let process = Self {
			fd,
			parent,
			started,
			parent_started,
			safe: pid > 1
				&& same_user && name.is_some()
				&& ![
					"kwin_wayland",
					"kwin_x11",
					"plasmashell",
					"Xwayland",
					"dbus-daemon",
					"dbus-broker",
					"systemd",
				]
				.contains(&name.as_deref().unwrap_or("").trim_end_matches(" (deleted)")),
		};
		if !process.alive() {
			return Err(io::Error::other("process exited while being pinned"));
		}
		Ok(process)
	}
	pub fn alive(&self) -> bool {
		let mut poll = libc::pollfd {
			fd: self.fd.as_raw_fd(),
			events: libc::POLLIN,
			revents: 0,
		};
		// SAFETY: One valid pollfd, zero timeout, owned pidfd remains open.
		unsafe { libc::poll(&mut poll, 1, 0) == 0 }
	}
	pub fn list() -> io::Result<BTreeMap<u32, Self>> {
		let mut processes = BTreeMap::new();
		for entry in fs::read_dir("/proc")? {
			let entry = entry?;
			if let Ok(pid) = entry.file_name().to_string_lossy().parse() {
				if let Ok(process) = Self::open(pid) {
					processes.insert(pid, process);
				}
			}
		}
		Ok(processes)
	}
	pub fn kill(&self) -> io::Result<()> {
		// SAFETY: Signal is addressed only through the owned pidfd, never through a reused PID.
		if unsafe {
			libc::syscall(
				libc::SYS_pidfd_send_signal,
				self.fd.as_raw_fd(),
				libc::SIGKILL,
				std::ptr::null::<libc::siginfo_t>(),
				0,
			)
		} < 0
		{
			let error = io::Error::last_os_error();
			if error.raw_os_error() != Some(libc::ESRCH) {
				return Err(error);
			}
		}
		Ok(())
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn proc_stat_allows_parentheses_and_spaces_in_name() {
		let value = format!("42 (some ) name) S 12 {} 456 0", "0 ".repeat(17));
		assert_eq!(stat(&value).unwrap(), (12, 456));
		assert!(stat("broken").is_err());
	}
}
