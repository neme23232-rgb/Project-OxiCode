<div align="center">
  <img src="oxicode-cli/assets/icon.png" alt="oxicode" width="96" height="96" />
  <h1>oxicode</h1>
  <p><strong>Быстрый и лёгкий терминальный AI-агент для разработки на Rust.</strong></p>
  <p>
    <img src="https://img.shields.io/badge/rust-2024-8b5f68?style=flat-square" alt="Rust 2024" />
    <img src="https://img.shields.io/badge/cli-ratatui-7b9b83?style=flat-square" alt="Ratatui TUI" />
    <img src="https://img.shields.io/badge/providers-10-557a95?style=flat-square" alt="10 Providers" />
  </p>
</div>

## Что это

`oxicode` — автономный консольный и TUI-агент для работы с кодом в одном бинарном файле (`oxicode.exe`) без внешних зависимостей и дополнительных файлов рядом.

## Возможности

- **Два режима работы** — интерактивный TUI (`oxicode`) и headless-режим для скриптов, конвейеров и CI (`oxicode "..."`, `--json`, `-t/--timeout`).
- **Стили интерфейса (`/style`)** — классический `OxiCode` с анимированной 3D-лисой, компактный `Antigravity` со статичной пиксельной лисой и строкой `> `, а также минималистичный текстовый логотип.
- **15 цветовых тем (`/themes`)** — `beige`, `classic`, `monochrome` (чёрно-белый интерфейс, где цветом выделяются только изменения и диффы), `tokyo-night`, `github-dark`, `nord` и другие.
- **Универсальная поддержка API** — OpenAI-совместимые эндпоинты (OpenRouter, DeepSeek, NVIDIA NIM, AnyModel, OpenCode Zen, Ollama, Custom), Anthropic Messages и Google Gemini с автоматическим повтором при временных сбоях.
- **Инструменты кодинга** — `read_file`, `write_file`, `edit_file` (с поддержкой Windows CRLF), `list_files`, `glob`, `grep`, `bash` (UTF-8 вывод в PowerShell), откат и повтор правок (`/undo`, `/redo`).
- **Режимы BUILD и PLAN** — переключение (`Tab`) между анализом и редактированием, авто-подтверждение (`Shift+Tab` / `-y`), вложенная навигация в меню с возвратом назад по `Esc`.
- **Выделение текста и буфер обмена** — нативное выделение и копирование мышью в терминале включено по умолчанию (`/mouse` или настройка `Mouse Capture` для переключения на колесо прокрутки).
- **Расширяемость и сессии** — загрузка `AGENTS.md` / `CLAUDE.md` / `GEMINI.md`, локальные навыки, плагины, кастомные команды, MCP-серверы по stdio, `@`-файлы, `!`-шелл, `/compact`, `/sessions` (`-c`, `--id`) и экспорт в Markdown.

## Скачать

Актуальные сборки бинарного файла доступны на странице [Releases](../../releases) или собираются из исходников через Cargo. Единственный файл, нужный для запуска — `oxicode.exe`.

## Быстрый старт

Сборка и установка в `PATH`:

```bash
cargo build --release -p oxicode
oxicode --install
```

Запуск и базовая работа:

```bash
# Интерактивный TUI
oxicode

# Настройка провайдера, модели, стиля и темы
oxicode --config

# Диагностика конфигурации и подключений
oxicode --doctor

# Однократный запрос с авто-подтверждением правок
oxicode -y "Найди и исправь предупреждения clippy"

# Работа через конвейер (piped stdin) и JSON-поток
git diff | oxicode --plan "Проверь изменения на ошибки"
oxicode --json "Перечисли модули проекта"
```

## Требования

- **Сборка из исходников** — Rust toolchain с поддержкой `edition = "2024"`.
- **Запуск бинарника** — терминал с поддержкой UTF-8 и 24-bit TrueColor (Windows Terminal, PowerShell, Alacritty, Kitty, iTerm2); API-ключ выбранного провайдера либо локальный сервер Ollama.

## Удаление

Удалите установленный бинарный файл и при необходимости каталог исходников и конфигурации:

- **Windows** — удалите `oxicode.exe` (в `%USERPROFILE%\.cargo\bin\oxicode.exe`), каталог проекта (например, `F:\Project OxiCode`) и каталог настроек `%APPDATA%\oxicode`.
- **Linux / macOS** — удалите бинарник `oxicode`, каталог проекта и `~/.config/oxicode`.
