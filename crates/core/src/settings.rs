use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::i18n::Language;

pub const MAX_DURATION_SECS: u32 = 24 * 60 * 60;
pub const MAX_GOAL_CHARS: usize = 1024;
pub const MAX_HISTORY: usize = 20;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrackedApp {
	pub id: String,
	pub name: String,
}

impl TrackedApp {
	pub fn is_valid(&self) -> bool {
		!self.id.trim().is_empty()
			&& self.id.len() <= 512
			&& !self.id.chars().any(char::is_control)
			&& !self.name.trim().is_empty()
			&& self.name.chars().count() <= 128
			&& !self.name.chars().any(char::is_control)
	}
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndMode {
	#[default]
	CloseWindows,
	Kill,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
	pub language: Language,
	pub duration_secs: u32,
	pub end_mode: EndMode,
	pub goal_history: Vec<String>,
	pub tracking_enabled: bool,
	pub warning_secs: u32,
	pub play_sound: bool,
	pub tracked_apps: Vec<TrackedApp>,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			language: Language::Ru,
			duration_secs: 15 * 60,
			end_mode: EndMode::CloseWindows,
			goal_history: Vec::new(),
			tracking_enabled: true,
			warning_secs: 2 * 60,
			play_sound: false,
			tracked_apps: Vec::new(),
		}
	}
}

#[derive(Debug, Error)]
pub enum ValidationError {
	#[error("duration must be between 1 and 86400 seconds")]
	Duration,
	#[error("goal must contain between 1 and 1024 characters")]
	Goal,
	#[error("goal history must contain at most 20 entries")]
	History,
	#[error("invalid warning interval or tracked application list")]
	Settings,
}

pub fn validate_duration(duration_secs: u32) -> Result<(), ValidationError> {
	if !(1..=MAX_DURATION_SECS).contains(&duration_secs) {
		return Err(ValidationError::Duration);
	}
	Ok(())
}

pub fn validate_goal(goal: &str) -> Result<(), ValidationError> {
	if goal.trim().is_empty() || goal.chars().count() > MAX_GOAL_CHARS {
		return Err(ValidationError::Goal);
	}
	Ok(())
}

impl Settings {
	pub fn validate(&self) -> Result<(), ValidationError> {
		validate_duration(self.duration_secs)?;
		let mut ids = std::collections::BTreeSet::new();
		if self.warning_secs > MAX_DURATION_SECS
			|| self.tracked_apps.len() > 64
			|| self
				.tracked_apps
				.iter()
				.any(|app| !app.is_valid() || !ids.insert(&app.id))
		{
			return Err(ValidationError::Settings);
		}
		if self.goal_history.len() > MAX_HISTORY {
			return Err(ValidationError::History);
		}
		for goal in &self.goal_history {
			validate_goal(goal)?;
		}
		Ok(())
	}

	pub fn remember_goal(&mut self, goal: &str) -> Result<(), ValidationError> {
		validate_goal(goal)?;
		let goal = goal.trim();
		self.goal_history.retain(|old| old != goal);
		self.goal_history.insert(0, goal.to_owned());
		self.goal_history.truncate(MAX_HISTORY);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn application_labels_reject_control_characters_but_keep_literal_format_prefixes() {
		let mut app = TrackedApp {
			id: "editor.desktop.desktop".into(),
			name: "@BEditor".into(),
		};
		assert!(app.is_valid());
		app.name.push('\x01');
		assert!(!app.is_valid());
		app.name = "Editor".into();
		app.id.push('\n');
		assert!(!app.is_valid());
	}

	#[test]
	fn new_settings_default_and_reject_invalid_tracking() {
		let mut settings: Settings = serde_json::from_str("{}").unwrap();
		assert!(settings.tracking_enabled);
		assert!(!settings.play_sound);
		assert_eq!(settings.warning_secs, 120);
		let app = TrackedApp {
			id: "editor.desktop".into(),
			name: "Editor".into(),
		};
		settings.tracked_apps = vec![app.clone(), app];
		assert!(settings.validate().is_err());
		settings.tracked_apps.pop();
		settings.validate().unwrap();
		settings.warning_secs = MAX_DURATION_SECS + 1;
		assert!(settings.validate().is_err());
	}

	#[test]
	fn old_settings_keep_defaults_for_new_fields() {
		let settings: Settings = serde_json::from_str(r#"{"duration_secs":120}"#).unwrap();
		assert_eq!(settings.duration_secs, 120);
		assert_eq!(settings.language, Language::Ru);
		assert_eq!(settings.end_mode, EndMode::CloseWindows);
		settings.validate().unwrap();
	}

	#[test]
	fn history_is_trimmed_unique_and_bounded() {
		let mut settings = Settings::default();
		for index in 0..30 {
			settings.remember_goal(&format!("Goal {index}")).unwrap();
		}
		settings.remember_goal("  Goal 20  ").unwrap();
		assert_eq!(settings.goal_history.len(), MAX_HISTORY);
		assert_eq!(settings.goal_history[0], "Goal 20");
		assert_eq!(
			settings
				.goal_history
				.iter()
				.filter(|s| *s == "Goal 20")
				.count(),
			1
		);
		assert!(settings.remember_goal(" \n ").is_err());
	}
}
