// Config is supplied by the daemon; this script never starts or kills processes.
(function () {
	const config = __WHY_HERE_CONFIG__;
	let blocked = config.blocked;
	let focus = {};
	let stopped = false;
	let busy = false;
	let again = false;
	const held = {};
	const watched = {};
	const pulse = new QTimer();
	pulse.interval = 1000;
	const watchdog = new QTimer();
	watchdog.interval = 3500;
	watchdog.singleShot = true;
	const debounce = new QTimer();
	debounce.interval = 20;
	debounce.singleShot = true;

	function appId(window) {
		if (!window.normalWindow || window.pid === config.pid) return null;
		const desktop = String(window.desktopFileName || "");
		if (desktop === "org.local.WhyHere") return null;
		for (let i = 0; i < config.apps.length; ++i) {
			const app = config.apps[i];
			if (desktop === app.desktop || desktop === app.file) return app.id;
		}
		return null;
	}
	function release(id) {
		const previous = held[id];
		if (!previous) return;
		delete held[id];
		try { previous.window.minimized = previous.minimized; } catch (_) {}
	}
	function releaseAll() {
		Object.keys(held).forEach(release);
	}
	function focusGoal(app) {
		const focusPid = focus[app];
		if (!focusPid) return;
		const list = workspace.windowList();
		for (let i = 0; i < list.length; ++i) {
			const window = list[i];
			// Titles must stay in sync with the Russian and English goal forms.
			if (window.pid === focusPid &&
				(String(window.caption).indexOf("Зачем я здесь? — ") === 0 ||
				 String(window.caption).indexOf("Why am I here? — ") === 0)) {
				window.minimized = false;
				workspace.activeWindow = window;
				return;
			}
		}
	}
	function enforce(window) {
		if (stopped) return;
		if (String(window.desktopFileName) === "org.local.WhyHere" &&
			(String(window.caption) === "Таймер" || String(window.caption) === "Timer")) {
			window.keepAbove = true;
		}
		const app = appId(window);
		const id = String(window.internalId);
		if (app && blocked.indexOf(app) >= 0 && window.minimizable) {
			if (!held[id]) {
				held[id] = { window: window, minimized: window.minimized };
				watchdog.start();
			}
			if (!window.minimized) { window.minimized = true; focusGoal(app); }
		} else {
			release(id);
		}
	}
	function schedule() {
		if (!stopped && !debounce.active) debounce.start();
	}
	function watch(window) {
		if (stopped) return;
		const id = String(window.internalId);
		if (!watched[id]) {
			watched[id] = true;
			window.minimizedChanged.connect(function () {
				if (!stopped && !window.minimized) enforce(window);
			});
			window.windowClassChanged.connect(schedule);
			window.desktopFileNameChanged.connect(schedule);
		}
		enforce(window);
	}
	function exchange() {
		if (stopped) return;
		if (busy) { again = true; return; }
		const list = workspace.windowList();
		const snapshot = [];
		for (let i = 0; i < list.length; ++i) {
			const window = list[i];
			watch(window);
			const app = appId(window);
			if (app) snapshot.push({ id: String(window.internalId), app_id: app,
				pid: window.pid, controllable: window.minimizable });
		}
		// No periodic wakeups without a tracked window.
		if (snapshot.length) pulse.start(); else pulse.stop();
		busy = true;
		if (snapshot.length) watchdog.start();
		callDBus(config.service, "/org/local/WhyHere/Windows", "org.local.WhyHere.Windows",
			"Exchange", "1", String(config.pid), JSON.stringify(snapshot), false, function (json) {
				if (stopped) return;
				busy = false;
				const reply = JSON.parse(json);
				if (reply.watch && snapshot.length) watchdog.start(); else watchdog.stop();
				blocked = reply.blocked;
				if (!reply.watch && !reply.close.length) pulse.stop();
				focus = reply.focus;
				if (!reply.running) {
					stopped = true;
					pulse.stop(); watchdog.stop(); debounce.stop();
					releaseAll();
					callDBus(config.service, "/org/local/WhyHere/Windows", "org.local.WhyHere.Windows",
						"Exchange", "1", String(config.pid), "[]", true);
					return;
				}
				const current = workspace.windowList();
				current.forEach(function (window) {
					enforce(window);
					if (reply.close.indexOf(String(window.internalId)) >= 0) {
						release(String(window.internalId));
						window.closeWindow();
					}
				});
				if (again) { again = false; schedule(); }
			});
	}
	pulse.timeout.connect(exchange);
	debounce.timeout.connect(exchange);
	watchdog.timeout.connect(function () {
		stopped = true;
		pulse.stop(); debounce.stop();
		releaseAll();
	});
	workspace.windowAdded.connect(function (window) { watch(window); schedule(); });
	workspace.windowRemoved.connect(function (window) {
		const id = String(window.internalId);
		delete held[id]; delete watched[id]; schedule();
	});
	workspace.windowActivated.connect(function (window) {
		if (window && !stopped) enforce(window);
	});
	exchange();
})();
