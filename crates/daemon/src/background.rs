use crate::{tray, UiProcess};
use anyhow::{Context, Result};
use platform::instance::{Instance, Request};
#[cfg(windows)]
use std::sync::mpsc::TryRecvError;
use std::{
	path::Path,
	sync::mpsc::{self, Receiver, Sender},
};

use why_core::{
	i18n::{tr, Language},
	ipc::{self, DaemonMessage, Screen, UiMessage, UiState},
	settings::TrackedApp,
};

pub(crate) enum Event {
	WindowsChanged,
	Tick,
	Request(Request),
	Ui(u32, Result<Option<UiMessage>, ipc::IpcError>),
}

#[derive(Clone)]
pub(crate) struct Events {
	sender: Sender<Event>,
	#[cfg(windows)]
	thread: u32,
}
impl Events {
	pub(crate) fn send(&self, event: Event) -> bool {
		if self.sender.send(event).is_err() {
			return false;
		}
		#[cfg(windows)]
		return platform::desktop::wake(self.thread);
		#[cfg(not(windows))]
		true
	}
	pub fn request(&self, request: Request) -> bool {
		self.send(Event::Request(request))
	}
}

fn receive(receiver: &Receiver<Event>, ticking: bool) -> Result<Option<Event>> {
	#[cfg(target_os = "linux")]
	return if ticking {
		match receiver.recv_timeout(std::time::Duration::from_secs(1)) {
			Ok(event) => Ok(Some(event)),
			Err(mpsc::RecvTimeoutError::Timeout) => Ok(Some(Event::Tick)),
			Err(error) => Err(error).context("daemon event queue stopped"),
		}
	} else {
		Ok(Some(receiver.recv().context("daemon event queue stopped")?))
	};
	#[cfg(windows)]
	loop {
		match receiver.try_recv() {
			Ok(event) => return Ok(Some(event)),
			Err(TryRecvError::Disconnected) => anyhow::bail!("daemon event queue stopped"),
			Err(TryRecvError::Empty) => match platform::desktop::wait_message()? {
				platform::desktop::Message::Quit => return Ok(None),
				platform::desktop::Message::Tick if ticking => return Ok(Some(Event::Tick)),
				_ => {}
			},
		}
	}
}

fn open_settings(
	path: &Path,
	language: Option<Language>,
	events: &Events,
	backend_available: bool,
	restricted: bool,
) -> Result<UiProcess> {
	let mut settings = platform::settings::load(path)?;
	if let Some(language) = language {
		settings.language = language;
	}
	let state = UiState {
		demo: false,
		backend_available,
		backend_restricted: restricted,
		app: TrackedApp {
			id: why_core::APP_ID.into(),
			name: tr(settings.language, "Зачем я здесь?").into(),
		},
		goal: String::new(),
		settings,
	};
	let events = events.clone();
	UiProcess::spawn_notifying(
		Screen::Settings,
		&state,
		Some(Box::new(move |pid, message| {
			events.send(Event::Ui(pid, message));
		})),
	)
}

pub fn run(background: bool, language: Option<Language>) -> Result<()> {
	let (sender, receiver) = mpsc::channel();
	let events = Events {
		sender,
		#[cfg(windows)]
		thread: platform::desktop::message_thread(),
	};
	let initial = if background {
		Request::Background
	} else {
		Request::Show(language)
	};
	let instance_events = events.clone();
	let Some(instance) = Instance::claim(initial, move |request| instance_events.request(request))?
	else {
		return Ok(());
	};
	let path = platform::settings::settings_path()?;
	let tray_language = language.unwrap_or(platform::settings::load(&path)?.language);
	let _tray = tray::Tray::new(tray_language, events.clone()).context("create system tray")?;
	let settings = platform::settings::load(&path)?;
	let mut tracking = crate::tracking::Tracking::new(settings.clone(), events.clone());
	tracking.configure(&instance, settings);
	#[cfg(windows)]
	let mut timer = None;
	let mut ui: Option<UiProcess> = None;
	let mut catalog_requested = false;
	events.request(initial);
	loop {
		#[cfg(windows)]
		if tracking.ticking() && timer.is_none() {
			timer = Some(platform::desktop::Timer::new()?);
		} else if !tracking.ticking() {
			timer = None;
		}
		let Some(event) = receive(&receiver, tracking.ticking())? else {
			break;
		};
		match event {
			Event::WindowsChanged => {
				if let Err(error) = tracking.refresh() {
					eprintln!("window tracking: {error:#}");
				}
			}
			Event::Tick => {
				if let Err(error) = tracking.tick() {
					eprintln!("session tick: {error:#}");
				}
			}
			Event::Ui(pid, message) if tracking.owns_ui(pid) => {
				if let Err(error) = tracking.message(pid, message) {
					eprintln!("session UI: {error:#}");
				}
			}
			Event::Request(Request::Quit) => break,
			Event::Request(Request::Background) => {}
			Event::Request(Request::Show(language)) => {
				let result = if let Some(ui) = ui.as_mut() {
					platform::desktop::allow_ui_foreground(ui.child.id());
					ui.send(&DaemonMessage::Present)
				} else {
					open_settings(
						&path,
						language,
						&events,
						tracking.available(),
						tracking.restricted,
					)
					.map(|process| {
						ui = Some(process);
						catalog_requested = false;
					})
				};
				if let Err(error) = result {
					eprintln!("show settings: {error:#}");
					ui = None;
				}
			}
			Event::Ui(pid, message) => {
				let Some(process) = ui.as_mut().filter(|process| process.child.id() == pid) else {
					continue;
				};
				let result = match message {
					Ok(Some(UiMessage::RequestInstalledApps)) => {
						if catalog_requested {
							Ok(())
						} else {
							catalog_requested = true;
							process.load_catalog(process.language)
						}
					}
					Ok(Some(UiMessage::SaveSettings { settings })) => {
						let result =
							save_settings(&path, &settings, platform::autostart::set_enabled);
						if result.is_ok() {
							tracking.configure(&instance, settings);
						}
						if let Err(error) = &result {
							eprintln!("save settings {}: {error}", path.display());
						}
						process.send(&DaemonMessage::SettingsSaved {
							success: result.is_ok(),
						})
					}
					Ok(Some(UiMessage::Closed)) | Ok(None) => {
						ui = None;
						continue;
					}
					Ok(Some(message)) => {
						Err(anyhow::anyhow!("unexpected settings response: {message:?}"))
					}
					Err(error) => Err(error.into()),
				};
				if let Err(error) = result {
					eprintln!("settings UI: {error:#}");
					ui = None;
				}
			}
		}
		if tracking.ticking() {
			if let Err(error) = tracking.tick() {
				eprintln!("session tick: {error:#}");
			}
		}
	}
	// Drop only our UI child before unregistering the tray and instance endpoint.
	drop(ui);
	drop(tracking);
	Ok(())
}

fn save_settings(
	path: &Path,
	settings: &why_core::settings::Settings,
	configure: impl FnOnce(bool) -> std::io::Result<()>,
) -> Result<()> {
	let previous = platform::settings::load(path)?;
	platform::settings::save(path, settings)?;
	if let Err(error) = configure(settings.autostart) {
		platform::settings::save(path, &previous)
			.context("restore settings after autostart failure")?;
		return Err(error).context("configure autostart");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{fs, io};
	use why_core::settings::Settings;

	#[test]
	fn failed_autostart_restores_preferences_and_keeps_unknown_fields() {
		let directory =
			std::env::temp_dir().join(format!("why-here-save-autostart-{}", std::process::id()));
		fs::create_dir_all(&directory).unwrap();
		let path = directory.join("settings.json");
		fs::write(&path, r#"{"future":{"keep":true}}"#).unwrap();
		let previous = platform::settings::load(&path).unwrap();
		let updated = Settings {
			autostart: true,
			duration_secs: 300,
			..previous.clone()
		};
		assert!(save_settings(&path, &updated, |enabled| {
			assert!(enabled);
			Err(io::Error::new(
				io::ErrorKind::PermissionDenied,
				"test failure",
			))
		})
		.is_err());
		assert_eq!(platform::settings::load(&path).unwrap(), previous);
		let document: serde_json::Value =
			serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
		assert_eq!(document["future"]["keep"], true);
		save_settings(&path, &updated, |enabled| {
			assert!(enabled);
			Ok(())
		})
		.unwrap();
		assert_eq!(platform::settings::load(&path).unwrap(), updated);
		let invalid = Settings {
			duration_secs: 0,
			..updated
		};
		assert!(save_settings(&path, &invalid, |_| panic!(
			"must not configure invalid preferences"
		))
		.is_err());
		fs::remove_dir_all(directory).unwrap();
	}
}
