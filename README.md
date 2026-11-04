# sati

> **sati** (Pāli) — mindfulness, recollection; the act of
> remembering or keeping something in mind.

A quiet place for the things you need to do.

sati is a minimalist, persistent terminal checklist written in Rust.

## Preview

```text
┌────────────────────────────────────────────────────────┐
│ [ ] Finish reviewing code                              │
│ [x] Clean up Sway configuration                        │
│ [ ] Read for 30 minutes                                │
│                                                        │
└────────────────────────────────────────────────────────┘

 j/k Navigate  Space Toggle  a Add    q Quit  u Undo
 J/K Move      d     Delete  D Clear  e Edit  x Clear done
```

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
```

## Data

Tasks are saved automatically to `$XDG_DATA_HOME/sati/tasks.json`, defaulting to
`~/.local/share/sati/tasks.json`.

## License

[MIT](LICENSE).
