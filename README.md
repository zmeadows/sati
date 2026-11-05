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

I wanted a simple to-do list that worked more like a checklist in a notebook.

Most task managers include features like calendars, priorities, tags, and reminders. These can be useful, but I generally don't need them for keeping track of everyday tasks.

`sati` is intended for writing down a few things, checking them off as they're completed, and clearing the list when the day is over or priorities change. That's about it.

I was surprised how much I enjoyed using it, so I decided to share it.

## Installation

Supports Linux, macOS, and Windows. Requires Rust 1.85 or newer.
CI builds and tests all three platforms. Local terminal checks used a Linux
pseudo-terminal.

```bash
cargo install --git https://github.com/zmeadows/sati --locked
```

Press `/` to search as you type. `Enter` confirms the query; `n` / `N` or
arrow keys navigate results, and `Enter` selects a task. `Esc` cancels search
and restores the previous selection.

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

## Credits

Inspired by [Meowdo](https://github.com/Sycorlax/Meowdo).

Built with [Ratatui](https://ratatui.rs).

## License

[MIT](LICENSE).
