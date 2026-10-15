#![forbid(unsafe_code)]

mod dashboard;
mod picker;
mod theme;

use fltk::{
	app,
	enums::{Event, Key, Shortcut},
	frame::Frame,
	input::Input,
	misc::Spinner,
	prelude::*,
	window::Window,
};
use std::{
	io::{self, BufReader},
	thread,
};
use theme::{button, text};
use thiserror::Error;
use why_core::{
	i18n::tr,
	ipc::{self, DaemonMessage, Screen, UiMessage, UiState},
	settings::{validate_goal, EndMode, TrackedApp},
	APP_ID,
};

#[derive(Debug, Error)]
pub enum UiError {
	#[error(transparent)]
	Ipc(#[from] ipc::IpcError),
	#[error(transparent)]
	Io(#[from] io::Error),
	#[error("invalid UI initialization")]
	Init,
	#[error("daemon disconnected or sent an invalid message")]
	Disconnected,
}

enum Action {
	Present,
	OpenPicker,
	AddApp(TrackedApp),
	InstalledApps(Vec<TrackedApp>, bool, bool),
	Reply(UiMessage),
	Tick(u64, bool),
	SettingsSaved(bool),
	Disconnected,
}

fn time_label(seconds: u64) -> String {
	format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn minutes_to_secs(value: f64, allow_zero: bool) -> Option<u32> {
	if !value.is_finite()
		|| value.fract() != 0.0
		|| !(if allow_zero { 0.0 } else { 1.0 }..=1440.0).contains(&value)
	{
		return None;
	}
	Some(value as u32 * 60)
}

fn spinner(x: i32, y: i32, w: i32, seconds: u32, allow_zero: bool) -> Spinner {
	let mut spinner = Spinner::new(x, y, w, 28, None);
	spinner.set_range(if allow_zero { 0.0 } else { 1.0 }, 1440.0);
	spinner.set_step(1.0);
	spinner.set_value(if allow_zero {
		f64::from(seconds) / 60.0
	} else {
		(f64::from(seconds) / 60.0).ceil()
	});
	spinner.set_text_size(13);
	spinner.set_text_color(theme::TEXT);
	spinner
}

pub fn run(screen: Screen) -> Result<(), UiError> {
	let mut reader = BufReader::new(io::stdin());
	let Some(DaemonMessage::Init {
		screen: requested,
		state,
	}) = ipc::read_message(&mut reader)?
	else {
		return Err(UiError::Init);
	};
	if requested != screen
		|| state.settings.validate().is_err()
		|| !state.app.is_valid()
		|| state.goal.chars().count() > 1024
	{
		return Err(UiError::Init);
	}
	let application = app::App::default().with_scheme(app::Scheme::Gtk);
	theme::apply();
	let (sender, receiver) = app::channel::<Action>();
	thread::Builder::new()
		.name("daemon-ipc".into())
		.spawn(move || loop {
			match ipc::read_message::<DaemonMessage>(&mut reader) {
				Ok(Some(DaemonMessage::Present)) => sender.send(Action::Present),
				Ok(Some(DaemonMessage::SettingsSaved { success })) => {
					sender.send(Action::SettingsSaved(success))
				}
				Ok(Some(DaemonMessage::Tick {
					remaining_secs,
					warning,
				})) => sender.send(Action::Tick(remaining_secs, warning)),
				Ok(Some(DaemonMessage::InstalledApps {
					apps,
					complete,
					error,
				})) if apps.iter().all(TrackedApp::is_valid) => {
					sender.send(Action::InstalledApps(apps, complete, error.is_some()))
				}
				_ => {
					sender.send(Action::Disconnected);
					break;
				}
			}
		})?;
	let language = state.settings.language;
	let title = match screen {
		Screen::Goal => format!("{} — {}", tr(language, "Зачем я здесь?"), state.app.name),
		Screen::Settings => tr(language, "Зачем я здесь?").to_owned(),
		Screen::Timer => tr(language, "Таймер").to_owned(),
		Screen::Reminder => tr(language, "Сессия завершена").to_owned(),
	};
	let (width, height) = match screen {
		Screen::Settings => (820, 780),
		Screen::Goal => (580, 310),
		Screen::Timer => (360, 172),
		Screen::Reminder => (460, 240),
	};
	let mut window = Window::new(0, 0, width, height, None).center_screen();
	window.set_label(&title);
	window.set_xclass(APP_ID);
	window.set_color(theme::BG);
	window.set_callback(move |_| sender.send(Action::Reply(UiMessage::Closed)));
	let mut dashboard = None;
	let mut picker = None::<picker::AppPicker>;
	let mut timer = match screen {
		Screen::Settings => {
			dashboard = Some(dashboard::build(&state, sender, &mut window));
			None
		}
		Screen::Goal => {
			goal_form(&state, sender, &mut window);
			None
		}
		Screen::Timer => Some(session_panel(&state, sender, &mut window)),
		Screen::Reminder => {
			reminder(&state, sender);
			None
		}
	};
	window.end();
	window.show();
	while application.wait() {
		while let Some(action) = receiver.recv() {
			match action {
				Action::Present => {
					window.show();
					let _ = window.take_focus();
				}
				Action::OpenPicker if dashboard.is_some() => {
					if picker.is_none() {
						picker = Some(picker::AppPicker::new(language, sender));
						ipc::write_message(
							&mut io::stdout().lock(),
							&UiMessage::RequestInstalledApps,
						)?;
					}
					if let Some(picker) = picker.as_mut() {
						picker.window.show();
					}
				}
				Action::OpenPicker => {}
				Action::AddApp(app) => {
					if let Some(dashboard) = dashboard.as_mut() {
						dashboard.add_app(app);
					}
				}
				Action::InstalledApps(apps, complete, error) => {
					if let Some(picker) = picker.as_mut() {
						picker.receive(apps, complete, error);
					}
				}
				Action::SettingsSaved(success) => {
					if success && !state.demo && screen == Screen::Settings {
						ipc::write_message(&mut io::stdout().lock(), &UiMessage::Closed)?;
						window.hide();
						return Ok(());
					} else if let Some(dashboard) = dashboard.as_mut() {
						dashboard.save_failed();
					}
				}
				Action::Reply(message @ UiMessage::SaveSettings { .. }) => {
					ipc::write_message(&mut io::stdout().lock(), &message)?;
				}
				Action::Reply(message) => {
					ipc::write_message(&mut io::stdout().lock(), &message)?;
					window.hide();
					return Ok(());
				}
				Action::Tick(seconds, warning) => {
					if let Some((time, status)) = timer.as_mut() {
						time.set_label(&time_label(seconds));
						time.set_label_color(if warning { theme::WARNING } else { theme::TEXT });
						status.set_label(tr(
							language,
							if warning {
								"Время заканчивается"
							} else {
								if state.demo {
									"Демосессия"
								} else {
									"Сессия"
								}
							},
						));
					}
				}
				Action::Disconnected => {
					window.hide();
					return Err(UiError::Disconnected);
				}
			}
		}
	}
	Ok(())
}

fn goal_form(state: &UiState, sender: app::Sender<Action>, window: &mut Window) {
	let language = state.settings.language;
	text(
		16,
		12,
		216,
		28,
		tr(language, "Что нужно сделать в этой программе?"),
	);
	let mut goal = Input::new(240, 12, 324, 28, None);
	goal.set_value(&state.goal);
	goal.set_text_size(13);
	goal.set_tooltip(tr(language, "Например: ответить на письмо"));
	text(
		16,
		50,
		216,
		38,
		tr(language, "Сколько времени вы хотите потратить на задачу?"),
	);
	let duration = spinner(240, 54, 324, state.settings.duration_secs, false);
	text(16, 94, 216, 28, tr(language, "Быстрый выбор, мин"));
	for (index, minutes) in [5, 15, 30].into_iter().enumerate() {
		let mut preset = button(240 + index as i32 * 112, 94, 100, 28, &minutes.to_string());
		let mut duration = duration.clone();
		preset.set_callback(move |_| duration.set_value(f64::from(minutes)));
	}
	text(
		16,
		134,
		548,
		38,
		tr(
			language,
			if state.demo {
				"Деморежим: другие программы и настройки не изменяются."
			} else {
				"Программа скрыта до ответа. Закрытие вопроса закроет программу."
			},
		),
	);
	text(
		16,
		174,
		548,
		40,
		tr(
			language,
			if !state.demo {
				"Закрытие вопроса закроет программу. После ответа начнётся таймер."
			} else {
				match state.settings.end_mode {
					EndMode::CloseWindows => {
						"До начала сессии форму нельзя закрыть. Введите цель и время."
					}
					EndMode::Kill => {
						"Закрытие формы или Escape имитирует завершение программы без сохранения."
					}
				}
			},
		),
	);
	let mut error = text(16, 220, 548, 32, "");
	error.set_label_color(theme::WARNING);
	let mut start = button(444, 266, 120, 28, tr(language, "Начать"));
	start.set_shortcut(Shortcut::from_key(Key::Enter));
	if validate_goal(&goal.value()).is_err() {
		start.deactivate();
	}
	let mut enabled_start = start.clone();
	goal.set_trigger(fltk::enums::CallbackTrigger::Changed);
	goal.set_callback(move |goal| {
		if validate_goal(&goal.value()).is_ok() {
			enabled_start.activate();
		} else {
			enabled_start.deactivate();
		}
	});
	let mut close_error = error.clone();
	start.set_callback(move |_| {
		if validate_goal(&goal.value()).is_err() {
			error.set_label(tr(language, "Цель должна содержать от 1 до 1024 символов."));
			return;
		}
		let Some(duration_secs) = minutes_to_secs(duration.value(), false) else {
			error.set_label(tr(language, "Введите целое число от 1 до 1440 минут."));
			return;
		};
		sender.send(Action::Reply(UiMessage::GoalSubmitted {
			goal: goal.value(),
			duration_secs,
		}));
	});
	let mode = state.settings.end_mode;
	let demo = state.demo;
	window.set_callback(move |_| {
		if !demo {
			sender.send(Action::Reply(UiMessage::GoalCancelled));
			return;
		}
		match mode {
			EndMode::Kill => sender.send(Action::Reply(UiMessage::GoalCancelled)),
			EndMode::CloseWindows => close_error.set_label(tr(language, "Сначала введите цель")),
		}
	});
}

fn session_panel(
	state: &UiState,
	sender: app::Sender<Action>,
	window: &mut Window,
) -> (Frame, Frame) {
	let language = state.settings.language;
	window.set_border(false);
	let mut app_name = text(16, 10, 328, 20, &state.app.name);
	app_name.set_label_size(14);
	let mut goal = text(16, 32, 328, 30, &state.goal);
	goal.set_tooltip(&state.goal);
	let time = text(
		16,
		64,
		130,
		22,
		&time_label(u64::from(state.settings.duration_secs)),
	);
	let mut status = text(
		150,
		64,
		194,
		22,
		tr(
			language,
			if state.demo {
				"Демосессия"
			} else {
				"Сессия"
			},
		),
	);
	status.set_label_color(theme::MUTED);
	text(
		16,
		88,
		328,
		20,
		tr(
			language,
			match state.settings.end_mode {
				EndMode::CloseWindows => "Закрыть окна",
				EndMode::Kill => "Завершить процессы",
			},
		),
	);
	let mut end = button(12, 126, 336, 30, tr(language, "Закончить сессию"));
	end.set_callback(move |_| sender.send(Action::Reply(UiMessage::EndNow)));
	let mut drag = None;
	window.handle(move |window, event| match event {
		Event::Push if app::event_y() < 120 => {
			drag = Some((
				app::event_x_root() - window.x(),
				app::event_y_root() - window.y(),
			));
			true
		}
		Event::Drag => {
			if let Some((x, y)) = drag {
				window.set_pos(app::event_x_root() - x, app::event_y_root() - y);
				true
			} else {
				false
			}
		}
		Event::Released => {
			drag = None;
			false
		}
		_ => false,
	});
	(time, status)
}

fn reminder(state: &UiState, sender: app::Sender<Action>) {
	let language = state.settings.language;
	let mut title = text(20, 16, 420, 32, tr(language, "Сессия завершена"));
	title.set_label_size(23);
	text(20, 56, 420, 24, &state.app.name);
	text(20, 86, 420, 44, &state.goal);
	text(
		20,
		140,
		420,
		42,
		tr(
			language,
			match state.settings.end_mode {
				EndMode::CloseWindows => {
					"Имитировано закрытие окон. Другие программы не затронуты."
				}
				EndMode::Kill => "Имитировано завершение процессов. Другие программы не затронуты.",
			},
		),
	);
	let mut retry = button(20, 194, 200, 28, tr(language, "Ещё раз"));
	retry.set_callback(move |_| sender.send(Action::Reply(UiMessage::Retry)));
	let mut close = button(240, 194, 200, 28, tr(language, "Закрыть"));
	close.set_callback(move |_| sender.send(Action::Reply(UiMessage::Closed)));
}

#[cfg(test)]
mod tests {
	#[test]
	fn minutes_reject_invalid_values() {
		assert_eq!(super::minutes_to_secs(15.0, false), Some(900));
		assert_eq!(super::minutes_to_secs(0.0, true), Some(0));
		for value in [0.0, -1.0, 1.5, 1441.0, f64::NAN, f64::INFINITY] {
			assert_eq!(super::minutes_to_secs(value, false), None);
		}
	}
	#[test]
	fn timer_format_handles_hour_and_day_boundaries() {
		assert_eq!(super::time_label(0), "00:00");
		assert_eq!(super::time_label(3661), "61:01");
		assert_eq!(super::time_label(86400), "1440:00");
	}
}
