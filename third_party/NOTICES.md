# Third-party provenance

Эта реализация написана отдельно. Исходные файлы HeyAgent, Telecodex и OMP Rust SDK не копировались; они использовались как архитектурные и protocol references, перечисленные в AGENT_HUB_PLAN.md. Generated OMP wire.rs не включён в текущий ограниченный RPC v1 adapter.

Зависимости фиксируются Cargo.lock и apps/desktop/package-lock.json. Основные библиотеки: Tauri, Tokio, rusqlite/SQLite, reqwest/rustls, serde, RustCrypto Argon2/HMAC/SHA-256, image, React, Fluent UI. Их собственные LICENSE/COPYING/NOTICE находятся в скачанных пакетах. До внешнего распространения сборки требуется закончить inventory применимых лицензий и включить полные notices всех распространяемых компонентов.

Windows-MCP, whisper.cpp, FFmpeg, OMP CLI, Claude Code CLI и Codex CLI в установщик не включены. Hub использует уже установленный native executable; установкой и авторизацией CLI владеет пользователь.
