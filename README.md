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

It should also feel familiar if you already use Vim or Emacs. Most checklist actions use Vim-style keys, many also have an Emacs-style alternative, and the text prompts use the editing keys you know from Emacs and the shell: move with `j` / `k` or `Ctrl+N` / `Ctrl+P`, copy and paste a task with `y` / `p` or `Alt+W` / `Ctrl+Y`, and jump to the end of a line with `Ctrl+E`. Most shortcuts should work the way your fingers expect.

## Features

- **One local list.** Changes are saved automatically, without accounts or synchronization.
- **Shortcuts on demand.** Press `?` for a keyboard reference instead of a permanent shortcut footer. A muted `? for help` in the bottom-right corner is the only on-screen hint.
- **Keyboard-first, with optional mouse support.** Navigate, reorder, and complete tasks from the keyboard. Click a task to toggle it, or scroll with the mouse wheel; `--no-mouse` leaves the mouse to your terminal.
- **Search in place.** Find tasks as you type, see matching text highlighted, and jump to a result without filtering or rearranging the list.
- **Sections.** Start a task with `Name: ` to group it; a tab bar then lets you focus on one section at a time.
- **Carryover.** Mark unfinished tasks with `m` (`[↪]`) to keep them when clearing the list.
- **Undo and redo.** Up to 256 changes in either direction during the current session.
- **Display options.** Wrap long tasks, pick a theme, and adjust borders, task markers, and colors with command-line flags or a configuration file. When tasks don't fit, a scrollbar on the right border shows where you are.

## Suggested usage

There's no prescribed way to use `sati`, but a few habits have worked well for me:

- **Keep it always one keystroke away.** I leave `sati` running in a dedicated terminal and use `Alt+grave` to show or hide it through my window manager. It's always available without occupying screen space.
- **Keep tasks short and actionable.** Think of it as a checklist, not a place for detailed notes or project plans.
- **Start fresh regularly.** At the end of the day, clear the list and plan for tomorrow. Carry over the handful of unfinished tasks that still matter. The carryover marker (`m`) protects them from being cleared.
- **Use ordering however you like.** Move important tasks toward the top, or place tasks in the order you know you'll want to complete them.

## Installation

Prebuilt executables are available from [GitHub Releases](https://github.com/zmeadows/sati/releases).
Download and extract the archive for your platform; Rust is not required. Archives
are `.tar.gz` on Linux/macOS and `.zip` on Windows, and include LICENSE.
Run the extracted executable from a terminal with `./sati` (Linux/macOS) or
`.\sati.exe` (Windows PowerShell), or put it in a directory on your `PATH`.
Compare the archive's SHA-256 hash with `SHA256SUMS` to check download integrity.

Builds target Linux x86_64 and ARM64 (GNU/glibc, built on Ubuntu 22.04), macOS
Apple Silicon and Intel, and Windows x86_64. Ubuntu 22.04 provides an older glibc
baseline; compatibility with older Linux systems is not guaranteed.

macOS downloads are not Developer ID signed or notarized; Windows downloads are not
Authenticode signed. [Gatekeeper](https://support.apple.com/en-us/102445) or
[SmartScreen](https://learn.microsoft.com/en-us/windows/security/operating-system-security/virus-and-threat-protection/microsoft-defender-smartscreen/)
may warn when launching them.

With [Rust and Cargo](https://www.rust-lang.org/tools/install) (Rust 1.89 or newer):

```sh
cargo install --git https://github.com/zmeadows/sati --locked
```

Then run `sati`.

## Keyboard shortcuts

Where a row lists several keys, Vim-style keys usually come first, followed by arrow-key and Emacs-style alternatives.

On macOS, Alt shortcuts need Option to send Meta/Escape-prefixed input. Terminal.app
calls this “Use Option as Meta key”; iTerm2 uses “+Esc”. Settings vary by terminal,
and terminal or OS shortcuts can intercept key combinations before sati receives them.

### Checklist

| Action | Keys |
| --- | --- |
| Navigate | `j` / `k`, `↓` / `↑`, `Ctrl+N` / `Ctrl+P` |
| Next / previous section tab (wraps) | `l` / `h`, `Tab` / `Shift+Tab`, `Alt+F` / `Alt+B` |
| Go to section tab 1–10 | `s1` … `s9`, `s0`; `Alt+S` then a digit |
| Find section tab by name | `` ` ``, `Alt+G` |
| Back to previous tab and task | `'` |
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
| Clear except carried | `D` |
| Undo | `u`, `Ctrl+Z` |
| Redo | `Ctrl+R` |
| Search | `/`, `Ctrl+F`, `Ctrl+S` |
| Toggle wrapping | `w` |
| Next / previous color theme | `t` / `T` |
| Keyboard reference | `?` |
| Quit | `q` |

`?` opens a keyboard reference over the checklist, in place of a permanent shortcut
footer; the muted `? for help` in the bottom-right corner is the only reminder of it
(hidden with `show_help_hint = false`).
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

`X` clears the current tab's completed tasks. `D` asks first, then clears every task in
the current tab, completed or not, except those marked for carryover with `m`.

Task copies use one in-memory register, separate from the system clipboard. `y` (or
`Alt+W`) puts the selected task's text in the register, and `p`, `P`, or `Ctrl+Y` paste
it as a new task. Pasted tasks start unchecked and not marked for carryover; the
register remains available for repeated pastes until exit. `c` sends the current tab's
tasks as Markdown to the terminal using OSC 52 (see [Export](#export)).

### Sections

Start a task with one word, a colon, and a space, such as `Work: Fix bug`, to put it in
the `Work` section. Sections are optional: with none, sati looks exactly as before. Once
any task has a section, tabs appear on the top border, with the current one shown in
the accent color and a blank row separating them from the tasks:

- `[*]` shows every task in your order, with each section prefix as a bold heading.
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
left among the named sections; a count such as `3L` moves it further. `[*]` and `[-]` stay
first, the ends don't wrap, and the tasks themselves keep their order; `u` undoes a move. Renaming a section keeps its place.
A section disappears from the order with its last task, and comes back at the right end
if you use it again.

A section tab hides the shared prefix and keeps navigation, counts, reordering (`J` / `K`), and search within its own tasks.
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

Press `'` to go back to the tab and task you were on before your last jump. Switching tabs
any way, `g` / `G` / `Home` / `End`, accepting a search, and changes that move to another
tab all count as jumps; ordinary movement such as `j` / `k` and paging doesn't. Press `'`
again to return, so it toggles between two places. If the task has since been deleted or
moved out of that tab, the tab still opens; if the tab no longer exists, `'` says so and
stays put. The previous position lasts only for the session.

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
or edit, suggestion included. Adding a task from a section tab starts with that section
suggested; from `[*]` or `[-]`, nothing is suggested until you press `Tab`. A section with no task text after it
adds nothing.

**Search:** Press `/` and type. Each change to the query focuses the first match starting from the task that was selected when search began (that task itself if it matches), wrapping to the top. `Tab` / `Shift+Tab` (or `Ctrl+N` / `Ctrl+P`) cycle through the matches, and moving the cursor within the query keeps the focused match. `Enter` selects the focused match and returns to the checklist. `Esc` cancels and restores your previous selection. Exiting search clears highlighting; the query is not remembered.

**Pasting on Windows:** Sati's current Crossterm Windows input backend does not report bracketed-paste events, so pasted text can arrive as ordinary keystrokes. Open a prompt (`a`, `e`, or `/`) before pasting; in the checklist itself, pasted letters act as shortcuts. A line break acts as `Enter`, potentially submitting the prompt and letting subsequent text execute shortcuts; a tab acts as `Tab`. Paste one line at a time. Windows Terminal asks before pasting multiple lines by default; keep that warning on. `u` or `Ctrl+Z` undoes accidental changes only while sati stays open: if a pasted `q` exits sati, its undo history is lost.

Crossterm 0.29.0 may also silently drop certain Unicode characters on Windows,
particularly emoji entered through the Windows emoji panel (`Win+.`); see
[upstream issue #1072](https://github.com/crossterm-rs/crossterm/issues/1072).
This upstream limitation has not been reproduced in Sati through native Windows testing.

## Options

| Flag | Effect |
| --- | --- |
| `--theme NAME` | Select a built-in theme (see `--list-themes`). |
| `--list-themes` | Print the built-in theme identifiers, one per line, and exit. |
| `--config PATH` | Use an explicit configuration file. |
| `--data-dir PATH` | Keep `tasks.json` and `tasks.lock` in this directory (see [Data](#data)). Also works with `export`. |
| `--wrap` / `--no-wrap` | Enable / disable task wrapping. |
| `--border-style square\|rounded` | Draw square (default) or rounded border corners. |
| `--task-markers unicode\|ascii` | Draw the focus marker and checkboxes with Unicode (default) or ASCII symbols. |
| `--status-messages` / `--no-status-messages` | Show / hide routine status messages; errors and validation notices are always shown. |
| `--mouse` / `--no-mouse` | Enable (default) / disable mouse capture. |
| `--accent COLOR` | Override the theme/TOML accent foreground: ANSI name, hex RGB, or palette index 0–255. |

Display and input flags override the matching [configuration](#configuration) settings.
`--data-dir` is global: it can go before or after `export`. The other options apply only
to the interactive checklist.

Task markers are the focus marker and checkboxes: `›`, `[ ]`, `[✓]`, and `[↪]` by default. `--task-markers ascii` uses `>`, `[ ]`, `[x]`, and `[>]` instead. Other symbols, such as borders, prompts, and search markers, are unaffected.

The `default` theme uses your terminal’s foreground and background, with dimming for muted
items, ANSI bright yellow (`lightyellow`) for its accent, bright cyan (`lightcyan`) for
headings, and yellow for the carryover marker. Named themes supply their own colors; TOML
styles can override them, followed by CLI `--accent` for the accent foreground only.

```sh
sati
sati --accent lightblue
sati --accent '#a9b665'
sati --accent 3
```

Run `sati --help` for the command-line help (`-h` gives a shorter summary), `sati --version`
for the version, and `sati export --help` for export's own help.

## Configuration

Configuration is optional. Precedence is **CLI > TOML > built-in defaults**.

| Platform | Default file |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/sati/config.toml`, falling back to `~/.config/sati/config.toml` |
| macOS | `~/Library/Application Support/sati/config.toml` |
| Windows | `%APPDATA%\sati\config.toml` |

Every section and key is optional. A configuration usually sets only a few of them:

```toml
theme = "catppuccin-mocha"

[display]
wrap = false
border_style = "rounded"

[input]
mouse = false
```

[`config_defaults.toml`](config_defaults.toml) lists every option at its built-in default,
with the accepted values and commented-out style examples. Copy it as a starting point and keep
only the lines you change.

Status messages appear briefly on the border. `status_messages = false` (or
`--no-status-messages`) hides the routine ones, such as undo/redo descriptions,
confirmations like “Cleared 3 tasks”, and hints like “Nothing to undo”. Errors, such as a
failed copy, and validation notices, such as why a section name can't be used, are always
shown.

Two display preferences exist only in the configuration file, without command-line flags:

- `status_message_duration_ms` (default `3560`) is how long each status message stays on
  the border, in milliseconds. It must be a positive integer. A message is accented for its
  first 300 ms (or all of it, if shorter), then drawn in the normal style. It applies to every
  status message, including the errors and validation notices that `status_messages = false`
  doesn't hide.
- `show_help_hint` (default `true`) shows the muted `? for help` in the bottom-right corner.
  Set it to `false` to hide that hint; `?` still opens the keyboard reference.

`[input] mouse` (default `true`) lets sati capture the mouse: clicking a task toggles it,
clicking a tab switches to it, and the wheel scrolls the checklist and the keyboard reference.
Set it to `false` (or pass `--no-mouse`; `--mouse` turns it back on) to stop sati from
requesting mouse reporting at all. The mouse then behaves as in any other terminal program,
so you can select and copy text with it, and every action stays available from the keyboard.

Use `sati --config ./sati.toml` for another file. sati never creates or edits
configuration files. A missing default file is harmless; an explicit file must
exist. Invalid files and unknown keys are errors. Environment directory paths
must be absolute; `--config` also accepts relative paths without expanding `~`.

sati reads its configuration once, at startup. After editing the file, restart sati
for the changes to take effect. A configuration problem (malformed TOML, an unknown
key, a bad value or color, or a missing `--config` file) stops sati with an error
before the interface opens. `--help`, `--version`, `--list-themes`, and `sati export` don't
read the configuration file.

CLI pairs override either TOML value: `--no-wrap` disables wrapping, and `--wrap`
re-enables it. When both flags of a pair are given, the last one wins.

The `default` theme uses terminal-native colors and remains the default. Other
built-in themes reproduce established Vim, Neovim, and Emacs color schemes, in both dark
and light variants (such as `solarized-light`, `catppuccin-latte`, `modus-operandi`, and
`ef-summer`). Each provides explicit RGB foregrounds for all seven style roles, one shared RGB
background, and no base text modifiers. `default` keeps the terminal background unless
explicitly overridden.

Run `sati --list-themes` to print the available theme identifiers.

Select a theme with `theme = "catppuccin-mocha"` at the TOML root or
`sati --theme catppuccin-mocha`; CLI selection takes precedence.
Cycle themes during a session with `t` / `T`. Changes are temporary and never
modify configuration; restarting restores the CLI/configured theme. TOML style
overrides and CLI accent remain active while cycling.

Seven style roles color the interface, each configurable as `[styles.ROLE]`:

| Role | Used for | Inherits |
| --- | --- | --- |
| `normal` | Task text, other body text, and the shared background | — |
| `muted` | Secondary text: inactive tabs, hints, and the scrollbar | `normal` |
| `completed` | Completed task text, section prefix included | `muted` |
| `border` | Window and dialog borders | `muted` |
| `accent` | Focus marker, current tab, prompts, and new status messages | `normal` |
| `heading` | Section prefixes such as `Work:`, and the keyboard reference's titles | `normal` |
| `carryover_marker` | The arrow in a carried task's checkbox (`↪` or `>`), not the task's text | `normal` |

Each role starts from its parent's resolved style and keeps whatever the theme or an
override doesn't change. In the `default` theme, muted adds DIM to normal; completed and
border are muted unchanged; accent, heading, and carryover_marker change only the
foreground, to `lightyellow`, `lightcyan`, and `yellow` from the terminal's palette. The
RGB themes instead supply explicit foregrounds and empty base modifier sets for each role,
and their background reaches every role through normal.

Explicit `[styles.*]` settings override the chosen theme, and
`--accent` takes final precedence for the accent foreground only, preserving
configured background and modifiers; it never changes heading or carryover_marker.

Every role supports `fg`, `bg`, and `modifiers`. Foregrounds accept Ratatui ANSI
names, hex RGB, and palette indices (as strings). `reset` inherits the terminal
foreground and is allowed except for accent.

Omitted properties retain those inherited values. A modifier array **replaces**
inherited modifiers; `modifiers = []` clears them. Supported names are `bold`,
`dim`, `italic`, `underline`, and `strikethrough`, without duplicates. Completion
strikethrough, focus/prefix boldness, and search highlighting remain contextual: they are
not roles, and they apply on top of the role styles. A search match inside a section
prefix is highlighted like any other match.
Backgrounds accept exact hex RGB strings or `reset` (terminal default). A normal
background cascades to every role unless that role overrides it. For example,
`[styles.normal]` with `bg = "#20242d"` replaces the selected theme's background.

Sati fills character cells through Ratatui and sends OSC 11 for the resolved RGB normal
background, including terminal padding. Runtime `t` / `T` changes both. Terminals
that ignore OSC 11 still get cell backgrounds. On terminals supporting OSC 111, it restores
the configured terminal background, if Sati changed it, on exit or when switching to a theme
without an effective background. It cannot recover a temporary OSC override that existed
before Sati started.

### Accessibility-oriented themes

Five more themes join `modus-operandi` with palettes derived from the GNU Emacs Modus
themes and adapted to sati's seven style roles:

- `modus-operandi` and `modus-vivendi`: high-contrast light and dark appearances.
- `modus-operandi-deuteranopia` and `modus-vivendi-deuteranopia`: designed with red-green
  color-vision deficiencies in mind.
- `modus-operandi-tritanopia` and `modus-vivendi-tritanopia`: designed with blue-yellow
  color-vision deficiencies in mind.

Their text, accent, heading, and carryover-marker colors reach at least 7:1 contrast
against the background. That helps many readers but can't suit everyone; completed tasks,
focus, and search matches also keep their non-color cues (strikethrough, the `›` marker,
and reversed highlighting).

## Export

Export Markdown to stdout, even while the TUI is running:

```sh
sati export
sati export > tasks.md
sati export | grep -F '[ ]'
sati export work                           # only the Work section
sati export --unsectioned                  # only tasks without a section
sati export --data-dir ~/checklists/work   # or: sati --data-dir ~/checklists/work export
```

A section name matches regardless of case, and that section's tasks are written without their
`Work: ` prefix. An unknown name is an error that lists the checklist's sections in tab order.
An invalid `tasks.json` is reported on stderr, never partly exported.

Stdout carries only the Markdown, so it can be redirected or piped; errors go to stderr.
Shell redirection with `>` replaces an existing file, and sati adds no overwrite
protection of its own. Carried tasks export as ordinary unchecked items, with task
order and literal text preserved.

Press `c` in the checklist to send the same Markdown, limited to the current section
tab, to the terminal using OSC 52. The brief “Copy sent to terminal” message
clears on subsequent activity or after `status_message_duration_ms`; it confirms sending,
not that the system clipboard changed. The terminal must accept clipboard writes,
and any multiplexer must pass them through; sati cannot confirm acceptance.
Like other routine status messages, it is hidden by `--no-status-messages` (or
`status_messages = false`); a failed copy is always reported.

If OSC 52 does not work, pipe `sati export` into a clipboard utility in your local
desktop session (install `wl-copy` or `xclip` separately if needed):

| Platform | Command |
| --- | --- |
| Linux Wayland | `sati export \| wl-copy` |
| Linux X11 | `sati export \| xclip -selection clipboard` |
| macOS (UTF-8 locale) | `sati export \| pbcopy` |
| Windows PowerShell | `sati export \| Set-Clipboard` |

For Windows PowerShell, first run `[Console]::OutputEncoding = [System.Text.Encoding]::UTF8`
so the pipeline decodes sati's UTF-8 output correctly.

## Data

Tasks are saved locally in `tasks.json`:

| Platform | Location |
| --- | --- |
| Linux | `~/.local/share/sati/tasks.json` (or `$XDG_DATA_HOME/sati/tasks.json`) |
| macOS | `~/Library/Application Support/sati/tasks.json` |
| Windows | `%APPDATA%\sati\tasks.json` |

`--data-dir PATH` uses another directory instead: sati keeps `tasks.json` and its
`tasks.lock` directly in `PATH` (no `sati` subdirectory is added; a relative path is taken
from the current directory). sati doesn't expand `~` itself: write `--data-dir ~/work` with a
space so the shell expands it, since `--data-dir=~/work` would name a directory called `~`
in some shells. Each directory is an independent checklist with its own lock,
so separate directories can be open at the same time. The interactive checklist creates the
directory (and any missing parents) if needed; `sati export` only reads it, never creates
it, and reports an error if it is missing or not a directory. An existing directory without
a `tasks.json` is an empty checklist.

Each running sati shows exactly one checklist; there is no switching between lists inside
the app.

`--data-dir` only chooses where tasks live. Configuration is still read from the default
`config.toml` (or `--config PATH`), and neither option changes the other. For example, to
keep separate personal and work checklists:

```sh
sati --data-dir ~/checklists/personal
sati --data-dir ~/checklists/work --config ~/.config/sati/work.toml
sati export --data-dir ~/checklists/work > work.md
```

A shell alias such as `alias work='sati --data-dir ~/checklists/work'` makes the second
list one word away.

Tasks are saved automatically to `tasks.json`. Each save replaces the file atomically, so
an interrupted save leaves the previous list rather than a partial file. Undo/redo history
is session-only: exiting sati, even accidentally, discards it. Old `history.json` files
are ignored and can be removed manually if desired.

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
