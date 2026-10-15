use crate::background::Events;
use ksni::blocking::TrayMethods;
use platform::instance::Request;
use why_core::{
	i18n::{tr, Language},
	APP_ID,
};

struct Item {
	events: Events,
	language: Language,
}

impl ksni::Tray for Item {
	fn id(&self) -> String {
		APP_ID.into()
	}
	fn title(&self) -> String {
		tr(self.language, "Зачем я здесь?").into()
	}
	fn activate(&mut self, _: i32, _: i32) {
		self.events.request(Request::Show(None));
	}
	fn icon_pixmap(&self) -> Vec<ksni::Icon> {
		let mut argb = super::ICON.to_vec();
		for pixel in argb.as_chunks_mut::<4>().0 {
			pixel.rotate_right(1);
		}
		vec![ksni::Icon {
			width: 32,
			height: 32,
			data: argb,
		}]
	}
	fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
		use ksni::menu::StandardItem;
		vec![
			StandardItem {
				label: tr(self.language, "Настройки").into(),
				activate: Box::new(|item: &mut Self| {
					item.events.request(Request::Show(None));
				}),
				..Default::default()
			}
			.into(),
			ksni::MenuItem::Separator,
			StandardItem {
				label: tr(self.language, "Выйти из приложения").into(),
				activate: Box::new(|item: &mut Self| {
					item.events.request(Request::Quit);
				}),
				..Default::default()
			}
			.into(),
		]
	}
}

pub struct Tray(ksni::blocking::Handle<Item>);
impl Tray {
	pub fn new(language: Language, events: Events) -> anyhow::Result<Self> {
		Ok(Self(Item { events, language }.spawn()?))
	}
}
impl Drop for Tray {
	fn drop(&mut self) {
		self.0.shutdown().wait();
	}
}
