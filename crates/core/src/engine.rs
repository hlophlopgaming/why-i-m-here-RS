use std::collections::BTreeMap;

use thiserror::Error;

use crate::settings::{validate_duration, validate_goal, EndMode, ValidationError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
	RequestGoal { app_id: String },
	Finish { app_id: String, mode: EndMode },
}

#[derive(Debug)]
pub struct Session {
	pub mode: EndMode,
	pub goal: Option<String>,
	deadline_ms: Option<u64>,
}

impl Session {
	pub fn remaining_ms(&self, now_ms: u64) -> Option<u64> {
		self.deadline_ms
			.map(|deadline| deadline.saturating_sub(now_ms))
	}
}

#[derive(Debug, Default)]
pub struct Engine {
	sessions: BTreeMap<String, Session>,
}

#[derive(Debug, Error)]
pub enum EngineError {
	#[error(transparent)]
	Validation(#[from] ValidationError),
	#[error("application ID is empty or too long")]
	AppId,
	#[error("no pending goal for this application")]
	NoPendingGoal,
	#[error("a goal cannot be cancelled in close-windows mode")]
	CannotCancel,
	#[error("session deadline overflow")]
	DeadlineOverflow,
}

impl Engine {
	/// Repeated observations (including additional windows) share one session.
	pub fn observe(&mut self, app_id: &str, mode: EndMode) -> Result<Option<Command>, EngineError> {
		if app_id.trim().is_empty() || app_id.len() > 512 {
			return Err(EngineError::AppId);
		}
		if self.sessions.contains_key(app_id) {
			return Ok(None);
		}
		self.sessions.insert(
			app_id.to_owned(),
			Session {
				mode,
				goal: None,
				deadline_ms: None,
			},
		);
		Ok(Some(Command::RequestGoal {
			app_id: app_id.to_owned(),
		}))
	}

	pub fn session(&self, app_id: &str) -> Option<&Session> {
		self.sessions.get(app_id)
	}

	pub fn submit_goal(
		&mut self,
		app_id: &str,
		goal: &str,
		duration_secs: u32,
		now_ms: u64,
	) -> Result<(), EngineError> {
		validate_goal(goal)?;
		validate_duration(duration_secs)?;
		let deadline = now_ms
			.checked_add(u64::from(duration_secs) * 1000)
			.ok_or(EngineError::DeadlineOverflow)?;
		let session = self
			.sessions
			.get_mut(app_id)
			.filter(|session| session.goal.is_none())
			.ok_or(EngineError::NoPendingGoal)?;
		session.goal = Some(goal.trim().to_owned());
		session.deadline_ms = Some(deadline);
		Ok(())
	}

	pub fn cancel_goal(&mut self, app_id: &str) -> Result<Command, EngineError> {
		let session = self
			.sessions
			.get(app_id)
			.filter(|session| session.goal.is_none())
			.ok_or(EngineError::NoPendingGoal)?;
		if session.mode != EndMode::Kill {
			return Err(EngineError::CannotCancel);
		}
		self.sessions.remove(app_id);
		Ok(Command::Finish {
			app_id: app_id.to_owned(),
			mode: EndMode::Kill,
		})
	}

	pub fn tick(&mut self, now_ms: u64) -> Vec<Command> {
		let mut commands = Vec::new();
		self.sessions.retain(|app_id, session| {
			if session.remaining_ms(now_ms) == Some(0) {
				commands.push(Command::Finish {
					app_id: app_id.clone(),
					mode: session.mode,
				});
				false
			} else {
				true
			}
		});
		commands
	}

	pub fn finish_now(&mut self, app_id: &str) -> Option<Command> {
		let session = self.sessions.get(app_id)?;
		session.goal.as_ref()?;
		let mode = session.mode;
		self.sessions.remove(app_id);
		Some(Command::Finish {
			app_id: app_id.to_owned(),
			mode,
		})
	}

	/// Disabling tracking, losing the last window, and quitting never finish programs.
	pub fn forget(&mut self, app_id: &str) {
		self.sessions.remove(app_id);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn early_finish_uses_frozen_mode_and_only_runs_once() {
		let mut engine = Engine::default();
		engine.observe("a", EndMode::Kill).unwrap();
		assert_eq!(engine.finish_now("a"), None);
		engine.submit_goal("a", "Read", 900, 0).unwrap();
		engine.observe("a", EndMode::CloseWindows).unwrap();
		assert_eq!(
			engine.finish_now("a"),
			Some(Command::Finish {
				app_id: "a".into(),
				mode: EndMode::Kill
			})
		);
		assert_eq!(engine.finish_now("a"), None);
		assert!(engine.tick(u64::MAX).is_empty());
	}

	#[test]
	fn pending_goal_has_no_timer_and_windows_share_session() {
		let mut engine = Engine::default();
		assert!(engine
			.observe("editor.desktop", EndMode::CloseWindows)
			.unwrap()
			.is_some());
		assert!(engine
			.observe("editor.desktop", EndMode::Kill)
			.unwrap()
			.is_none());
		assert!(engine.tick(u64::MAX).is_empty());
		assert_eq!(
			engine.session("editor.desktop").unwrap().mode,
			EndMode::CloseWindows
		);
		assert!(engine.session("editor").is_none());
		assert!(engine.cancel_goal("editor.desktop").is_err());
	}

	#[test]
	fn independent_timers_include_sleep_and_finish_once() {
		let mut engine = Engine::default();
		engine.observe("a", EndMode::CloseWindows).unwrap();
		engine.observe("b", EndMode::Kill).unwrap();
		engine.submit_goal("a", "Read", 10, 1000).unwrap();
		engine.submit_goal("b", "Write", 20, 1000).unwrap();
		assert!(engine.tick(10_999).is_empty());
		assert_eq!(
			engine.tick(11_000),
			vec![Command::Finish {
				app_id: "a".into(),
				mode: EndMode::CloseWindows
			}]
		);
		assert_eq!(
			engine.tick(100_000),
			vec![Command::Finish {
				app_id: "b".into(),
				mode: EndMode::Kill
			}]
		);
		assert!(engine.tick(100_001).is_empty());
	}

	#[test]
	fn cancellation_and_untracking_obey_end_mode() {
		let mut engine = Engine::default();
		engine.observe("a", EndMode::Kill).unwrap();
		assert_eq!(
			engine.cancel_goal("a").unwrap(),
			Command::Finish {
				app_id: "a".into(),
				mode: EndMode::Kill
			}
		);
		engine.observe("b", EndMode::Kill).unwrap();
		engine.submit_goal("b", "Read", 1, 0).unwrap();
		assert!(engine.cancel_goal("b").is_err());
		engine.forget("b");
		assert!(engine.tick(u64::MAX).is_empty());
	}

	#[test]
	fn invalid_input_does_not_start_or_replace_timer() {
		let mut engine = Engine::default();
		engine.observe("a", EndMode::CloseWindows).unwrap();
		assert!(engine.submit_goal("a", " ", 1, 0).is_err());
		assert!(engine.submit_goal("a", "Read", 0, 0).is_err());
		assert!(engine.submit_goal("a", "Read", 1, u64::MAX).is_err());
		assert_eq!(engine.session("a").unwrap().remaining_ms(0), None);
		engine.submit_goal("a", "Read", 1, 0).unwrap();
		assert!(engine.submit_goal("a", "Replace", 20, 0).is_err());
		assert_eq!(engine.session("a").unwrap().remaining_ms(0), Some(1000));
	}
}
