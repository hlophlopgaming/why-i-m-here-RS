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

enum Event {
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
	fn send(&self, event: Event) -> bool {
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

fn receive(receiver: &Receiver<Event>) -> Result<Option<Event>> {
	#[cfg(target_os = "linux")]
	return Ok(Some(receiver.recv().context("daemon event queue stopped")?));
	#[cfg(windows)]
	loop {
		match receiver.try_recv() {
			Ok(event) => return Ok(Some(event)),
			Err(TryRecvError::Disconnected) => anyhow::bail!("daemon event queue stopped"),
			Err(TryRecvError::Empty) => {
				if !platform::desktop::wait_message()? {
					return Ok(None);
				}
			}
		}
	}
}

fn open_settings(path: &Path, language: Option<Language>, events: &Events) -> Result<UiProcess> {
	let mut settings = platform::settings::load(path)?;
	if let Some(language) = language {
		settings.language = language;
	}
	let state = UiState {
		demo: false,
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
	let Some(_instance) =
		Instance::claim(initial, move |request| instance_events.request(request))?
	else {
		return Ok(());
	};
	let path = platform::settings::settings_path()?;
	let tray_language = language.unwrap_or(platform::settings::load(&path)?.language);
	let _tray = tray::Tray::new(tray_language, events.clone()).context("create system tray")?;
	let mut ui: Option<UiProcess> = None;
	let mut catalog_requested = false;
	events.request(initial);
	while let Some(event) = receive(&receiver)? {
		match event {
			Event::Request(Request::Quit) => break,
			Event::Request(Request::Background) => {}
			Event::Request(Request::Show(language)) => {
				let result = if let Some(ui) = ui.as_mut() {
					platform::desktop::allow_ui_foreground(ui.child.id());
					ui.send(&DaemonMessage::Present)
				} else {
					open_settings(&path, language, &events).map(|process| {
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
						let result = platform::settings::save(&path, &settings);
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
	}
	// Drop only our UI child before unregistering the tray and instance endpoint.
	drop(ui);
	Ok(())
}
