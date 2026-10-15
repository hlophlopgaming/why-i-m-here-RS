use super::*;
use crate::applications::windows::{Apartment, ShellString, APP_USER_MODEL_ID};
use std::{cell::RefCell, collections::BTreeMap, marker::PhantomData, rc::Rc};
use windows::Win32::{
	Foundation::HWND as ShellHwnd,
	System::Com::{
		CoInitializeEx,
		StructuredStorage::{PropVariantClear, PropVariantToStringAlloc, PROPVARIANT},
		COINIT_APARTMENTTHREADED,
	},
	UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::{
	Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM},
	Globalization::{CompareStringOrdinal, CSTR_EQUAL},
	Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
	Storage::Packaging::Appx::GetApplicationUserModelId,
	System::Threading::{
		GetProcessTimes, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
		PROCESS_QUERY_LIMITED_INFORMATION,
	},
	UI::{Accessibility::*, WindowsAndMessaging::*},
};

thread_local! { static NOTIFY: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None); }
struct Handle(HANDLE);
impl Drop for Handle {
	fn drop(&mut self) {
		// SAFETY: An owned process/token handle, closed once.
		unsafe { CloseHandle(self.0) };
	}
}
struct Hook(HWINEVENTHOOK);
impl Drop for Hook {
	fn drop(&mut self) {
		// SAFETY: Hook belongs to this thread and remains installed until Drop.
		unsafe { UnhookWinEvent(self.0) };
	}
}
unsafe extern "system" fn event(
	_: HWINEVENTHOOK,
	event: u32,
	_: HWND,
	object: i32,
	child: i32,
	_: u32,
	_: u32,
) {
	if child == 0 && (object == OBJID_WINDOW || event < EVENT_OBJECT_CREATE) {
		NOTIFY.with(|notify| {
			if let Ok(notify) = notify.try_borrow() {
				if let Some(notify) = notify.as_ref() {
					notify();
				}
			}
		});
	}
}
struct Window {
	hwnd: HWND,
	pid: u32,
	owner_pid: u32,
	process: Handle,
	started: u64,
	app: String,
	controllable: bool,
	was_minimized: Option<bool>,
}
impl Window {
	fn valid(&self) -> bool {
		let mut pid = 0;
		// SAFETY: Reads window ownership only; no messages are sent to a potentially stale HWND.
		unsafe { GetWindowThreadProcessId(self.hwnd, &mut pid) };
		pid == self.owner_pid
			&& real_pid(self.hwnd) == self.pid
			&& started(&self.process) == Some(self.started)
	}
	fn release(&mut self) {
		if let Some(minimized) = self.was_minimized.take() {
			if self.valid() && !minimized {
				// SAFETY: Revalidated tracked window; async show avoids waiting on a hung GUI.
				unsafe { ShowWindowAsync(self.hwnd, SW_RESTORE) };
			}
		}
	}
}
impl Drop for Window {
	fn drop(&mut self) {
		self.release();
	}
}

pub struct Backend {
	windows: BTreeMap<usize, Window>,
	selected: BTreeSet<String>,
	targets: Vec<(String, String)>,
	blocked: BTreeSet<String>,
	focus: BTreeMap<String, u32>,
	_hooks: Vec<Hook>,
	_apartment: Apartment,
	_thread: PhantomData<Rc<()>>,
}
impl Backend {
	pub fn new(
		_: &crate::instance::Instance,
		apps: &[TrackedApp],
		blocked: BTreeSet<String>,
		notify: impl Fn() + Send + Sync + 'static,
	) -> io::Result<Self> {
		// SAFETY: The main message-loop thread owns the apartment until Backend drops.
		unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
			.ok()
			.map_err(io::Error::other)?;
		let apartment = Apartment;
		let selected = apps.iter().map(|app| app.id.clone()).collect();
		let targets = crate::applications::windows::targets(&selected)?;
		let mut hooks = Vec::new();
		for (first, last) in [
			(EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE),
			(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
			(EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZEEND),
		] {
			// SAFETY: Callback lives for the process lifetime; delivery is on this message-loop thread.
			let hook = unsafe {
				SetWinEventHook(
					first,
					last,
					std::ptr::null_mut(),
					Some(event),
					0,
					0,
					WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
				)
			};
			if hook.is_null() {
				return Err(io::Error::last_os_error());
			}
			hooks.push(Hook(hook));
		}
		NOTIFY.with(|slot| *slot.borrow_mut() = Some(Box::new(notify)));
		Ok(Self {
			windows: BTreeMap::new(),
			selected,
			targets,
			blocked,
			focus: BTreeMap::new(),
			_hooks: hooks,
			_apartment: apartment,
			_thread: PhantomData,
		})
	}
	fn focus(&self, app_id: &str) {
		let Some(focus_pid) = self.focus.get(app_id) else {
			return;
		};
		for hwnd in top_windows() {
			let mut pid = 0;
			// SAFETY: Reads a window from EnumWindows, validated again before foreground request.
			unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
			if pid == *focus_pid {
				// SAFETY: Foreground request for our own goal UI process; OS may decline focus stealing.
				unsafe { SetForegroundWindow(hwnd) };
			}
		}
	}
}
impl WindowBackend for Backend {
	fn snapshot(&mut self) -> io::Result<Vec<WindowInfo>> {
		let mut live = BTreeSet::new();
		for hwnd in top_windows() {
			let key = hwnd as usize;
			if self.windows.get(&key).is_some_and(Window::valid) {
				live.insert(key);
				continue;
			}
			if let Some(window) = identify(hwnd, &self.selected, &self.targets) {
				self.windows.insert(key, window);
				live.insert(key);
			}
		}
		self.windows
			.retain(|key, window| live.contains(key) && window.valid());
		self.set_blocked(self.blocked.clone(), true)?;
		Ok(self
			.windows
			.iter()
			.map(|(id, window)| WindowInfo {
				id: id.to_string(),
				app_id: window.app.clone(),
				pid: window.pid,
				controllable: window.controllable,
			})
			.collect())
	}
	fn set_blocked(&mut self, apps: BTreeSet<String>, _: bool) -> io::Result<()> {
		self.blocked = apps;
		let mut needs_focus = None;
		for window in self.windows.values_mut() {
			if self.blocked.contains(&window.app) && window.controllable && window.valid() {
				// SAFETY: Live, non-elevated window of the selected application.
				let minimized = unsafe { IsIconic(window.hwnd) != 0 };
				window.was_minimized.get_or_insert(minimized);
				if !minimized {
					// SAFETY: Async minimize of the revalidated tracked HWND, restored on release/Drop.
					if unsafe { ShowWindowAsync(window.hwnd, SW_MINIMIZE) } == 0 {
						window.controllable = false;
						window.was_minimized = None;
						continue;
					}
				}
				if !minimized {
					needs_focus = Some(window.app.clone());
				}
			} else {
				window.release();
			}
		}
		if let Some(app) = needs_focus {
			self.focus(&app);
		}
		Ok(())
	}
	fn close_app(&mut self, app_id: &str) -> io::Result<()> {
		for window in self
			.windows
			.values_mut()
			.filter(|window| window.app == app_id && window.controllable)
		{
			window.release();
			if window.valid() {
				// SAFETY: Revalidated HWND/PID/start time; WM_CLOSE lets the app save or veto closing.
				if unsafe { PostMessageW(window.hwnd, WM_CLOSE, 0, 0) } == 0 {
					return Err(io::Error::last_os_error());
				}
			}
		}
		Ok(())
	}
	fn focus_goal(&mut self, app_id: &str, pid: u32) {
		self.focus.insert(app_id.to_owned(), pid);
		self.focus(app_id);
	}
}
impl Drop for Backend {
	fn drop(&mut self) {
		self._hooks.clear();
		NOTIFY.with(|slot| *slot.borrow_mut() = None);
		self.windows.clear();
	}
}
fn top_windows() -> Vec<HWND> {
	unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> i32 {
		// SAFETY: EnumWindows calls synchronously with a pointer to the live Vec below.
		unsafe {
			(&mut *(data as *mut Vec<HWND>)).push(hwnd);
		}
		1
	}
	let mut list = Vec::new();
	// SAFETY: Callback and Vec are live for this synchronous enumeration.
	unsafe { EnumWindows(Some(collect), &mut list as *mut Vec<HWND> as LPARAM) };
	list
}
fn started(process: &Handle) -> Option<u64> {
	let mut create = FILETIME::default();
	let mut exit = FILETIME::default();
	let mut kernel = FILETIME::default();
	let mut user = FILETIME::default();
	// SAFETY: Owned process handle and four initialized output structures.
	if unsafe { GetProcessTimes(process.0, &mut create, &mut exit, &mut kernel, &mut user) } == 0 {
		return None;
	}
	Some((u64::from(create.dwHighDateTime) << 32) | u64::from(create.dwLowDateTime))
}
fn window_pid(hwnd: HWND) -> u32 {
	let mut pid = 0;
	// SAFETY: Reads ownership without sending messages; handles can disappear during enumeration.
	unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
	pid
}
fn real_pid(hwnd: HWND) -> u32 {
	unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> i32 {
		let mut class = [0u16; 128];
		// SAFETY: Live fixed class-name buffer, and synchronous pointer to the output PID.
		unsafe {
			let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
			if String::from_utf16_lossy(&class[..length.max(0) as usize])
				== "Windows.UI.Core.CoreWindow"
			{
				*(data as *mut u32) = window_pid(hwnd);
				return 0;
			}
		}
		1
	}
	let mut child_pid = 0u32;
	// SAFETY: Synchronous callback writes only the provided PID; UWP frames use this child owner.
	unsafe { EnumChildWindows(hwnd, Some(find), &mut child_pid as *mut u32 as LPARAM) };
	if child_pid == 0 {
		window_pid(hwnd)
	} else {
		child_pid
	}
}
fn identify(
	hwnd: HWND,
	selected: &BTreeSet<String>,
	targets: &[(String, String)],
) -> Option<Window> {
	// SAFETY: Read-only properties of an enumerated top-level window; exclude hidden/tool windows.
	if unsafe {
		IsWindowVisible(hwnd) == 0
			|| GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0
	} {
		return None;
	}
	let pid = real_pid(hwnd);
	if pid == 0 || pid == std::process::id() {
		return None;
	}
	// SAFETY: Opens only limited query rights to the identified process, owned below.
	let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
	if raw.is_null() {
		return None;
	}
	let process = Handle(raw);
	let started = started(&process)?;
	let mut session = 0;
	// SAFETY: Output session variable is valid; session 0 windows are never controlled.
	if unsafe { ProcessIdToSessionId(pid, &mut session) } == 0 || session == 0 {
		return None;
	}
	let mut path = [0u16; 32768];
	let mut size = path.len() as u32;
	// SAFETY: Owned query handle and initialized output path buffer.
	if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut size) } == 0 {
		return None;
	}
	let path = String::from_utf16_lossy(&path[..size as usize]);
	if std::env::current_exe().is_ok_and(|own| same_path(&own.to_string_lossy(), &path)) {
		return None;
	}
	let name = std::path::Path::new(&path)
		.file_name()?
		.to_string_lossy()
		.to_ascii_lowercase();
	if [
		"explorer.exe",
		"dwm.exe",
		"csrss.exe",
		"winlogon.exe",
		"applicationframehost.exe",
		"why-here.exe",
	]
	.contains(&name.as_str())
	{
		return None;
	}
	let app = match window_app_id(hwnd).or_else(|| process_app_id(&process)) {
		Some(app) if selected.contains(&app) => app,
		Some(_) => return None,
		None => {
			let matches = targets
				.iter()
				.filter(|(_, target)| same_path(target, &path))
				.map(|(id, _)| id)
				.collect::<Vec<_>>();
			// An ambiguous executable requires an explicit window AppID.
			if matches.len() != 1 {
				return None;
			}
			matches[0].clone()
		}
	};
	let mut token = std::ptr::null_mut();
	// SAFETY: Only query access; no privileges or process state are changed.
	let controllable = if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } != 0 {
		let token = Handle(token);
		let mut elevation = TOKEN_ELEVATION::default();
		let mut size = 0;
		// SAFETY: The owned token is queried into a correctly sized elevation structure.
		unsafe {
			GetTokenInformation(
				token.0,
				TokenElevation,
				&mut elevation as *mut _ as *mut _,
				std::mem::size_of_val(&elevation) as u32,
				&mut size,
			) != 0 && elevation.TokenIsElevated == 0
		}
	} else {
		false
	};
	Some(Window {
		hwnd,
		pid,
		owner_pid: window_pid(hwnd),
		process,
		started,
		app,
		controllable,
		was_minimized: None,
	})
}
fn process_app_id(process: &Handle) -> Option<String> {
	let mut value = [0u16; 512];
	let mut size = value.len() as u32;
	// SAFETY: Owned query process handle and bounded UTF-16 output.
	if unsafe { GetApplicationUserModelId(process.0, &mut size, value.as_mut_ptr()) } != 0
		|| size < 2
		|| size as usize > value.len()
	{
		return None;
	}
	Some(format!(
		"windows:{}",
		String::from_utf16_lossy(&value[..size as usize - 1])
	))
}
struct Variant(PROPVARIANT);
impl Drop for Variant {
	fn drop(&mut self) {
		// SAFETY: Owned variant initialized by GetValue, cleared exactly once.
		let _ = unsafe { PropVariantClear(&mut self.0) };
	}
}

fn window_app_id(hwnd: HWND) -> Option<String> {
	// SAFETY: COM is initialized on this thread; property store owns its returned interface.
	let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(ShellHwnd(hwnd)) }.ok()?;
	// SAFETY: Constant property key; returned variant is cleared on every path below.
	let value = Variant(unsafe { store.GetValue(&APP_USER_MODEL_ID) }.ok()?);
	// SAFETY: Initialized PROPVARIANT; conversion returns a COM task allocation.
	let result = unsafe { PropVariantToStringAlloc(&value.0) }
		.ok()
		.and_then(|text| ShellString(text).decode().ok());
	result
		.filter(|id| !id.is_empty())
		.map(|id| format!("windows:{id}"))
}
fn same_path(a: &str, b: &str) -> bool {
	let a: Vec<u16> = a.encode_utf16().collect();
	let b: Vec<u16> = b.encode_utf16().collect();
	// SAFETY: Both slices are live and passed with explicit lengths, ordinal case-insensitive comparison.
	unsafe {
		CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1)
			== CSTR_EQUAL
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{
		fs,
		process::{Child, Command, Stdio},
		time::{Duration, Instant},
	};
	#[test]
	fn executable_paths_compare_with_unicode_case_folding() {
		assert!(same_path(r"C:\ТЕСТ\Editor.exe", r"c:\тест\editor.EXE"));
		assert!(!same_path(r"C:\one\editor.exe", r"C:\two\editor.exe"));
	}
	struct Fixture(Child);
	impl Drop for Fixture {
		fn drop(&mut self) {
			let _ = self.0.kill();
			let _ = self.0.wait();
		}
	}
	fn wait(mut condition: impl FnMut() -> bool) {
		let deadline = Instant::now() + Duration::from_secs(8);
		while !condition() {
			let mut message = MSG::default();
			// SAFETY: Test owns this message loop, all messages have initialized storage.
			unsafe {
				while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
					TranslateMessage(&message);
					DispatchMessageW(&message);
				}
			}
			assert!(Instant::now() < deadline, "Win32 fixture timeout");
			std::thread::sleep(Duration::from_millis(20));
		}
	}
	#[test]
	#[ignore = "requires an interactive Windows session and WHY_HERE_WINDOW_FIXTURE pointing to ui example"]
	fn native_win32_blocks_restores_and_closes_only_own_fixture() {
		let source = std::env::var_os("WHY_HERE_WINDOW_FIXTURE")
			.expect("build ui --example window_fixture first");
		let root = std::env::temp_dir().join(format!("why-here-win32-test-{}", std::process::id()));
		fs::create_dir_all(&root).unwrap();
		let executable = root.join("fixture.exe");
		fs::copy(source, &executable).unwrap();
		let mut child = Fixture(
			Command::new(&executable)
				.stdout(Stdio::null())
				.spawn()
				.unwrap(),
		);
		let _thread = crate::desktop::message_thread();
		let instance =
			crate::instance::Instance::claim(crate::instance::Request::Background, |_| true)
				.unwrap()
				.expect("stop the running WhyHere instance before the Win32 fixture test");
		let app = TrackedApp {
			id: "windows:WhyHerePrivateFixture".into(),
			name: "Fixture".into(),
		};
		let mut backend = Backend::new(
			&instance,
			std::slice::from_ref(&app),
			BTreeSet::new(),
			|| {},
		)
		.unwrap();
		backend.targets = vec![(app.id.clone(), executable.to_str().unwrap().to_owned())];
		wait(|| backend.snapshot().unwrap().len() == 2);
		assert!(backend
			.snapshot()
			.unwrap()
			.iter()
			.all(|window| window.pid == child.0.id() && window.controllable));
		backend
			.set_blocked(BTreeSet::from([app.id.clone()]), true)
			.unwrap();
		wait(|| {
			backend.windows.values().all(|window| {
				// SAFETY: Windows were validated and belong only to the copied fixture child.
				unsafe { IsIconic(window.hwnd) != 0 }
			})
		});
		backend.set_blocked(BTreeSet::new(), true).unwrap();
		wait(|| {
			backend.windows.values().all(|window| {
				// SAFETY: Only the fixture's enumerated windows are inspected.
				unsafe { IsIconic(window.hwnd) == 0 }
			})
		});
		backend.close_app(&app.id).unwrap();
		wait(|| child.0.try_wait().unwrap().is_some());
		drop(backend);
		drop(instance);
		drop(child);
		fs::remove_dir_all(root).unwrap();
	}
}
