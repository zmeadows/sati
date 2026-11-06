# sati

[![CI](https://github.com/zmeadows/sati/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/zmeadows/sati/actions/workflows/ci.yml?query=branch%3Amain) [![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> **sati** (Pāli) — mindfulness, recollection; the act of remembering or keeping something in mind.

A small, persistent checklist for the terminal.

<p align="center">Linux · macOS · Windows</p>

<p align="center">
  <img src="sati_screenshot.png" alt="sati displaying a checklist in the terminal">
</p>

## Why sati?

I wanted a to-do list that felt more like a checklist in a notebook. Calendars, priorities, tags, and reminders can be useful, but I usually don't need them for everyday tasks.

`sati` is for writing things down, checking them off, and clearing the list when the day is over or priorities change.

## Features

- **One local list.** Changes are saved automatically, without accounts or synchronization. The border shows `[completed/total]`.
- **Keyboard-first, with mouse support.** Navigate, reorder, and complete tasks from the keyboard. Click a task to toggle it, or scroll with the mouse wheel.
- **Search in place.** Find tasks as you type, see matching text highlighted, and jump to a result without filtering or rearranging the list.
- **Carryover.** Mark unfinished tasks with `m` (`[↪]`) to keep them when clearing the list.
- **Undo and redo.** Up to 256 changes in either direction, with history still available after restarting the program.
- **Display options.** Wrap long tasks, group completed items, and change some style settings with command-line flags.

## Suggested usage

There's no prescribed way to use `sati`, but a few habits have worked well for me:

- **Keep it always one keystroke away.** I leave `sati` running in a dedicated terminal and use `Alt+grave` to show or hide it through my window manager. It's always available without occupying screen space.
- **Keep tasks short and actionable.** Think of it as a checklist, not a place for detailed notes or project plans.
- **Start fresh regularly.** At the end of the day, clear the list and plan for tomorrow. Carry over the handful of unfinished tasks that still matter. The carryover marker (`m`) protects them from being cleared.
- **Use ordering however you like.** Move important tasks toward the top, or place tasks in the order you know you'll want to complete them.

## Installation

Prebuilt executables are available from [GitHub Releases](https://github.com/zmeadows/sati/releases).
Download and extract the archive for your platform; Rust is not required. Archives
include LICENSE, and SHA-256 checksums are provided in `SHA256SUMS`.

Builds target Linux x86_64 and ARM64 (GNU/glibc, built on Ubuntu 22.04), macOS
Apple Silicon and Intel, and Windows x86_64. Ubuntu 22.04 provides an older glibc
baseline; compatibility with older Linux systems is not guaranteed.

With [Rust and Cargo](https://www.rust-lang.org/tools/install) (Rust 1.88 or newer):

```sh
cargo install --git https://github.com/zmeadows/sati --locked
```

Then run `sati`.

## Keyboard shortcuts

### Checklist

| Action | Keys |
| --- | --- |
| Navigate | `j` / `k`, `↓` / `↑`, `Ctrl+N` / `Ctrl+P` |
| Next / previous unfinished task (wraps) | `Tab` / `Shift+Tab` |
| First / last task | `g` / `G`, `Home` / `End` |
| Move one page | `PageDown` / `PageUp` |
| Reorder task | `J` / `K`, `Shift+↓` / `Shift+↑` |
| Group tasks by prefix | `s` |
| Copy checklist as Markdown | `c` |
| Copy selected task internally | `y`, `Alt+W` |
| Paste task below / above | `p` / `P` (`Ctrl+Y` also pastes below) |
| Append task | `a` |
| Insert below selection | `o` |
| Insert above selection | `O` |
| Edit task | `e` |
| Check / uncheck | `Space`, `Enter` |
| Delete task | `d`, `Delete` |
| Toggle carryover | `m` |
| Clear completed | `x` |
| Clear unmarked tasks (preserve carried) | `D` |
| Undo | `u`, `Ctrl+Z` |
| Redo | `Ctrl+R` |
| Search | `/`, `Ctrl+F` |
| Resume previous search forward / backward | `n` / `N` |
| Toggle wrapping | `w` |
| Quit | `q` |

With `--completed-at-bottom`, new tasks stay in the incomplete section.

Task copies use one in-memory register, separate from the system clipboard. Pasted
tasks start incomplete and unmarked; the register remains available for repeated
pastes until exit. `c` still copies the entire checklist as Markdown to the system
clipboard.

Press `s` to group tasks manually by their leading prefix (such as `Work:`). Unprefixed tasks come first, followed by case-insensitive alphabetical prefix groups; order within each group is preserved. With `--completed-at-bottom`, unfinished and completed tasks are grouped separately.

### While typing

The add, edit, and search prompts share these shortcuts:

| Action | Keys |
| --- | --- |
| Move cursor | `←` / `→`, `Ctrl+B` / `Ctrl+F` |
| Start / end of text | `Home` / `End`, `Ctrl+A` / `Ctrl+E` |
| Previous / next word | `Ctrl+←` / `Ctrl+→`, `Alt+B` / `Alt+F` |
| Delete next word | `Alt+D` |
| Delete previous word | `Ctrl+Backspace`, `Ctrl+W` |
| Delete character at cursor | `Delete`, `Ctrl+D` |
| Delete to end / start of input | `Ctrl+K` / `Ctrl+U` |
| Cancel current operation | `Esc`, `Ctrl+G` |

`Ctrl+G` also cancels search results and confirmation dialogs; it does nothing in Normal mode.

**Search:** Press `/` and type to find matches. `Enter` selects a sole matching task immediately; with multiple matches, it opens results. Use `n` / `N`, `↓` / `↑`, or `Ctrl+N` / `Ctrl+P` to move between results, then `Enter` to select one. `Esc` cancels and restores your previous selection. Exiting search clears highlighting but remembers the last nonempty query for this session. From the checklist, `n` / `N` resume it, staying on the current task if it matches; subsequent presses cycle through matches.

## Options

| Flag | Effect |
| --- | --- |
| `--theme NAME` | Select a built-in theme (listed below). |
| `--config PATH` | Use an explicit configuration file. |
| `--wrap` / `--no-wrap` | Enable / disable task wrapping. |
| `--logo` / `--no-logo` | Show / hide the border title. |
| `--keybinds` / `--no-keybinds` | Show / hide shortcut hints. |
| `--round-corners` / `--square-corners` | Use rounded / sharp borders. |
| `--ascii-markers` / `--unicode-markers` | Use ASCII / Unicode task markers. |
| `--completed-at-bottom` / `--no-completed-at-bottom` | Enable / disable completion grouping. |
| `--accent COLOR` | Set an ANSI color name, hex RGB, or palette index 0–255 (default `lightyellow`). |

Task markers default to `›`, `[ ]`, `[✓]`, and `[↪]`. `--ascii-markers` changes only task markers; other UI symbols remain Unicode.

By default, Sati uses your terminal’s color scheme: normal text inherits its foreground, muted
items use dimming, and the default accent uses its ANSI bright-yellow palette entry.

```sh
sati
sati --accent lightblue
sati --accent '#a9b665'
sati --accent 3
```

Run `sati --help` for the full command-line help.

## Configuration

Configuration is optional. Precedence is **CLI > TOML > built-in defaults**.

| Platform | Default file |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/sati/config.toml`, falling back to `~/.config/sati/config.toml` |
| macOS | `~/Library/Application Support/sati/config.toml` |
| Windows | `%APPDATA%\sati\config.toml` |

Every section and key is optional. This example combines display defaults with optional style customizations:

```toml
theme = "default"

[display]
wrap = true
logo = true
keybinds = true
rounded_corners = false
ascii_markers = false
completed_at_bottom = false

[styles.normal]
fg = "reset"

[styles.muted]
modifiers = ["dim"]

[styles.completed]
fg = "darkgray"

[styles.border]
fg = "darkgray"
modifiers = []

[styles.accent]
fg = "#e78a4e"
modifiers = ["bold"]
```

Use `sati --config ./sati.toml` for another file. sati never creates or edits
configuration files. A missing default file is harmless; an explicit file must
exist. Invalid files and unknown keys are errors. Environment directory paths
must be absolute; `--config` also accepts relative paths without expanding `~`.

CLI pairs override either TOML value: `--no-wrap` disables wrapping, and `--wrap`
re-enables it. Contradictory pairs are rejected.

The `default` theme uses terminal-native colors and remains the default. Other
built-in themes provide five RGB foregrounds with no base text modifiers; all
leave the terminal background unchanged.

| Theme | Identifier | Variant |
| --- | --- | --- |
| Terminal default | `default` | Terminal-native |
| Catppuccin | `catppuccin-mocha` | Mocha |
| Tokyo Night | `tokyonight-storm` | Storm |
| Kanagawa | `kanagawa-wave` | Wave |
| Rosé Pine | `rose-pine` | Main |
| Nightfox | `nightfox` | Nightfox |
| One Dark | `onedark` | Dark |
| Gruvbox Material | `gruvbox-material` | Dark / Medium |
| GitHub Dark | `github-dark` | GitHub Dark Default / classic dark palette |
| Everforest | `everforest-medium` | Dark / Medium |
| Cyberdream | `cyberdream-muted` | Dark / Muted |
| Nord | `nord` | Dark |
| Dracula | `dracula` | Dark |
| Darcula | `darcula` | JetBrains Darcula (Vim port) |
| Gruvbox Classic | `gruvbox` | Dark / Medium |
| Solarized | `solarized-dark` | Dark |

Select a theme with `theme = "catppuccin-mocha"` at the TOML root or
`sati --theme catppuccin-mocha`; CLI selection takes precedence.
Explicit `[styles.*]` settings override the chosen theme, and
`--accent` takes final precedence for the accent foreground only, preserving
configured modifiers.

The five style roles support `fg` and `modifiers`. Foregrounds accept Ratatui ANSI
names, hex RGB, and palette indices (as strings). `reset` inherits the terminal
foreground and is allowed except for accent. In the `default` theme, muted
inherits normal and adds DIM; completed and border inherit resolved muted.
Accent inherits normal, then defaults its foreground to `lightyellow`. The RGB
themes instead supply explicit foregrounds and empty base modifier sets for each
role.

Omitted properties retain those inherited values. A modifier array **replaces**
inherited modifiers; `modifiers = []` clears them. Supported names are `bold`,
`dim`, `italic`, `underlined`, and `crossed_out`, without duplicates. Completion
strikethrough, focus/prefix boldness, and search highlighting remain contextual.
Backgrounds are not configurable.
Help, version output, and Markdown export do not load display configuration.

## Export

Export Markdown to stdout or a file, even while the TUI is running:

```sh
sati export
sati export > tasks.md
sati export -o tasks.md
sati export -o tasks.md --force
```

Use `--output PATH` or `-o PATH` for file output. Existing files require `--force`;
sati's own state files are protected. Carried tasks export as ordinary unchecked
items, with task order and literal text preserved.

Press `c` in the checklist to send the same Markdown to the terminal clipboard
using OSC 52. The brief “Copy sent to terminal” message clears on subsequent
activity; clipboard acceptance depends on terminal and multiplexer support.

## Data

Tasks are saved locally in `tasks.json`:

| Platform | Location |
| --- | --- |
| Linux | `~/.local/share/sati/tasks.json` (or `$XDG_DATA_HOME/sati/tasks.json`) |
| macOS | `~/Library/Application Support/sati/tasks.json` |
| Windows | `%APPDATA%\sati\tasks.json` |

Undo/redo history is saved separately in `history.json` when you quit normally. If that history is missing or no longer matches your tasks, `sati` starts with an empty history; your saved tasks are unaffected.

## Credits

Inspired by [Meowdo](https://github.com/Sycorlax/Meowdo). Built with [Ratatui](https://ratatui.rs).

## License

[MIT](LICENSE).
