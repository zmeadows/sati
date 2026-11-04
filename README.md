# sati

> **sati** (Pāli) — mindfulness, recollection; the act of
> remembering or keeping something in mind.

sati is a minimalist, persistent terminal checklist. A quiet place for the things you need to do.

## Preview

<p align="center">
  <img src="sati_screenshot.png" alt="sati showing a checklist and keyboard shortcuts">
</p>

## Installation

Requires Linux and Rust 1.85 or newer. From a repository checkout:

```bash
cargo install --path . --locked
```

## Usage

```bash
sati
```

| Key | Action |
| --- | --- |
| `j` / `Down`, `k` / `Up` | Select next / previous task |
| `J` / `Shift+Down`, `K` / `Shift+Up` | Move task down / up |
| `Space` / `Enter` | Toggle completion |
| `a` / `e` | Add / edit task |
| `d` | Delete task |
| `x` | Clear completed tasks |
| `D` | Clear all tasks |
| `u` | Undo last change |
| `q` | Quit |

When adding, editing, or clearing all tasks, `Enter` confirms and `Esc` cancels.

## Options

```bash
sati --no-wrap             # Keep long tasks on one row
sati --accent '#a9b665'    # Set the selection accent
sati --version            # Show the version
```

## Data

Tasks are saved automatically to:

- Linux: `$XDG_DATA_HOME/sati/tasks.json`, defaulting to
  `~/.local/share/sati/tasks.json`.
- macOS: `~/Library/Application Support/sati/tasks.json`.
- Windows: `%APPDATA%\sati\tasks.json`.

## License

[MIT](LICENSE).
