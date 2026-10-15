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
pub fn wait_message() -> io::Result<Message> {
	let mut message = MSG::default();
	// SAFETY: GetMessage writes to a valid MSG for this thread.
	let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
	if result == -1 {
		return Err(io::Error::last_os_error());
	}
	if result == 0 {
		return Ok(Message::Quit);
	}
	// SAFETY: message was initialized by GetMessage; dispatch runs on the owning thread.
	unsafe {
		TranslateMessage(&message);
		DispatchMessageW(&message);
	}
	Ok(if message.message == WM_TIMER && message.hwnd.is_null() {
		Message::Tick
	} else {
		Message::Wake
	})
}

pub fn allow_ui_foreground(_pid: u32) {
	#[cfg(windows)]
	// SAFETY: this grants only our own newly spawned UI permission to request the foreground.
	unsafe {
		AllowSetForegroundWindow(_pid);
	}
}

#[cfg(windows)]
pub enum Message {
	Quit,
	Tick,
	Wake,
}

#[cfg(windows)]
pub struct Timer(usize, std::marker::PhantomData<std::rc::Rc<()>>);
#[cfg(windows)]
impl Timer {
	pub fn new() -> io::Result<Self> {
		// SAFETY: Thread timer on the daemon's message-loop thread, owned until Drop.
		let id = unsafe { SetTimer(std::ptr::null_mut(), 0, 1000, None) };
		if id == 0 {
			Err(io::Error::last_os_error())
		} else {
			Ok(Self(id, std::marker::PhantomData))
		}
	}
}
#[cfg(windows)]
impl Drop for Timer {
	fn drop(&mut self) {
		// SAFETY: Stops the thread timer created and owned by this object on the same thread.
		unsafe { KillTimer(std::ptr::null_mut(), self.0) };
	}
}
