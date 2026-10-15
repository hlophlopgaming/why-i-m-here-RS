use super::*;
use windows_sys::{
	core::w,
	Win32::{
		Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
		System::Registry::*,
	},
};

pub(super) const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
struct Key(HKEY);
impl Drop for Key {
	fn drop(&mut self) {
		// SAFETY: the key is owned, opened successfully and closed exactly once.
		unsafe { RegCloseKey(self.0) };
	}
}
fn check(status: u32) -> io::Result<()> {
	if status == ERROR_SUCCESS {
		Ok(())
	} else {
		Err(io::Error::from_raw_os_error(status as i32))
	}
}

pub(super) fn configure(subkey: &str, enabled: bool, executable: &Path) -> io::Result<()> {
	let command = if enabled {
		let command = format!("\"{}\" --background", executable_text(executable)?);
		if command.contains('\0') || command.encode_utf16().count() > 260 {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"autostart command exceeds 260 characters",
			));
		}
		Some(command.encode_utf16().chain(Some(0)).collect::<Vec<_>>())
	} else {
		None
	};
	let subkey: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
	let mut handle = std::ptr::null_mut();
	// SAFETY: constant current-user root and live NUL-terminated key; output points to an HKEY.
	let status = unsafe {
		if enabled {
			RegCreateKeyExW(
				HKEY_CURRENT_USER,
				subkey.as_ptr(),
				0,
				std::ptr::null(),
				0,
				KEY_SET_VALUE,
				std::ptr::null(),
				&mut handle,
				std::ptr::null_mut(),
			)
		} else {
			RegOpenKeyExW(
				HKEY_CURRENT_USER,
				subkey.as_ptr(),
				0,
				KEY_SET_VALUE,
				&mut handle,
			)
		}
	};
	if !enabled && status == ERROR_FILE_NOT_FOUND {
		return Ok(());
	}
	check(status)?;
	let key = Key(handle);
	// SAFETY: owned key and constant value name; command buffer includes its UTF-16 NUL.
	let status = unsafe {
		match command {
			Some(command) => RegSetValueExW(
				key.0,
				w!("WhyHere"),
				0,
				REG_SZ,
				command.as_ptr().cast(),
				(command.len() * 2) as u32,
			),
			None => RegDeleteValueW(key.0, w!("WhyHere")),
		}
	};
	if !enabled && status == ERROR_FILE_NOT_FOUND {
		return Ok(());
	}
	check(status)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn registration_uses_private_registry_key() {
		let subkey = format!(r"Software\WhyHereTests\Autostart{}", std::process::id());
		let executable = Path::new(r"C:\Program Files\Тест\why-here.exe");
		configure(&subkey, true, executable).unwrap();
		let wide: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
		let mut data = [0u16; 261];
		let mut size = std::mem::size_of_val(&data) as u32;
		// SAFETY: private test key, fixed output buffer and size in bytes.
		check(unsafe {
			RegGetValueW(
				HKEY_CURRENT_USER,
				wide.as_ptr(),
				w!("WhyHere"),
				RRF_RT_REG_SZ,
				std::ptr::null_mut(),
				data.as_mut_ptr().cast(),
				&mut size,
			)
		})
		.unwrap();
		assert_eq!(
			String::from_utf16(&data[..(size / 2 - 1) as usize]).unwrap(),
			format!("\"{}\" --background", executable.display())
		);
		configure(&subkey, false, executable).unwrap();
		configure(&subkey, false, executable).unwrap();
		// SAFETY: removes only the unique key created by this test; never touches Run.
		check(unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide.as_ptr()) }).unwrap();
		assert!(configure(
			&subkey,
			true,
			Path::new(&format!(r"C:\{}\why-here.exe", "x".repeat(260)))
		)
		.is_err());
	}
}
