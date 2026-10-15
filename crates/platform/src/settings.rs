use std::{
	fs::{self, File, OpenOptions},
	io::{self, Read, Write},
	path::{Path, PathBuf},
	sync::atomic::{AtomicU64, Ordering},
};

use thiserror::Error;
use why_core::settings::{Settings, ValidationError};

const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum SettingsError {
	#[error(transparent)]
	Io(#[from] io::Error),
	#[error(transparent)]
	Json(#[from] serde_json::Error),
	#[error(transparent)]
	Validation(#[from] ValidationError),
	#[error("settings exceed 1 MiB")]
	TooLarge,
	#[error("cannot locate the user configuration directory")]
	ConfigDirectory,
}

pub fn settings_path() -> Result<PathBuf, SettingsError> {
	#[cfg(target_os = "linux")]
	let directory = std::env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.filter(|path| path.is_absolute())
		.or_else(|| {
			std::env::var_os("HOME")
				.map(PathBuf::from)
				.filter(|path| path.is_absolute())
				.map(|path| path.join(".config"))
		})
		.map(|path| path.join("why-here"));
	#[cfg(windows)]
	let directory = std::env::var_os("APPDATA")
		.map(PathBuf::from)
		.filter(|path| path.is_absolute())
		.map(|path| path.join("WhyHere"));
	directory
		.map(|path| path.join("settings.json"))
		.ok_or(SettingsError::ConfigDirectory)
}

fn read_document(path: &Path) -> Result<Option<serde_json::Value>, SettingsError> {
	let file = match File::open(path) {
		Ok(file) => file,
		Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
		Err(error) => return Err(error.into()),
	};
	let mut bytes = Vec::new();
	file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
	if bytes.len() as u64 > MAX_SETTINGS_BYTES {
		return Err(SettingsError::TooLarge);
	}
	let document: serde_json::Value = serde_json::from_slice(&bytes)?;
	let settings: Settings = serde_json::from_value(document.clone())?;
	settings.validate()?;
	Ok(Some(document))
}

pub fn load(path: &Path) -> Result<Settings, SettingsError> {
	match read_document(path)? {
		Some(document) => Ok(serde_json::from_value(document)?),
		None => Ok(Settings::default()),
	}
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
	settings.validate()?;
	// Preserve fields from newer versions and refuse to overwrite a damaged file.
	let mut document = read_document(path)?.unwrap_or_else(|| serde_json::json!({}));
	if let (Some(existing), serde_json::Value::Object(updated)) =
		(document.as_object_mut(), serde_json::to_value(settings)?)
	{
		existing.extend(updated);
	}
	let bytes = serde_json::to_vec_pretty(&document)?;
	if bytes.len() as u64 + 1 > MAX_SETTINGS_BYTES {
		return Err(SettingsError::TooLarge);
	}
	let mut bytes = bytes;
	bytes.push(b'\n');
	write_atomic(path, &bytes)?;
	Ok(())
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
	let directory = path
		.parent()
		.ok_or_else(|| io::Error::other("missing parent directory"))?;
	fs::create_dir_all(directory)?;
	let temporary = directory.join(format!(
		".settings-{}-{}.tmp",
		std::process::id(),
		NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
	));
	let mut options = OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(target_os = "linux")]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}
	let mut file = options.open(&temporary)?;
	let result = (|| {
		file.write_all(bytes)?;
		file.sync_all()?;
		drop(file);
		replace(&temporary, path)
	})();
	if result.is_err() {
		let _ = fs::remove_file(&temporary);
	}
	result
}

#[cfg(target_os = "linux")]
fn replace(source: &Path, destination: &Path) -> io::Result<()> {
	fs::rename(source, destination)
}

#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> io::Result<()> {
	use std::os::windows::ffi::OsStrExt;
	use windows_sys::Win32::Storage::FileSystem::{
		MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
	};
	let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
	let destination: Vec<u16> = destination
		.as_os_str()
		.encode_wide()
		.chain(Some(0))
		.collect();
	// SAFETY: both paths are NUL-terminated UTF-16 buffers alive for the call.
	if unsafe {
		MoveFileExW(
			source.as_ptr(),
			destination.as_ptr(),
			MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
		)
	} == 0
	{
		return Err(io::Error::last_os_error());
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	struct TempDirectory(PathBuf);
	impl TempDirectory {
		fn new() -> Self {
			let path = std::env::temp_dir().join(format!(
				"why-here-settings-{}-{}",
				std::process::id(),
				NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
			));
			fs::create_dir(&path).unwrap();
			Self(path)
		}
	}
	impl Drop for TempDirectory {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}

	#[test]
	fn defaults_old_files_and_round_trip_preserve_history_ids_and_unknown_fields() {
		let directory = TempDirectory::new();
		let path = directory.0.join("nested/settings.json");
		assert_eq!(load(&path).unwrap(), Settings::default());
		assert!(!path.exists());
		save(&path, &Settings::default()).unwrap();
		fs::write(&path, r#"{"duration_secs":120,"future":{"keep":true}}"#).unwrap();
		let mut settings = load(&path).unwrap();
		assert_eq!(settings.duration_secs, 120);
		assert_eq!(settings.warning_secs, 120);
		settings.remember_goal("Разобраться с настройками").unwrap();
		settings.tracked_apps.push(why_core::settings::TrackedApp {
			id: "editor.desktop.desktop".into(),
			name: "Редактор".into(),
		});
		save(&path, &settings).unwrap();
		assert_eq!(load(&path).unwrap(), settings);
		let document: serde_json::Value =
			serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
		assert_eq!(document["future"]["keep"], true);
		assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
		#[cfg(target_os = "linux")]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(
				fs::metadata(&path).unwrap().permissions().mode() & 0o777,
				0o600
			);
		}
	}

	#[test]
	fn bad_input_and_failed_replace_leave_existing_data_untouched() {
		let directory = TempDirectory::new();
		let path = directory.0.join("settings.json");
		for bytes in [b"{broken".as_slice(), br#"{"duration_secs":0}"#, b"null"] {
			fs::write(&path, bytes).unwrap();
			assert!(load(&path).is_err());
			assert!(save(&path, &Settings::default()).is_err());
			assert_eq!(fs::read(&path).unwrap(), bytes);
		}
		fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES as usize + 1]).unwrap();
		assert!(matches!(load(&path), Err(SettingsError::TooLarge)));
		fs::remove_file(&path).unwrap();
		save(&path, &Settings::default()).unwrap();
		let before = fs::read(&path).unwrap();
		let invalid = Settings {
			duration_secs: 0,
			..Settings::default()
		};
		assert!(save(&path, &invalid).is_err());
		assert_eq!(fs::read(&path).unwrap(), before);
		let temporary = directory.0.join("replacement.tmp");
		fs::write(&temporary, "new").unwrap();
		assert!(replace(&temporary, &directory.0).is_err());
		assert_eq!(fs::read(&path).unwrap(), before);
	}
}
