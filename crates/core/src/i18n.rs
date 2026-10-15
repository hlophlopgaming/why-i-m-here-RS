use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
	#[default]
	Ru,
	En,
}

pub fn tr(language: Language, source: &'static str) -> &'static str {
	if language == Language::Ru {
		return source;
	}
	match source {
		"Автозапуск недоступен в деморежиме." => "Autostart is unavailable in demo mode.",
		"Применяется при сохранении. Запускает приложение в трее без окна." => "Applied when saving. Starts in the tray without a window.",
		"Не удалось сохранить настройки или автозапуск. Проверьте права и свободное место; повторите попытку." => "Unable to save settings or autostart. Check permissions and free space, then retry.",
		"Закончить сессию" => "Finish session",
		"Сессия" => "Session",
		"Программа скрыта до ответа. Закрытие вопроса закроет программу." => "The application is hidden until you answer. Closing this question closes the application.",
		"Закрытие вопроса закроет программу. После ответа начнётся таймер." => "Closing this question closes the application. The timer starts after you answer.",
		"Некоторыми окнами нельзя управлять: повышенные права или запрет сворачивания." => "Some windows cannot be controlled: elevated privileges or minimizing is unavailable.",
		"Отслеживание окон подключено" => "Window tracking is connected",
		"Отслеживание выключено" => "Tracking is disabled",
		"Добавьте программы для отслеживания." => "Add applications to track.",
		"Бэкенд окон недоступен. Проверьте KWin и права доступа; сохраните настройки для переподключения." => "Window backend unavailable. Check KWin and permissions; save settings to reconnect.",
		"Отслеживаемые программы" => "Tracked applications",
		"При запуске выбранной программы появится вопрос о цели. По окончании сессии программа закроется." => "Launching a selected application opens a goal question. The application closes when the session ends.",
		"Обычное закрытие позволяет программе предложить сохранение документов." => "Normal closing lets the application offer to save documents.",
		"Доступны отслеживание окон, таймер, трей, настройки и деморежим." => "Window tracking, timer, tray, settings and demo mode are available.",
		"Зачем я здесь?" => "Why am I here?",
		"Доступен деморежим. KWin, Win32 и трей пока не подключены." => {
			"Demo mode is available. KWin, Win32 and tray are not connected yet."
		}
		"Демонстрационная программа" => "Demo application",
		"Деморежим: другие программы и настройки не изменяются." => {
			"Demo: other programs and settings are untouched."
		}
		"Цель" => "Goal",
		"Время (секунды)" => "Time (seconds)",
		"Начать" => "Start",
		"Настройки" => "Settings",
		"Настройки деморежима" => "Demo settings",
		"Способ завершения" => "End mode",
		"Закрыть окна" => "Close windows",
		"Завершить процессы" => "Terminate processes",
		"Применить к демосессии" => "Apply to demo session",
		"Цель должна содержать от 1 до 1024 символов." => {
			"Enter a goal containing 1 to 1024 characters."
		}
		"Введите целое число от 1 до 86400 секунд." => {
			"Enter a whole number from 1 to 86400 seconds."
		}
		"Таймер" => "Timer",
		"Остановить демосессию" => "Stop demo session",
		"Сессия завершена" => "Session finished",
		"Имитировано закрытие окон. Другие программы не затронуты." => {
			"Window closing simulated. Other programs were untouched."
		}
		"Имитировано завершение процессов. Другие программы не затронуты." => {
			"Process termination simulated. Other programs were untouched."
		}
		"Ещё раз" => "Try again",
		"Закрыть" => "Close",
		"Сначала введите цель" => "Enter a goal first",
		"Прочитать документацию" => "Read the documentation",
		"Пока доступен только деморежим. Запустите why-here --demo." => {
			"Only demo mode is available yet. Run why-here --demo."
		}
		"Введите целое число от 1 до 1440 минут." => "Enter a whole number from 1 to 1440 minutes.",
		"Интервал предупреждения: от 0 до 1440 минут." => "Warning interval: 0 to 1440 minutes.",
		"Что нужно сделать в этой программе?" => "What do you need to do in this app?",
		"Например: ответить на письмо" => "For example: reply to an email",
		"Сколько времени вы хотите потратить на задачу?" => "How much time do you want to spend on this task?",
		"Быстрый выбор, мин" => "Quick choice, min",
		"До начала сессии форму нельзя закрыть. Введите цель и время." => "This form cannot be closed before the session starts. Enter a goal and time.",
		"Закрытие формы или Escape имитирует завершение программы без сохранения." => "Closing this form or pressing Escape simulates quitting the app without saving.",
		"Демосессия" => "Demo session",
		"Время заканчивается" => "Time is running out",
		"Завершить сессию сейчас" => "End session now",
		"Откройте программу. Поставьте цель. Выделите время." => "Open an app. Set a goal. Make time for it.",
		"Деморежим · KWin / Win32 не подключены" => "Demo mode · KWin / Win32 not connected",
		"Отслеживание включено" => "Tracking enabled",
		"Отслеживаемые программы (демосписок)" => "Tracked applications (demo list)",
		"Добавить демопрограмму…" => "Add a demo application…",
		"Из установленных программ…" => "From installed apps…",
		"Будет доступно после подключения платформенного бэкенда." => "Available once the platform backend is connected.",
		"Удалить" => "Remove",
		"Длительность по умолчанию, мин" => "Default duration, min",
		"Предупредить за, мин (0 = в конце)" => "Warn before end, min (0 = at end)",
		"Непрозрачность напоминания" => "Reminder opacity",
		"Прозрачность пока недоступна в нативном Wayland." => "Opacity is not yet available in native Wayland.",
		"Звуковое предупреждение" => "Play a warning sound",
		"При завершении новых сессий" => "When new sessions end",
		"Принудительное завершение может потерять несохранённые данные.\nВ деморежиме действие только имитируется." => "Terminating a process may discard unsaved work.\nIn demo mode the action is only simulated.",
		"Запускать при входе в систему" => "Start when signing in",
		"Язык следующего UI-процесса" => "Language of the next UI process",
		"Текущие сессии" => "Current sessions",
		"Программа" => "Application",
		"Осталось" => "Remaining",
		"Активных сессий нет. Выберите программу и откройте форму цели." => "No active sessions. Select an application and open its goal form.",
		"Открыть форму цели" => "Open goal form",
		"Переподключить KWin / Win32" => "Reconnect KWin / Win32",
		"Выйти" => "Quit",
		"Браузер (демо)" => "Browser (demo)",
		"Редактор (демо)" => "Editor (demo)",
		"Мессенджер (демо)" => "Messenger (demo)",
		"Один сигнал перед концом сессии. В Linux требуется pw-play." => "One warning per session. Linux requires pw-play.",
		"Отслеживаемые программы (демосессии)" => "Tracked applications (demo sessions)",
		"Можно выбрать не более 64 программ." => "You can select up to 64 applications.",
		"Установленные программы" => "Installed applications",
		"Поиск по названию или ID" => "Search by name or ID",
		"Загрузка списка программ…" => "Loading applications…",
		"Выбор добавит программу в демосессию. Она не будет запущена или завершена." => "Selection adds the app to a demo session. It will not be launched or terminated.",
		"Добавить выбранную" => "Add selected",
		"Отмена" => "Cancel",
		"Не удалось загрузить список. Закройте настройки и повторите попытку." => "Unable to load apps. Close settings and try again.",
		"Установленные программы не найдены." => "No installed applications found.",
		"Выберите программу из списка." => "Select an application from the list.",
		"Настройки сохраняются · KWin / Win32 не подключены" => "Settings are saved · KWin / Win32 not connected",
		"Выбранные программы (отслеживание пока недоступно)" => "Selected applications (tracking not yet available)",
		"Демопрограммы доступны только с --demo." => "Demo applications are only available with --demo.",
		"Настройки не запускают сессии и не закрывают программы." => "Settings do not start sessions or close applications.",
		"Сохранить и закрыть" => "Save and close",
		"Не удалось сохранить настройки. Проверьте права и свободное место; повторите попытку." => "Unable to save settings. Check permissions and free space, then retry.",
		"Выбор добавит программу в список. Она не будет запущена или завершена." => "Selection adds the app to the list. It will not be launched or terminated.",
		"Доступны деморежим и сохранение настроек (--show). KWin, Win32 и трей пока не подключены." => "Demo mode and saved settings (--show) are available. KWin, Win32 and tray are not connected yet.",
		"Отслеживание пока недоступно. Запустите why-here --demo или why-here --show." => "Tracking is not available yet. Run why-here --demo or why-here --show.",
		"Выйти из приложения" => "Quit application",
		"Закрыть настройки" => "Close settings",
		"Доступны трей, настройки и деморежим. Отслеживание окон пока не подключено." => "Tray, preferences and demo mode are available. Window tracking is not connected yet.",
		_ => source,
	}
}
