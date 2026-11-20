<div align="center">
  <img src="assets/icon.png" alt="oxicode logo" width="96" height="96" />
  <h1>oxicode</h1>
  <p><strong>Быстрый и легковесный терминальный AI-агент на Rust.</strong></p>
  <p>
    <img src="https://img.shields.io/badge/language-Rust-8b5f68?style=flat-square" alt="Rust" />
    <img src="https://img.shields.io/badge/interface-TUI-7b9b83?style=flat-square" alt="TUI" />
    <img src="https://img.shields.io/badge/styles-OxiCode%20%7C%20Antigravity-557a95?style=flat-square" alt="Styles" />
  </p>
</div>

## Что это

**oxicode** — нативный терминальный ИИ-помощник для разработки на Rust. Поставляется и работает как единый автономный бинарный файл (`oxicode.exe`) без сопутствующих файлов.

## Возможности

- **10 встроенных провайдеров** — AnyModel, OpenCode Zen, OpenRouter, NVIDIA NIM, Google Gemini, OpenAI, Anthropic, DeepSeek, Ollama и Custom OpenAI-compatible.
- **Стили интерфейса и темы** — переключение раскладки через `/style` (`OxiCode` с анимированной 3D-лисой, `Antigravity` со статичной пиксельной лисой и чистым вводом `> `, `Logo`), 15 цветовых тем (`/themes`, включая `beige`, `classic` и `monochrome`).
- **Интерактивная настройка (`/connect`, `/settings`)** — ввод ключа и URL с возвратом назад по `Esc` по всем вложенным окнам, живой подбор моделей (`/models`) и сохранение сессий (`/sessions`, `--continue`).
- **Нативное выделение текста** — захват мыши отключён по умолчанию для свободного выделения и копирования мышью (`/mouse` включает прокрутку колесом и трекинг курсора).
- **Инструменты и безопасность** — `read_file`, `write_file`, `edit_file`, `list_files`, `glob`, `grep`, `bash` (UTF-8), постраничный diff перед подтверждением, `/undo`, `/redo` и отчёты об изменениях.
- **Контекст и расширения** — `/compact`, MCP по stdio (`/mcp`), плагины (`/plugins`), скилы (`/skills`), пользовательские `/` команды, `@` и `!`.

## Скачать

Скомпилируйте единственный исполняемый файл `oxicode.exe` через Cargo и установите его в `PATH` командой `oxicode --install`. Остальные каталоги (`src`, `assets`, `Cargo.toml`) требуются только для сборки.

## Быстрый старт

1. Сборка и установка:

```bash
cargo build --release -p oxicode
oxicode --install
```

2. Запуск и базовая работа:

```bash
# Интерактивный TUI
oxicode

# Список поддерживаемых провайдеров
oxicode --providers

# Живой список моделей провайдера OpenCode Zen
oxicode --provider opencode --models

# Запуск с нужным провайдером и моделью
oxicode --provider openrouter --model deepseek/deepseek-chat-v3-0324:free
```

## Требования

- **Rust** — версия 1.85+ (edition 2024) для сборки из исходников.
- **Терминал** — Windows Terminal, PowerShell, Alacritty, Kitty или iTerm2 с поддержкой UTF-8 и TrueColor.

## Удаление

Удалите `oxicode.exe` (например, из `%USERPROFILE%\.cargo\bin\oxicode.exe`), при необходимости каталог проекта (`F:\Project OxiCode`) и каталог пользовательского конфига (`%APPDATA%\oxicode` на Windows или `~/.config/oxicode` на Linux/macOS).
