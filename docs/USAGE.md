# Using sati

This guide covers behavior that isn't obvious from the keyboard reference (`?` in sati) or `sati --help`.

## Sections

Start a task with one word, a colon, and a space to put it in a section: `Work: Fix bug` belongs to `Work`. `Work:Fix`, URLs, and times such as `10:30` don't create sections. Section names are case-insensitive; a tab is labelled with the first spelling in the list.

Once any task has a section, tabs appear on the top border:

- `[*]` shows every task, with section prefixes as headings.
- `[-]` shows tasks without a section, when there are any.
- Each section follows with its own tab. A new section starts at the right end, and a section disappears with its last task.

A section tab is a view of the same list, not a separate list. It hides the prefix, and movement, reordering, search, clearing, and `C` apply only to its tasks. Adding a task from a section tab suggests that section, and while the input is empty or a single partial word, `Tab` cycles through matching sections.

`s` then a digit jumps to a tab by position (`s1` is `[*]`, `s0` the tenth), and `` ` `` finds a tab by fuzzy name. Switching to a tab that doesn't contain the selected task selects its first unfinished task. `'` returns to the tab and task you were on before your last jump; press it again to go back. `E` renames the current section on every task in it, and `L` / `H` move its tab without reordering any tasks. `M` moves the selected task to the bottom of a section you find by name, or to a new one after you confirm its name, and follows it to that tab.

`/` searches the current tab for exact text. `f` finds a task on any tab by fuzzy match instead: type a few of its letters in order, such as `wfb` for `Work: Fix bug`, and `Enter` goes to the focused match. The view stays on the current tab if it shows that task, and otherwise switches to the task's own tab; `'` returns to where you were.

## Links and emphasis

A task can link to web pages in two ways:

- `[title](https://example.com/page)` shows as an underlined `title`.
- A bare `https://` or `http://` URL, such as one you paste, shows shortened to its host and the start of its path, like `github.com/zmeadows/sati/issu…`.

Asterisks emphasize text as in Markdown: `*italic*`, `**bold**`, and `***bold italic***`. They count only in pairs that hug the text they wrap, so `2 * 3`, `5*3`, and `*.rs` stay as typed. Some terminals don't show italics.

Only `http` and `https` links are recognized; anything else stays as typed. Editing a task (`e`) shows its text exactly as stored, asterisks included, and copying, pasting, and `tasks.json` all keep it that way. Search matches what a row shows, so it finds a link's title but not the URL behind it.

Click a link to open it in your default browser (clicking elsewhere on the task still checks it off), or press `v` to open every link in the selected task. sati also marks links as terminal hyperlinks (OSC 8), so with `--no-mouse` the terminal's own link click works in terminals that support them; tmux needs `set -as terminal-features ',*:hyperlinks'`. `sati export` and `C` write links and emphasis as Markdown.

## Templates

A template is a named list of tasks you add again and again, such as a daily routine or a packing list. Press `i` to open them: names are listed above the entries of the focused one, with the most recently used first.

`Space` or `Enter` adds a template's entries as new, unchecked tasks. On a section tab they join that section after its last task; on `[*]` and `[-]` they go to the end of the list, and an entry that starts with a section prefix such as `Work: ` goes to that section, as if you had typed it. Undo removes them all at once.

`I` opens the same list, but on a section tab the template replaces the section's tasks instead: pending and completed tasks are removed, while carried tasks stay and the entries follow them. Undo brings the old tasks back.

`a` creates a template, `e` edits its entries with the usual task keys, and `E` renames it. Undo in the editor covers only changes made since it opened. `d` deletes a template after asking, and can't be undone.

## Carryover and clearing

sati is meant to be cleared regularly. Both clearing commands act on the current tab, or the whole list on `[*]`:

- `X` removes completed tasks.
- `D` asks for confirmation, then removes every task not marked for carryover.

`m` marks a task for carryover, shown as `[↪]`. The mark stays after `D`, so the task survives later clears too, until you complete it or press `m` again. Completed tasks can't be carried.

Undo history covers clears, but it lasts only while sati is open.

## Data and multiple checklists

Tasks are saved to `tasks.json`, and templates to `templates.json`, after every change:

| Platform | Directory |
| --- | --- |
| Linux | `$XDG_DATA_HOME/sati/`, or `~/.local/share/sati/` |
| macOS | `~/Library/Application Support/sati/` |
| Windows | `%APPDATA%\sati\` |

Each save replaces the file atomically, so an interrupted save leaves the previous version intact. sati won't open a `tasks.json` or `templates.json` that is malformed or was written by a newer version; it reports the problem and leaves the file untouched.

`--data-dir PATH` keeps these files and a lock file directly in `PATH` instead. Each directory is an independent checklist with its own templates, and only one sati can have it open at a time. `--data-dir` doesn't change where configuration is read from. sati doesn't expand `~` itself, so write `--data-dir ~/work` with a space: some shells don't expand `~` after `=`.

```sh
alias work='sati --data-dir ~/checklists/work'
sati export --data-dir ~/checklists/work > work.md
```

### Backup and restore

`tasks.json` is the whole checklist and `templates.json` its templates, so to back one up, copy those files, whether they are in the default directory or a `--data-dir` directory. To restore, quit sati first, then put the copies back under the same names in the same directory. While sati is open, it would save over a restored file on your next change or when it quits.

`sati export` is for reading, not backup: it drops carryover marks and tab order, and sati can't import it. A backup written by a newer sati may not open in an older one.

## Pasting lists

Pasting several lines into the checklist, or into a new task's prompt, asks whether to add them as separate tasks and previews them. `Enter` adds them where the new task would have gone, `J` joins the lines into one task's text instead, and `Esc` cancels. Bullets, numbers, and checkboxes from Markdown, Org, Obsidian, and todo.txt lists are removed: checked items (`[x]`, `DONE`) are added checked, `[>]` items are carried, and a Markdown heading such as `## Groceries` puts the items below it in that section. A list copied with `c` or `sati export` pastes back as the same tasks, except that carried ones come back unchecked.

## Export and clipboard

`sati export` writes Markdown to stdout and errors to stderr. It only reads `tasks.json`, so it is safe to run while sati is open. Completed tasks become `- [x]` items; pending and carried tasks become `- [ ]`. When exporting a section, its tasks are written without their prefix.

`C` sends the current tab's Markdown to the terminal with OSC 52, and `c` sends only the selected task's text, exactly as stored and without its section prefix inside a section tab. The terminal must allow clipboard writes, and a multiplexer such as tmux must pass them through. sati can't detect whether that worked: "Copy sent to terminal" means only that it was sent. If it doesn't work, pipe an export into a clipboard tool:

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
- **Task ages:** holding `b` shows how long ago each task was added. Terminals that don't report key releases (including many VTE-based terminals, Terminal.app, and tmux) keep the ages shown until you press `b` again.
- **`Ctrl+S`** searches unless your terminal or multiplexer intercepts it; `/` and `Ctrl+F` also search.
- **Mouse:** sati captures the mouse by default. To select and copy text with it instead, use `--no-mouse` or `mouse = false`; links then open with the terminal's own link click, often Ctrl+click or Cmd+click.
- **Pasting on Windows:** pasted text arrives as ordinary keystrokes, so pasted lists aren't recognized. Open a prompt (`a`, `e`, or `/`) before pasting, and paste one line at a time: in the checklist, pasted letters run as shortcuts, and a line break acts as `Enter`. Keep Windows Terminal's multi-line paste warning on.
