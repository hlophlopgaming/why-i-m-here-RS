use std::io;
use why_core::i18n::Language;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Instance;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Instance;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
	Show(Option<Language>),
	Background,
	Quit,
}

impl Request {
	fn encode(self) -> [u8; 3] {
		match self {
			Self::Show(language) => [
				1,
				1,
				match language {
					None => 0,
					Some(Language::Ru) => 1,
					Some(Language::En) => 2,
				},
			],
			Self::Background => [1, 2, 0],
			Self::Quit => [1, 3, 0],
		}
	}

	fn decode(bytes: [u8; 3]) -> io::Result<Self> {
		match bytes {
			[1, 1, language @ 0..=2] => Ok(Self::Show(match language {
				1 => Some(Language::Ru),
				2 => Some(Language::En),
				_ => None,
			})),
			[1, 2, 0] => Ok(Self::Background),
			[1, 3, 0] => Ok(Self::Quit),
			_ => Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"unsupported instance request",
			)),
		}
	}
}

type Handler = Box<dyn Fn(Request) -> bool + Send + Sync>;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn instance_protocol_round_trips_and_rejects_invalid_commands() {
		for request in [
			Request::Show(None),
			Request::Show(Some(Language::Ru)),
			Request::Show(Some(Language::En)),
			Request::Background,
			Request::Quit,
		] {
			assert_eq!(Request::decode(request.encode()).unwrap(), request);
		}
		for bytes in [
			[0, 1, 0],
			[2, 1, 0],
			[1, 0, 0],
			[1, 1, 3],
			[1, 2, 1],
			[1, 3, 1],
		] {
			assert!(Request::decode(bytes).is_err());
		}
	}
}
