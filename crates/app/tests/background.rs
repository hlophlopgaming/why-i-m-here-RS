#![cfg(target_os = "linux")]

use std::{
	fs,
	process::{Child, Command, Stdio},
	sync::{Arc, Mutex},
	thread,
	time::{Duration, Instant},
};
use zbus::{
	blocking::{connection::Builder, Connection, Proxy},
	zvariant::{OwnedObjectPath, Value},
};

struct Watcher(Arc<Mutex<Vec<String>>>);
#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
	fn register_status_notifier_item(&self, service: String) {
		self.0.lock().unwrap().push(service);
	}
	#[zbus(property)]
	fn is_status_notifier_host_registered(&self) -> bool {
		true
	}
	#[zbus(property)]
	fn registered_status_notifier_items(&self) -> Vec<String> {
		self.0.lock().unwrap().clone()
	}
	#[zbus(property)]
	fn protocol_version(&self) -> i32 {
		0
	}
}

struct Daemon(Child);
impl Drop for Daemon {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}
struct Profile(std::path::PathBuf);
impl Drop for Profile {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.0);
	}
}
fn wait_until(mut check: impl FnMut() -> bool) {
	let deadline = Instant::now() + Duration::from_secs(10);
	while !check() {
		assert!(Instant::now() < deadline, "condition timed out");
		thread::sleep(Duration::from_millis(20));
	}
}
fn children(pid: u32) -> Vec<u32> {
	fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
		.unwrap_or_default()
		.split_whitespace()
		.map(|pid| pid.parse().unwrap())
		.collect()
}
fn secondary(profile: &Profile, args: &[&str]) {
	assert!(Command::new(env!("CARGO_BIN_EXE_why-here"))
		.args(args)
		.env("XDG_CONFIG_HOME", &profile.0)
		.status()
		.unwrap()
		.success());
}

// RSS comes from procps ps; CPU time comes from the kernel's process counters.
fn measure(pid: u32, phase: &str) {
	if std::env::var_os("WHY_HERE_MEASURE_MEMORY").is_none() {
		return;
	}
	thread::sleep(Duration::from_secs(1));
	let output = Command::new("ps")
		.args(["-o", "rss=", "-p", &pid.to_string()])
		.output()
		.unwrap();
	assert!(output.status.success());
	println!(
		"MEMORY {phase} rss_kib={}",
		String::from_utf8(output.stdout).unwrap().trim()
	);
}
fn cpu_ticks(pid: u32) -> u64 {
	let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
	let fields: Vec<_> = stat
		.rsplit_once(')')
		.unwrap()
		.1
		.split_whitespace()
		.collect();
	fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

#[test]
#[ignore = "requires a private D-Bus bus and graphical desktop: dbus-run-session -- cargo test -p app --test background -- --ignored"]
fn tray_single_instance_settings_and_quit() {
	let registered = Arc::new(Mutex::new(Vec::new()));
	// Refuse a real desktop's watcher; this test must run on its own session bus.
	let connection = Builder::session()
		.unwrap()
		.serve_at("/StatusNotifierWatcher", Watcher(registered.clone()))
		.unwrap()
		.build()
		.unwrap();
	let reply = connection
		.request_name_with_flags(
			"org.kde.StatusNotifierWatcher",
			zbus::fdo::RequestNameFlags::DoNotQueue.into(),
		)
		.unwrap();
	assert_eq!(
		reply,
		zbus::fdo::RequestNameReply::PrimaryOwner,
		"use dbus-run-session"
	);
	let bus = zbus::blocking::fdo::DBusProxy::new(&connection).unwrap();
	assert!(!bus
		.name_has_owner(why_core::APP_ID.try_into().unwrap())
		.unwrap());
	let profile = Profile(
		std::env::temp_dir().join(format!("why-here-background-test-{}", std::process::id())),
	);
	fs::create_dir(&profile.0).unwrap();
	let settings_path = profile.0.join("why-here/settings.json");
	if std::env::var("WHY_HERE_PROFILE").as_deref() == Ok("full") {
		let settings = why_core::settings::Settings {
			tracked_apps: (0..64)
				.map(|index| why_core::settings::TrackedApp {
					id: format!("org.test.Editor{index}.desktop.desktop"),
					name: format!("Редактор {index}"),
				})
				.collect(),
			goal_history: (0..20)
				.map(|index| format!("Задача {index}: {}", "Прочитать документацию. ".repeat(10)))
				.collect(),
			..Default::default()
		};
		fs::create_dir(settings_path.parent().unwrap()).unwrap();
		fs::write(&settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();
	}
	let initial_settings = fs::read(&settings_path).ok();
	let mut daemon = Daemon(
		Command::new(env!("CARGO_BIN_EXE_why-here"))
			.arg("--background")
			.env("XDG_CONFIG_HOME", &profile.0)
			.stdin(Stdio::null())
			.spawn()
			.unwrap(),
	);
	wait_until(|| !registered.lock().unwrap().is_empty());
	let service = registered.lock().unwrap()[0].clone();
	measure(daemon.0.id(), "idle");
	assert!(children(daemon.0.id()).is_empty());
	secondary(&profile, &["--background"]);
	assert!(children(daemon.0.id()).is_empty());
	let control = Proxy::new(
		&connection,
		why_core::APP_ID,
		"/org/local/WhyHere",
		"org.local.WhyHere.Control",
	)
	.unwrap();
	assert!(control
		.call::<_, _, ()>("Activate", &(99u8, 1u8, 0u8))
		.is_err());
	let tray = Proxy::new(
		&connection,
		service.clone(),
		"/StatusNotifierItem",
		"org.kde.StatusNotifierItem",
	)
	.unwrap();
	let icon: Vec<(i32, i32, Vec<u8>)> = tray.get_property("IconPixmap").unwrap();
	assert_eq!((icon[0].0, icon[0].1, icon[0].2.len()), (32, 32, 4096));
	tray.call::<_, _, ()>("Activate", &(0i32, 0i32)).unwrap();
	wait_until(|| children(daemon.0.id()).len() == 1);
	let ui = children(daemon.0.id());
	measure(ui[0], "settings-ui");
	measure(daemon.0.id(), "settings-daemon");
	secondary(&profile, &["--show"]);
	assert_eq!(children(daemon.0.id()), ui);
	if std::env::var_os("WHY_HERE_MEASURE_MEMORY").is_some() {
		assert!(Command::new("kill")
			.args(["-TERM", &ui[0].to_string()])
			.status()
			.unwrap()
			.success());
		wait_until(|| children(daemon.0.id()).is_empty());
		measure(daemon.0.id(), "after-ui-exit");
		let before = cpu_ticks(daemon.0.id());
		thread::sleep(Duration::from_secs(3));
		println!(
			"MEMORY idle_cpu_ticks_3s={}",
			cpu_ticks(daemon.0.id()) - before
		);
	}
	let menu: OwnedObjectPath = tray.get_property("Menu").unwrap();
	let menu = Proxy::new(&connection, service.clone(), menu, "com.canonical.dbusmenu").unwrap();
	menu.call_noreply("Event", &(3i32, "clicked", Value::from(0i32), 0u32))
		.unwrap();
	wait_until(|| daemon.0.try_wait().unwrap().is_some());
	assert!(daemon.0.wait().unwrap().success());
	assert!(!std::path::Path::new(&format!("/proc/{}", ui[0])).exists());
	assert_eq!(fs::read(&settings_path).ok(), initial_settings);
	wait_until(|| {
		!bus.name_has_owner(why_core::APP_ID.try_into().unwrap())
			.unwrap()
	});
	// Ownership can be reacquired after graceful exit.
	let second = Connection::session().unwrap();
	assert_eq!(
		second
			.request_name_with_flags(
				why_core::APP_ID,
				zbus::fdo::RequestNameFlags::DoNotQueue.into()
			)
			.unwrap(),
		zbus::fdo::RequestNameReply::PrimaryOwner
	);
}
