# Agent Hub

Локальный Windows-мост между установленными CLI-агентами и личным Telegram-чатом. Rust/Tokio Core, SQLite, Tauri 2, React/TypeScript и Fluent UI 9.

**Первая сборка MVP 0.1.0, ещё не завершённая приёмка.** Проверенные механизмы и открытые пункты перечислены в [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md). Исследование и целевая архитектура: [AGENT_HUB_PLAN.md](AGENT_HUB_PLAN.md).

## Запуск

После сборки приложение находится в `target/release/agent-hub.exe`, per-user установщик — в `target/release/bundle/nsis/Agent Hub_0.1.0_x64-setup.exe`. Эта разработческая поставка не подписана Authenticode. Установка и WebView2 работают от текущего пользователя.

1. **Агенты → Найти агенты.** CLI должны быть установлены и авторизованы вами заранее. Включите нужные. Для Claude/OMP требуется локально принять отсутствие подтверждённой OS sandbox.
2. **Настройки → Разрешённые папки.** Добавьте название и существующий каталог. Это список доступных проектов для Telegram.
3. **Telegram.** Укажите bot token из BotFather, собственный **числовой User ID** и PIN (рекомендуется 6 цифр). Username / `@username` не подходит: свой ID можно получить у [@userinfobot](https://t.me/userinfobot), нажав Start. Скопируйте число из строки Id, сохраните настройки и нажмите «Подключить».
4. **На телефоне:** `/start`, PIN, выбор/создание сессии, агент и папка; затем отправьте задачу. Другие poller/webhook этого бота должны быть остановлены.
5. Закрытие окна скрывает его в трей. «Остановить всё» отменяет задачи; «Выйти» в трее завершает Core. Автозапуск включается отдельно в настройках; соединение Telegram в этой сборке включается вручную после запуска.

Desktop содержит только **Агенты / Telegram / Настройки**. Сессии, задачи и файлы управляются через Telegram. Локальные подтверждения появляются отдельными карточками при поступлении native request.

## Telegram

| Команда | Действие |
|---|---|
| `/start`, `/sessions` | Выбрать или создать сессию |
| `/pcs`, `/status` | Состояние единственного ПК |
| `/stop` | Отменить задачи выбранной сессии |
| `/lock` | Закрыть доступ и остановить remote-задачи |
| `/files` | Предложить файлы из `.agent-hub/outbox` для явной отправки |
| `/answer ID JSON` | Ответить конкретному native question; формат зависит от агента |
| `/help` | Краткая справка |

Approvals — отдельные кнопки с одноразовым ID. Ответ «да» в обычном чате не разрешает действие. Codex questions ожидают JSON вида `{"question_id":{"answers":["ответ"]}}`, Claude — `{"текст вопроса":"ответ"}`. В OMP native dialogs подтверждаются локально. Неизвестные permission requests требуют остановки задачи.

Прикреплённые файлы накапливаются в наборе, закреплённом за сессией: нажмите «Отправить вложения». Для сохранения оригинала отправляйте изображение как document. Голос пока передаётся файлом без транскрипции. Вложения появляются в `.agent-hub/inbox` под случайными именами. Hub не изменяет `.gitignore` проекта.

Чтобы получить результат, попросите агента записать его в `.agent-hub/outbox`, вызовите `/files` и выберите файл. Hub сверяет SHA-256 и разрешённый путь перед отправкой. Файл вне outbox не экспортируется. Большие тексты в чате ограничены; полный доступный журнал — по кнопке у итогового сообщения.

## Разработка

```powershell
# Из корня репозитория
npm.cmd --prefix apps/desktop ci
.\scripts\dev.ps1
.\scripts\dev.ps1 -Build
.\scripts\verify.ps1
```

Требуются Node.js, Rust MSVC, Visual Studio Build Tools и WebView2. В этой рабочей папке установлен локальный Rust в `.tools`; scripts используют его без изменения системного PATH. `.tools`, `target`, `node_modules` и `dist` не входят в Git. `Cargo.lock` и `package-lock.json` фиксируют зависимости.

`hub-core` содержит domain, storage worker, process supervisor, adapters, access control и Telegram. `apps/desktop/src-tauri` — тонкий desktop host. `fake-cli` — только executable для integration fixtures, он не включается в desktop installer.

## Данные и восстановление

Данные находятся в `%LOCALAPPDATA%\AgentHub`; bot token и pepper — в Windows Credential Manager. PIN не сохраняется в Telegram inbox, event log или аудите. После перезапуска grants недействительны, pending approvals устаревают, незавершённые задачи не переотправляются. Неопределённая отправка Telegram остаётся `unknown`.

Job Object управляет жизненным циклом принадлежащих Hub процессов; это не файловая или сетевая песочница. Доверенными остаются локальная Windows-сессия и выбранные CLI. Telegram cloud chat не обеспечивает E2EE.
