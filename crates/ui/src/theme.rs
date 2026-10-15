use fltk::{
	app,
	button::Button,
	enums::{Align, Color, FrameType},
	frame::Frame,
	prelude::*,
};

pub const BG: Color = Color::from_hex(0x202426);
pub const INPUT: Color = Color::from_hex(0x141719);
pub const TEXT: Color = Color::from_hex(0xeff0f1);
pub const MUTED: Color = Color::from_hex(0xb0b6bb);
pub const ACCENT: Color = Color::from_hex(0x3daee9);
pub const WARNING: Color = Color::from_hex(0xf0b665);

pub fn apply() {
	app::background(32, 36, 38);
	app::background2(20, 23, 25);
	app::foreground(239, 240, 241);
	app::set_color(Color::Selection, 61, 174, 233);
	app::set_font_size(13);
}

pub fn text(x: i32, y: i32, w: i32, h: i32, value: &str) -> Frame {
	let mut frame = Frame::new(x, y, w, h, value);
	frame.set_align(Align::Left | Align::Inside | Align::Wrap);
	frame.set_label_size(13);
	frame
}

pub fn button(x: i32, y: i32, w: i32, h: i32, value: &str) -> Button {
	let mut button = Button::new(x, y, w, h, value);
	button.set_color(Color::from_hex(0x383d41));
	button.set_selection_color(ACCENT);
	button.set_frame(FrameType::ThinUpBox);
	button.set_label_size(13);
	button
}
