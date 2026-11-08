# sati

[![CI](https://github.com/zmeadows/sati/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/zmeadows/sati/actions/workflows/ci.yml?query=branch%3Amain) [![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> **sati** (Pāli) — mindfulness, recollection; the act of remembering or keeping something in mind.

A small, persistent checklist for the terminal on Linux, macOS, and Windows.

<p align="center">
  <img src="sati_screenshot.png" alt="sati displaying a checklist in the terminal">
</p>

## Why sati?

I wanted a to-do list that felt more like a checklist in a notebook. Calendars, priorities, tags, and reminders can be useful, but I usually don't need them for everyday tasks.

`sati` is for writing things down, checking them off, and clearing the list when the day is over or priorities change.

## Features

- **One local list.** Changes are saved automatically, without accounts or synchronization. A muted `? for help` in the bottom-right corner is the only on-screen hint.
- **Shortcuts on demand.** Press `?` for a keyboard reference instead of a permanent shortcut footer.
- **Keyboard-first, with mouse support.** Navigate, reorder, and complete tasks from the keyboard. Click a task to toggle it, or scroll with the mouse wheel.
- **Search in place.** Find tasks as you type, see matching text highlighted, and jump to a result without filtering or rearranging the list.
- **Sections.** Start a task with `Name: ` to group it; a tab bar then lets you focus on one section at a time.
- **Carryover.** Mark unfinished tasks with `m` (`[↪]`) to keep them when clearing the list.
- **Undo and redo.** Up to 256 changes in either direction during the current session.
- **Display options.** Wrap long tasks and change some style settings with command-line flags. When tasks don't fit, a scrollbar on the right border shows where you are.

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

With [Rust and Cargo](https://www.rust-lang.org/tools/install) (Rust 1.89 or newer):

```sh
cargo install --git https://github.com/zmeadows/sati --locked
```

Then run `sati`.

## Keyboard shortcuts

### Checklist

| Action | Keys |
| --- | --- |
| Navigate | `j` / `k`, `↓` / `↑`, `Ctrl+N` / `Ctrl+P` |
| Next / previous section tab (wraps) | `l` / `h`, `Tab` / `Shift+Tab`, `Alt+F` / `Alt+B` |
| Go to section tab 1–10 | `s1` … `s9`, `s0`; `Alt+S` then a digit |
| Find section tab by name | `` ` ``, `Alt+G` |
| Move current section right / left | `L` / `H`, `Alt+N` / `Alt+P` |
| First / last task | `g` / `G`, `Home` / `End` |
| Move one page | `PageDown` / `PageUp`, `Ctrl+V` / `Alt+V` |
| Move half a screen | `Ctrl+D` / `Ctrl+U` |
| Reorder task | `J` / `K`, `Alt+↓` / `Alt+↑` |
| Copy checklist as Markdown | `c` |
| Copy selected task internally | `y`, `Alt+W` |
| Paste task below / above | `p` / `P` (`Ctrl+Y` also pastes below) |
| Append task | `a` |
| Insert below selection | `o`, `Ctrl+O` |
| Insert above selection | `O` |
| Edit task | `e`, `Alt+E` |
| Rename current section | `E` |
| Check / uncheck | `Space`, `Enter` |
| Delete task | `d`, `Delete` |
| Toggle carryover | `m` |
| Clear completed | `X` |
| Clear unmarked tasks (preserve carried) | `D` |
| Undo | `u`, `Ctrl+Z` |
| Redo | `Ctrl+R` |
| Search | `/`, `Ctrl+F`, `Ctrl+S` |
| Toggle wrapping | `w` |
| Next / previous color theme | `t` / `T` |
| Keyboard reference | `?` |
| Quit | `q` |

`?` opens a keyboard reference over the checklist, in place of a permanent shortcut
footer; the muted `? for help` in the bottom-right corner is the only reminder of it.
Scroll it with `j` / `k`, `↓` / `↑`, `Ctrl+N` / `Ctrl+P`, `PageDown` / `PageUp`,
`Home` / `End`, or the mouse wheel, and close it with `?`, `Esc`, or `Ctrl+G`. It changes
nothing: the tasks, selection, and section tab are exactly as you left them. On a narrow
terminal it lists each action's keys under it, and on a very small one it asks for more room.

As in Vim, type a count before navigating or reordering to repeat it: `5j` moves
down five tasks, `3K` moves the selected task up three, `3l` switches three section tabs
right, and `2H` moves the current section two places left. Counts also work with the
arrow and `Ctrl+N` / `Ctrl+P` aliases of `j` / `k` and the `Alt+↓` / `Alt+↑` aliases of
`J` / `K`, and counted moves stop at the first or last task or tab instead of wrapping.
A counted reorder is a single change for `u`.
`Esc` cancels a count, and sati forgets one after two seconds without another key.

Holding a key repeats navigation (`j` / `k`, `h` / `l`, arrows, `Tab`, `Alt+B` / `Alt+F`,
Page Up/Down, `Ctrl+V` / `Alt+V`, `Ctrl+D` / `Ctrl+U`), but commands that change tasks or
section order, such as `d`, `X`, `J` / `K`, `Alt+↓` / `Alt+↑`, or `H` / `L`, run once per
key press.

`Ctrl+D` / `Ctrl+U` move the selection and the view half a screen, counting wrapped lines,
and stop at the first or last task. While typing they keep their editing meanings (see
below), as does `Ctrl+K`, which does nothing in the checklist itself.
sati turns off XON/XOFF flow control while it runs, so `Ctrl+S` reaches it; if your
terminal or multiplexer binds `Ctrl+S` itself, use `/` or `Ctrl+F`.

Task copies use one in-memory register, separate from the system clipboard. `y` (or
`Alt+W`) puts the selected task's text in the register, and `p`, `P`, or `Ctrl+Y` paste
it as a new task. Pasted tasks start incomplete and unmarked; the register remains
available for repeated pastes until exit. `c` still copies the entire checklist as
Markdown to the system clipboard.

### Sections

Start a task with one word, a colon, and a space, such as `Work: Fix bug`, to put it in
the `Work` section. Sections are optional: with none, sati looks exactly as before. Once
any task has a section, tabs appear on the top border, with the current one shown in
the accent color and a blank row separating them from the tasks:

- `[*]` shows every task in your order, with each section prefix in bold.
- `[-]` comes next and shows tasks without a section, when there are any.
- Each section follows with its own tab, in an order you choose. A new section starts at
  the right end. `Work:` and `work:` are the same section, labelled with the first
  spelling in the list.

Press `l` / `h` (or `Tab` / `Shift+Tab`, `Alt+F` / `Alt+B`) or click a tab to switch; the
ends wrap around. `s` followed by a digit jumps straight to a tab by position: `s1` is
`[*]`, `s2` the next tab, and `s0` the tenth. `Alt+S` works the same way. While sati waits
for the digit, each of the first ten tabs shows its number just below its name. A digit
past the last tab does nothing, and any other key after `s` just cancels the jump. After
two seconds without a digit, the jump is forgotten and the numbers disappear.

Press `` ` `` to find a section by name. `[*]` turns into a small input, and the tabs to
its right narrow to the sections that fuzzy-match what you type (`wl` finds `Wishlist`),
best match first; `[-]` matches as "unsectioned". Each change to the query shows the best
match's tasks right away. `Tab` / `Shift+Tab` (or `Ctrl+N` / `Ctrl+P`) step through the
matches and wrap around. `Enter` stays on the highlighted match; opened from `[*]`, it
waits until typing or `Tab` highlights one. `Esc` or `Ctrl+G` returns to the tab, task,
and scroll position you started from. While nothing matches, the last shown section stays
on screen, and `Tab` and `Enter` wait until the query matches again. On a narrow
terminal, a long section name is shortened with `…` so the current tab stays visible.

Press `L` / `H` (or `Alt+N` / `Alt+P`) on a section tab to move it one place right or
left among the named sections; a count such as `3L` moves it further. `[*]` and `[-]` stay first, the ends don't wrap, and the
tasks themselves keep their order; `u` undoes a move. Renaming a section keeps its place.
A section disappears from the order with its last task, and comes back at the right end
if you use it again. On macOS, the `Alt` shortcuts need the terminal to send Option as
Meta (Esc+).

 A section tab hides the shared prefix
and keeps navigation, counts, reordering (`J` / `K`), and search within its own tasks.
`X`, `D`, and `c` clear or copy only the current tab's tasks; `sati export` always
exports the whole list.
If a change moves the selected task out of the tab (for example editing its prefix, or
undo), sati follows the task to its section. Deleting a section's last task returns to
`[*]`. The current tab is remembered between sessions. `Work:Fix`, URLs such as
`https://…`, and times such as `10:30` don't create sections.

Press `E` on a section tab to rename it. The tab turns into an input with the current
name; `Enter` renames the section on every task in it and `Esc` cancels. While the name
is already used by another section, or isn't one word without colons, it is shown
struck through, a notice explains why, and `Enter` does nothing. Changing only the case
(`work` to `Work`) is allowed and gives every task the same spelling. `u` undoes the
whole rename at once.

### While typing

The add, edit, search, rename, and section-picker prompts share these shortcuts:

| Action | Keys |
| --- | --- |
| Move cursor | `←` / `→`, `Ctrl+B` / `Ctrl+F` |
| Start / end of text | `Home` / `End`, `Ctrl+A` / `Ctrl+E` |
| Previous / next word | `Ctrl+←` / `Ctrl+→`, `Alt+B` / `Alt+F` |
| Delete next word | `Alt+D` |
| Delete previous word | `Ctrl+Backspace`, `Alt+Backspace`, `Ctrl+W` |
| Delete character at cursor | `Delete`, `Ctrl+D` |
| Delete to end / start of input | `Ctrl+K` / `Ctrl+U` |
| Cancel current operation | `Esc`, `Ctrl+G` |

`Ctrl+G` also cancels confirmation dialogs and closes the keyboard reference; it does
nothing in Normal mode.

**Section suggestions:** While adding or editing, if the input is empty or a single
partial word (such as `wo`), `Tab` / `Shift+Tab` (or `Ctrl+N` / `Ctrl+P`) cycle through
matching sections, shown as dim text after the cursor. `Space` or `Enter` accepts the
suggestion, and typing any other character accepts it and continues after `Work: `.
`Backspace` dismisses it and keeps the input open; `Esc` or `Ctrl+G` cancels the whole add
or edit, suggestion included. Adding a task from a section tab starts with that section suggested; from `[*]` or
`[-]`, nothing is suggested until you press `Tab`. A section with no task text after it
adds nothing.

**Search:** Press `/` and type. Each change to the query focuses the first match starting from the task that was selected when search began (that task itself if it matches), wrapping to the top. `Tab` / `Shift+Tab` (or `Ctrl+N` / `Ctrl+P`) cycle through the matches, and moving the cursor within the query keeps the focused match. `Enter` selects the focused match and returns to the checklist. `Esc` cancels and restores your previous selection. Exiting search clears highlighting; the query is not remembered.

## Options

| Flag | Effect |
| --- | --- |
| `--theme NAME` | Select a built-in theme (listed below). |
| `--config PATH` | Use an explicit configuration file. |
| `--wrap` / `--no-wrap` | Enable / disable task wrapping. |
| `--round-corners` / `--square-corners` | Use rounded / sharp borders. |
| `--ascii-markers` / `--unicode-markers` | Use ASCII / Unicode task markers. |
| `--notifications` / `--no-notifications` | Show / hide brief confirmation messages. |
| `--accent COLOR` | Override the theme/TOML accent foreground: ANSI name, hex RGB, or palette index 0–255. |

Task markers default to `›`, `[ ]`, `[✓]`, and `[↪]`. `--ascii-markers` changes only task markers; other UI symbols remain Unicode.

The `default` theme uses your terminal’s foreground and background, with dimming for muted
items and ANSI bright yellow (`lightyellow`) for its accent. Named themes supply their own
colors; TOML styles can override them, followed by CLI `--accent` for the accent foreground.

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
rounded_corners = false
ascii_markers = false
notifications = true

[styles.normal]
fg = "reset"
bg = "#20242d"

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

sati reads its configuration once, at startup. After editing the file, restart sati
for the changes to take effect. A configuration problem (malformed TOML, an unknown
key, a bad value or color, or a missing `--config` file) stops sati with an error
before the interface opens.

CLI pairs override either TOML value: `--no-wrap` disables wrapping, and `--wrap`
re-enables it. Contradictory pairs are rejected.

The `default` theme uses terminal-native colors and remains the default. Other
built-in themes provide five RGB foregrounds, their original Neovim-inspired dark
backgrounds, and no base text modifiers. `default` keeps the terminal background
unless explicitly overridden.

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
Cycle themes during a session with `t` / `T`. Changes are temporary and never
modify configuration; restarting restores the CLI/configured theme. TOML style
overrides and CLI accent remain active while cycling.

Explicit `[styles.*]` settings override the chosen theme, and
`--accent` takes final precedence for the accent foreground only, preserving
configured background and modifiers.

The five style roles support `fg`, `bg`, and `modifiers`. Foregrounds accept Ratatui ANSI
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
Backgrounds accept exact hex RGB strings or `reset` (terminal default). A normal
background cascades to every role unless that role overrides it. For example,
`[styles.normal]` with `bg = "#20242d"` replaces the selected theme's background.

Sati fills character cells through Ratatui and sends OSC 11 for the resolved RGB normal
background, including terminal padding. Runtime `t` / `T` changes both. Terminals
that ignore OSC 11 still get cell backgrounds. OSC 111 restores the configured
terminal background, if Sati changed it, on exit or when switching to a theme without an
effective background. It cannot recover a temporary OSC override that existed before
Sati started.
Help, version output, and Markdown export do not load display configuration.

## Export

Export Markdown to stdout, even while the TUI is running:

```sh
sati export
sati export > tasks.md
sati export | grep -F '[ ]'
```

Stdout carries only the Markdown, so it can be redirected or piped; errors go to stderr.
Shell redirection with `>` replaces an existing file, and sati adds no overwrite
protection of its own. Carried tasks export as ordinary unchecked items, with task
order and literal text preserved.

Press `c` in the checklist to send the same Markdown, limited to the current section
tab, to the terminal clipboard using OSC 52. The brief “Copy sent to terminal” message
clears on subsequent activity; clipboard acceptance depends on terminal and
multiplexer support.
`--no-notifications` (or `notifications = false`) hides this and sati's other brief
messages (undo/redo descriptions, wrap, delete and clear confirmations, and hints such as
“Nothing to undo”), but a failed copy is still reported.

## Data

Tasks are saved locally in `tasks.json`:

| Platform | Location |
| --- | --- |
| Linux | `~/.local/share/sati/tasks.json` (or `$XDG_DATA_HOME/sati/tasks.json`) |
| macOS | `~/Library/Application Support/sati/tasks.json` |
| Windows | `%APPDATA%\sati\tasks.json` |

Tasks are saved automatically to `tasks.json`. Each save replaces the file atomically, so
an interrupted save leaves the previous list rather than a partial file. Undo/redo history
is session-only and is not preserved across restarts. Old `history.json` files are ignored
and can be removed manually if desired.

The task file stores the checklist, the selected task, the current section tab, and the
order of the section tabs (`section_order`, the lowercase section names from left to
right).
Navigation is saved on normal quit; a crash may lose selection changes since the last
task save.

Each task has a unique `id` (a UUID) that stays the same as the task is edited, completed,
or moved, a `created_at` time in UTC (RFC 3339, such as `2026-11-07T19:24:05Z`), and a
`status`: `"pending"`, `"completed"`, or `"carried"` (unfinished and kept by `D`). A task
can't be both completed and carried; completing a carried task clears its carryover.
Pasting a copied task creates a new task with its own `id`. A task's section is stored in
its own `section` field, separate from its `text`; typing `Work: fix bug` sets the section
to `Work` and the text to `fix bug`, and the task is still shown, edited, and exported as
`Work: fix bug`.

The file starts with a `schema_version` (currently `5`). Files from older formats, including
the original bare task lists, are no longer upgraded: sati refuses them and leaves them
untouched, so open such a file once with an older sati release to upgrade it. Future format
changes will be upgraded in memory when sati starts and written in the current format on
the next save or normal quit; starting sati never rewrites the file by itself. A file
written by a newer version of sati is refused and left untouched, as is any file that cannot
be read (including a task with a missing or unknown `status`), that gives two tasks the
same `id`, or whose `section_order` doesn't list each section exactly once.

## Credits

Inspired by [Meowdo](https://github.com/Sycorlax/Meowdo). Built with [Ratatui](https://ratatui.rs).

## License

[MIT](LICENSE).
