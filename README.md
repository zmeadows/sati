# sati

> **sati** (Pāli) — mindfulness, recollection; the act of
> remembering or keeping something in mind.

sati is a minimalist, persistent terminal checklist. A quiet place for the things you need to do.

## Preview

<p align="center">
  <img src="sati_screenshot.png" alt="sati showing a checklist and keyboard shortcuts">
</p>

## Installation

Supports Linux, macOS, and Windows. Requires Rust 1.85 or newer.
CI builds and tests all three platforms. Local terminal checks used a Linux
pseudo-terminal.

```bash
cargo install --git https://github.com/zmeadows/sati --locked
```

## Usage

```bash
sati
```

| Key | Action |
| --- | --- |
| `j` / `Down`, `k` / `Up` | Select next / previous task |
| `g` / `Home`, `G` / `End` | Select first / last task |
| `J` / `Shift+Down`, `K` / `Shift+Up` | Move task down / up |
| `Space` / `Enter` | Toggle completion |
| `a` / `e` | Add / edit task |
| `d` | Delete task |
| `x` | Clear completed tasks |
| `D` | Clear all tasks |
| `u` | Undo last change |
| `q` | Quit |

When adding, editing, or clearing all tasks, `Enter` confirms and `Esc` cancels.
New tasks are inserted below the focused task and become selected.

Enhanced keyboard reporting, mouse interaction, and bracketed paste depend on
terminal and platform support; unbracketed paste is treated as ordinary keystrokes.

## Options

```bash
sati --no-wrap             # Keep long tasks on one row
sati --no-logo             # Hide the sati border title
sati --round-corners       # Use rounded checklist corners (default: sharp)
sati --add-to-bottom       # Add new tasks at the bottom instead of below selection
sati --accent '#a9b665'     # Set the selection accent
sati --version             # Show the version
```

## Data

Tasks are saved automatically to:

| Platform | Location |
| --- | --- |
| Linux | `~/.local/share/sati/tasks.json` (honors `XDG_DATA_HOME`) |
| macOS | `~/Library/Application Support/sati/tasks.json` |
| Windows | `%APPDATA%\sati\tasks.json` |

## License

[MIT](LICENSE).
