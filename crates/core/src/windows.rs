use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowInfo {
	pub id: String,
	pub app_id: String,
	pub pid: u32,
	pub controllable: bool,
}

/// Only catalog desktop IDs have a removable desktop-file suffix.
pub fn desktop_matches(selected: &str, reported: &str) -> bool {
	match selected.strip_prefix("desktop:") {
		Some(file) => file == reported || file.strip_suffix(".desktop") == Some(reported),
		None => selected == reported,
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn suffix_is_removed_only_once_from_catalog_ids() {
		use super::desktop_matches as matches;
		assert!(matches("desktop:editor.desktop.desktop", "editor.desktop"));
		assert!(matches(
			"desktop:editor.desktop.desktop",
			"editor.desktop.desktop"
		));
		assert!(!matches("desktop:editor.desktop.desktop", "editor"));
		assert!(!matches("editor.desktop", "editor"));
	}
}
