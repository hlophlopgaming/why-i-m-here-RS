use super::*;
use windows_sys::Win32::{
	Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT},
	System::{
		Diagnostics::ToolHelp::{
			CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
			TH32CS_SNAPPROCESS,
		},
		RemoteDesktop::ProcessIdToSessionId,
		Threading::{
			GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
			WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
			PROCESS_TERMINATE,
		},
	},
};
struct Handle(HANDLE);
impl Drop for Handle {
	fn drop(&mut self) {
		// SAFETY: Uniquely owned process or snapshot handle, closed once.
		unsafe { CloseHandle(self.0) };
	}
}
pub(super) struct Process {
	handle: Option<Handle>,
	pub parent: u32,
	pub started: u64,
	pub parent_started: Option<u64>,
	pub safe: bool,
}
impl Process {
	pub fn open(pid: u32) -> io::Result<Self> {
		// SAFETY: Opens a pinned process identity; rights are checked by Windows.
		let mut raw = unsafe {
			OpenProcess(
				PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
				0,
				pid,
			)
		};
		let can_terminate = !raw.is_null();
		if raw.is_null() {
			// SAFETY: Query-only fallback keeps protected ancestors in the process snapshot.
			raw = unsafe {
				OpenProcess(
					PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
					0,
					pid,
				)
			};
		}
		if raw.is_null() {
			return Err(io::Error::last_os_error());
		}
		let handle = Handle(raw);
		let (mut create, mut exit, mut kernel, mut user) = (
			FILETIME::default(),
			FILETIME::default(),
			FILETIME::default(),
			FILETIME::default(),
		);
		// SAFETY: Owned handle and four correctly sized output buffers.
		if unsafe { GetProcessTimes(handle.0, &mut create, &mut exit, &mut kernel, &mut user) } == 0
		{
			return Err(io::Error::last_os_error());
		}
		let mut path = [0u16; 32768];
		let mut size = path.len() as u32;
		// SAFETY: Owned query handle and bounded UTF-16 buffer.
		if unsafe { QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut size) } == 0 {
			return Err(io::Error::last_os_error());
		}
		let path = String::from_utf16_lossy(&path[..size as usize]);
		let name = path.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
		let mut session = 0;
		// SAFETY: Valid output pointer; session zero and unqueryable processes are excluded.
		let safe = unsafe { ProcessIdToSessionId(pid, &mut session) } != 0
			&& session != 0
			&& can_terminate
			&& ![
				"explorer.exe",
				"dwm.exe",
				"csrss.exe",
				"winlogon.exe",
				"applicationframehost.exe",
				"why-here.exe",
			]
			.contains(&name.as_str());
		Ok(Self {
			handle: Some(handle),
			parent: 0,
			parent_started: None,
			started: (u64::from(create.dwHighDateTime) << 32) | u64::from(create.dwLowDateTime),
			safe,
		})
	}
	pub fn alive(&self) -> bool {
		// SAFETY: Owned process handle with PROCESS_SYNCHRONIZE rights, non-blocking query.
		self.handle
			.as_ref()
			.is_some_and(|handle| unsafe { WaitForSingleObject(handle.0, 0) == WAIT_TIMEOUT })
	}
	pub fn list() -> io::Result<BTreeMap<u32, Self>> {
		let mut processes = BTreeMap::new();
		for pid in snapshot()?.into_keys() {
			let process = Self::open(pid).unwrap_or(Self {
				handle: None,
				parent: 0,
				parent_started: None,
				started: 0,
				safe: false,
			});
			processes.insert(pid, process);
		}
		// Read parent links after pinning: a recycled PID cannot inherit a stale snapshot link.
		let parents = snapshot()?;
		for (&pid, process) in &mut processes {
			if let Some(&parent) = parents.get(&pid) {
				// Exited launchers are absent from the live snapshot, especially on Windows.
				process.parent = if parents.contains_key(&parent) {
					parent
				} else {
					0
				};
				process.parent_started = Self::open(parent).ok().map(|parent| parent.started);
			}
		}
		Ok(processes)
	}

	pub fn kill(&self) -> io::Result<()> {
		if !self.alive() {
			return Ok(());
		}
		let handle = self
			.handle
			.as_ref()
			.ok_or_else(|| io::Error::other("process is inaccessible"))?;
		// SAFETY: Termination uses only the pinned, validated HANDLE, never a bare PID.
		if unsafe { TerminateProcess(handle.0, 1) } == 0 {
			return Err(io::Error::last_os_error());
		}
		Ok(())
	}
}

fn snapshot() -> io::Result<BTreeMap<u32, u32>> {
	// SAFETY: Read-only system process snapshot, owned below.
	let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
	if raw == INVALID_HANDLE_VALUE {
		return Err(io::Error::last_os_error());
	}
	let snapshot = Handle(raw);
	let mut entry = PROCESSENTRY32W {
		dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
		..Default::default()
	};
	let mut parents = BTreeMap::new();
	// SAFETY: Live snapshot and correctly initialized output structure.
	let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) };
	while found != 0 {
		parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
		// SAFETY: Same snapshot and output buffer throughout synchronous enumeration.
		found = unsafe { Process32NextW(snapshot.0, &mut entry) };
	}
	Ok(parents)
}
