#![cfg(target_os = "linux")]
use std::{
	fs,
	process::{Child, Command, Stdio},
	sync::mpsc,
	time::{Duration, Instant},
};
use why_core::{
	ipc::{self, UiMessage},
	settings::{Settings, TrackedApp},
	windows::WindowInfo,
	APP_ID,
};
use zbus::blocking::{connection::Builder, Proxy};
struct Watcher;
#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
	fn register_status_notifier_item(&self, _service: &str) {}
	#[zbus(property)]
	fn is_status_notifier_host_registered(&self) -> bool {
		true
	}
}
struct Scripting(mpsc::Sender<String>);
#[zbus::interface(name = "org.kde.kwin.Scripting")]
impl Scripting {
	#[zbus(name = "loadScript")]
	fn load_script(&self, path: &str, _name: &str) -> i32 {
		self.0.send(fs::read_to_string(path).unwrap()).unwrap();
		1
	}
	#[zbus(name = "unloadScript")]
	fn unload_script(&self, _name: &str) -> bool {
		true
	}
}
struct Script;
#[zbus::interface(name = "org.kde.kwin.Script")]
impl Script {
	#[zbus(name = "run")]
	fn run(&self) {}
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
fn wait(mut check: impl FnMut() -> bool) {
	let deadline = Instant::now() + Duration::from_secs(8);
	while !check() {
		assert!(
			Instant::now() < deadline,
			"real session condition timed out"
		);
		std::thread::sleep(Duration::from_millis(30));
	}
}
fn child(daemon: u32, screen: &str) -> Option<u32> {
	let list =
		fs::read_to_string(format!("/proc/{daemon}/task/{daemon}/children")).unwrap_or_default();
	list.split_whitespace()
		.filter_map(|pid| {
			let command = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
			let args = command.split(|byte| *byte == 0).collect::<Vec<_>>();
			(args.get(1) == Some(&b"ui".as_slice()) && args.get(2) == Some(&screen.as_bytes()))
				.then(|| pid.parse().unwrap())
		})
		.next()
}
fn reply(pid: u32, message: &UiMessage) {
	// Writes only to the stdout pipe of this test's UI child, simulating its callback.
	let mut output = fs::OpenOptions::new()
		.write(true)
		.open(format!("/proc/{pid}/fd/1"))
		.unwrap();
	ipc::write_message(&mut output, message).unwrap();
}
#[test]
#[ignore = "requires private dbus-run-session and graphical desktop; mock KWin never controls actual windows"]
fn real_goal_cancel_expiry_and_finish_button_route_to_window_close() {
	let (sender, receiver) = mpsc::channel();
	let connection = Builder::session()
		.unwrap()
		.serve_at("/StatusNotifierWatcher", Watcher)
		.unwrap()
		.serve_at("/Scripting", Scripting(sender))
		.unwrap()
		.serve_at("/Scripting/Script1", Script)
		.unwrap()
		.build()
		.unwrap();
	for name in ["org.kde.KWin", "org.kde.StatusNotifierWatcher"] {
		assert_eq!(
			connection
				.request_name_with_flags(name, zbus::fdo::RequestNameFlags::DoNotQueue.into())
				.unwrap(),
			zbus::fdo::RequestNameReply::PrimaryOwner,
			"use a private D-Bus bus"
		);
	}
	let root =
		Profile(std::env::temp_dir().join(format!("why-here-real-test-{}", std::process::id())));
	fs::create_dir_all(root.0.join("why-here")).unwrap();
	let settings = Settings {
		tracked_apps: vec![TrackedApp {
			id: "desktop:test.desktop".into(),
			name: "Fixture".into(),
		}],
		..Settings::default()
	};
	let path = root.0.join("why-here/settings.json");
	fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
	let mut daemon = Daemon(
		Command::new(env!("CARGO_BIN_EXE_why-here"))
			.arg("--background")
			.env("XDG_CONFIG_HOME", &root.0)
			.stdin(Stdio::null())
			.spawn()
			.unwrap(),
	);
	let script = receiver.recv_timeout(Duration::from_secs(8)).unwrap();
	assert!(script.contains("const config ="));
	let bridge = Proxy::new(
		&connection,
		APP_ID,
		"/org/local/WhyHere/Windows",
		"org.local.WhyHere.Windows",
	)
	.unwrap();
	let pid = daemon.0.id().to_string();
	let exchange = |windows: &[WindowInfo], released: bool| -> serde_json::Value {
		let response: String = bridge
			.call(
				"Exchange",
				&(
					"1",
					pid.as_str(),
					serde_json::to_string(windows).unwrap(),
					released,
				),
			)
			.unwrap();
		serde_json::from_str(&response).unwrap()
	};
	let windows = vec![
		WindowInfo {
			id: "one".into(),
			app_id: settings.tracked_apps[0].id.clone(),
			pid: 123456,
			controllable: true,
		},
		WindowInfo {
			id: "two".into(),
			app_id: settings.tracked_apps[0].id.clone(),
			pid: 123456,
			controllable: true,
		},
	];
	let initial = exchange(&windows, false);
	assert_eq!(initial["blocked"][0], settings.tracked_apps[0].id);
	wait(|| child(daemon.0.id(), "goal").is_some());
	let goal = child(daemon.0.id(), "goal").unwrap();
	reply(goal, &UiMessage::GoalCancelled);
	wait(|| exchange(&windows, false)["close"].as_array().unwrap().len() == 2);
	exchange(&[], false);
	wait(|| child(daemon.0.id(), "goal").is_none());
	// A fresh launch asks again, while two windows still produce one goal UI.
	exchange(&windows, false);
	wait(|| child(daemon.0.id(), "goal").is_some());
	let goal = child(daemon.0.id(), "goal").unwrap();
	reply(
		goal,
		&UiMessage::GoalSubmitted {
			goal: "Short session".into(),
			duration_secs: 1,
		},
	);
	wait(|| child(daemon.0.id(), "timer").is_some());
	assert!(exchange(&windows, false)["blocked"]
		.as_array()
		.unwrap()
		.is_empty());
	wait(|| exchange(&windows, false)["close"].as_array().unwrap().len() == 2);
	exchange(&[], false);
	wait(|| child(daemon.0.id(), "timer").is_none());
	exchange(&windows, false);
	wait(|| child(daemon.0.id(), "goal").is_some());
	reply(
		child(daemon.0.id(), "goal").unwrap(),
		&UiMessage::GoalSubmitted {
			goal: "Finish early".into(),
			duration_secs: 900,
		},
	);
	wait(|| child(daemon.0.id(), "timer").is_some());
	reply(child(daemon.0.id(), "timer").unwrap(), &UiMessage::EndNow);
	wait(|| exchange(&windows, false)["close"].as_array().unwrap().len() == 2);
	exchange(&[], false);
	let control = Proxy::new(
		&connection,
		APP_ID,
		"/org/local/WhyHere",
		"org.local.WhyHere.Control",
	)
	.unwrap();
	std::thread::sleep(Duration::from_millis(100));
	exchange(&windows, false);
	wait(|| child(daemon.0.id(), "goal").is_some());
	control
		.call::<_, _, ()>("Activate", &(1u8, 1u8, 0u8))
		.unwrap();
	wait(|| child(daemon.0.id(), "settings").is_some());
	let updated = Settings {
		tracking_enabled: false,
		autostart: true,
		..settings.clone()
	};
	reply(
		child(daemon.0.id(), "settings").unwrap(),
		&UiMessage::SaveSettings {
			settings: updated.clone(),
		},
	);
	wait(|| {
		if let Ok(response) = bridge.call::<_, _, String>(
			"Exchange",
			&(
				"1",
				pid.as_str(),
				serde_json::to_string(&windows).unwrap(),
				false,
			),
		) {
			let response: serde_json::Value = serde_json::from_str(&response).unwrap();
			assert!(response["close"].as_array().unwrap().is_empty());
			if response["running"] == false {
				let _: String = bridge
					.call("Exchange", &("1", pid.as_str(), "[]", true))
					.unwrap();
			}
		}
		child(daemon.0.id(), "settings").is_none()
	});
	assert!(child(daemon.0.id(), "goal").is_none());
	assert_eq!(
		serde_json::from_slice::<Settings>(&fs::read(&path).unwrap()).unwrap(),
		updated
	);
	assert!(
		fs::read_to_string(root.0.join("autostart/org.local.WhyHere.desktop"))
			.unwrap()
			.contains("--background")
	);
	control
		.call::<_, _, ()>("Activate", &(1u8, 3u8, 0u8))
		.unwrap();
	wait(|| daemon.0.try_wait().unwrap().is_some());
	assert!(daemon.0.wait().unwrap().success());
}
