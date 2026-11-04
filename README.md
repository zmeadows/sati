# sati

> **sati** (Pāli) — mindfulness, recollection; the act of
> remembering or keeping something in mind.

A quiet place for the things you need to do.

sati is a minimalist terminal checklist written in Rust. It keeps one persistent,
ordered list, without projects, tags, scheduling, or other productivity features.

## Preview

```text
┌────────────────────────────────────────────────┐
│ [ ] Finish reviewing code                      │
│ [x] Clean up Sway configuration                │
│ [ ] Read for 30 minutes                        │
│                                                │
└────────────────────────────────────────────────┘

 j/k Navigate  Space Toggle  a Add    q Quit
 J/K Move      d     Delete  D Clear
```

The selected checkbox has a muted orange accent. Completed text is dimmed and
struck through; completing a task leaves it in place.

## Features

- Add and delete tasks.
- Toggle completion.
- Navigate with the keyboard or mouse.
- Move tasks up or down with the keyboard.
- Clear the list with confirmation.
- Save every change automatically.

## Keybindings

| Key | Action |
| --- | --- |
| `j` / `Down` | Select next task, wrapping at the end |
| `k` / `Up` | Select previous task, wrapping at the beginning |
| `Shift+j` (`J`) / `Shift+Down` | Move the selected task down one row |
| `Shift+k` (`K`) / `Shift+Up` | Move the selected task up one row |
| `Space` / `Enter` | Toggle the selected task |
| `a` | Add a task |
| `d` | Delete the selected task immediately |
| `D` (Shift+D) | Confirm clearing all tasks |
| `Esc` | Cancel input or confirmation; no action in normal mode |
| `q` | Quit immediately in normal mode |

Moving a task keeps it selected and saves the new order immediately. Moves stop
at the first or last row without wrapping.

While adding, type a task, then press `Enter` to save or `Esc` to cancel.
`Backspace` removes a character. Surrounding whitespace is trimmed; blank tasks
are ignored. List shortcuts are ordinary text while typing.

In the clear dialog, **only `Enter` confirms**; `Esc` cancels. Other keys do nothing.

Left-click any line of a task to select it and toggle completion. The mouse wheel
scrolls one display row at a time, stopping at either end; selection follows when
the selected task leaves the view. Clicks on the border or outside the list are
ignored.

Long tasks wrap at word boundaries, with continuation lines indented under the
text. Long words split as needed without splitting Unicode grapheme clusters.
Use the wheel to read a task taller than the window. Keyboard navigation still
selects whole tasks. Very narrow columns display `�` for a glyph too wide to fit.

## Building

Use a stable Rust toolchain supporting edition 2024 (Rust 1.85 or newer), with
Cargo. Dependencies come from crates.io; no system-wide installation is needed.

```bash
cargo build --release
./target/release/sati
```

To keep long tasks on one truncated row instead, disable wrapping for that run:

```bash
./target/release/sati --no-wrap
```

`sati --help` shows the available option. Wrapping changes only the display;
saved task text is unchanged.

Run it directly from the build directory, or open a dedicated Foot window:

```bash
foot --app-id=sati /absolute/path/to/sati/target/release/sati
```

sati works well as a small floating terminal under Sway. It does not change your
terminal or desktop configuration.

## Data storage

Tasks are saved automatically as a human-readable JSON array at:

```text
$XDG_DATA_HOME/sati/tasks.json
```

When `XDG_DATA_HOME` is unset or empty, the path is
`~/.local/share/sati/tasks.json`. A set `XDG_DATA_HOME` must be absolute.
The data directory is created on the first change.

Every change writes and syncs a temporary file in the same directory, then
atomically replaces `tasks.json` and syncs the directory. If a save fails, sati
restores the terminal and reports the error. The original file is preserved on
write or replacement failure; the error identifies any temporary recovery file.
A failure to sync the directory after replacement is reported separately.
Invalid JSON is reported at startup and left untouched. Repair it or move it
aside before restarting.

Use one running instance at a time; the data file is not locked between instances.

## Uninstalling

Delete the project directory. To remove saved tasks too, delete
`$XDG_DATA_HOME/sati/`, or `~/.local/share/sati/` when using the fallback path.
There are no services or other application files to remove.

## License

[MIT](LICENSE).
