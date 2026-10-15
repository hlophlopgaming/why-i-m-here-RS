use super::{Handler, Request};
use std::{
	io,
	os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
	ptr,
	sync::Arc,
	thread::{self, JoinHandle},
	time::{Duration, Instant},
};
use windows_sys::Win32::{
	Foundation::{
		GetLastError, ERROR_ALREADY_EXISTS, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE,
		INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
	},
	Storage::FileSystem::{
		CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
		OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
	},
	System::{
		Pipes::{
			ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW,
			PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
		},
		RemoteDesktop::ProcessIdToSessionId,
		Threading::{
			CreateEventW, CreateMutexW, GetCurrentProcessId, SetEvent, WaitForMultipleObjects,
			INFINITE,
		},
		IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
	},
};

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
	if handle.is_null() || handle == INVALID_HANDLE_VALUE {
		return Err(io::Error::last_os_error());
	}
	// SAFETY: a fresh owned kernel handle is transferred exactly once into OwnedHandle.
	Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}
fn event() -> io::Result<OwnedHandle> {
	// SAFETY: no name or security descriptor; returns a fresh manual-reset event.
	owned(unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) })
}
fn wide(value: &str) -> Vec<u16> {
	value.encode_utf16().chain(Some(0)).collect()
}

// Always drain cancelled I/O before its OVERLAPPED or buffer leaves scope.
fn finish(
	pipe: &OwnedHandle,
	stop: &OwnedHandle,
	overlapped: &mut OVERLAPPED,
	success: bool,
	timeout: u32,
) -> io::Result<u32> {
	if !success {
		// SAFETY: reads this thread's most recent Win32 error immediately after the I/O call.
		let error = unsafe { GetLastError() };
		if error != ERROR_IO_PENDING {
			return Err(io::Error::from_raw_os_error(error as i32));
		}
		let handles = [stop.as_raw_handle(), overlapped.hEvent];
		// SAFETY: both event handles stay alive until all outstanding I/O is drained.
		let result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout) };
		if result != WAIT_OBJECT_0 + 1 {
			let mut transferred = 0;
			// SAFETY: only this operation is cancelled; wait until the kernel releases its buffers.
			unsafe {
				CancelIoEx(pipe.as_raw_handle(), overlapped);
				GetOverlappedResult(pipe.as_raw_handle(), overlapped, &mut transferred, 1);
			}
			return Err(io::Error::new(
				if result == WAIT_TIMEOUT {
					io::ErrorKind::TimedOut
				} else {
					io::ErrorKind::Interrupted
				},
				"instance I/O interrupted",
			));
		}
	}
	let mut transferred = 0;
	// SAFETY: the completed operation and its OVERLAPPED belong to this pipe.
	if unsafe { GetOverlappedResult(pipe.as_raw_handle(), overlapped, &mut transferred, 0) } == 0 {
		return Err(io::Error::last_os_error());
	}
	Ok(transferred)
}

fn transfer(
	pipe: &OwnedHandle,
	stop: &OwnedHandle,
	bytes: &mut [u8],
	write: bool,
	timeout: u32,
) -> io::Result<()> {
	let ready = event()?;
	let mut overlapped = OVERLAPPED {
		hEvent: ready.as_raw_handle(),
		..Default::default()
	};
	// SAFETY: buffer and OVERLAPPED live until finish completes or drains the cancelled operation.
	let success = unsafe {
		if write {
			WriteFile(
				pipe.as_raw_handle(),
				bytes.as_ptr(),
				bytes.len() as u32,
				ptr::null_mut(),
				&mut overlapped,
			)
		} else {
			ReadFile(
				pipe.as_raw_handle(),
				bytes.as_mut_ptr(),
				bytes.len() as u32,
				ptr::null_mut(),
				&mut overlapped,
			)
		}
	};
	let transferred = finish(pipe, stop, &mut overlapped, success != 0, timeout)?;
	if transferred as usize != bytes.len() {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			"invalid instance frame length",
		));
	}
	Ok(())
}

pub struct Instance {
	_mutex: OwnedHandle,
	stop: Arc<OwnedHandle>,
	worker: Option<JoinHandle<()>>,
}

impl Instance {
	pub fn claim(
		request: Request,
		handler: impl Fn(Request) -> bool + Send + Sync + 'static,
	) -> io::Result<Option<Self>> {
		Self::claim_named(why_core::APP_ID, request, Box::new(handler))
	}

	fn claim_named(name: &str, request: Request, handler: Handler) -> io::Result<Option<Self>> {
		let mut session = 0;
		// SAFETY: session points to a writable u32; GetCurrentProcessId requires no arguments.
		if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
			return Err(io::Error::last_os_error());
		}
		let mutex_name = wide(&format!("Local\\{name}"));
		// SAFETY: name is NUL-terminated and alive; the mutex is not initially acquired by the thread.
		let mutex = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
		// SAFETY: capture last error before any other Win32 call.
		let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
		let mutex = owned(mutex)?;
		let name = wide(&format!("\\\\.\\pipe\\{name}-{session}"));
		if exists {
			forward(&name, request)?;
			return Ok(None);
		}
		// SAFETY: pipe name is valid UTF-16; default DACL restricts access to this user's token;
		// reject remote clients and refuse an existing first instance to prevent pipe takeover.
		let pipe = owned(unsafe {
			CreateNamedPipeW(
				name.as_ptr(),
				PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
				PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
				1,
				3,
				3,
				0,
				ptr::null(),
			)
		})?;
		let stop = Arc::new(event()?);
		let worker_stop = stop.clone();
		let worker = thread::Builder::new()
			.name("instance-pipe".into())
			.spawn(move || serve(pipe, worker_stop, handler))?;
		Ok(Some(Self {
			_mutex: mutex,
			stop,
			worker: Some(worker),
		}))
	}
}

fn serve(pipe: OwnedHandle, stop: Arc<OwnedHandle>, handler: Handler) {
	loop {
		let ready = match event() {
			Ok(event) => event,
			Err(error) => {
				eprintln!("instance event: {error}");
				return;
			}
		};
		let mut overlapped = OVERLAPPED {
			hEvent: ready.as_raw_handle(),
			..Default::default()
		};
		// SAFETY: valid pipe and OVERLAPPED remain alive through finish.
		let success = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), &mut overlapped) };
		// SAFETY: capture the result of ConnectNamedPipe immediately.
		let connected = success == 0 && unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
		if !connected {
			if let Err(error) = finish(&pipe, &stop, &mut overlapped, success != 0, INFINITE) {
				if error.kind() != io::ErrorKind::Interrupted {
					eprintln!("instance accept: {error}");
				}
				return;
			}
		}
		let mut bytes = [0; 3];
		let result = transfer(&pipe, &stop, &mut bytes, false, 3000)
			.and_then(|()| Request::decode(bytes))
			.and_then(|request| {
				if handler(request) {
					Ok(())
				} else {
					Err(io::Error::other("daemon is stopping"))
				}
			});
		let mut acknowledgement = [u8::from(result.is_ok())];
		if transfer(&pipe, &stop, &mut acknowledgement, true, 3000).is_ok() {
			let _ = transfer(&pipe, &stop, &mut [0], false, 3000);
		}
		// SAFETY: all operations have completed or were cancelled and drained.
		unsafe {
			DisconnectNamedPipe(pipe.as_raw_handle());
		}
	}
}

fn forward(name: &[u16], request: Request) -> io::Result<()> {
	let deadline = Instant::now() + Duration::from_secs(3);
	let pipe = loop {
		// SAFETY: valid pipe name and a bounded startup-only wait; no polling during daemon idle.
		unsafe {
			WaitNamedPipeW(name.as_ptr(), 100);
		}
		// SAFETY: name is NUL-terminated, requesting read/write on an existing local pipe.
		let handle = unsafe {
			CreateFileW(
				name.as_ptr(),
				0x80000000 | 0x40000000,
				0,
				ptr::null(),
				OPEN_EXISTING,
				FILE_FLAG_OVERLAPPED,
				ptr::null_mut(),
			)
		};
		match owned(handle) {
			Ok(pipe) => break pipe,
			Err(error) if Instant::now() >= deadline => return Err(error),
			Err(_) => thread::sleep(Duration::from_millis(10)),
		}
	};
	if matches!(request, Request::Show(_)) {
		let mut pid = 0;
		// SAFETY: the pipe is connected and pid is writable; foreground permission is advisory.
		if unsafe {
			windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId(
				pipe.as_raw_handle(),
				&mut pid,
			)
		} != 0
		{
			crate::desktop::allow_ui_foreground(pid);
		}
	}
	let stop = event()?;
	let mut bytes = request.encode();
	transfer(&pipe, &stop, &mut bytes, true, 3000)?;
	let mut acknowledgement = [0];
	transfer(&pipe, &stop, &mut acknowledgement, false, 3000)?;
	transfer(&pipe, &stop, &mut [1], true, 3000)?;
	if acknowledgement != [1] {
		return Err(io::Error::other("instance rejected the request"));
	}
	Ok(())
}

impl Drop for Instance {
	fn drop(&mut self) {
		// SAFETY: the stop event remains alive until the worker has drained pending I/O.
		unsafe {
			SetEvent(self.stop.as_raw_handle());
		}
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	#[ignore = "requires Windows kernel pipe/mutex and must run only against its unique test name"]
	fn named_pipe_forwards_and_shutdown_releases_waiting_server() {
		let name = format!("org.local.WhyHere.Test{}", std::process::id());
		let (sender, receiver) = std::sync::mpsc::channel();
		let first = Instance::claim_named(
			&name,
			Request::Background,
			Box::new(move |request| sender.send(request).is_ok()),
		)
		.unwrap()
		.unwrap();
		for request in [
			Request::Show(None),
			Request::Show(Some(why_core::i18n::Language::En)),
			Request::Background,
		] {
			assert!(Instance::claim_named(&name, request, Box::new(|_| false))
				.unwrap()
				.is_none());
			assert_eq!(
				receiver.recv_timeout(Duration::from_secs(3)).unwrap(),
				request
			);
		}
		drop(first);
		let next = Instance::claim_named(&name, Request::Background, Box::new(|_| true))
			.unwrap()
			.unwrap();
		drop(next);
	}
}
