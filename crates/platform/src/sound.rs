use std::io;

#[repr(C, align(4))]
struct Wave([u8; include_bytes!("../../../assets/warning.wav").len()]);
static WARNING: Wave = Wave(*include_bytes!("../../../assets/warning.wav"));

/// Called on a worker thread; the daemon's timer and event loop remain responsive.
#[cfg(target_os = "linux")]
pub fn play_warning() -> io::Result<()> {
	use std::{
		io::Write,
		process::{Command, Stdio},
	};
	let mut child = Command::new("pw-play")
		.arg("-")
		.stdin(Stdio::piped())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()?;
	let result = match child.stdin.take() {
		Some(mut stdin) => stdin.write_all(&WARNING.0),
		None => Err(io::Error::other("pw-play stdin was not piped")),
	};
	if let Err(error) = result {
		let _ = child.kill();
		let _ = child.wait();
		return Err(error);
	}
	if !child.wait()?.success() {
		return Err(io::Error::other("pw-play failed to play the warning"));
	}
	Ok(())
}

#[cfg(windows)]
pub fn play_warning() -> io::Result<()> {
	use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
	// SAFETY: the 4-byte-aligned static buffer holds a valid RIFF/WAVE and remains alive for playback.
	let success = unsafe {
		PlaySoundW(
			WARNING.0.as_ptr().cast(),
			std::ptr::null_mut(),
			SND_MEMORY | SND_NODEFAULT | SND_SYNC,
		)
	};
	if success == 0 {
		return Err(io::Error::other("PlaySoundW failed to play the warning"));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	#[test]
	fn embedded_warning_has_valid_pcm_wave_lengths() {
		let bytes = &super::WARNING.0;
		assert_eq!(&bytes[..4], b"RIFF");
		assert_eq!(&bytes[8..12], b"WAVE");
		assert_eq!(&bytes[36..40], b"data");
		assert_eq!(
			u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8,
			bytes.len()
		);
		assert_eq!(
			u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize + 44,
			bytes.len()
		);
		assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 1);
		assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);
	}
}
