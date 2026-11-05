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
CI builds and tests all three platforms. 

```bash
cargo install --git https://github.com/zmeadows/sati --locked
```

Undo/redo history normally survives closing and reopening sati. It is best-effort and may be
discarded after crashes or external task-file changes; tasks are saved independently.
A new change clears redo history.

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

## Keybinds

| Mode | Action | Keybindings |
| --- | --- | --- |
| Normal | Select next / previous task | `j` / `k`, `↓` / `↑` |
| Normal | Select first / last task | `g` / `G`, `Home` / `End` |
| Normal | Move one visible page up / down | `PageUp` / `PageDown` |
| Normal | Reorder task up / down | `K` / `J`, `Shift+↑` / `Shift+↓` |
| Normal | Add / edit task | `a` / `e` |
| Normal | Toggle completion | `Space`, `Enter` |
| Normal | Delete selected task | `d`, `Delete` |
| Normal | Clear completed tasks | `x` |
| Normal | Request clear-all confirmation | `D` |
| Normal | Undo (up to 256 changes) | `u`, `Ctrl+Z` |
| Normal | Redo (up to 256 changes) | `Ctrl+R`, `Ctrl+Y` |
| Normal | Start incremental search | `/`, `Ctrl+F` |
| Normal | Toggle line wrapping | `w` |
| Normal | Quit | `q` |
| Add/edit/search input | Move by grapheme left / right | `←` / `→` |
| Add/edit/search input | Move to start / end | `Home` / `End`, `Ctrl+A` / `Ctrl+E` |
| Add/edit/search input | Move by word left / right | `Ctrl+←` / `Ctrl+→` |
| Add/edit/search input | Delete previous / next grapheme | `Backspace` / `Delete` |
| Add/edit/search input | Delete previous word | `Ctrl+Backspace` |
| Add/edit | Save / cancel input | `Enter` / `Esc` |
| Search input | Confirm query and show results | `Enter` (stays editable if no matches; exits if empty) |
| Search input | Cancel and restore previous selection | `Esc` |
| Search results | Next / previous matching task | `n` / `N`, `↓` / `↑`; `Shift+n` also moves backward |
| Search results | Select result / cancel and restore previous selection | `Enter` / `Esc` |
| Clear-all confirmation | Confirm / cancel | `Enter` / `Esc` |

Search updates as you type. Text input accepts Unicode and bracketed paste.
Words are separated by whitespace. Modified keys and repeat reporting depend on
terminal support.

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
