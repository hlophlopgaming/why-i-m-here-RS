use super::{Handler, Request};
use why_core::APP_ID;
use zbus::blocking::{connection::Builder, Connection, Proxy};

const PATH: &str = "/org/local/WhyHere";
const INTERFACE: &str = "org.local.WhyHere.Control";

struct Control(Handler);

#[zbus::interface(name = "org.local.WhyHere.Control")]
impl Control {
	fn activate(&self, version: u8, command: u8, language: u8) -> zbus::fdo::Result<()> {
		let request = Request::decode([version, command, language])
			.map_err(|error| zbus::fdo::Error::InvalidArgs(error.to_string()))?;
		if !(self.0)(request) {
			return Err(zbus::fdo::Error::Failed("daemon is stopping".into()));
		}
		Ok(())
	}
}

pub struct Instance {
	_connection: Connection,
}

impl Instance {
	pub(crate) fn connection(&self) -> &Connection {
		&self._connection
	}

	/// Returns None after forwarding to the existing owner; never queues for ownership.
	pub fn claim(
		request: Request,
		handler: impl Fn(Request) -> bool + Send + Sync + 'static,
	) -> zbus::Result<Option<Self>> {
		Self::claim_name(APP_ID, request, Box::new(handler))
	}

	fn claim_name(name: &str, request: Request, handler: Handler) -> zbus::Result<Option<Self>> {
		let connection = Builder::session()?
			.method_timeout(std::time::Duration::from_secs(3))
			.serve_at(PATH, Control(handler))?
			.build()?;
		match connection
			.request_name_with_flags(name, zbus::fdo::RequestNameFlags::DoNotQueue.into())
		{
			Ok(
				zbus::fdo::RequestNameReply::PrimaryOwner
				| zbus::fdo::RequestNameReply::AlreadyOwner,
			) => Ok(Some(Self {
				_connection: connection,
			})),
			Ok(_) | Err(zbus::Error::NameTaken) => {
				let [version, command, language] = request.encode();
				Proxy::new(&connection, name, PATH, INTERFACE)?
					.call::<_, _, ()>("Activate", &(version, command, language))?;
				Ok(None)
			}
			Err(error) => Err(error),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	#[ignore = "requires a D-Bus session; use dbus-run-session"]
	fn ownership_forwarding_and_release_use_one_owner() {
		let name = format!("org.local.WhyHere.Test{}", std::process::id());
		let (sender, receiver) = std::sync::mpsc::channel();
		let first = Instance::claim_name(
			&name,
			Request::Background,
			Box::new(move |request| sender.send(request).is_ok()),
		)
		.unwrap()
		.unwrap();
		assert!(Instance::claim_name(
			&name,
			Request::Show(Some(why_core::i18n::Language::En)),
			Box::new(|_| false)
		)
		.unwrap()
		.is_none());
		assert_eq!(
			receiver
				.recv_timeout(std::time::Duration::from_secs(3))
				.unwrap(),
			Request::Show(Some(why_core::i18n::Language::En))
		);
		drop(first);
		assert!(
			Instance::claim_name(&name, Request::Background, Box::new(|_| true))
				.unwrap()
				.is_some()
		);
	}
}
