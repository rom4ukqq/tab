# Third-party provenance

## Agent icons

Codex and Claude SVGs are from Lobe Icons, MIT, Copyright (c) 2023 LobeHub.
Origin: https://github.com/lobehub/lobe-icons/tree/8b7bbc433d38437fa2b0cca58ae896917162b812
Codex path: `packages/static-svg/icons/codex.svg`.
Codex is displayed using the OpenAI (ChatGPT) logo: https://unpkg.com/@lobehub/icons-static-svg@1.101.0/icons/openai.svg
Claude source: https://unpkg.com/@lobehub/icons-static-svg@1.101.0/icons/claude.svg
Geometry is unchanged; these monochrome SVGs are displayed using CSS masks.

OMP uses the official website favicon, unchanged, retrieved 2026-10-29.
Origin: https://omp.sh/favicon.svg
Project: https://github.com/can1357/oh-my-pi
MIT, Copyright (c) 2025 Mario Zechner; 2025-2026 Can Bölük; 2026 Stencil Labs, Inc.
Both full notices are shipped in `apps/desktop/public/icons/NOTICE.txt`.
Logos identify the respective agents; no endorsement is implied.

## ASCII theme renderer

Theme Generator and the packaged background renderer adapt rendering formulas from
collidingScopes/ascii, commit `af6b92f7c48e2b46f7f29d18ba8409a0f93900ff`, `ASCII.js`.
Copyright (c) 2024 Alan Ang, MIT. Full license: `tools/theme-generator/LICENSE-ascii.txt`,
also shipped with the generator and in `apps/desktop/public/themes/LICENSE-ascii.txt`.
Local changes: offline luminance frames, two-color palette, independent timing,
pointer trails, ZIP export, and desktop theme transitions. Theme media was supplied
by the installation owner; the renderer's MIT license does not license that media.

## Agent runtime

Эта реализация написана отдельно. Исходные файлы HeyAgent, Telecodex и OMP Rust SDK не копировались; они использовались как архитектурные и protocol references, перечисленные в AGENT_HUB_PLAN.md. Generated OMP wire.rs не включён в текущий ограниченный RPC v1 adapter.

Зависимости фиксируются Cargo.lock и apps/desktop/package-lock.json. Основные библиотеки: Tauri, Tokio, rusqlite/SQLite, reqwest/rustls, serde, RustCrypto Argon2/HMAC/SHA-256, image, React, Fluent UI. Их собственные LICENSE/COPYING/NOTICE находятся в скачанных пакетах. До внешнего распространения сборки требуется закончить inventory применимых лицензий и включить полные notices всех распространяемых компонентов.

Windows-MCP, whisper.cpp, FFmpeg, OMP CLI, Claude Code CLI и Codex CLI в установщик не включены. Hub использует уже установленный native executable; установкой и авторизацией CLI владеет пользователь.
