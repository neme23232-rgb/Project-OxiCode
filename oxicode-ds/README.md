<div align="center">
  <img src="src-tauri/icons/icon.png" alt="OxiCode Desktop" width="96" height="96" />
  <h1>OxiCode Desktop</h1>
  <p><strong>Настольный AI-агент для разработки на базе Tauri v2 и общего ядра oxicode-core (Soon).</strong></p>
  <p>
    <img src="https://img.shields.io/badge/status-coming%20soon-8b5f68?style=flat-square" alt="Coming Soon" />
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20WebView2-7b9b83?style=flat-square" alt="Platform" />
    <img src="https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%20v2-557a95?style=flat-square" alt="Stack" />
  </p>
  <p><strong>Русский</strong> | <a href="README.en.md">English</a></p>
</div>

## Что это

`oxicode-ds` — настольная редакция **Project OxiCode** с графическим интерфейсом (структурированные карточки инструментов, встроенные diff-подтверждения, боковые панели) на общем Rust-ядре `oxicode-core`. На данный момент находится в активной разработке; для повседневной работы используйте [OxiCode CLI](../oxicode-cli/README.md).

## Возможности

- **Графический интерфейс** — панель истории сессий, индикатор контекстного окна, сворачиваемые карточки вызовов инструментов и встроенная панель подтверждения diff-изменений.
- **Единая совместимость с CLI** — использует общую конфигурацию, 10 API-провайдеров, темы, инструменты кодинга и расширения (`Skills`, `Plugins`, `Rules`, `MCP`) из `oxicode-core`.

## Скачать

Готовые сборки появятся в разделе [Releases](../../../releases) после завершения разработки (**Soon**).

## Быстрый старт

Сборка ранней версии из корня репозитория:

```powershell
cargo test --workspace
cargo build --release -p oxicode-ds
```

## Требования

- **ОС** — Windows 10 / 11 (x64) с установленным Microsoft Edge WebView2 Runtime.
- **Сборка** — Rust toolchain (`edition = "2024"`).

## Удаление

Удалите каталог `oxicode-ds` и при необходимости общий каталог конфигурации `%APPDATA%\oxicode` (`~/.config/oxicode`).
