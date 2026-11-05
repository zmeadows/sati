# sati

[![CI](https://github.com/zmeadows/sati/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/zmeadows/sati/actions/workflows/ci.yml?query=branch%3Amain) [![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> **sati** (Pāli) — mindfulness, recollection; the act of
> remembering or keeping something in mind.

sati is a minimalist, persistent terminal checklist. A quiet place for the things you need to do.

<p align="center">
🐧 Linux · 🍎 macOS · 🪟 Windows
</p>

## Preview

<p align="center">
  <img src="sati_screenshot.png" alt="sati showing a checklist and keyboard shortcuts">
</p>

## Why sati?

I wanted a to-do list that worked like a piece of paper.

Most task managers offer calendars, priorities, tags, reminders, and elaborate ways to organize your work. Those features can be useful, but they can also turn a simple checklist into something that needs managing itself.

`sati` takes the opposite approach. Write down what you need to do, check things off as you finish them, and keep going. When the day is over or your priorities change, clear the list and start fresh.

Nothing more than you need.

## Installation

Supports Linux, macOS, and Windows. Requires Rust 1.85 or newer.
CI builds and tests all three platforms. Local terminal checks used a Linux
pseudo-terminal.

```bash
cargo install --git https://github.com/zmeadows/sati --locked
```

## Options

| Flag | Description |
| --- | --- |
| `--no-wrap` | Keep tasks on one row. |
| `--no-logo` | Hide the border title. |
| `--no-keybinds` | Hide keyboard hints. |
| `--round-corners` | Use rounded borders. |
| `--add-to-bottom` | Append new tasks instead of inserting below selection. |
| `--completed-at-bottom` | Group completed tasks at the bottom, newly completed first. |
| `--accent '#RRGGBB'` | Set the selection accent color. |
| `--version` | Show the version. |
| `--help`, `-h` | Show help. |

## Data

Tasks are saved automatically to:

| Platform | Location |
| --- | --- |
| Linux | `~/.local/share/sati/tasks.json` (honors `XDG_DATA_HOME`) |
| macOS | `~/Library/Application Support/sati/tasks.json` |
| Windows | `%APPDATA%\sati\tasks.json` |

## License

[MIT](LICENSE).
