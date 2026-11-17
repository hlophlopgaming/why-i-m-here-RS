//! Pinned process identities; never send a termination request to a bare PID.
use std::{
	collections::{BTreeMap, BTreeSet},
	io,
};
use why_core::windows::WindowInfo;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::Process;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Process;

#[derive(Default)]
pub struct Targets(BTreeMap<u32, Process>);
impl Targets {
	pub fn update(&mut self, app: &str, windows: &[WindowInfo]) {
		for window in windows.iter().filter(|w| w.app_id == app && w.controllable) {
			if self.0.contains_key(&window.pid) {
				continue;
			}
			match Process::open(window.pid) {
				Ok(process) => {
					self.0.insert(window.pid, process);
				}
				Err(error) => eprintln!("pin application process {}: {error}", window.pid),
			}
		}
	}

	pub fn terminate(&self, app: &str, windows: &[WindowInfo]) -> io::Result<()> {
		// shortcut: only the live descendant tree is owned; track detached helpers if needed.
		let processes = Process::list()?;
		let mut protected = BTreeSet::new();
		protect_ancestors(std::process::id(), &processes, &mut protected)?;
		for window in windows
			.iter()
			.filter(|w| w.app_id != app || !w.controllable)
		{
			protect_ancestors(window.pid, &processes, &mut protected)?;
		}
		let mut ordered = Vec::new();
		let mut visited = BTreeSet::new();
		for (&pid, root) in &self.0 {
			if let Some(process) = processes.get(&pid) {
				if root.alive() && root.started == process.started {
					collect(pid, &processes, &protected, &mut visited, &mut ordered);
				}
			}
		}
		let mut error = None;
		for pid in ordered {
			if let Err(failure) = processes[&pid].kill() {
				error = Some(failure);
			}
		}
		if let Some(error) = error {
			return Err(error);
		}
		Ok(())
	}
}

fn protect_ancestors(
	pid: u32,
	processes: &BTreeMap<u32, Process>,
	protected: &mut BTreeSet<u32>,
) -> io::Result<()> {
	let mut current = pid;
	while current > 0 && protected.insert(current) {
		let process = processes
			.get(&current)
			.ok_or_else(|| io::Error::other("cannot verify protected process ancestry"))?;
		current = process.parent;
	}
	Ok(())
}

fn collect(
	pid: u32,
	processes: &BTreeMap<u32, Process>,
	protected: &BTreeSet<u32>,
	visited: &mut BTreeSet<u32>,
	ordered: &mut Vec<u32>,
) {
	if protected.contains(&pid) || !visited.insert(pid) {
		return;
	}
	let parent = &processes[&pid];
	if !parent.safe {
		return;
	}
	for (&child_pid, child) in processes {
		// A reused parent PID must not adopt an older, unrelated process.
		if child.parent == pid
			&& child.parent_started == Some(parent.started)
			&& child.started >= parent.started
			&& parent.alive()
		{
			collect(child_pid, processes, protected, visited, ordered);
		}
	}
	ordered.push(pid);
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
	use super::*;
	use std::{
		fs,
		process::{Child, Command},
		time::{Duration, Instant},
	};
	struct Fixture(Child);
	impl Drop for Fixture {
		fn drop(&mut self) {
			let _ = self.0.kill();
			let _ = self.0.wait();
		}
	}
	fn window(pid: u32, app: &str) -> WindowInfo {
		WindowInfo {
			id: pid.to_string(),
			pid,
			app_id: app.into(),
			controllable: true,
		}
	}
	fn fixture() -> Fixture {
		Fixture(Command::new("sleep").arg("30").spawn().unwrap())
	}
	#[test]
	fn terminates_only_pinned_fixture_and_preserves_other_app() {
		let mut target = fixture();
		let mut other = fixture();
		let windows = [
			window(target.0.id(), "target"),
			window(other.0.id(), "other"),
		];
		let mut targets = Targets::default();
		targets.update("target", &windows);
		targets.terminate("target", &windows).unwrap();
		assert!(!target.0.wait().unwrap().success());
		assert!(other.0.try_wait().unwrap().is_none());
	}
	#[test]
	fn never_terminates_self_or_a_root_shared_with_another_app() {
		let mut target = fixture();
		let mut windows = vec![
			window(target.0.id(), "target"),
			window(target.0.id(), "other"),
		];
		let mut targets = Targets::default();
		targets.update("target", &windows);
		targets.terminate("target", &windows).unwrap();
		assert!(target.0.try_wait().unwrap().is_none());
		let mut targets = Targets::default();
		windows = vec![window(std::process::id(), "target")];
		targets.update("target", &windows);
		targets.terminate("target", &windows).unwrap();
	}
	#[test]
	fn terminates_descendants_before_root_and_rejects_changed_parent_identity() {
		let mut root = Fixture(
			Command::new("sh")
				.args(["-c", "sleep 30 & wait"])
				.spawn()
				.unwrap(),
		);
		let deadline = Instant::now() + Duration::from_secs(3);
		let child_pid = loop {
			let children =
				fs::read_to_string(format!("/proc/{0}/task/{0}/children", root.0.id())).unwrap();
			if let Some(pid) = children.split_whitespace().next() {
				break pid.parse::<u32>().unwrap();
			}
			assert!(Instant::now() < deadline);
			std::thread::sleep(Duration::from_millis(10));
		};
		let child = Process::open(child_pid).unwrap();
		let mut processes = Process::list().unwrap();
		let mut ordered = Vec::new();
		collect(
			root.0.id(),
			&processes,
			&BTreeSet::new(),
			&mut BTreeSet::new(),
			&mut ordered,
		);
		assert_eq!(ordered, [child_pid, root.0.id()]);
		processes.get_mut(&child_pid).unwrap().parent_started = Some(u64::MAX);
		ordered.clear();
		collect(
			root.0.id(),
			&processes,
			&BTreeSet::new(),
			&mut BTreeSet::new(),
			&mut ordered,
		);
		assert_eq!(ordered, [root.0.id()]);
		let windows = [window(root.0.id(), "target")];
		let mut targets = Targets::default();
		targets.update("target", &windows);
		targets.terminate("target", &windows).unwrap();
		assert!(!root.0.wait().unwrap().success());
		// pidfd becomes readable even while the dead child awaits reaping.
		while child.alive() {
			assert!(Instant::now() < deadline);
			std::thread::sleep(Duration::from_millis(10));
		}
	}
	#[test]
	fn changed_root_identity_and_system_process_are_not_selected() {
		let mut fixture = fixture();
		let windows = [window(fixture.0.id(), "target")];
		let mut targets = Targets::default();
		targets.update("target", &windows);
		targets.0.get_mut(&fixture.0.id()).unwrap().started = u64::MAX;
		targets.update("target", &windows);
		targets.terminate("target", &windows).unwrap();
		assert!(fixture.0.try_wait().unwrap().is_none());
		let mut processes = Process::list().unwrap();
		processes.get_mut(&fixture.0.id()).unwrap().safe = false;
		let mut ordered = Vec::new();
		collect(
			fixture.0.id(),
			&processes,
			&BTreeSet::new(),
			&mut BTreeSet::new(),
			&mut ordered,
		);
		assert!(ordered.is_empty());
	}
}
