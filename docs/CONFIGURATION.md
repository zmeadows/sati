# Configuration

sati needs no configuration. Settings are taken from command-line options first, then `config.toml`, then the built-in defaults.

## The configuration file

| Platform | Default file |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/sati/config.toml`, or `~/.config/sati/config.toml` |
| macOS | `~/Library/Application Support/sati/config.toml` |
| Windows | `%APPDATA%\sati\config.toml` |

`--config PATH` reads another file instead. A missing default file is ignored, but a file given with `--config` must exist.

sati reads the file once, at startup, and never writes it; restart sati to apply changes. Malformed TOML, unknown keys, and invalid values stop sati with an error before the interface opens. `--help`, `--version`, `--list-themes`, and `sati export` don't read the file.

## Settings

Every key is optional. [`config_defaults.toml`](config_defaults.toml) is a complete file with every setting at its default; copy it and keep only the lines you change.

| Setting | Values | Default | Command line |
| --- | --- | --- | --- |
| `theme` | A built-in theme name | `"default"` | `--theme NAME` |
| `[display] wrap` | `true`, `false` | `true` | `--wrap`, `--no-wrap` |
| `[display] border_style` | `"square"`, `"rounded"` | `"square"` | `--border-style` |
| `[display] task_markers` | `"unicode"` (`›`, `[✓]`, `[↪]`), `"ascii"` (`>`, `[x]`, `[>]`) | `"unicode"` | `--task-markers` |
| `[display] status_messages` | `true`, `false` | `true` | `--status-messages`, `--no-status-messages` |
| `[display] status_message_duration_ms` | A positive integer | `3560` | — |
| `[display] show_help_hint` | `true`, `false` | `true` | — |
| `[input] mouse` | `true`, `false` | `true` | `--mouse`, `--no-mouse` |

- `status_messages = false` hides routine messages, such as undo descriptions and "Cleared 3 tasks". Errors and validation notices are always shown, each for `status_message_duration_ms`.
- `show_help_hint = false` hides the `? for help` hint in the bottom-right corner; `?` still works.
- `mouse = false` leaves the mouse to the terminal, which disables clicking tasks and tabs and wheel scrolling.
- When both flags of a pair are given, the last one wins.

## Themes

`sati --list-themes` prints the built-in theme names. Select one with `theme = "NAME"` or `--theme NAME`. `t` / `T` cycle through themes in a running session; the change lasts only for that session.

The `default` theme uses your terminal's colors: its foreground and background, dimmed text for muted elements, and the palette's bright yellow, bright cyan, and yellow for the accent, headings, and carryover marker. The other themes reproduce established Vim, Neovim, and Emacs color schemes, in dark and light variants, with their own RGB colors and background.

The `modus-*` themes are adapted from the GNU Emacs Modus themes. `modus-operandi` (light) and `modus-vivendi` (dark) are high contrast, and their `-deuteranopia` and `-tritanopia` variants are designed for red-green and blue-yellow color-vision deficiencies. Their text, accent, heading, and carryover-marker colors reach at least 7:1 contrast against the background.

In every theme, completed tasks, the focused task, and search matches are also marked without color: by strikethrough, the `›` marker, and reversed highlighting.

When the effective background is an RGB color, sati also sets the terminal's own background (OSC 11) so the window padding matches, and restores the terminal's configured background on exit where the terminal supports it (OSC 111).

## Styles

`[styles.ROLE]` tables override the selected theme for individual parts of the interface:

| Role | Used for | Inherits from |
| --- | --- | --- |
| `normal` | Task text and the background | — |
| `muted` | Secondary text: inactive tabs, hints, and the scrollbar | `normal` |
| `completed` | Completed tasks | `muted` |
| `border` | Window and dialog borders | `muted` |
| `accent` | Focus marker, current tab, prompts, and new status messages | `normal` |
| `heading` | Section prefixes and keyboard reference titles | `normal` |
| `carryover_marker` | The arrow in a carried task's checkbox | `normal` |

Each role accepts three optional keys:

- `fg`: a color name, `"#RRGGBB"`, a palette index as a string such as `"208"`, or `"reset"` for the terminal's foreground (not allowed for `accent`). Names are `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `gray`, `darkgray`, `white`, and `light` variants such as `lightyellow`.
- `bg`: `"#RRGGBB"`, or `"reset"` for the terminal's background.
- `modifiers`: a list of `"bold"`, `"dim"`, `"italic"`, `"underline"`, and `"strikethrough"`. The list replaces the inherited modifiers, and `[]` clears them.

A role starts from its parent's final style, then applies the theme's settings for that role, then your override. A `normal` background therefore reaches every role that doesn't set its own. In the `default` theme, a `normal` foreground also reaches `muted`, `completed`, and `border`; the other themes set every role's foreground and modifiers themselves, so override each role you want to change.

`--accent COLOR` replaces the accent foreground after everything else. Overrides stay in effect while cycling themes with `t` / `T`. Completed-task strikethrough, bold focus and section prefixes, and search highlighting are applied on top of the role styles.

## Examples

A darker background for any theme, with a bold orange accent:

```toml
[styles.normal]
bg = "#20242d"

[styles.accent]
fg = "#e78a4e"
modifiers = ["bold"]
```

In the `default` theme, gray instead of dimmed secondary text, for terminals that render dim text poorly; `completed` and `border` inherit it:

```toml
[styles.muted]
fg = "darkgray"
modifiers = []
```

Command-line options apply to a single run: `sati --theme nord --accent lightblue`.
