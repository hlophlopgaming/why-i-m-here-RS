use crate::{
	minutes_to_secs, spinner,
	theme::{self, button, text},
	Action,
};
use fltk::{
	browser::HoldBrowser,
	button::{Button, CheckButton},
	draw,
	enums::{Color, FrameType},
	group::Flex,
	menu::Choice,
	prelude::*,
	valuator::{Slider, SliderType},
	window::Window,
};
use std::{cell::RefCell, rc::Rc};
use why_core::{
	i18n::{tr, Language},
	ipc::{UiMessage, UiState},
	settings::{EndMode, TrackedApp},
	VERSION,
};

pub(super) struct Dashboard {
	apps: Rc<RefCell<Vec<TrackedApp>>>,
	list: HoldBrowser,
	tracking: CheckButton,
	open: Button,
	status: fltk::frame::Frame,
	language: Language,
}

impl Dashboard {
	pub fn add_app(&mut self, app: TrackedApp) {
		let existing = self.apps.borrow().iter().position(|old| old.id == app.id);
		if let Some(index) = existing {
			self.list.select(index as i32 + 1);
		} else if self.apps.borrow().len() >= 64 {
			self.status
				.set_label(tr(self.language, "Можно выбрать не более 64 программ."));
			return;
		} else {
			self.list.add(&format!("{} — {}", app.name, app.id));
			self.apps.borrow_mut().push(app);
			self.list.select(self.list.size());
		}
		update_open(&self.tracking, &self.list, &mut self.open);
	}
}

fn field(main: &mut Flex, caption: &str) -> Flex {
	let mut row = Flex::default().row();
	row.set_pad(8);
	let label = text(0, 0, 0, 0, caption);
	row.fixed(&label, 276);
	main.fixed(&row, 28);
	row
}

fn update_open(tracking: &CheckButton, apps: &HoldBrowser, open: &mut Button) {
	if tracking.value() && apps.value() > 0 {
		open.activate();
	} else {
		open.deactivate();
	}
}

pub(super) fn build(
	state: &UiState,
	sender: fltk::app::Sender<Action>,
	window: &mut Window,
) -> Dashboard {
	let language = state.settings.language;
	let mut main = Flex::default_fill().column();
	main.set_margin(12);
	main.set_pad(4);
	let mut header = Flex::default().row();
	header.set_pad(10);
	let mut icon = fltk::frame::Frame::default();
	icon.draw(|icon| {
		let (x, y) = (icon.x() + 3, icon.y() + 3);
		draw::set_draw_color(Color::from_hex(0x6c56dc));
		draw::draw_pie(x, y, 38, 38, 0.0, 360.0);
		draw::set_draw_color(theme::TEXT);
		draw::set_line_style(draw::LineStyle::Solid, 2);
		draw::draw_arc(x + 8, y + 6, 23, 23, 0.0, 360.0);
		draw::draw_line(x + 19, y + 11, x + 19, y + 18);
		draw::draw_line(x + 19, y + 18, x + 26, y + 23);
		draw::draw_line(x + 10, y + 25, x + 6, y + 32);
		draw::set_line_style(draw::LineStyle::Solid, 0);
	});
	header.fixed(&icon, 44);
	let mut heading = text(0, 0, 0, 0, tr(language, "Зачем я здесь?"));
	heading.set_label_size(28);
	header.end();
	main.fixed(&header, 46);
	let subtitle = text(
		0,
		0,
		0,
		0,
		tr(
			language,
			"Откройте программу. Поставьте цель. Выделите время.",
		),
	);
	main.fixed(&subtitle, 20);
	let mut connection = text(
		0,
		0,
		0,
		0,
		tr(language, "Деморежим · KWin / Win32 не подключены"),
	);
	connection.set_label_color(theme::WARNING);
	main.fixed(&connection, 20);
	let mut tracking = CheckButton::default().with_label(tr(language, "Отслеживание включено"));
	tracking.set_value(state.settings.tracking_enabled);
	main.fixed(&tracking, 24);
	let label = text(
		0,
		0,
		0,
		0,
		tr(language, "Отслеживаемые программы (демосессии)"),
	);
	main.fixed(&label, 20);
	let apps = Rc::new(RefCell::new(state.settings.tracked_apps.clone()));
	let mut list = HoldBrowser::default();
	list.set_frame(FrameType::FlatBox);
	list.set_color(theme::INPUT);
	list.set_text_size(13);
	// Control characters are rejected in names, so SOH cannot activate formatting.
	list.set_format_char('\x01');
	for app in apps.borrow().iter() {
		list.add(&format!("{} — {}", app.name, app.id));
	}
	if list.size() > 0 {
		list.select(1);
	}
	let mut toolbar = Flex::default().row();
	toolbar.set_pad(8);
	let mut add = button(0, 0, 0, 0, tr(language, "Добавить демопрограмму…"));
	let mut installed = button(0, 0, 0, 0, tr(language, "Из установленных программ…"));
	installed.set_callback(move |_| sender.send(Action::OpenPicker));
	let mut remove = button(0, 0, 0, 0, tr(language, "Удалить"));
	toolbar.end();
	main.fixed(&toolbar, 28);
	let duration_row = field(&mut main, tr(language, "Длительность по умолчанию, мин"));
	let duration = spinner(0, 0, 0, state.settings.duration_secs, false);
	duration_row.end();
	let warning_row = field(
		&mut main,
		tr(language, "Предупредить за, мин (0 = в конце)"),
	);
	let warning = spinner(0, 0, 0, state.settings.warning_secs, true);
	warning_row.end();
	let opacity_row = field(&mut main, tr(language, "Непрозрачность напоминания"));
	let mut opacity = Slider::default().with_type(SliderType::Horizontal);
	opacity.set_range(0.3, 1.0);
	opacity.set_value(1.0);
	opacity.set_selection_color(theme::ACCENT);
	opacity.deactivate();
	opacity.set_tooltip(tr(
		language,
		"Прозрачность пока недоступна в нативном Wayland.",
	));
	opacity_row.end();
	let mut sound = CheckButton::default().with_label(tr(language, "Звуковое предупреждение"));
	sound.set_value(state.settings.play_sound);
	sound.set_tooltip(tr(
		language,
		"Один сигнал перед концом сессии. В Linux требуется pw-play.",
	));
	main.fixed(&sound, 24);
	let mode_row = field(&mut main, tr(language, "При завершении новых сессий"));
	let mut mode = Choice::default();
	mode.add_choice(tr(language, "Закрыть окна"));
	mode.add_choice(tr(language, "Завершить процессы"));
	mode.set_value(if state.settings.end_mode == EndMode::Kill {
		1
	} else {
		0
	});
	mode_row.end();
	let mut caution = text(0, 0, 0, 0, tr(language, "Принудительное завершение может потерять несохранённые данные.\nВ деморежиме действие только имитируется."));
	caution.set_label_color(theme::MUTED);
	main.fixed(&caution, 38);
	let mut autostart =
		CheckButton::default().with_label(tr(language, "Запускать при входе в систему"));
	autostart.deactivate();
	autostart.set_tooltip(tr(
		language,
		"Будет доступно после подключения платформенного бэкенда.",
	));
	main.fixed(&autostart, 24);
	let language_row = field(&mut main, tr(language, "Язык следующего UI-процесса"));
	let mut selected_language = Choice::default();
	selected_language.add_choice("Русский|English");
	selected_language.set_value(if language == Language::En { 1 } else { 0 });
	language_row.end();
	let label = text(0, 0, 0, 0, tr(language, "Текущие сессии"));
	main.fixed(&label, 20);
	let columns = Flex::default().row();
	for title in ["Программа", "Цель", "Осталось"] {
		let mut column = text(0, 0, 0, 0, tr(language, title));
		column.set_frame(FrameType::BorderBox);
		column.set_color(Color::from_hex(0x383d41));
	}
	columns.end();
	main.fixed(&columns, 24);
	let mut sessions = text(
		0,
		0,
		0,
		0,
		tr(
			language,
			"Активных сессий нет. Выберите программу и откройте форму цели.",
		),
	);
	sessions.set_frame(FrameType::FlatBox);
	sessions.set_color(theme::INPUT);
	sessions.set_label_color(theme::MUTED);
	let mut footer = Flex::default().row();
	footer.set_pad(8);
	let mut open = button(0, 0, 0, 0, tr(language, "Открыть форму цели"));
	let mut reconnect = button(0, 0, 0, 0, tr(language, "Переподключить KWin / Win32"));
	reconnect.deactivate();
	reconnect.set_tooltip(tr(
		language,
		"Будет доступно после подключения платформенного бэкенда.",
	));
	let mut quit = button(0, 0, 0, 0, tr(language, "Выйти"));
	quit.set_callback(move |_| sender.send(Action::Reply(UiMessage::Closed)));
	footer.end();
	main.fixed(&footer, 30);
	let mut version = text(0, 0, 0, 0, &format!("v{VERSION}"));
	version.set_label_size(11);
	version.set_label_color(theme::MUTED);
	main.fixed(&version, 14);
	main.end();
	window.resizable(&main);
	window.size_range(760, 740, 0, 0);
	update_open(&tracking, &list, &mut open);
	let mut list_open = open.clone();
	let list_tracking = tracking.clone();
	list.set_callback(move |list| update_open(&list_tracking, list, &mut list_open));
	let tracking_list = list.clone();
	let mut tracking_open = open.clone();
	tracking
		.set_callback(move |tracking| update_open(tracking, &tracking_list, &mut tracking_open));
	let remove_apps = apps.clone();
	let mut remove_list = list.clone();
	let remove_tracking = tracking.clone();
	let mut remove_open = open.clone();
	remove.set_callback(move |_| {
		let index = remove_list.value();
		if index > 0 && (index as usize) <= remove_apps.borrow().len() {
			remove_apps.borrow_mut().remove(index as usize - 1);
			remove_list.remove(index);
			if remove_list.size() > 0 {
				remove_list.select(index.min(remove_list.size()));
			}
		}
		update_open(&remove_tracking, &remove_list, &mut remove_open);
	});
	let add_apps = apps.clone();
	let mut add_list = list.clone();
	let add_tracking = tracking.clone();
	let mut add_open = open.clone();
	add.set_callback(move |_| {
		let candidates = [
			("demo.browser", "Браузер (демо)"),
			("demo.editor", "Редактор (демо)"),
			("demo.chat", "Мессенджер (демо)"),
		];
		if let Some((id, name)) = candidates
			.into_iter()
			.find(|(id, _)| !add_apps.borrow().iter().any(|app| app.id == *id))
		{
			let name = tr(language, name).to_owned();
			add_apps.borrow_mut().push(TrackedApp {
				id: id.to_owned(),
				name: name.clone(),
			});
			add_list.add(&format!("{name} — {id}"));
			add_list.select(add_list.size());
		}
		update_open(&add_tracking, &add_list, &mut add_open);
	});
	let dashboard = Dashboard {
		apps: apps.clone(),
		list: list.clone(),
		tracking: tracking.clone(),
		open: open.clone(),
		status: connection.clone(),
		language,
	};
	let mut settings = state.settings.clone();
	open.set_callback(move |_| {
		let Some(duration_secs) = minutes_to_secs(duration.value(), false) else {
			connection.set_label(tr(language, "Введите целое число от 1 до 1440 минут."));
			return;
		};
		let Some(warning_secs) = minutes_to_secs(warning.value(), true) else {
			connection.set_label(tr(language, "Интервал предупреждения: от 0 до 1440 минут."));
			return;
		};
		let Some(app) = usize::try_from(list.value() - 1)
			.ok()
			.and_then(|index| apps.borrow().get(index).cloned())
		else {
			return;
		};
		if !tracking.value() {
			return;
		}
		settings.duration_secs = duration_secs;
		settings.warning_secs = warning_secs;
		settings.play_sound = sound.value();
		settings.end_mode = if mode.value() == 1 {
			EndMode::Kill
		} else {
			EndMode::CloseWindows
		};
		settings.language = if selected_language.value() == 1 {
			Language::En
		} else {
			Language::Ru
		};
		settings.tracking_enabled = tracking.value();
		settings.tracked_apps = apps.borrow().clone();
		sender.send(Action::Reply(UiMessage::StartSession {
			settings: settings.clone(),
			app_id: app.id,
		}));
	});
	dashboard
}
