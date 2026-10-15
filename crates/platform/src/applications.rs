use std::io;
use why_core::{i18n::Language, settings::TrackedApp};

pub fn installed_apps(language: Language) -> io::Result<Vec<TrackedApp>> {
	let mut apps = enumerate(language)?;
	apps.retain(TrackedApp::is_valid);
	normalize(&mut apps);
	Ok(apps)
}

fn normalize(apps: &mut Vec<TrackedApp>) {
	// Sort by ID first so duplicates with different display names are adjacent.
	apps.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.name.cmp(&b.name)));
	apps.dedup_by(|a, b| a.id == b.id);
	apps.sort_by_cached_key(|app| app.name.to_lowercase());
}

#[cfg(windows)]
#[path = "applications_windows.rs"]
mod windows;
#[cfg(windows)]
use windows::enumerate;

#[cfg(target_os = "linux")]
fn enumerate(language: Language) -> io::Result<Vec<TrackedApp>> {
	use std::{env, path::PathBuf};
	let mut directories = Vec::new();
	let home = env::var_os("XDG_DATA_HOME")
		.map(PathBuf::from)
		.filter(|path| path.is_absolute())
		.or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
	if let Some(home) = home {
		directories.push(home.join("applications"));
	}
	let data_dirs = env::var_os("XDG_DATA_DIRS")
		.filter(|value| !value.is_empty())
		.unwrap_or_else(|| "/usr/local/share:/usr/share".into());
	directories.extend(
		env::split_paths(&data_dirs)
			.filter(|path| path.is_absolute())
			.map(|path| path.join("applications")),
	);
	let desktops = env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "KDE".into());
	linux::scan(&directories, language, &desktops)
}

#[cfg(target_os = "linux")]
mod linux {
	use super::*;
	use std::{
		collections::BTreeSet,
		fs,
		io::Read,
		os::unix::fs::OpenOptionsExt,
		path::{Path, PathBuf},
	};

	pub(super) fn scan(
		directories: &[PathBuf],
		language: Language,
		desktops: &str,
	) -> io::Result<Vec<TrackedApp>> {
		let mut apps = Vec::new();
		let mut seen = BTreeSet::new();
		for directory in directories {
			let mut files = Vec::new();
			collect(directory, &mut files, 0)?;
			files.sort();
			for file in files {
				let Ok(relative) = file.strip_prefix(directory) else {
					continue;
				};
				let Some(parts) = relative
					.components()
					.map(|part| part.as_os_str().to_str())
					.collect::<Option<Vec<_>>>()
				else {
					continue;
				};
				let id = parts.join("-");
				if !seen.insert(id.clone()) || id == format!("{}.desktop", why_core::APP_ID) {
					continue;
				}
				let Ok(file) = fs::OpenOptions::new()
					.read(true)
					.custom_flags(libc::O_NONBLOCK)
					.open(file)
				else {
					continue;
				};
				if !file.metadata()?.is_file() {
					continue;
				}
				let mut content = String::new();
				if file
					.take(256 * 1024 + 1)
					.read_to_string(&mut content)
					.is_err() || content.len() > 256 * 1024
				{
					continue;
				}
				if let Some(name) = entry_name(&content, language, desktops) {
					apps.push(TrackedApp {
						id: format!("desktop:{id}"),
						name,
					});
				}
			}
		}
		Ok(apps)
	}

	fn collect(directory: &Path, files: &mut Vec<PathBuf>, depth: usize) -> io::Result<()> {
		// shortcut: do not recurse beyond 16 directories; revisit if desktop catalogs need deeper paths.
		if depth > 16 {
			return Ok(());
		}
		let entries = match fs::read_dir(directory) {
			Ok(entries) => entries,
			Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
			Err(error) => return Err(error),
		};
		for entry in entries {
			let entry = entry?;
			let kind = entry.file_type()?;
			if kind.is_dir() {
				collect(&entry.path(), files, depth + 1)?;
			} else if entry
				.path()
				.extension()
				.is_some_and(|extension| extension == "desktop")
			{
				// Desktop-file symlinks are common; directory symlinks are not followed.
				files.push(entry.path());
			}
		}
		Ok(())
	}

	fn entry_name(content: &str, language: Language, desktops: &str) -> Option<String> {
		let mut group = false;
		let keys = match language {
			Language::Ru => ["Name[ru_RU]", "Name[ru]", "Name"],
			Language::En => ["Name[en_US]", "Name[en]", "Name"],
		};
		let mut names = [None; 3];
		let (mut kind, mut hidden, mut no_display) = (None, None, None);
		let (mut only_show_in, mut not_show_in) = (None, None);
		for line in content.lines() {
			let line = line.trim();
			if line.starts_with('[') {
				group = line == "[Desktop Entry]";
				continue;
			}
			if !group || line.starts_with('#') {
				continue;
			}
			if let Some((key, value)) = line.split_once('=') {
				let value = Some(value.trim());
				match key.trim() {
					"Type" => kind = value,
					"Hidden" => hidden = value,
					"NoDisplay" => no_display = value,
					"OnlyShowIn" => only_show_in = value,
					"NotShowIn" => not_show_in = value,
					key => {
						if let Some(index) = keys.iter().position(|candidate| *candidate == key) {
							names[index] = value;
						}
					}
				}
			}
		}
		if kind != Some("Application") || hidden == Some("true") || no_display == Some("true") {
			return None;
		}
		let listed = |value: &str| {
			value.split(';').any(|desktop| {
				!desktop.is_empty() && desktops.split(':').any(|current| desktop == current)
			})
		};
		if only_show_in.is_some_and(|value| !listed(value)) || not_show_in.is_some_and(listed) {
			return None;
		}
		let name = names.into_iter().flatten().next()?;
		let mut decoded = String::new();
		let mut chars = name.chars();
		while let Some(character) = chars.next() {
			decoded.push(if character == '\\' {
				match chars.next()? {
					's' | 'n' | 't' | 'r' => ' ',
					'\\' => '\\',
					_ => return None,
				}
			} else {
				character
			});
		}
		Some(decoded.trim().to_owned())
	}

	#[cfg(test)]
	mod tests {
		use super::*;
		#[test]
		fn localized_names_and_visibility_are_respected() {
			let source = "[Desktop Entry]\nType=Application\nName=Editor\\sPlus\nName[ru]=Редактор\nOnlyShowIn=KDE;\n[Desktop Action Open]\nName=Wrong";
			assert_eq!(
				entry_name(source, Language::Ru, "KDE"),
				Some("Редактор".into())
			);
			assert_eq!(
				entry_name(source, Language::En, "KDE"),
				Some("Editor Plus".into())
			);
			assert_eq!(entry_name(source, Language::En, "GNOME"), None);
			for flag in ["Hidden=true", "NoDisplay=true", "NotShowIn=KDE;"] {
				assert_eq!(
					entry_name(
						&format!("[Desktop Entry]\nType=Application\nName=App\n{flag}"),
						Language::En,
						"KDE"
					),
					None
				);
			}
		}
		#[test]
		fn names_use_locale_priority_and_last_value_within_desktop_groups() {
			let source = "[Desktop Entry]\nType=Application\nName=Fallback\nName[ru]=Old\nName[ru]=New\nName[ru_RU]=Regional\nName[en]=English\nName[en_US]=American\nOnlyShowIn=GNOME;KDE;\nNotShowIn=XFCE;\n[Other]\nHidden=true\nName[ru_RU]=Wrong";
			assert_eq!(
				entry_name(source, Language::Ru, "Plasma:KDE"),
				Some("Regional".into())
			);
			assert_eq!(
				entry_name(source, Language::En, "KDE"),
				Some("American".into())
			);
			assert_eq!(
				entry_name(
					&source.replace("Name[ru_RU]=Regional\n", ""),
					Language::Ru,
					"KDE"
				),
				Some("New".into())
			);
			assert_eq!(entry_name(source, Language::En, "KDE:XFCE"), None);
		}
		#[test]
		fn user_entries_mask_system_entries_and_ids_keep_desktop_suffix() {
			let root = std::env::temp_dir().join(format!(
				"why-here-catalog-{}-{}",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap()
					.as_nanos()
			));
			let user = root.join("user");
			let system = root.join("system");
			fs::create_dir_all(&user).unwrap();
			fs::create_dir_all(system.join("nested")).unwrap();
			fs::write(user.join("hidden.desktop"), "[Desktop Entry]\nHidden=true").unwrap();
			fs::write(
				system.join("hidden.desktop"),
				"[Desktop Entry]\nType=Application\nName=Hidden",
			)
			.unwrap();
			fs::write(
				system.join("nested/editor.desktop.desktop"),
				"[Desktop Entry]\nType=Application\nName=Editor",
			)
			.unwrap();
			let apps = scan(&[user, system], Language::En, "KDE").unwrap();
			assert_eq!(
				apps,
				vec![TrackedApp {
					id: "desktop:nested-editor.desktop.desktop".into(),
					name: "Editor".into()
				}]
			);
			fs::remove_dir_all(root).unwrap();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn duplicate_ids_with_different_names_are_removed_before_name_sort() {
		let mut apps = vec![
			TrackedApp {
				id: "a".into(),
				name: "Zulu".into(),
			},
			TrackedApp {
				id: "b".into(),
				name: "middle".into(),
			},
			TrackedApp {
				id: "a".into(),
				name: "Alpha".into(),
			},
		];
		normalize(&mut apps);
		assert_eq!(
			apps.iter().map(|app| app.id.as_str()).collect::<Vec<_>>(),
			["a", "b"]
		);
		assert_eq!(apps[0].name, "Alpha");
	}
}
