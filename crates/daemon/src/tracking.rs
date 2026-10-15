use crate::{
	background::{Event, Events},
	UiProcess,
};
use anyhow::{Context, Result};
use platform::{
	instance::Instance,
	windows::{Backend, WindowBackend},
};
use std::collections::{BTreeMap, BTreeSet};
use why_core::{
	engine::{Command, Engine},
	ipc::{self, DaemonMessage, Screen, UiMessage, UiState},
	settings::{EndMode, Settings},
};

#[derive(Clone, Copy, PartialEq)]
enum Phase {
	Goal,
	Timer,
	Closing,
}
struct Session {
	ui: Option<UiProcess>,
	phase: Phase,
	state: UiState,
	warned: bool,
}

pub(crate) struct Tracking {
	backend: Option<Backend>,
	engine: Engine,
	sessions: BTreeMap<String, Session>,
	settings: Settings,
	events: Events,
	pub restricted: bool,
	next_tick_ms: u64,
}
impl Tracking {
	pub fn new(settings: Settings, events: Events) -> Self {
		Self {
			backend: None,
			engine: Engine::default(),
			sessions: BTreeMap::new(),
			settings,
			events,
			restricted: false,
			next_tick_ms: 0,
		}
	}
	pub fn available(&self) -> bool {
		self.backend.is_some()
	}
	pub fn ticking(&self) -> bool {
		self.sessions
			.values()
			.any(|session| session.phase == Phase::Timer)
	}
	pub fn configure(&mut self, instance: &Instance, settings: Settings) {
		self.settings = settings;
		let selected: BTreeSet<_> = if self.settings.tracking_enabled {
			self.settings
				.tracked_apps
				.iter()
				.map(|app| app.id.clone())
				.collect()
		} else {
			BTreeSet::new()
		};
		self.sessions.retain(|id, _| {
			if selected.contains(id) {
				true
			} else {
				self.engine.forget(id);
				false
			}
		});
		// Restore the old backend's windows before replacing its hooks/script.
		self.backend = None;
		if selected.is_empty() {
			return;
		}
		let events = self.events.clone();
		match Backend::new(
			instance,
			&self.settings.tracked_apps,
			self.blocked(),
			move || {
				events.send(Event::WindowsChanged);
			},
		) {
			Ok(backend) => {
				self.backend = Some(backend);
				if let Err(error) = self.refresh() {
					eprintln!("window tracking: {error:#}");
				}
				if let Some(backend) = self.backend.as_mut() {
					for (id, session) in &self.sessions {
						if session.phase == Phase::Goal {
							if let Some(ui) = &session.ui {
								backend.focus_goal(id, ui.child.id());
							}
						}
					}
				}
			}
			Err(error) => eprintln!("connect window tracking: {error}"),
		}
	}
	fn blocked(&self) -> BTreeSet<String> {
		self.settings
			.tracked_apps
			.iter()
			.filter(|app| {
				self.sessions
					.get(&app.id)
					.is_none_or(|session| session.phase == Phase::Goal)
			})
			.map(|app| app.id.clone())
			.collect()
	}
	fn apply(&mut self) -> Result<()> {
		let blocked = self.blocked();
		let active = self
			.sessions
			.values()
			.any(|session| session.phase != Phase::Closing);
		if let Some(backend) = self.backend.as_mut() {
			backend.set_blocked(blocked, active)?;
		}
		Ok(())
	}
	pub fn refresh(&mut self) -> Result<()> {
		let Some(backend) = self.backend.as_mut() else {
			return Ok(());
		};
		let windows = backend.snapshot()?;
		self.restricted = windows.iter().any(|window| !window.controllable);
		let live: BTreeSet<_> = windows
			.iter()
			.filter(|window| window.controllable)
			.map(|window| window.app_id.clone())
			.collect();
		self.sessions.retain(|id, _| {
			if live.contains(id) {
				true
			} else {
				self.engine.forget(id);
				false
			}
		});
		for id in live {
			if self.sessions.contains_key(&id) {
				continue;
			}
			let Some(app) = self
				.settings
				.tracked_apps
				.iter()
				.find(|app| app.id == id)
				.cloned()
			else {
				continue;
			};
			self.engine.observe(&id, EndMode::CloseWindows)?;
			let mut settings = self.settings.clone();
			settings.end_mode = EndMode::CloseWindows;
			let state = UiState {
				demo: false,
				backend_available: true,
				backend_restricted: false,
				app,
				goal: String::new(),
				settings,
			};
			match self.spawn(Screen::Goal, &state) {
				Ok(ui) => {
					if let Some(backend) = self.backend.as_mut() {
						backend.focus_goal(&id, ui.child.id());
					}
					self.sessions.insert(
						id,
						Session {
							ui: Some(ui),
							phase: Phase::Goal,
							state,
							warned: false,
						},
					);
				}
				Err(error) => {
					self.engine.forget(&id);
					// A failed UI must never leave a program trapped behind an invisible question.
					self.sessions.insert(
						id,
						Session {
							ui: None,
							phase: Phase::Closing,
							state,
							warned: false,
						},
					);
					eprintln!("show goal: {error:#}");
				}
			}
		}
		self.apply()
	}
	fn spawn(&self, screen: Screen, state: &UiState) -> Result<UiProcess> {
		let events = self.events.clone();
		UiProcess::spawn_notifying(
			screen,
			state,
			Some(Box::new(move |pid, message| {
				events.send(Event::Ui(pid, message));
			})),
		)
	}
	pub fn owns_ui(&self, pid: u32) -> bool {
		self.sessions
			.values()
			.any(|session| session.ui.as_ref().is_some_and(|ui| ui.child.id() == pid))
	}
	pub fn message(
		&mut self,
		pid: u32,
		message: Result<Option<UiMessage>, ipc::IpcError>,
	) -> Result<()> {
		let Some(id) = self
			.sessions
			.iter()
			.find(|(_, session)| session.ui.as_ref().is_some_and(|ui| ui.child.id() == pid))
			.map(|(id, _)| id.clone())
		else {
			return Ok(());
		};
		let phase = self.sessions[&id].phase;
		if phase == Phase::Goal {
			if let Ok(Some(UiMessage::GoalSubmitted {
				goal,
				duration_secs,
			})) = message
			{
				let now = match platform::boottime_ms() {
					Ok(now) => now,
					Err(error) => {
						self.finish(&id)?;
						return Err(error.into());
					}
				};
				if let Err(error) = self.engine.submit_goal(&id, &goal, duration_secs, now) {
					self.finish(&id)?;
					return Err(error.into());
				}
				let state = {
					let session = self.sessions.get_mut(&id).context("missing session")?;
					session.ui = None;
					session.phase = Phase::Timer;
					session.state.goal = goal.trim().to_owned();
					session.state.settings.duration_secs = duration_secs;
					session.state.clone()
				};
				self.apply()?;
				match self.spawn(Screen::Timer, &state) {
					Ok(ui) => self.sessions.get_mut(&id).context("missing session")?.ui = Some(ui),
					Err(error) => eprintln!("show timer: {error:#}"),
				}
			} else {
				self.finish(&id)?;
			}
		} else if phase == Phase::Timer {
			if matches!(message, Ok(Some(UiMessage::EndNow))) {
				self.finish(&id)?;
			} else {
				self.sessions.get_mut(&id).context("missing session")?.ui = None;
			}
		}
		Ok(())
	}
	fn finish(&mut self, id: &str) -> Result<()> {
		self.engine.forget(id);
		if let Some(session) = self.sessions.get_mut(id) {
			if session.phase == Phase::Timer && !session.warned && session.state.settings.play_sound
			{
				if let Err(error) = platform::sound::play_warning() {
					eprintln!("warning sound: {error}");
				}
			}
			session.phase = Phase::Closing;
			session.ui = None;
		}
		if let Some(backend) = self.backend.as_mut() {
			backend.close_app(id)?;
		}
		self.apply()
	}
	pub fn tick(&mut self) -> Result<()> {
		let now = platform::boottime_ms()?;
		if now < self.next_tick_ms {
			return Ok(());
		}
		self.next_tick_ms = now.saturating_add(1000);
		for command in self.engine.tick(now) {
			if let Command::Finish { app_id, .. } = command {
				if let Err(error) = self.finish(&app_id) {
					eprintln!("close session {app_id}: {error:#}");
				}
			}
		}
		for (id, session) in &mut self.sessions {
			if session.phase != Phase::Timer {
				continue;
			}
			let Some(remaining) = self
				.engine
				.session(id)
				.and_then(|session| session.remaining_ms(now))
			else {
				continue;
			};
			let remaining_secs = remaining.div_ceil(1000);
			let warning = session.state.settings.warning_secs > 0
				&& remaining_secs <= u64::from(session.state.settings.warning_secs);
			if warning && !session.warned {
				session.warned = true;
				if session.state.settings.play_sound {
					if let Err(error) = platform::sound::play_warning() {
						eprintln!("warning sound: {error}");
					}
				}
			}
			if let Some(ui) = session.ui.as_mut() {
				if let Err(error) = ui.send(&DaemonMessage::Tick {
					remaining_secs,
					warning,
				}) {
					eprintln!("timer UI: {error:#}");
					session.ui = None;
				}
			}
		}
		Ok(())
	}
}
