use crate::{
	theme::{self, button, text},
	Action,
};
use fltk::{
	app,
	browser::HoldBrowser,
	enums::{CallbackTrigger, FrameType},
	frame::Frame,
	group::Flex,
	input::Input,
	prelude::*,
	window::Window,
};
use std::{cell::RefCell, rc::Rc};
use why_core::{
	i18n::{tr, Language},
	settings::TrackedApp,
	APP_ID,
};

pub(super) struct AppPicker {
	pub window: Window,
	apps: Rc<RefCell<Vec<TrackedApp>>>,
	indices: Rc<RefCell<Vec<usize>>>,
	list: HoldBrowser,
	search: Input,
	status: Frame,
	language: Language,
}

fn matching_indices(apps: &[TrackedApp], query: &str) -> Vec<usize> {
	let query = query.trim().to_lowercase();
	apps.iter()
		.enumerate()
		.filter(|(_, app)| {
			app.name.to_lowercase().contains(&query) || app.id.to_lowercase().contains(&query)
		})
		.map(|(index, _)| index)
		.collect()
}

fn refresh(list: &mut HoldBrowser, apps: &[TrackedApp], query: &str, indices: &mut Vec<usize>) {
	let selected = usize::try_from(list.value() - 1)
		.ok()
		.and_then(|index| indices.get(index).copied());
	*indices = matching_indices(apps, query);
	list.clear();
	for index in indices.iter() {
		let app = &apps[*index];
		list.add(&format!("{} — {}", app.name, app.id));
	}
	if list.size() > 0 {
		let row = selected
			.and_then(|old| indices.iter().position(|index| *index == old))
			.unwrap_or(0);
		list.select(row as i32 + 1);
	}
}

impl AppPicker {
	pub fn new(language: Language, sender: app::Sender<Action>) -> Self {
		let mut window =
			Window::new(0, 0, 640, 480, tr(language, "Установленные программы")).center_screen();
		window.set_xclass(APP_ID);
		window.make_modal(true);
		window.set_color(theme::BG);
		window.set_callback(|window| window.hide());
		let mut body = Flex::default_fill().column();
		body.set_margin(12);
		body.set_pad(8);
		let label = text(0, 0, 0, 0, tr(language, "Поиск по названию или ID"));
		body.fixed(&label, 20);
		let mut search = Input::default();
		search.set_text_size(13);
		search.set_trigger(CallbackTrigger::Changed);
		body.fixed(&search, 30);
		let mut list = HoldBrowser::default();
		list.set_frame(FrameType::FlatBox);
		list.set_color(theme::INPUT);
		list.set_text_size(13);
		list.set_format_char('\x01');
		let status = text(0, 0, 0, 0, tr(language, "Загрузка списка программ…"));
		body.fixed(&status, 24);
		let notice = text(
			0,
			0,
			0,
			0,
			tr(
				language,
				"Выбор добавит программу в список. Она не будет запущена или завершена.",
			),
		);
		body.fixed(&notice, 38);
		let mut buttons = Flex::default().row();
		buttons.set_pad(8);
		let mut add = button(0, 0, 0, 0, tr(language, "Добавить выбранную"));
		let mut cancel = button(0, 0, 0, 0, tr(language, "Отмена"));
		buttons.end();
		body.fixed(&buttons, 30);
		body.end();
		window.resizable(&body);
		window.size_range(520, 400, 0, 0);
		window.end();
		let apps = Rc::new(RefCell::new(Vec::<TrackedApp>::new()));
		let indices = Rc::new(RefCell::new(Vec::<usize>::new()));
		let search_apps = apps.clone();
		let search_indices = indices.clone();
		let mut search_list = list.clone();
		search.set_callback(move |search| {
			refresh(
				&mut search_list,
				&search_apps.borrow(),
				&search.value(),
				&mut search_indices.borrow_mut(),
			)
		});
		let selected_apps = apps.clone();
		let selected_indices = indices.clone();
		let selected_list = list.clone();
		let mut selected_window = window.clone();
		add.set_callback(move |_| {
			let Some(index) = usize::try_from(selected_list.value() - 1)
				.ok()
				.and_then(|index| selected_indices.borrow().get(index).copied())
			else {
				return;
			};
			if let Some(app) = selected_apps.borrow().get(index) {
				sender.send(Action::AddApp(app.clone()));
				selected_window.hide();
			}
		});
		list.set_callback(move |_| {
			if app::event_clicks() {
				add.do_callback();
			}
		});
		let mut cancelled_window = window.clone();
		cancel.set_callback(move |_| cancelled_window.hide());
		Self {
			window,
			apps,
			indices,
			list,
			search,
			status,
			language,
		}
	}

	pub fn receive(&mut self, apps: Vec<TrackedApp>, complete: bool, error: bool) {
		self.apps.borrow_mut().extend(apps);
		refresh(
			&mut self.list,
			&self.apps.borrow(),
			&self.search.value(),
			&mut self.indices.borrow_mut(),
		);
		self.status.set_label(tr(
			self.language,
			if error {
				"Не удалось загрузить список. Закройте настройки и повторите попытку."
			} else if complete && self.apps.borrow().is_empty() {
				"Установленные программы не найдены."
			} else if complete {
				"Выберите программу из списка."
			} else {
				"Загрузка списка программ…"
			},
		));
	}
}

#[cfg(test)]
mod tests {
	use fltk::prelude::*;

	#[test]
	#[ignore = "requires a graphical desktop; opens only the application's picker"]
	fn native_picker_updates_and_filters_while_open() {
		let _application = fltk::app::App::default();
		crate::theme::apply();
		let (sender, _receiver) = fltk::app::channel::<crate::Action>();
		let mut picker = super::AppPicker::new(why_core::i18n::Language::En, sender);
		picker.window.show();
		fltk::app::wait_for(0.2).unwrap();
		picker.receive(
			vec![
				why_core::settings::TrackedApp {
					id: "desktop:browser.desktop".into(),
					name: "Browser".into(),
				},
				why_core::settings::TrackedApp {
					id: "desktop:editor.desktop.desktop".into(),
					name: "Editor".into(),
				},
			],
			true,
			false,
		);
		picker.list.select(2);
		picker.receive(
			vec![why_core::settings::TrackedApp {
				id: "desktop:third.desktop".into(),
				name: "Third".into(),
			}],
			true,
			false,
		);
		assert_eq!(picker.list.value(), 2);
		picker.search.set_value("EDITOR");
		picker.search.do_callback();
		assert!(picker.window.shown());
		assert_eq!(*picker.indices.borrow(), vec![1]);
		assert!(picker
			.list
			.text(1)
			.unwrap()
			.contains("editor.desktop.desktop"));
		picker.window.hide();
	}

	#[test]
	fn search_matches_unicode_names_and_exact_desktop_ids() {
		let apps = vec![why_core::settings::TrackedApp {
			id: "desktop:editor.desktop.desktop".into(),
			name: "Редактор".into(),
		}];
		assert_eq!(super::matching_indices(&apps, " РЕДАК "), vec![0]);
		assert_eq!(super::matching_indices(&apps, "desktop.desktop"), vec![0]);
		assert!(super::matching_indices(&apps, "browser").is_empty());
	}
}
