<div align="center">
  <img src="src-tauri/icons/icon.png" alt="OxiCode Desktop" width="96" height="96" />
  <h1>OxiCode Desktop</h1>
  <p><strong>Desktop AI coding agent powered by Tauri v2 and the shared oxicode-core engine (Soon).</strong></p>
  <p>
    <img src="https://img.shields.io/badge/status-coming%20soon-8b5f68?style=flat-square" alt="Coming Soon" />
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20WebView2-7b9b83?style=flat-square" alt="Platform" />
    <img src="https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%20v2-557a95?style=flat-square" alt="Stack" />
  </p>
  <p><a href="README.md">Русский</a> | <strong>English</strong></p>
</div>

## What is this

`oxicode-ds` is the desktop edition of **Project OxiCode** featuring structured tool call cards, inline diff approvals, and session sidebars built on the shared `oxicode-core` Rust crate. It is currently under active development; for daily usage, please use [OxiCode CLI](../oxicode-cli/README.en.md).

## Features

- **Desktop Workspace** — session history sidebar, context usage indicator, collapsible tool call cards, and built-in diff preview panel.
- **Shared Core Compatibility** — shares configuration, 10 API providers, themes, coding tools, and extensions (`Skills`, `Plugins`, `Rules`, `MCP`) with OxiCode CLI.

## Download

Prebuilt desktop installers and binaries will appear under [Releases](../../../releases) once ready (**Soon**).

## Quick Start

Build the early preview from the workspace root:

```powershell
cargo test --workspace
cargo build --release -p oxicode-ds
```

## Requirements

- **OS** — Windows 10 / 11 (x64) with Microsoft Edge WebView2 Runtime installed.
- **Build** — Rust toolchain (`edition = "2024"`).

## Uninstall

Delete the `oxicode-ds` directory and optionally remove `%APPDATA%\oxicode` (`~/.config/oxicode`).
