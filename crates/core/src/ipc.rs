use std::io::{self, BufRead, Write};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::settings::{Settings, TrackedApp};

pub const PROTOCOL_VERSION: u16 = 6;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
	Goal,
	Timer,
	Reminder,
	Settings,
}

impl Screen {
	pub fn argument(self) -> &'static str {
		match self {
			Self::Goal => "goal",
			Self::Timer => "timer",
			Self::Reminder => "reminder",
			Self::Settings => "settings",
		}
	}
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UiState {
	pub demo: bool,
	#[serde(default)]
	pub backend_available: bool,
	#[serde(default)]
	pub backend_restricted: bool,
	pub app: TrackedApp,
	pub goal: String,
	pub settings: Settings,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonMessage {
	Present,
	SettingsSaved {
		success: bool,
	},
	Init {
		screen: Screen,
		state: UiState,
	},
	Tick {
		remaining_secs: u64,
		warning: bool,
	},
	InstalledApps {
		apps: Vec<TrackedApp>,
		complete: bool,
		error: Option<String>,
	},
}

/// Borrowed catalog response; wire-compatible with `DaemonMessage::InstalledApps`.
#[derive(Serialize)]
#[serde(tag = "type", rename = "installed_apps")]
pub struct InstalledApps<'a> {
	pub apps: &'a [TrackedApp],
	pub complete: bool,
	pub error: Option<&'a str>,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiMessage {
	SaveSettings { settings: Settings },
	RequestInstalledApps,
	GoalSubmitted { goal: String, duration_secs: u32 },
	GoalCancelled,
	StartSession { settings: Settings, app_id: String },
	EndNow,
	Retry,
	Closed,
}

#[derive(Debug, Deserialize, Serialize)]
struct Envelope<T> {
	version: u16,
	payload: T,
}

#[derive(Debug, Error)]
pub enum IpcError {
	#[error(transparent)]
	Io(#[from] io::Error),
	#[error(transparent)]
	Json(#[from] serde_json::Error),
	#[error("unsupported IPC version: {0}")]
	Version(u16),
	#[error("IPC message exceeds 64 KiB or is missing its newline")]
	Framing,
}

pub fn read_message<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>, IpcError> {
	let mut line = Vec::new();
	// A malformed peer must not force unbounded allocation before validation.
	let count =
		std::io::Read::take(reader, (MAX_MESSAGE_BYTES + 1) as u64).read_until(b'\n', &mut line)?;
	if count == 0 {
		return Ok(None);
	}
	if count > MAX_MESSAGE_BYTES || line.last() != Some(&b'\n') {
		return Err(IpcError::Framing);
	}
	let envelope: Envelope<T> = serde_json::from_slice(&line)?;
	if envelope.version != PROTOCOL_VERSION {
		return Err(IpcError::Version(envelope.version));
	}
	Ok(Some(envelope.payload))
}

pub fn write_message<T: Serialize>(writer: &mut impl Write, payload: &T) -> Result<(), IpcError> {
	let mut line = serde_json::to_vec(&Envelope {
		version: PROTOCOL_VERSION,
		payload,
	})?;
	line.push(b'\n');
	if line.len() > MAX_MESSAGE_BYTES {
		return Err(IpcError::Framing);
	}
	writer.write_all(&line)?;
	writer.flush()?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn borrowed_catalog_fits_frame_and_matches_owned_response() {
		let apps = vec![
			TrackedApp {
				id: "\\".repeat(512),
				name: "😀".repeat(128)
			};
			16
		];
		assert!(apps.iter().all(TrackedApp::is_valid));
		for error in [None, Some("catalog unavailable")] {
			let mut bytes = Vec::new();
			write_message(
				&mut bytes,
				&InstalledApps {
					apps: &apps,
					complete: true,
					error,
				},
			)
			.unwrap();
			let Some(DaemonMessage::InstalledApps {
				apps: actual,
				complete,
				error: actual_error,
			}) = read_message(&mut bytes.as_slice()).unwrap()
			else {
				panic!("wrong response")
			};
			assert_eq!(actual, apps);
			assert!(complete);
			assert_eq!(actual_error.as_deref(), error);
		}
	}

	#[test]
	fn round_trip_keeps_embedded_newlines_in_one_frame() {
		let message = UiMessage::GoalSubmitted {
			goal: "Read\nnotes".into(),
			duration_secs: 5,
		};
		let mut bytes = Vec::new();
		write_message(&mut bytes, &message).unwrap();
		assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
		let mut reader = bytes.as_slice();
		assert_eq!(
			read_message::<UiMessage>(&mut reader).unwrap(),
			Some(message)
		);
		assert_eq!(read_message::<UiMessage>(&mut reader).unwrap(), None);
	}

	#[test]
	fn rejects_wrong_version_truncated_and_oversized_messages() {
		let mut previous = &b"{\"version\":2,\"payload\":{\"type\":\"closed\"}}\n"[..];
		assert!(matches!(
			read_message::<UiMessage>(&mut previous),
			Err(IpcError::Version(2))
		));
		let mut bytes = &b"{\"version\":99,\"payload\":{\"type\":\"closed\"}}\n"[..];
		assert!(matches!(
			read_message::<UiMessage>(&mut bytes),
			Err(IpcError::Version(99))
		));
		let mut truncated = &b"{}"[..];
		assert!(matches!(
			read_message::<UiMessage>(&mut truncated),
			Err(IpcError::Framing)
		));
		let bytes = vec![b' '; MAX_MESSAGE_BYTES + 1];
		assert!(matches!(
			read_message::<UiMessage>(&mut bytes.as_slice()),
			Err(IpcError::Framing)
		));
	}

	#[test]
	fn persistent_settings_save_and_acknowledgement_round_trip() {
		let message = UiMessage::SaveSettings {
			settings: Settings::default(),
		};
		let mut bytes = Vec::new();
		write_message(&mut bytes, &message).unwrap();
		assert_eq!(
			read_message::<UiMessage>(&mut bytes.as_slice()).unwrap(),
			Some(message)
		);
		for success in [false, true] {
			let mut bytes = Vec::new();
			write_message(&mut bytes, &DaemonMessage::SettingsSaved { success }).unwrap();
			assert!(
				matches!(read_message::<DaemonMessage>(&mut bytes.as_slice()).unwrap(), Some(DaemonMessage::SettingsSaved { success: actual }) if actual == success)
			);
		}
	}

	#[test]
	fn dashboard_selection_round_trips_all_settings() {
		let settings = Settings {
			language: crate::i18n::Language::En,
			duration_secs: 300,
			warning_secs: 60,
			play_sound: true,
			tracked_apps: vec![TrackedApp {
				id: "demo.editor.desktop".into(),
				name: "Editor".into(),
			}],
			..Settings::default()
		};
		let message = UiMessage::StartSession {
			settings,
			app_id: "demo.editor.desktop".into(),
		};
		let mut bytes = Vec::new();
		write_message(&mut bytes, &message).unwrap();
		assert_eq!(
			read_message::<UiMessage>(&mut bytes.as_slice()).unwrap(),
			Some(message)
		);
	}
}
