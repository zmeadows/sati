# Using sati

This guide covers behavior that isn't obvious from the keyboard reference (`?` in sati) or `sati --help`.

## Sections

Start a task with one word, a colon, and a space to put it in a section: `Work: Fix bug` belongs to `Work`. `Work:Fix`, URLs, and times such as `10:30` don't create sections. Section names are case-insensitive; a tab is labelled with the first spelling in the list.

Once any task has a section, tabs appear on the top border:

- `[*]` shows every task, with section prefixes as headings.
- `[-]` shows tasks without a section, when there are any.
- Each section follows with its own tab. A new section starts at the right end, and a section disappears with its last task.

A section tab is a view of the same list, not a separate list. It hides the prefix, and movement, reordering, search, clearing, and `c` apply only to its tasks. Adding a task from a section tab suggests that section, and while the input is empty or a single partial word, `Tab` cycles through matching sections.

`s` then a digit jumps to a tab by position (`s1` is `[*]`, `s0` the tenth), and `` ` `` finds a tab by fuzzy name. `'` returns to the tab and task you were on before your last jump; press it again to go back. `E` renames the current section on every task in it, and `L` / `H` move its tab without reordering any tasks.

## Carryover and clearing

sati is meant to be cleared regularly. Both clearing commands act on the current tab, or the whole list on `[*]`:

- `X` removes completed tasks.
- `D` asks for confirmation, then removes every task not marked for carryover.

`m` marks a task for carryover, shown as `[↪]`. The mark stays after `D`, so the task survives later clears too, until you complete it or press `m` again. Completed tasks can't be carried.

Undo history covers clears, but it lasts only while sati is open.

## Data and multiple checklists

Tasks are saved to `tasks.json` after every change:

| Platform | Directory |
| --- | --- |
| Linux | `$XDG_DATA_HOME/sati/`, or `~/.local/share/sati/` |
| macOS | `~/Library/Application Support/sati/` |
| Windows | `%APPDATA%\sati\` |

Each save replaces the file atomically, so an interrupted save leaves the previous version intact. sati won't open a `tasks.json` that is malformed or was written by a newer version; it reports the problem and leaves the file untouched.

`--data-dir PATH` keeps `tasks.json` and its lock file directly in `PATH` instead. Each directory is an independent checklist, and only one sati can have it open at a time. `--data-dir` doesn't change where configuration is read from. sati doesn't expand `~` itself, so write `--data-dir ~/work` with a space: some shells don't expand `~` after `=`.

```sh
alias work='sati --data-dir ~/checklists/work'
sati export --data-dir ~/checklists/work > work.md
```

### Backup and restore

`tasks.json` is the whole checklist, so to back one up, copy that file, whether it is in the default directory or a `--data-dir` directory. To restore, quit sati first, then put the copy back as `tasks.json` in the same directory. While sati is open, it would save over the restored file on your next change or when it quits.

`sati export` is for reading, not backup: it drops carryover marks and tab order, and sati can't import it. A backup written by a newer sati may not open in an older one.

## Export and clipboard

`sati export` writes Markdown to stdout and errors to stderr. It only reads `tasks.json`, so it is safe to run while sati is open. Completed tasks become `- [x]` items; pending and carried tasks become `- [ ]`. When exporting a section, its tasks are written without their prefix.

`c` sends the current tab's Markdown to the terminal with OSC 52. The terminal must allow clipboard writes, and a multiplexer such as tmux must pass them through. sati can't detect whether that worked: "Copy sent to terminal" means only that it was sent. If it doesn't work, pipe an export into a clipboard tool:

| Platform | Command |
| --- | --- |
| Linux (Wayland) | `sati export \| wl-copy` |
| Linux (X11) | `sati export \| xclip -selection clipboard` |
| macOS (UTF-8 locale) | `sati export \| pbcopy` |
| Windows PowerShell | `sati export \| Set-Clipboard` |

In PowerShell, first run `[Console]::OutputEncoding = [System.Text.Encoding]::UTF8` so non-ASCII text survives the pipe.

`y` and `p` copy and paste whole tasks using sati's own register, not the system clipboard. A pasted task is a new, unchecked task.

## Terminal notes

- **macOS:** `Alt` shortcuts need Option to send Meta. Enable "Use Option as Meta key" in Terminal.app, or set the Option key to "Esc+" in iTerm2.
- **`Ctrl+S`** searches unless your terminal or multiplexer intercepts it; `/` and `Ctrl+F` also search.
- **Mouse:** sati captures the mouse by default. To select and copy text with it instead, use `--no-mouse` or `mouse = false`.
- **Pasting on Windows:** pasted text arrives as ordinary keystrokes. Open a prompt (`a`, `e`, or `/`) before pasting, and paste one line at a time: in the checklist, pasted letters run as shortcuts, and a line break acts as `Enter`. Keep Windows Terminal's multi-line paste warning on.
