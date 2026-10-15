use std::io;

use thiserror::Error;

pub mod applications;
pub mod autostart;
pub mod desktop;
pub mod instance;
pub mod settings;
pub mod sound;
pub mod windows;
pub use windows::WindowBackend;

#[derive(Debug, Error)]
pub enum PlatformError {
	#[error(transparent)]
	Io(#[from] io::Error),
	#[error("the platform clock returned an invalid value")]
	Clock,
}

/// Milliseconds since boot, including time spent suspended.
#[cfg(target_os = "linux")]
pub fn boottime_ms() -> Result<u64, PlatformError> {
	let mut time = libc::timespec {
		tv_sec: 0,
		tv_nsec: 0,
	};
	// SAFETY: time points to a valid writable timespec; CLOCK_BOOTTIME takes no other resources.
	if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } != 0 {
		return Err(io::Error::last_os_error().into());
	}
	let seconds = u64::try_from(time.tv_sec).map_err(|_| PlatformError::Clock)?;
	let nanos = u64::try_from(time.tv_nsec).map_err(|_| PlatformError::Clock)?;
	seconds
		.checked_mul(1000)
		.and_then(|ms| ms.checked_add(nanos / 1_000_000))
		.ok_or(PlatformError::Clock)
}

#[cfg(windows)]
pub fn boottime_ms() -> Result<u64, PlatformError> {
	let mut time = 0_u64;
	// SAFETY: QueryInterruptTime writes one u64 to this valid pointer and cannot fail.
	unsafe { windows_sys::Win32::System::WindowsProgramming::QueryInterruptTime(&mut time) };
	Ok(time / 10_000)
}

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("why-here supports Linux and Windows only");

#[cfg(test)]
mod tests {
	#[test]
	fn boot_clock_is_monotonic() {
		let before = super::boottime_ms().unwrap();
		let after = super::boottime_ms().unwrap();
		assert!(after >= before);
	}
}
