use std::{env, process::Command};

fn main() {
	for variable in [
		"PKG_CONFIG_PATH",
		"PKG_CONFIG_LIBDIR",
		"PKG_CONFIG_SYSROOT_DIR",
	] {
		println!("cargo:rerun-if-env-changed={variable}");
	}
	if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
		return;
	}
	// FLTK otherwise silently drops Wayland support when these packages are missing.
	let available = Command::new("pkg-config")
		.args([
			"--exists",
			"wayland-client >= 1.18",
			"wayland-cursor",
			"wayland-protocols >= 1.15",
			"xkbcommon",
		])
		.status()
		.is_ok_and(|status| status.success());
	assert!(available, "Native Wayland build requires pkg-config, wayland-client >= 1.18, wayland-cursor, wayland-protocols >= 1.15 and xkbcommon. Install the development packages listed in README.md, then run `cargo clean -p fltk-sys` before rebuilding.");
}
