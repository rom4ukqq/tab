# Third-party provenance

## Offline ASCII theme preparation

`scripts/ascii-themes.mjs` adapts the character ramp and luminance mapping from
`collidingScopes/ascii`, `ASCII.js`, commit `af6b92f7c48e2b46f7f29d18ba8409a0f93900ff`.
Origin: https://github.com/collidingScopes/ascii/tree/af6b92f7c48e2b46f7f29d18ba8409a0f93900ff
Copyright (c) 2024 Alan Ang, MIT; full notice: `notices/ascii-LICENSE.txt`.
Changes: deterministic original procedural grayscale scenes, offline SVG export,
no video input, webcam, canvas rendering, third-party media, dat.gui or muxer.
Only ready-made SVG artwork is shipped in the desktop frontend; the conversion
script is a development tool. Runtime motion is a CSS transform of the exported image.

## Agent runtime

Эта реализация написана отдельно. Исходные файлы HeyAgent, Telecodex и OMP Rust SDK не копировались; они использовались как архитектурные и protocol references, перечисленные в AGENT_HUB_PLAN.md. Generated OMP wire.rs не включён в текущий ограниченный RPC v1 adapter.

Зависимости фиксируются Cargo.lock и apps/desktop/package-lock.json. Основные библиотеки: Tauri, Tokio, rusqlite/SQLite, reqwest/rustls, serde, RustCrypto Argon2/HMAC/SHA-256, image, React, Fluent UI. Их собственные LICENSE/COPYING/NOTICE находятся в скачанных пакетах. До внешнего распространения сборки требуется закончить inventory применимых лицензий и включить полные notices всех распространяемых компонентов.

Windows-MCP, whisper.cpp, FFmpeg, OMP CLI, Claude Code CLI и Codex CLI в установщик не включены. Hub использует уже установленный native executable; установкой и авторизацией CLI владеет пользователь.
