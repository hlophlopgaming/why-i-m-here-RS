use crate::background::Events;
use platform::instance::Request;
use tray_icon::{
	menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
	Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use why_core::i18n::{tr, Language};

pub struct Tray {
	_icon: TrayIcon,
}
impl Tray {
	pub fn new(language: Language, events: Events) -> anyhow::Result<Self> {
		let menu = Menu::new();
		let settings = MenuItem::new(tr(language, "Настройки"), true, None);
		let quit = MenuItem::new(tr(language, "Выйти из приложения"), true, None);
		menu.append_items(&[&settings, &PredefinedMenuItem::separator(), &quit])?;
		let settings_id = settings.id().clone();
		let quit_id = quit.id().clone();
		let menu_events = events.clone();
		MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
			if event.id == settings_id {
				menu_events.request(Request::Show(None));
			} else if event.id == quit_id {
				menu_events.request(Request::Quit);
			}
		}));
		TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
			if matches!(
				event,
				TrayIconEvent::Click {
					button: MouseButton::Left,
					button_state: MouseButtonState::Up,
					..
				}
			) {
				events.request(Request::Show(None));
			}
		}));
		let icon = TrayIconBuilder::new()
			.with_menu(Box::new(menu))
			.with_tooltip(tr(language, "Зачем я здесь?"))
			.with_menu_on_left_click(false)
			.with_icon(Icon::from_rgba(super::ICON.to_vec(), 32, 32)?)
			.build()?;
		Ok(Self { _icon: icon })
	}
}
impl Drop for Tray {
	fn drop(&mut self) {
		MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
		TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
	}
}
