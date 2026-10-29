# Agent Hub 0.1.0 — состояние реализации

Дата: 28.10.2026. Это первая реализованная сборка, **не подтверждение выполнения всех AC-01–AC-21**. Исходный план сохранён; по уточнению владельца desktop упрощён до трёх разделов.

## Реализовано

| Область | Фактическая реализация |
|---|---|
| Desktop | Агенты, Telegram, Настройки; Fluent UI, light/dark, focus, правила forced colors; tray, single instance, opt-in autostart, NSIS current-user |
| Core | SQLite WAL/FULL, отдельный DB worker, bounded channels, очередь сессии, 2 concurrent sessions, project lock, cancel, local/remote turns |
| Recovery | Incomplete execution не воспроизводится, grants сбрасываются, pending interactions obsolete, outbox sending → unknown, запрет newer schema |
| Security | User ID/private-chat binding, Argon2id 64 MiB/3/1 + HMAC pepper, Credential Manager, persistent PIN counters, idle/absolute grants, revoke |
| Processes | Native exe + argv/stdin, минимизированное окружение, suspended creation → Job Object → resume, KILL_ON_JOB_CLOSE, отдельные stdout/stderr readers |
| Adapters | Codex app-server, Claude stream-json/control, OMP rpc-ui v1; stream, native session reference, approvals/questions, native cancellation; conditional steering |
| Telegram | Единственный poller, getWebhookInfo check, durable update IDs/cursor, outbox, event projection cursor, coalesced edits 1.5 s, callbacks, session picker, PIN, lock |
| Files | Несколько вложений, explicit send draft, ограничение скачивания 20 MiB, случайные имена, MIME signature + image dimensions, SHA-256; explicit outbox export до 50 MiB |
| Audit | Минимальные action/target records, просмотр на ПК; события и экспорт JSONL, без bot token и PIN в этих структурах |

## Подтверждено проверками

`cargo test -p hub-core --locked --offline`: **13 passed** (7 unit + 6 integration). Проверены bounded framing, неизвестный approval, PIN/pepper, path escape, Unicode, mock streaming; очередь и одноразовый consume; отмена и чужой user; recovery без replay; целостность вложений/подмена asset path; три synthetic structured protocol flows; фактическое завершение Windows descendant через Job Object.

Protocol fixtures — собственный детерминированный executable, не реальные модели. Они проверяют реализацию ожидаемых контрактов, а не совместимость конкретного upstream релиза.

React/TypeScript production build прошёл. Windows desktop debug и release build прошли; NSIS bundle создан и пересобран с тремя разделами (4.50 MiB). Real CLI prompts, login, Telegram token и сообщения в реальный чат не использовались.

Визуальная проверка через Computer Use не завершена: tool вернул `Computer Use app approval timed out`. Нельзя считать подтверждёнными GUI interaction, DPI/high contrast, установку/удаление или sleep/resume.

## Незавершённые пункты MVP

| Пункт | Безопасное текущее поведение / следующая работа |
|---|---|
| Qualification реальных CLI | UI явно показывает отсутствие runtime qualification. Нужны opt-in smoke в disposable project: prompt/resume/cancel/approval/question; версии автоматически не признаются совместимыми |
| Полная Windows приёмка | Проверить интерфейс, установку, WebView2, tray/autostart, sleep/resume и отсутствие stale dispatch на целевой машине |
| Telegram E2E и fault injection | Нужен локально настроенный bot token/User ID/PIN. Mock Bot API, 429/outage/409, lost reply и реальная media group ещё не испытаны |
| Удобные вопросы | Сейчас native JSON в карточке и `/answer ID JSON`; reply-to-question picker и inline варианты предстоит довести |
| Session management | Создание/выбор/resume по сохранённому native ID работают в Core; rename/archive, список native sessions и удобный importer пока не представлены в продукте |
| OMP | RPC v1; input frame до 1 MiB, изображения до 512 KiB; dialogs local-only. v2 negotiation/chunk reassembly пока отсутствует |
| Codex requests | Command/file approvals и questions; permissions/elicitation не разрешаются автоматически, требуют остановки. Account/model/limits discovery ещё не доведён |
| Storage operations | Нет production migration checksum/backup tooling, automatic retention и общих disk quotas. Нельзя считать реализованными параметры retention из плана |
| Ingress/export hardening | Размеры и пути проверяются, Windows export handle deny-write/delete + final path + hard-link/reparse checks. Полного quarantine pipeline/антивирусного анализа нет; hostile race/security suite ещё не проведён |
| Delivery | Progress/final/cards durable; outgoing document сейчас явный вызов без durable upload retry. Draft вложений хранится в памяти и после restart нужно собрать заново |
| Lifecycle/notifications | Close → tray, explicit Exit, native interrupt + job termination. Нет системного sleep/session-lock обработчика, desktop notifications и автоматического восстановления bot connection |
| Packaging/legal | Build не подписан; полная license/notices inventory зависимостей и external distribution review ещё нужны |

## За пределами текущего MVP

Multi-PC coordinator/mTLS, Computer Use, транскрипция, Topics, подписанный updater и daemon относятся к следующим этапам и не реализованы. Никаких платных аккаунтов, серверов или сертификатов автоматически не создавалось.

## Следующая приёмка

1. Открыть сборку, найти CLI; в Telegram создать сессию выбором агента без предварительной настройки папок.
2. Локально настроить Telegram, проверить PIN и остановку задачи.
3. С явным opt-in запустить короткую реальную задачу каждого агента и approval/question.
4. Проверить два изображения + document и исходящий файл по SHA-256.
5. Провести restart/outage/sleep и installer acceptance; затем закрывать оставшиеся пункты таблицы.
