<div align="center">
  <img src="oxicode-cli/assets/icon.png" alt="Project OxiCode" width="96" height="96" />
  <h1>Project OxiCode</h1>
  <p><strong>Fast Rust AI coding agent available in two editions: CLI and Desktop.</strong></p>
  <p>
    <img src="https://img.shields.io/badge/CLI-v0.1.0%20ready-7b9b83?style=flat-square" alt="CLI Ready" />
    <img src="https://img.shields.io/badge/Desktop-soon-8b5f68?style=flat-square" alt="Desktop Soon" />
    <img src="https://img.shields.io/badge/rust-2024-557a95?style=flat-square" alt="Rust 2024" />
  </p>
  <p><a href="README.md">Русский</a> | <strong>English</strong></p>
</div>

## What is this

**Project OxiCode** is a Rust AI developer workspace built on a shared core ([`oxicode-core`](oxicode-core)). It brings together a standalone terminal coding agent and an upcoming desktop application in a single repository.

## Features

- **OxiCode CLI (`oxicode-cli`)** — standalone single-binary terminal agent (`oxicode.exe`) featuring interactive TUI, `OxiCode` (3D animated fox) and `Antigravity` (static pixel fox) UI styles, 15 color themes (including `monochrome`), native mouse selection, and headless mode ([details](oxicode-cli/README.en.md)).
- **OxiCode Desktop (`oxicode-ds`)** — Tauri v2 + WebView2 desktop client with tool call cards, session sidebars, and inline diff review (under active development, [details](oxicode-ds/README.en.md)).
- **Shared Core (`oxicode-core`)** — 10 API providers (OpenAI-compatible, Anthropic, Google Gemini), coding tools (`read_file`, `write_file`, `edit_file`, `list_files`, `glob`, `grep`, `bash`), `/undo` and `/redo`, stdio MCP servers, skills, and plugins.

## Download

Choose your preferred edition:

| Edition | Description | Status | Download |
| --- | --- | --- | --- |
| **OxiCode CLI** (`oxicode-cli`) | Single-binary terminal TUI and CLI coding agent | **Available** (`v0.1.0`) | [Download from Releases](../../releases) |
| **OxiCode Desktop** (`oxicode-ds`) | Desktop application built with Tauri v2 + WebView2 | **Soon** | Planned for upcoming releases |

## Quick Start

Build and install the CLI from source:

```bash
# Build and install oxicode to PATH
cargo build --release -p oxicode
oxicode --install

# Launch interactive TUI
oxicode

# Configure provider, model, UI style, and theme
oxicode --config
```

Diagnostics, one-shot prompts, and piped input:

```bash
oxicode --doctor
oxicode -y "Find and fix clippy warnings"
git diff | oxicode --plan "Review changes for bugs"
```

## Requirements

- **Running OxiCode CLI** — Windows, Linux, or macOS with a UTF-8 and TrueColor terminal; an API key for your chosen provider or a local Ollama server.
- **Building from source** — Rust toolchain with `edition = "2024"` support.

## Uninstall

Remove the installed binary and optionally the source and configuration directories:

- **Windows** — remove `oxicode.exe` (`%USERPROFILE%\.cargo\bin\oxicode.exe`), the project folder, and `%APPDATA%\oxicode`.
- **Linux / macOS** — remove the `oxicode` binary, the project folder, and `~/.config/oxicode`.
