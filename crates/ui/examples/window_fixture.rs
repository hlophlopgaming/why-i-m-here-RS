//! Private fixture for ignored window-backend tests; never opens a user's document.
use fltk::{app, prelude::*, window::Window};
fn main() {
	let application = app::App::default();
	let mut first = Window::new(40, 40, 240, 100, "WhyHere fixture one");
	first.set_xclass("org.local.WhyHere.Fixture");
	first.end();
	first.show();
	let mut second = Window::new(300, 40, 240, 100, "WhyHere fixture two");
	second.set_xclass("org.local.WhyHere.Fixture");
	second.end();
	second.show();
	println!("ready");
	application.run().unwrap();
}
