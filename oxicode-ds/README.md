<div align="center">
  <img src="src-tauri/icons/icon.png" alt="OXICODE Desktop" width="96" height="96" />
  <h1>OXICODE Desktop</h1>
  <p><strong>Настольный AI-агент для разработки на базе Tauri v2 и общего ядра oxicode-core</strong></p>

  <p>
    <img src="https://img.shields.io/badge/version-0.1.0-8b5f68?style=flat-square" alt="Version" />
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20WebView2-7b9b83?style=flat-square" alt="Platform" />
    <img src="https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%20v2-557a95?style=flat-square" alt="Stack" />
  </p>
</div>

---

## Что это

`oxicode-ds` — настольная версия ИИ-ассистента OXICODE с интерфейсом в стиле Cline (структурированные карточки инструментов, встроенные diff-подтверждения, боковые панели) и функциональностью OpenCode на общем Rust-ядре `oxicode-core`.

## Возможности

- **Интерфейс в стиле Cline** — левая панель истории сессий, верхняя полоса задачи с индикатором контекстного окна, структурированные сворачиваемые карточки вызовов инструментов и встроенная панель подтверждения diff-изменений.
- **Провайдеры и живой выбор моделей** — поддержка 10 API-провайдеров (включая OpenAI-совместимые, Anthropic, Google Gemini и локальные Ollama/LM Studio) с загрузкой списка моделей через `/models`.
- **14 цветовых тем** — встроенная система палитр с темой по умолчанию OpenCode Beige и мгновенным переключением в реальном времени.
- **Полный набор инструментов разработки** — режимы BUILD / PLAN, auto-approve, Undo / Redo для файловых правок, сжатие истории (`/compact`) и экспорт диалога (`/export`).
- **Расширяемость** — поддержка Skills, Plugins, Rules, кастомных команд и stdio MCP-серверов из общего каталога конфигурации.

## Быстрый старт

Сборка и запуск из корня рабочего пространства `F:\Project OxiCode`:

```powershell
# Проверка и запуск тестов
cargo test --workspace

# Сборка релиза
cargo build --release -p oxicode-ds
```

После сборки исполняемый файл `oxicode-ds.exe` можно запускать напрямую; при первом запуске откроется графическое окно WebView2.

## Требования

- **ОС** — Windows 10 / 11 (x64) с установленным Microsoft Edge WebView2 Runtime.
- **Сборка** — Rust toolchain (2024 edition) и доступ к crates.io при первой компиляции зависимостей Tauri v2.

## Удаление

Достаточно удалить каталог проекта `oxicode-ds` (и при необходимости общую директорию конфигурации `~/.config/oxicode` или `%APPDATA%\oxicode`).
