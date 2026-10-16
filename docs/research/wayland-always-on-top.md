# Панель таймера поверх приложений в Wayland без KWin Scripting

Дата исследования: 16 октября 2026. Область: существующая панель FLTK 1.5.23, KDE Plasma 6, без изменения пользовательских настроек и без прототипа.

## Вывод

**Нативный путь для текущего KDE-проекта существует:** клиент `org_kde_plasma_window_management` может найти собственное окно таймера и запросить `keep_above`, не меняя FLTK и не загружая JavaScript в KWin. Это KDE-специфичный внутренний протокол с ограничениями доступа; нужен небольшой Wayland-клиент в `platform`, вызываемый демоном, и корректная установка `.desktop`. Это наиболее подходящий вариант, если принципиально сохранить нативный Wayland. Самый маленький обходной путь — запускать только дочерний процесс таймера через XWayland: `FLTK_BACKEND=x11`, затем существующий `set_on_top()`. Последний работает в Wayland-сессии, но окно уже X11. Ни один вариант `keep_above` сам по себе не означает «только над выбранным приложением» или «над fullscreen».

## Почему текущий вызов не работает

В установленном `fltk-rs 1.5.23` Linux-реализация `Window::set_on_top()` явно вызывает нативный код только при `!app::using_wayland()`. Поэтому нынешний вызов после `window.show()` в `crates/ui/src/lib.rs` для нативного Wayland не посылает запрос закрепления. [Исходник fltk-rs 1.5.23](https://github.com/fltk-rs/fltk-rs/blob/1.5.23/fltk/src/window.rs).

В стандартном `xdg_toplevel` есть maximize, fullscreen, minimize, move, parent и метаданные, но нет запроса always-on-top. `set_parent` принимает объект родителя, доступный на этом Wayland-соединении; он не позволяет произвольно выбрать чужое приложение по PID. [Официальная спецификация xdg-shell](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/xdg-shell/xdg-shell.xml).

Протокол `xdg-toplevel-tag` помогает композитору узнавать окна для сохранённых правил, включая закрепление, но сам не запрашивает его. Это не замена always-on-top API. [Спецификация xdg-toplevel-tag](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/xdg-toplevel-tag/xdg-toplevel-tag-v1.xml).

## Сравнение вариантов

| Вариант | Сохранит текущую FLTK-панель | Нативный Wayland | Практический результат |
| --- | --- | --- | --- |
| KDE plasma-window-management | Да | Да | Запрос keep-above для существующего окна; нужен доступ к протоколу |
| Правило окна KDE | Да | Да | Мало кода, но отдельная настройка/импорт правила пользователем |
| XWayland только для таймера | Да | Нет | Минимальный автоматический обход через существующий set_on_top |
| wlr-layer-shell | Не без нового backend/панели | Да | Полноценный overlay, включая fullscreen; существенно больше работы |
| KDE plasma-shell OSD | Потребуется нативная интеграция | Да | Внутренний shell API с неподходящей семантикой OSD |

## KDE plasma-window-management: нативный путь

В XML есть `get_window_by_uuid`, события UUID, app_id/PID и `org_kde_plasma_window.set_state(flags, state)`. Для закрепления достаточно `flags=0x10`, `state=0x10` (`keep_above`), а не пересылать полный набор состояний. В актуальном XML есть предупреждение, что это внутренний протокол KDE и обычным приложениям не следует на него полагаться как на стабильный общий API. [Официальный XML KDE](https://github.com/KDE/plasma-wayland-protocols/blob/master/src/protocols/plasma-window-management.xml).

Это не только декларация: KWin преобразует бит запроса в `keepAboveRequested`, а окно вызывает `setKeepAbove(set)`. Таким способом можно управлять уже существующим FLTK-окном с отдельного Wayland-соединения демона; доступ к `wl_surface` FLTK не требуется. [Обработчик протокола KWin 6.7.0](https://github.com/KDE/kwin/blob/v6.7.0/src/wayland/plasmawindowmanagement.cpp), [связь с Window](https://github.com/KDE/kwin/blob/v6.7.0/src/window.cpp).

Ограничение установки существенно: в проверенных тегах KWin 6.0.0, 6.2.0, 6.3.0, 6.4.0, 6.5.0, 6.6.0 и 6.7.0 интерфейс находится в `interfacesBlackList`; доступ требует объявления `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` в `.desktop`, который KWin сопоставит исполняемому файлу. Просто выставить `app_id` недостаточно. В `master` политика уже другая: несандбоксированным клиентам интерфейс разрешён, sandbox ограничен. Нельзя переносить поведение master на все Plasma 6. Не нужно отключать permission checks глобально. [KWin 6.7.0: allowInterface](https://github.com/KDE/kwin/blob/v6.7.0/src/wayland_server.cpp), [текущий master](https://github.com/KDE/kwin/blob/master/src/wayland_server.cpp).

Для проекта минимальная нативная реализация означает:

1. Установленный `.desktop` в applications с корректным `Exec` и объявлением интерфейса; корректный отказ при отсутствии доступа в dev-запуске. В репозитории пока нет шаблона такого установочного файла: запись автозапуска не заменяет пункт applications.
2. Helper в `crates/platform`: соединение с compositor, registry/event dispatch, поиск окна по PID запущенного UI-процесса плюс `APP_ID`, получение UUID и установка одного бита. Заголовок «Таймер» не годится как единственный идентификатор.
3. Вызов из демона после появления окна таймера; подтверждение по событию состояния. UI не управляет чужими окнами; `unsafe`, если потребуется, остаётся в platform.
4. Завершение соединения без постоянного polling; корректный отказ при перезапуске KWin/закрытии UI. Совпадение всех окон приложения по одному app_id не должно закрепить настройки или форму цели.

Это проектная рекомендация по исходникам, а не проверенная работающая реализация.

## Самый маленький обход: XWayland

FLTK официально поддерживает выбор backend переменной `FLTK_BACKEND=x11`, даже когда запущен Wayland compositor; hybrid-сборка должна включать X11. Можно передавать переменную только `Command` дочернего `ui timer`, сохраняя Wayland для остальных окон и окружение демона. [Официальный README FLTK](https://github.com/fltk/fltk/blob/master/README.Wayland.txt).

Текущий Linux-код fltk-rs после этого действительно проходит в X11-ветку `set_on_top()`. Нужно проверить XWayland и наличие DISPLAY, сохранение app_id/class для существующего bridge, масштабирование и перемещение на нескольких мониторах. Поверх нативных Wayland-окон stacking определяет KWin; fullscreen-гарантии нет. Это fallback, если важнее минимальный объём изменений, чем нативный Wayland.

## wlr-layer-shell: путь для настоящего overlay

Протокол назначает отдельную роль поверхности. Обычные окна располагаются между `bottom` и `top`, fullscreen обычно на `top`; `overlay` выше. Для таймера разумны отсутствие exclusive keyboard focus и `exclusive_zone=0` либо `-1` в зависимости от нужного пересечения панелей. При `-1` поверхность игнорирует exclusive zones. [Официальный XML wlr-layer-shell](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-layer-shell-unstable-v1.xml).

KWin реализует `top` как `AboveLayer`, `overlay` как `OverlayLayer`. [Исходник KWin](https://github.com/KDE/kwin/blob/master/src/layershellv1window.cpp). Это слой экрана, а не дочернее окно конкретной программы.

Превратить уже показанное FLTK `xdg_toplevel` в layer-surface одним вызовом нельзя: layer-shell отвергает поверхность с другой ролью или уже committed buffer. Потребуется создавать поверхность с нужной ролью изначально, менять FLTK backend или писать отдельную Linux-панель (отрисовка, ввод, масштабирование, configure/ack, выводы). Публичные native handles FLTK не отменяют конфликт ролей. В проекте это неоправданно большая переделка ради обычного keep-above; имеет смысл, если обязательны fullscreen-overlay и готовность поддерживать собственный Wayland backend.

## Почему не plasma-shell и не автоматическая правка правил

`org_kde_plasma_shell` — внутренний API desktop shell. Роль `onscreendisplay` предназначена для кратких индикаторов вроде громкости, не для постоянного интерактивного таймера. `set_panel_behavior` в Plasma 6 прямо помечен как не действующий, с рекомендацией layer-shell. [XML plasma-shell](https://github.com/KDE/plasma-wayland-protocols/blob/master/src/protocols/plasma-shell.xml). KWin действительно может добавлять plasma-shell метаданные к xdg-toplevel; это не тот же конфликт ролей, что у wlr-layer-shell, но семантически неподходящий и внутренний обход. [Регистрация xdg-toplevel](https://github.com/KDE/kwin/blob/v6.7.0/src/wayland_server.cpp).

Оконные правила KDE поддерживают Keep above, сопоставление окна, импорт и экспорт. Можно поставлять отдельное правило, которое пользователь импортирует, не устанавливая script. [Официальная документация редактора](https://docs.kde.org/stable_kf6/en/kwin/kcontrol/windowspecific/kwin-rule-editor.html), [атрибуты правил](https://userbase.kde.org/KWin_Rules_Window_Attributes/en). Самостоятельно переписывать `kwinrulesrc` ради одного флага хуже прямого запроса: это постоянная пользовательская конфигурация, которую придётся сливать и сопровождать.

## Семантика, которую надо выбрать до реализации

- **Поверх обычных приложений:** keep-above подходит, но таймер останется над другими приложениями, когда пользователь переключится на них.
- **Только поверх отслеживаемого приложения:** отдельная задача привязки видимости/desktop/output к активному окну. Ни keep-above, ни layer-shell не создают автоматически такую связь. Для произвольного чужого приложения стандартный transient-parent нельзя установить без его сотрудничества.
- **Fullscreen:** KeepAbove в KWin находится ниже активного fullscreen слоя. Для требования «видно даже в fullscreen» нужен overlay или отдельная compositor policy. [Описание слоёв KWin](https://github.com/KDE/kwin/blob/master/src/layers.cpp).
- **Виртуальные рабочие столы:** keep-above и on-all-desktops — разные состояния; закрепление само по себе не переносит таймер вслед за приложением. Layer-shell привязан к output, не к конкретному desktop/application.
- **PiP:** в KWin 6.7 есть экспериментальная `xx-pip-v1` интеграция, но это отдельная роль поверхности, не вызов always-on-top для текущего FLTK окна. Старую дискуссию о PiP/always-on-top нельзя принимать за уже готовый стандартный метод `xdg_toplevel`. [Исходник интеграции KWin 6.7](https://github.com/KDE/kwin/blob/v6.7.0/src/xxpipv1integration.cpp).

## Что проверено и что осталось

Проверены первичные XML, исходники FLTK/fltk-rs и KWin, включая версионные теги. Основной агент выполнил read-only проверку текущей среды: `kwin_wayland --version` сообщил 6.7.4; `wayland-info` показал `org_kde_plasma_shell v8`, `xdg_wm_base v6`, `zwlr_layer_shell_v1 v5`, но не `org_kde_plasma_window_management`. Это подтверждает ограниченную видимость global для обычного dev-клиента, а не невозможность получить интерфейс после корректной установки.

Код, правила KDE и личные настройки не менялись. Реальные окна не создавались, функциональный прототип, fullscreen и несколько мониторов не проверялись. Перед внедрением нативного пути нужен небольшой тест на отдельно запущенной тестовой панели: доступ после установки `.desktop`, точный выбор собственного окна, подтверждение keep-above, пользовательские правила и перезапуск KWin.
