use std::io;
use why_core::{i18n::Language, settings::TrackedApp};
use windows::{
	core::{w, Interface, PWSTR},
	Win32::{
		Foundation::PROPERTYKEY,
		System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED},
		UI::Shell::{
			BHID_EnumItems, IEnumShellItems, IShellItem, IShellItem2, SHCreateItemFromParsingName,
			SIGDN_NORMALDISPLAY,
		},
	},
};

// https://learn.microsoft.com/windows/win32/properties/props-system-appusermodel-id
const APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
	fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
	pid: 5,
};

struct Apartment;
impl Drop for Apartment {
	fn drop(&mut self) {
		// SAFETY: Created only after successful CoInitializeEx, dropped on the same thread.
		unsafe { CoUninitialize() };
	}
}

struct ShellString(PWSTR);
impl ShellString {
	fn decode(self) -> io::Result<String> {
		if self.0.is_null() {
			return Err(io::Error::other("Shell returned a null string"));
		}
		// SAFETY: Shell returns an owned, NUL-terminated UTF-16 allocation held until Drop.
		unsafe { self.0.to_string() }.map_err(io::Error::other)
	}
}
impl Drop for ShellString {
	fn drop(&mut self) {
		// SAFETY: GetString/GetDisplayName allocate with the COM task allocator; freed once.
		unsafe { CoTaskMemFree(Some(self.0.as_ptr().cast())) };
	}
}

pub(super) fn enumerate(_language: Language) -> io::Result<Vec<TrackedApp>> {
	// SAFETY: This catalog worker owns its apartment and releases all interfaces before it.
	unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
		.ok()
		.map_err(io::Error::other)?;
	let _apartment = Apartment;
	// SAFETY: Constant parsing name; returned interfaces own their references through Drop.
	let folder: IShellItem = unsafe { SHCreateItemFromParsingName(w!("shell:AppsFolder"), None) }
		.map_err(io::Error::other)?;
	// SAFETY: The live shell folder supports enumeration; no bind context is needed.
	let items: IEnumShellItems =
		unsafe { folder.BindToHandler(None, &BHID_EnumItems) }.map_err(io::Error::other)?;
	let mut apps = Vec::new();
	loop {
		let mut next = [None];
		// SAFETY: Buffer has one slot; fetched may be null when requesting a single item.
		unsafe { items.Next(&mut next, None) }.map_err(io::Error::other)?;
		let Some(item) = next[0].take() else {
			break;
		};
		let item: IShellItem2 = item.cast().map_err(io::Error::other)?;
		// SAFETY: Live shell item and a constant string property key; both results are owned.
		let id =
			ShellString(unsafe { item.GetString(&APP_USER_MODEL_ID) }.map_err(io::Error::other)?)
				.decode()?;
		// SAFETY: The live item returns an owned display name, freed by ShellString.
		let name = ShellString(
			unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.map_err(io::Error::other)?,
		)
		.decode()?;
		apps.push(TrackedApp {
			id: format!("windows:{id}"),
			name,
		});
	}
	Ok(apps)
}

#[cfg(test)]
mod tests {
	#[test]
	fn apps_folder_catalog_on_worker_thread() {
		let apps = std::thread::spawn(|| super::super::installed_apps(super::Language::En))
			.join()
			.unwrap()
			.unwrap();
		assert!(!apps.is_empty());
		assert!(apps.iter().all(super::TrackedApp::is_valid));
		assert!(apps.iter().all(|app| app.id.starts_with("windows:")));
	}
}
