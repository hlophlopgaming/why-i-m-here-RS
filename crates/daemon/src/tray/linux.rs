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
		vec![ksni::Icon {
			width: 32,
			height: 32,
			data: ARGB.to_vec(),
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

const ARGB: [u8; 32 * 32 * 4] = {
	let mut bytes = *super::ICON;
	let mut i = 0;
	while i < bytes.len() {
		bytes[i] = super::ICON[i + 3];
		bytes[i + 1] = super::ICON[i];
		bytes[i + 2] = super::ICON[i + 1];
		bytes[i + 3] = super::ICON[i + 2];
		i += 4;
	}
	bytes
};

#[cfg(test)]
mod tests {
	#[test]
	fn icon_is_argb() {
		for (rgba, argb) in super::super::ICON
			.as_chunks::<4>()
			.0
			.iter()
			.zip(super::ARGB.as_chunks::<4>().0.iter())
		{
			assert_eq!(*argb, [rgba[3], rgba[0], rgba[1], rgba[2]]);
		}
	}
}
