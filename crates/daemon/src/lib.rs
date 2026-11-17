#![forbid(unsafe_code)]

mod background;
mod tracking;
mod tray;

pub use background::run;

use std::{
	io::BufReader,
	process::{Child, ChildStdin, Command, Stdio},
	sync::{
		mpsc::{self, Receiver, RecvTimeoutError},
		Arc, Mutex,
	},
	thread,
	time::Duration,
};

use anyhow::{bail, Context, Result};
use why_core::{
	engine::Engine,
	i18n::tr,
	ipc::{self, DaemonMessage, Screen, UiMessage, UiState},
	settings::{Settings, TrackedApp},
};

type UiNotification = Box<dyn Fn(u32, Result<Option<UiMessage>, ipc::IpcError>) + Send>;

struct UiProcess {
	child: Child,
	language: why_core::i18n::Language,
	stdin: Arc<Mutex<ChildStdin>>,
	events: Receiver<Result<Option<UiMessage>, ipc::IpcError>>,
}

enum Event {
	Message(UiMessage),
	Timeout,
}

impl UiProcess {
	fn spawn(screen: Screen, state: &UiState) -> Result<Self> {
		Self::spawn_notifying(screen, state, None)
	}

	fn spawn_notifying(
		screen: Screen,
		state: &UiState,
		notify: Option<UiNotification>,
	) -> Result<Self> {
		let mut child = Command::new(std::env::current_exe()?)
			.args(["ui", screen.argument()])
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::inherit())
			.spawn()
			.context("start UI process")?;
		platform::desktop::allow_ui_foreground(child.id());
		let pid = child.id();
		let Some(stdin) = child.stdin.take() else {
			let _ = child.kill();
			let _ = child.wait();
			bail!("UI stdin was not piped");
		};
		let Some(stdout) = child.stdout.take() else {
			let _ = child.kill();
			let _ = child.wait();
			bail!("UI stdout was not piped");
		};
		let (sender, events) = mpsc::channel();
		let mut process = Self {
			child,
			language: state.settings.language,
			stdin: Arc::new(Mutex::new(stdin)),
			events,
		};
		thread::Builder::new()
			.name("ui-ipc".into())
			.spawn(move || {
				let mut reader = BufReader::new(stdout);
				loop {
					let message = ipc::read_message(&mut reader);
					let keep_reading = matches!(
						&message,
						Ok(Some(
							UiMessage::RequestInstalledApps | UiMessage::SaveSettings { .. }
						))
					);
					let delivered = if let Some(notify) = &notify {
						notify(pid, message);
						true
					} else {
						sender.send(message).is_ok()
					};
					if !delivered || !keep_reading {
						break;
					}
				}
			})?;
		process.send(&DaemonMessage::Init {
			screen,
			state: state.clone(),
		})?;
		Ok(process)
	}

	fn send(&mut self, message: &DaemonMessage) -> Result<()> {
		let mut stdin = self
			.stdin
			.lock()
			.map_err(|_| anyhow::anyhow!("UI stdin lock poisoned"))?;
		ipc::write_message(&mut *stdin, message).context("send UI message")
	}

	fn load_catalog(&self, language: why_core::i18n::Language) -> Result<()> {
		let stdin = self.stdin.clone();
		thread::Builder::new()
			.name("installed-apps".into())
			.spawn(move || {
				let result = platform::applications::installed_apps(language);
				let Ok(mut stdin) = stdin.lock() else {
					return;
				};
				match result {
					Ok(apps) => {
						for chunk in apps.chunks(16) {
							if ipc::write_message(
								&mut *stdin,
								&ipc::InstalledApps {
									apps: chunk,
									complete: false,
									error: None,
								},
							)
							.is_err()
							{
								return;
							}
						}
						let _ = ipc::write_message(
							&mut *stdin,
							&ipc::InstalledApps {
								apps: &[],
								complete: true,
								error: None,
							},
						);
					}
					Err(error) => {
						eprintln!("installed apps: {error}");
						let _ = ipc::write_message(
							&mut *stdin,
							&ipc::InstalledApps {
								apps: &[],
								complete: true,
								error: Some("catalog unavailable"),
							},
						);
					}
				}
			})?;
		Ok(())
	}

	fn receive(&self, timeout: Option<Duration>) -> Result<Event> {
		let received = if let Some(timeout) = timeout {
			match self.events.recv_timeout(timeout) {
				Ok(message) => message,
				Err(RecvTimeoutError::Timeout) => return Ok(Event::Timeout),
				Err(RecvTimeoutError::Disconnected) => bail!("UI reader stopped"),
			}
		} else {
			self.events.recv().context("UI reader stopped")?
		};
		match received? {
			Some(message) => Ok(Event::Message(message)),
			None => bail!("UI closed without a response"),
		}
	}
}

impl Drop for UiProcess {
	fn drop(&mut self) {
		// Reap only our own UI child, including on IPC or clock failures.
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

#[derive(Default)]
struct SoundCues {
	announced: bool,
	last_countdown: Option<u64>,
}
impl SoundCues {
	fn due(&mut self, settings: &Settings, seconds: u64) -> bool {
		if !settings.play_sound || seconds == 0 {
			return false;
		}
		let warning = !self.announced && seconds <= 60;
		self.announced |= warning;
		let countdown = seconds <= 10 && self.last_countdown.is_none_or(|last| seconds < last);
		if countdown {
			self.last_countdown = Some(seconds);
		}
		warning || countdown
	}
	fn tick(&mut self, settings: &Settings, seconds: u64) {
		if !self.due(settings, seconds) {
			return;
		}
		if let Err(error) = thread::Builder::new()
			.name("warning-sound".into())
			.spawn(|| {
				if let Err(error) = platform::sound::play_warning() {
					eprintln!("warning sound: {error}");
				}
			}) {
			eprintln!("warning sound worker: {error}");
		}
	}
}

fn timer(engine: &mut Engine, state: &UiState) -> Result<bool> {
	let mut ui = UiProcess::spawn(Screen::Timer, state)?;
	let mut sounds = SoundCues::default();
	loop {
		let now = platform::boottime_ms()?;
		if !engine.tick(now).is_empty() {
			return Ok(true);
		}
		let remaining_ms = engine
			.session(&state.app.id)
			.and_then(|session| session.remaining_ms(now))
			.context("demo timer has no active session")?;
		let warning = remaining_ms.div_ceil(1000) <= u64::from(state.settings.warning_secs);
		sounds.tick(&state.settings, remaining_ms.div_ceil(1000));
		ui.send(&DaemonMessage::Tick {
			remaining_secs: remaining_ms.div_ceil(1000),
			warning,
		})?;
		match ui.receive(Some(Duration::from_millis(remaining_ms.min(1000))))? {
			Event::Timeout => {}
			Event::Message(UiMessage::EndNow) => {
				engine
					.finish_now(&state.app.id)
					.context("no running demo session")?;
				return Ok(true);
			}
			Event::Message(UiMessage::Closed) => {
				engine.forget(&state.app.id);
				return Ok(false);
			}
			Event::Message(message) => bail!("unexpected timer response: {message:?}"),
		}
	}
}

/// Runs only simulated sessions: no settings I/O and no window/process backend.
pub fn run_demo(mut settings: Settings, first_screen: Screen, single_screen: bool) -> Result<()> {
	settings.validate()?;
	let mut engine = Engine::default();
	let mut screen = first_screen;
	if settings.tracked_apps.is_empty() {
		settings.tracked_apps.push(TrackedApp {
			id: "org.local.WhyHere.Demo".into(),
			name: tr(settings.language, "Демонстрационная программа").into(),
		});
	}
	let mut state = UiState {
		demo: true,
		backend_available: false,
		backend_restricted: false,
		app: settings.tracked_apps[0].clone(),
		goal: if matches!(screen, Screen::Timer | Screen::Reminder) {
			tr(settings.language, "Прочитать документацию").into()
		} else {
			String::new()
		},
		settings: settings.clone(),
	};
	loop {
		if screen == Screen::Timer {
			if engine.session(&state.app.id).is_none() {
				engine.observe(&state.app.id, state.settings.end_mode)?;
				engine.submit_goal(
					&state.app.id,
					&state.goal,
					state.settings.duration_secs,
					platform::boottime_ms()?,
				)?;
			}
			if !timer(&mut engine, &state)? || single_screen {
				return Ok(());
			}
			screen = Screen::Reminder;
			continue;
		}
		if screen == Screen::Goal {
			engine.observe(&state.app.id, state.settings.end_mode)?;
		}
		let ui = UiProcess::spawn(screen, &state)?;
		let mut catalog_requested = false;
		let message = loop {
			match ui.receive(None)? {
				Event::Message(UiMessage::RequestInstalledApps) if screen == Screen::Settings => {
					if !catalog_requested {
						ui.load_catalog(state.settings.language)?;
						catalog_requested = true;
					}
				}
				Event::Message(message) => break message,
				Event::Timeout => bail!("unexpected timeout"),
			}
		};
		// Release the previous UI before opening another screen.
		drop(ui);
		match (screen, message) {
			(
				Screen::Goal,
				UiMessage::GoalSubmitted {
					goal,
					duration_secs,
				},
			) => {
				engine.submit_goal(
					&state.app.id,
					&goal,
					duration_secs,
					platform::boottime_ms()?,
				)?;
				settings.remember_goal(&goal)?;
				state.settings.goal_history = settings.goal_history.clone();
				state.goal = goal.trim().to_owned();
				state.settings.duration_secs = duration_secs;
				screen = Screen::Timer;
			}
			(Screen::Goal, UiMessage::GoalCancelled) => {
				engine.cancel_goal(&state.app.id)?;
				screen = Screen::Reminder;
			}
			(
				Screen::Settings,
				UiMessage::StartSession {
					settings: updated,
					app_id,
				},
			) => {
				updated.validate()?;
				if !updated.tracking_enabled {
					bail!("tracking is disabled");
				}
				state.app = updated
					.tracked_apps
					.iter()
					.find(|app| app.id == app_id)
					.context("untracked demo application")?
					.clone();
				settings = updated;
				state.settings = settings.clone();
				state.goal.clear();
				screen = Screen::Goal;
			}
			(Screen::Reminder, UiMessage::Retry) => screen = Screen::Goal,
			(Screen::Settings | Screen::Reminder, UiMessage::Closed) => return Ok(()),
			(_, message) => bail!("unexpected {screen:?} response: {message:?}"),
		}
		if single_screen {
			return Ok(());
		}
	}
}

#[cfg(test)]
mod sound_tests {
	use super::*;
	#[test]
	fn warning_and_countdown_are_once_per_remaining_second() {
		let settings = Settings {
			play_sound: true,
			..Settings::default()
		};
		let mut cues = SoundCues::default();
		assert!(!cues.due(&settings, 61));
		assert!(cues.due(&settings, 60));
		assert!(!cues.due(&settings, 60));
		assert!(!cues.due(&settings, 11));
		for seconds in (1..=10).rev() {
			assert!(cues.due(&settings, seconds));
			assert!(!cues.due(&settings, seconds));
		}
		assert!(!cues.due(&settings, 0));
		let mut cues = SoundCues::default();
		assert!(cues.due(&settings, 7));
		assert!(cues.due(&settings, 2));
		assert!(!cues.due(&settings, 3));
		let muted = Settings {
			play_sound: false,
			..settings
		};
		assert!(!SoundCues::default().due(&muted, 10));
	}
}
