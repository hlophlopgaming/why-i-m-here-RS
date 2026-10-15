//! Windows message queue integration; the daemon never starts a GUI toolkit.
#[cfg(windows)]
use std::io;
#[cfg(windows)]
use windows_sys::Win32::{System::Threading::GetCurrentThreadId, UI::WindowsAndMessaging::*};

#[cfg(windows)]
pub fn message_thread() -> u32 {
	let mut message = MSG::default();
	// SAFETY: a valid MSG is provided; PeekMessage creates this thread's message queue.
	unsafe {
		PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
		GetCurrentThreadId()
	}
}

#[cfg(windows)]
pub fn wake(thread: u32) -> bool {
	// SAFETY: the destination is a thread ID with an initialized message queue; no pointers are sent.
	unsafe { PostThreadMessageW(thread, WM_APP, 0, 0) != 0 }
}

#[cfg(windows)]
pub fn wait_message() -> io::Result<bool> {
	let mut message = MSG::default();
	// SAFETY: GetMessage writes to a valid MSG for this thread.
	let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
	if result == -1 {
		return Err(io::Error::last_os_error());
	}
	if result == 0 {
		return Ok(false);
	}
	// SAFETY: message was initialized by GetMessage; dispatch runs on the owning thread.
	unsafe {
		TranslateMessage(&message);
		DispatchMessageW(&message);
	}
	Ok(true)
}

pub fn allow_ui_foreground(_pid: u32) {
	#[cfg(windows)]
	// SAFETY: this grants only our own newly spawned UI permission to request the foreground.
	unsafe {
		AllowSetForegroundWindow(_pid);
	}
}
