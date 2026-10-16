#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{bail, Result};
use why_core::{
	i18n::{tr, Language},
	ipc::Screen,
	settings::Settings,
	VERSION,
};

const HELP: &str = "
  why-here [--show | --background] [--language ru|en]
  why-here --demo [--language ru|en] [--show]
  why-here ui <goal|timer|reminder|settings> --demo [--language ru|en]
  why-here --help
  why-here --version";

#[derive(Debug, PartialEq)]
enum Options {
	Help,
	Version,
	Run {
		demo: bool,
		language: Option<Language>,
		screen: Screen,
		single_screen: bool,
		background: bool,
	},
}

fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Options> {
	let mut args = arguments.into_iter().peekable();
	let mut demo = false;
	let mut language = None;
	let mut screen = Screen::Goal;
	let mut single_screen = false;
	let mut show = false;
	let mut background = false;
	while let Some(argument) = args.next() {
		match argument.as_str() {
			"--help" | "-h" => return Ok(Options::Help),
			"--version" | "-V" => return Ok(Options::Version),
			"--demo" => demo = true,
			"--show" => show = true,
			"--background" => background = true,
			"--language" => {
				language = Some(match args.next().as_deref() {
					Some("ru") => Language::Ru,
					Some("en") => Language::En,
					_ => bail!("--language requires ru or en"),
				})
			}
			"ui" if !single_screen => {
				single_screen = true;
				screen = match args.next().as_deref() {
					Some("goal") => Screen::Goal,
					Some("timer") => Screen::Timer,
					Some("reminder") => Screen::Reminder,
					Some("settings") => Screen::Settings,
					_ => bail!("ui requires goal, timer, reminder or settings"),
				};
			}
			_ => bail!("unknown argument: {argument}"),
		}
	}
	if (single_screen && (show || background)) || (show && background) || (demo && background) {
		bail!("incompatible command-line options");
	}
	if show {
		screen = Screen::Settings;
	}
	Ok(Options::Run {
		demo,
		language,
		screen,
		single_screen,
		background,
	})
}

fn run() -> Result<()> {
	match parse(std::env::args().skip(1))? {
		Options::Help => {
			println!(
				"why-here — {}\n{HELP}\n",
				tr(Language::Ru, "Зачем я здесь?")
			);
			println!(
				"{}",
				tr(
					Language::Ru,
					"Доступны отслеживание окон, таймер, трей, настройки и деморежим."
				)
			);
		}
		Options::Version => println!("why-here {VERSION}"),
		Options::Run {
			demo,
			language,
			screen,
			single_screen,
			background,
		} => {
			if single_screen && !demo {
				ui::run(screen)?;
			} else if demo {
				daemon::run_demo(
					Settings {
						language: language.unwrap_or_default(),
						..Settings::default()
					},
					screen,
					single_screen,
				)?;
			} else {
				daemon::run(background, language)?;
			}
		}
	}
	Ok(())
}

#[cfg(target_os = "linux")]
fn linux_ui_backend(desktop: &str) -> &'static str {
	if desktop
		.split(':')
		.any(|name| name.eq_ignore_ascii_case("KDE"))
	{
		"wayland"
	} else {
		"x11"
	}
}

fn main() {
	#[cfg(target_os = "linux")]
	{
		// Choose before starting threads; every UI child inherits the same backend.
		let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
		std::env::set_var("FLTK_BACKEND", linux_ui_backend(&desktop));
	}
	if let Err(error) = run() {
		eprintln!("{error:#}");
		std::process::exit(1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn options(args: &[&str]) -> Result<Options> {
		parse(args.iter().map(|arg| (*arg).to_owned()))
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn linux_backend_uses_wayland_only_for_kde() {
		for desktop in ["KDE", "kde", "KDE:Plasma", "Something:KDE"] {
			assert_eq!(linux_ui_backend(desktop), "wayland");
		}
		for desktop in ["", "GNOME", "ubuntu:GNOME", "XFCE", "sway", "NotKDE"] {
			assert_eq!(linux_ui_backend(desktop), "x11");
		}
	}

	#[test]
	fn documented_demo_commands_parse() {
		assert_eq!(
			options(&["--demo", "--language", "en"]).unwrap(),
			Options::Run {
				demo: true,
				language: Some(Language::En),
				screen: Screen::Goal,
				single_screen: false,
				background: false,
			}
		);
		assert_eq!(
			options(&["ui", "timer", "--demo"]).unwrap(),
			Options::Run {
				demo: true,
				language: None,
				screen: Screen::Timer,
				single_screen: true,
				background: false,
			}
		);
		assert_eq!(
			options(&["--demo", "--show"]).unwrap(),
			Options::Run {
				demo: true,
				language: None,
				screen: Screen::Settings,
				single_screen: false,
				background: false,
			}
		);
	}

	#[test]
	fn persistent_settings_keep_saved_language_unless_explicitly_overridden() {
		assert_eq!(
			options(&["--show"]).unwrap(),
			Options::Run {
				demo: false,
				language: None,
				screen: Screen::Settings,
				single_screen: false,
				background: false,
			}
		);
		assert_eq!(
			options(&["--show", "--language", "ru"]).unwrap(),
			Options::Run {
				demo: false,
				language: Some(Language::Ru),
				screen: Screen::Settings,
				single_screen: false,
				background: false,
			}
		);
	}

	#[test]
	fn normal_start_opens_settings_and_background_stays_hidden() {
		for args in [vec![], vec!["--show"]] {
			let Options::Run {
				demo,
				background,
				single_screen,
				..
			} = options(&args).unwrap()
			else {
				panic!("expected run options");
			};
			assert!(!demo && !background && !single_screen);
		}
		assert!(matches!(
			options(&["--background"]).unwrap(),
			Options::Run {
				demo: false,
				background: true,
				single_screen: false,
				..
			}
		));
	}

	#[test]
	fn invalid_options_fail_before_launching_ui() {
		for args in [
			vec!["ui"],
			vec!["ui", "unknown"],
			vec!["--language", "de"],
			vec!["--demo", "--background"],
			vec!["--show", "--background"],
			vec!["ui", "goal", "--show"],
			vec!["--unknown"],
		] {
			assert!(options(&args).is_err(), "{args:?}");
		}
	}
}
