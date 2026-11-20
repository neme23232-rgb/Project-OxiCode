<div align="center">
  <img src="oxicode-cli/assets/icon.png" alt="Project OxiCode" width="96" height="96" />
  <h1>Project OxiCode</h1>
  <p><strong>Быстрый AI-агент для разработки на Rust в двух редакциях: CLI и Desktop.</strong></p>
  <p>
    <img src="https://img.shields.io/badge/CLI-v0.1.0%20ready-7b9b83?style=flat-square" alt="CLI Ready" />
    <img src="https://img.shields.io/badge/Desktop-soon-8b5f68?style=flat-square" alt="Desktop Soon" />
    <img src="https://img.shields.io/badge/rust-2024-557a95?style=flat-square" alt="Rust 2024" />
  </p>
  <p><strong>Русский</strong> | <a href="README.en.md">English</a></p>
</div>

## Что это

**Project OxiCode** — рабочее пространство AI-инструментов для разработки на базе общего Rust-ядра [`oxicode-core`](oxicode-core). Проект объединяет лёгкий автономный терминальный клиент и разрабатываемое настольное приложение в одном репозитории.

## Возможности

- **OxiCode CLI (`oxicode-cli`)** — готовый автономный бинарный файл (`oxicode.exe`) с интерактивным TUI, стилями `OxiCode` (3D-лиса) и `Antigravity` (статичная пиксельная лиса), 15 темами (включая `monochrome`), нативным выделением текста и headless-режимом ([подробнее](oxicode-cli/README.md)).
- **OxiCode Desktop (`oxicode-ds`)** — настольная версия на базе Tauri v2 + WebView2 с карточками вызовов инструментов, боковыми панелями сессий и встроенным просмотром diff (в активной разработке, [подробнее](oxicode-ds/README.md)).
- **Единое ядро (`oxicode-core`)** — 10 провайдеров API (OpenAI-совместимые, Anthropic, Google Gemini), инструменты кодинга (`read_file`, `write_file`, `edit_file`, `list_files`, `glob`, `grep`, `bash`), откат `/undo` и `/redo`, MCP-серверы, скилы и плагины.

## Скачать

Выберите нужную редакцию проекта:

| Редакция | Описание | Статус | Загрузка |
| --- | --- | --- | --- |
| **OxiCode CLI** (`oxicode-cli`) | Терминальный TUI/CLI-агент в одном бинарнике | **Доступна** (`v0.1.0`) | [Скачать из Releases](../../releases) |
| **OxiCode Desktop** (`oxicode-ds`) | Десктопный клиент на Tauri v2 + WebView2 | **Скоро (`Soon`)** | Ожидается в будущих релизах |

## Быстрый старт

Сборка и запуск актуальной CLI-версии из исходников:

```bash
# Сборка и установка oxicode в PATH
cargo build --release -p oxicode
oxicode --install

# Запуск интерактивного TUI
oxicode

# Настройка провайдера, модели, стиля и темы
oxicode --config
```

Диагностика, однократный запрос и пайпы:

```bash
oxicode --doctor
oxicode -y "Найди и исправь предупреждения clippy"
git diff | oxicode --plan "Проверь изменения на ошибки"
```

## Требования

- **Запуск OxiCode CLI** — Windows, Linux или macOS с терминалом, поддерживающим UTF-8 и TrueColor; API-ключ провайдера либо локальный сервер Ollama.
- **Сборка из исходников** — Rust toolchain с поддержкой `edition = "2024"`.

## Удаление

Удалите установленный бинарный файл и при необходимости каталоги проекта и настроек:

- **Windows** — удалите `oxicode.exe` (`%USERPROFILE%\.cargo\bin\oxicode.exe`), каталог проекта (например, `F:\Project OxiCode`) и настройки `%APPDATA%\oxicode`.
- **Linux / macOS** — удалите бинарник `oxicode`, каталог проекта и `~/.config/oxicode`.
