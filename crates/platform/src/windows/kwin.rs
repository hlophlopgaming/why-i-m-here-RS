use super::*;
use std::{
	fs,
	path::PathBuf,
	sync::{Arc, Condvar, Mutex},
	time::Duration,
};
use zbus::blocking::{Connection, Proxy};

const PATH: &str = "/org/local/WhyHere/Windows";
struct State {
	windows: Vec<WindowInfo>,
	blocked: BTreeSet<String>,
	close: BTreeSet<String>,
	focus: std::collections::BTreeMap<String, u32>,
	running: bool,
	released: bool,
	active: bool,
	ready: bool,
}
struct Bridge {
	state: Arc<(Mutex<State>, Condvar)>,
	notify: Box<dyn Fn() + Send + Sync>,
	owner: String,
	apps: BTreeSet<String>,
}
#[zbus::interface(name = "org.local.WhyHere.Windows")]
impl Bridge {
	fn exchange(
		&self,
		version: &str,
		pid: &str,
		snapshot: &str,
		released: bool,
		#[zbus(header)] header: zbus::message::Header<'_>,
	) -> zbus::fdo::Result<String> {
		if version != "1"
			|| pid.parse::<u32>().ok() != Some(std::process::id())
			|| snapshot.len() > 1024 * 1024
			|| header
				.sender()
				.is_none_or(|sender| sender.as_str() != self.owner)
		{
			return Err(zbus::fdo::Error::AccessDenied(
				"invalid KWin bridge peer or version".into(),
			));
		}
		let windows: Vec<WindowInfo> = serde_json::from_str(snapshot)
			.map_err(|error| zbus::fdo::Error::InvalidArgs(error.to_string()))?;
		if windows.len() > 4096
			|| windows.iter().any(|window| {
				window.id.is_empty()
					|| window.id.len() > 128
					|| !self.apps.contains(&window.app_id)
					|| window.pid == 0
					|| window.pid == std::process::id()
			}) {
			return Err(zbus::fdo::Error::InvalidArgs(
				"invalid window snapshot".into(),
			));
		}
		let (lock, wake) = &*self.state;
		let mut state = lock
			.lock()
			.map_err(|_| zbus::fdo::Error::Failed("bridge lock poisoned".into()))?;
		let changed = !state.ready || windows != state.windows;
		state.ready = true;
		wake.notify_all();
		state.windows = windows;
		state.released |= released;
		if released {
			wake.notify_all();
		}
		let close = std::mem::take(&mut state.close);
		let reply = serde_json::json!({"blocked":state.blocked, "close":close, "focus":state.focus, "running":state.running, "watch":state.active}).to_string();
		drop(state);
		if changed {
			(self.notify)();
		}
		Ok(reply)
	}
}

pub struct Backend {
	connection: Connection,
	state: Arc<(Mutex<State>, Condvar)>,
	name: String,
	script: PathBuf,
}
impl Backend {
	pub fn new(
		instance: &crate::instance::Instance,
		apps: &[TrackedApp],
		blocked: BTreeSet<String>,
		notify: impl Fn() + Send + Sync + 'static,
	) -> io::Result<Self> {
		let script = crate::settings::settings_path()
			.map_err(io::Error::other)?
			.with_file_name(format!("kwin-bridge-{}.js", std::process::id()));
		Self::connect(
			instance.connection().clone(),
			why_core::APP_ID,
			script,
			apps,
			blocked,
			notify,
		)
	}
	fn connect(
		connection: Connection,
		service: &str,
		script: PathBuf,
		apps: &[TrackedApp],
		blocked: BTreeSet<String>,
		notify: impl Fn() + Send + Sync + 'static,
	) -> io::Result<Self> {
		let owner = zbus::blocking::fdo::DBusProxy::new(&connection)
			.and_then(|bus| Ok(bus.get_name_owner("org.kde.KWin".try_into()?)?))
			.map_err(io::Error::other)?
			.to_string();
		let state = Arc::new((
			Mutex::new(State {
				windows: Vec::new(),
				blocked: blocked.clone(),
				close: BTreeSet::new(),
				focus: std::collections::BTreeMap::new(),
				running: true,
				released: false,
				active: true,
				ready: false,
			}),
			Condvar::new(),
		));
		connection
			.object_server()
			.at(
				PATH,
				Bridge {
					state: state.clone(),
					notify: Box::new(notify),
					owner,
					apps: apps.iter().map(|app| app.id.clone()).collect(),
				},
			)
			.map_err(io::Error::other)?;
		let config = serde_json::json!({"service":service,"pid":std::process::id(),"blocked":blocked,"apps":apps.iter().filter(|app|!app.id.starts_with("windows:")).map(|app| {
			let file = app.id.strip_prefix("desktop:").unwrap_or(&app.id);
			let desktop = if app.id.starts_with("desktop:") {file.strip_suffix(".desktop").unwrap_or(file)} else {file};
			serde_json::json!({"id":app.id,"file":file,"desktop":desktop})
		}).collect::<Vec<_>>()});
		let source = include_str!("../../../../kwin/bridge.js")
			.replace("__WHY_HERE_CONFIG__", &config.to_string());
		let backend = Self {
			connection,
			state,
			name: format!("{}.{}", service, std::process::id()),
			script,
		};
		crate::settings::write_atomic(&backend.script, source.as_bytes())?;
		let proxy = backend.scripting()?;
		let id: i32 = proxy
			.call(
				"loadScript",
				&(backend.script.to_string_lossy().as_ref(), &backend.name),
			)
			.map_err(io::Error::other)?;
		if id < 0 {
			return Err(io::Error::other("KWin refused bridge script"));
		}
		let path = format!("/Scripting/Script{id}");
		Proxy::new(
			&backend.connection,
			"org.kde.KWin",
			path.as_str(),
			"org.kde.kwin.Script",
		)
		.and_then(|proxy| proxy.call::<_, _, ()>("run", &()))
		.map_err(io::Error::other)?;
		drop(proxy);
		{
			let state = backend
				.state
				.0
				.lock()
				.map_err(|_| io::Error::other("bridge lock poisoned"))?;
			let (state, _) = backend
				.state
				.1
				.wait_timeout_while(state, Duration::from_secs(2), |state| !state.ready)
				.map_err(|_| io::Error::other("bridge lock poisoned"))?;
			if !state.ready {
				return Err(io::Error::new(
					io::ErrorKind::TimedOut,
					"KWin bridge did not connect",
				));
			}
		}
		Ok(backend)
	}
	fn scripting(&self) -> io::Result<Proxy<'_>> {
		Proxy::new(
			&self.connection,
			"org.kde.KWin",
			"/Scripting",
			"org.kde.kwin.Scripting",
		)
		.map_err(io::Error::other)
	}
}
impl WindowBackend for Backend {
	fn snapshot(&mut self) -> io::Result<Vec<WindowInfo>> {
		Ok(self
			.state
			.0
			.lock()
			.map_err(|_| io::Error::other("bridge lock poisoned"))?
			.windows
			.clone())
	}
	fn set_blocked(&mut self, apps: BTreeSet<String>, active: bool) -> io::Result<()> {
		let mut state = self
			.state
			.0
			.lock()
			.map_err(|_| io::Error::other("bridge lock poisoned"))?;
		state.blocked = apps;
		state.active = active;
		Ok(())
	}
	fn close_app(&mut self, app_id: &str) -> io::Result<()> {
		let mut state = self
			.state
			.0
			.lock()
			.map_err(|_| io::Error::other("bridge lock poisoned"))?;
		let ids = state
			.windows
			.iter()
			.filter(|window| window.app_id == app_id && window.controllable)
			.map(|window| window.id.clone())
			.collect::<Vec<_>>();
		state.blocked.remove(app_id);
		state.close.extend(ids);
		Ok(())
	}
	fn focus_goal(&mut self, app_id: &str, pid: u32) {
		if let Ok(mut state) = self.state.0.lock() {
			state.focus.insert(app_id.to_owned(), pid);
		}
	}
}
impl Drop for Backend {
	fn drop(&mut self) {
		if let Ok(mut state) = self.state.0.lock() {
			state.running = false;
			// Wait for restoration before unloading; watchdog also releases on daemon failure.
			if !state.windows.is_empty() {
				let _ =
					self.state
						.1
						.wait_timeout_while(state, Duration::from_millis(3600), |state| {
							!state.released
						});
			}
		}
		if let Ok(proxy) = self.scripting() {
			let _ = proxy.call::<_, _, bool>("unloadScript", &self.name);
		}
		let _ = self.connection.object_server().remove::<Bridge, _>(PATH);
		let _ = fs::remove_file(&self.script);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{
		process::{Child, Command, Stdio},
		time::Instant,
	};
	struct Fixture(Child);
	impl Drop for Fixture {
		fn drop(&mut self) {
			let _ = self.0.kill();
			let _ = self.0.wait();
		}
	}
	fn wait(mut check: impl FnMut() -> bool) {
		let deadline = Instant::now() + Duration::from_secs(8);
		while !check() {
			assert!(Instant::now() < deadline, "KWin fixture timeout");
			std::thread::sleep(Duration::from_millis(50));
		}
	}
	#[test]
	#[ignore = "requires Plasma 6 and WHY_HERE_WINDOW_FIXTURE pointing to the built ui example; uses a unique test service"]
	fn native_kwin_observes_groups_blocks_releases_and_closes_only_fixture() {
		let binary = std::env::var_os("WHY_HERE_WINDOW_FIXTURE")
			.expect("build ui --example window_fixture first");
		let mut child = Fixture(Command::new(binary).stdout(Stdio::null()).spawn().unwrap());
		let root = std::env::temp_dir().join(format!("why-here-kwin-test-{}", std::process::id()));
		let connection = Connection::session().unwrap();
		let service = format!("org.local.WhyHere.Test{}", std::process::id());
		connection.request_name(service.as_str()).unwrap();
		let app = TrackedApp {
			id: "desktop:org.local.WhyHere.Fixture.desktop".into(),
			name: "Fixture".into(),
		};
		let mut backend = Backend::connect(
			connection.clone(),
			&service,
			root.join("bridge.js"),
			std::slice::from_ref(&app),
			BTreeSet::from([app.id.clone()]),
			|| {},
		)
		.unwrap();
		let untrusted = Proxy::new(
			&connection,
			service.as_str(),
			PATH,
			"org.local.WhyHere.Windows",
		)
		.unwrap();
		assert!(untrusted
			.call::<_, _, String>(
				"Exchange",
				&("1", std::process::id().to_string(), "[]", false)
			)
			.is_err());
		wait(|| backend.snapshot().unwrap().len() == 2);
		assert!(backend
			.snapshot()
			.unwrap()
			.iter()
			.all(|window| window.pid == child.0.id() && window.app_id == app.id));
		let ids = backend
			.snapshot()
			.unwrap()
			.into_iter()
			.map(|window| window.id)
			.collect::<Vec<_>>();
		let proxy = Proxy::new(&connection, "org.kde.KWin", "/KWin", "org.kde.KWin").unwrap();
		let minimized = |id: &str| {
			let info: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
				proxy.call("getWindowInfo", &id).unwrap();
			bool::try_from(info.get("minimized").unwrap()).unwrap()
		};
		wait(|| ids.iter().all(|id| minimized(id)));
		backend.set_blocked(BTreeSet::new(), true).unwrap();
		wait(|| ids.iter().all(|id| !minimized(id)));
		backend
			.set_blocked(BTreeSet::from([app.id.clone()]), true)
			.unwrap();
		wait(|| ids.iter().all(|id| minimized(id)));
		drop(backend);
		wait(|| ids.iter().all(|id| !minimized(id)));
		assert!(child.0.try_wait().unwrap().is_none());
		let mut backend = Backend::connect(
			connection,
			&service,
			root.join("bridge.js"),
			std::slice::from_ref(&app),
			BTreeSet::new(),
			|| {},
		)
		.unwrap();
		backend.close_app(&app.id).unwrap();
		wait(|| child.0.try_wait().unwrap().is_some());
		wait(|| backend.snapshot().unwrap().is_empty());
		drop(backend);
		fs::remove_dir_all(root).unwrap();
	}
}
