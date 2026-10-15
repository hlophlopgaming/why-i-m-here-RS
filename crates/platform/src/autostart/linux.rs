use super::*;
use std::fs;

pub(super) fn configure(path: &Path, enabled: bool, executable: &Path) -> io::Result<()> {
	if !enabled {
		return match fs::remove_file(path) {
			Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
			result => result,
		};
	}
	let executable = executable_text(executable)?;
	let mut quoted = String::new();
	for character in executable.chars() {
		if matches!(character, '\\' | '"' | '`' | '$') {
			quoted.push('\\');
		}
		quoted.push(character);
		if character == '%' {
			quoted.push('%');
		}
	}
	// Desktop string escaping is applied before Exec argument unquoting.
	let quoted = quoted.replace('\\', "\\\\");
	let entry = format!("[Desktop Entry]\nType=Application\nName=Why am I here?\nName[ru]=Зачем я здесь?\nExec=\"{quoted}\" --background\nTerminal=false\n");
	crate::settings::write_atomic(path, entry.as_bytes())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn register_replace_and_remove_in_private_directory() {
		let root = std::env::temp_dir().join(format!("why-here-autostart-{}", std::process::id()));
		let path = root.join("autostart/org.local.WhyHere.desktop");
		configure(&path, false, Path::new("unused")).unwrap();
		configure(&path, true, Path::new("/opt/Моя программа/why-here")).unwrap();
		assert!(fs::read_to_string(&path)
			.unwrap()
			.contains("Exec=\"/opt/Моя программа/why-here\" --background\n"));
		let strange = Path::new("/opt/$`\\\"%/why-here");
		configure(&path, true, strange).unwrap();
		let entry = fs::read_to_string(&path).unwrap();
		assert!(entry.contains(r#"Exec="/opt/\\$\\`\\\\\\"%%/why-here" --background"#));
		assert!(configure(&path, true, Path::new("/opt/bad\nname")).is_err());
		assert_eq!(fs::read_to_string(&path).unwrap(), entry);
		configure(&path, false, strange).unwrap();
		configure(&path, false, strange).unwrap();
		assert!(!path.exists());
		fs::remove_dir_all(root).unwrap();
	}
}
