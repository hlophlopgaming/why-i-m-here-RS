#![forbid(unsafe_code)]

use anyhow::{bail, Result};
use why_core::{
	i18n::{tr, Language},
	ipc::Screen,
	settings::Settings,
	VERSION,
};

const HELP: &str = "
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
		language: Language,
		screen: Screen,
		single_screen: bool,
	},
}

fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Options> {
	let mut args = arguments.into_iter().peekable();
	let mut demo = false;
	let mut language = Language::Ru;
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
				language = match args.next().as_deref() {
					Some("ru") => Language::Ru,
					Some("en") => Language::En,
					_ => bail!("--language requires ru or en"),
				}
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
					"Доступен деморежим. KWin, Win32 и трей пока не подключены."
				)
			);
		}
		Options::Version => println!("why-here {VERSION}"),
		Options::Run {
			demo,
			language,
			screen,
			single_screen,
		} => {
			if single_screen && !demo {
				ui::run(screen)?;
			} else if demo {
				daemon::run_demo(
					Settings {
						language,
						..Settings::default()
					},
					screen,
					single_screen,
				)?;
			} else {
				bail!(
					"{}",
					tr(
						language,
						"Пока доступен только деморежим. Запустите why-here --demo."
					)
				);
			}
		}
	}
	Ok(())
}

fn main() {
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

	#[test]
	fn documented_demo_commands_parse() {
		assert_eq!(
			options(&["--demo", "--language", "en"]).unwrap(),
			Options::Run {
				demo: true,
				language: Language::En,
				screen: Screen::Goal,
				single_screen: false,
			}
		);
		assert_eq!(
			options(&["ui", "timer", "--demo"]).unwrap(),
			Options::Run {
				demo: true,
				language: Language::Ru,
				screen: Screen::Timer,
				single_screen: true,
			}
		);
		assert_eq!(
			options(&["--demo", "--show"]).unwrap(),
			Options::Run {
				demo: true,
				language: Language::Ru,
				screen: Screen::Settings,
				single_screen: false,
			}
		);
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
