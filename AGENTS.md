# Agent instructions for sati

## Project intent

`sati` is a small, persistent terminal checklist written in Rust. It aims to feel like a paper checklist: add tasks, cross them off, reorder them, and clear the list when the context changes. Simplicity is a product requirement, not an unfinished feature set.

- Favor clear, small changes over new abstractions, dependencies, settings, or features.
- Keep the interface quiet and keyboard-friendly. Do not add calendars, tags, priorities, reminders, projects, synchronization, or configurable keybindings unless explicitly requested. Sections (one optional section per task, shown as tabs) were explicitly requested. Each task stores its section in its own field; the `Word: ` typing syntax is only how users enter and edit it.
- Preserve existing behavior and cross-platform support. Avoid unrelated cleanup while implementing a focused change.
- Read the current code before editing; this file is orientation, not a replacement for the implementation.

## Stack and source layout

- Rust 2024 edition; minimum supported Rust version (MSRV) **1.88** (`Cargo.toml`).
- Ratatui **0.30.2** with its Crossterm backend (default features off; see `Cargo.toml` for the enabled set). Import Crossterm via **`ratatui::crossterm`**, not an independently versioned direct dependency.
- Serde/JSON for persisted tasks; `toml` for optional configuration; `fs4` for advisory file locks; `unicode-segmentation` and `unicode-width` for text handling; `nucleo-matcher` for the section picker's fuzzy matching (kept separate from task search).
- `src/main.rs`: CLI arguments (hand-parsed, no CLI framework), `export` subcommand dispatch, startup, applying the effective configuration, terminal lifecycle, event loop, and platform-specific terminal capabilities.
- `src/app.rs`: `Task`, application modes, key/mouse dispatch, task mutations, selection, scrolling, and bounded undo history.
- `src/input.rs`: single-line, grapheme-aware `TextInput`; cursor is a UTF-8 byte offset kept at a grapheme boundary.
- `src/ui.rs`: layout, wrapping, drawing, mouse row mapping, and input cursor placement.
- `src/storage.rs`: platform data paths, exclusive instance lock, JSON loading/validation, schema versioning and migrations, and atomic replacement saves.
- `src/config.rs`: TOML config discovery (`$XDG_CONFIG_HOME`/`~/.config`, macOS Application Support, `%APPDATA%`), reading, and parsing through `Config::load`. A missing default file means defaults; an explicit `--config` file must exist. Unknown keys and invalid files are errors; sati never writes config files. Precedence is CLI > TOML > defaults; contradictory CLI flag pairs are rejected.
- `src/themes.rs` / `src/styles.rs`: built-in theme palettes and the resolved `Styles` (theme + TOML overrides + `--accent`).
- `src/search.rs`: search matching and highlight ranges for `Searching`.
- `src/export.rs`: Markdown rendering shared by `sati export` (stdout only) and the `c` OSC 52 clipboard copy. Export runs without taking the instance lock (it works while the TUI is open) and never writes files.
- `src/notification.rs`: timed notifications on the checklist's bottom border (e.g. theme name after `t` / `T`).
- Unit tests live in `#[cfg(test)] mod tests` inside each module; `tests/` holds integration tests that run the binary (`cli`, `config`, `directories`, `export`, `locking`). Integration tests isolate data/config by overriding `HOME`, `XDG_DATA_HOME`, and `APPDATA`; never point tests at the real user data directory. They run the binary without a terminal, so every run must fail or exit before terminal setup (invalid configuration stops startup before storage, and an invalid `tasks.json` stops it after a valid configuration); otherwise crossterm can take over the developer's `/dev/tty`.

## Behavioral invariants

### Tasks and app state

- A task has `id` (UUID v4, unique within the file, kept through edits, toggles, moves, and undo; a paste is a new task with a new id), `created_at` (`time::OffsetDateTime`, stored as whole-second UTC RFC 3339 like `2026-11-07T19:24:05Z`; get new values from `current_time()`), optional `section` (the source of truth for the task's section; `text` holds only the body), `text`, and a required `status` (`TaskStatus`: `Pending`, `Completed`, or `Carried`, stored as `"pending"`, `"completed"`, `"carried"`); list order is meaningful. Create tasks from typed text with `Task::new`, which splits `Work: fix bug` into section and body; use `display_text()` for the composed form (editing, the copy register, search outside a section tab, export). A completed task cannot be carried, and the single `status` field makes that combination unrepresentable; match on `TaskStatus` rather than adding boolean accessors. `Space`/`Enter` complete a pending or carried task and return a completed one to pending; `m` toggles pending and carried (completed tasks only get a notice); `D` keeps carried tasks and `x` removes completed ones, both within the current tab. New and pasted tasks start pending. Markdown export writes `- [x]` only for completed tasks.
- Task text is a nonempty, trimmed, single-line description. Preserve arbitrary valid Unicode; reject corrupt persisted data rather than silently repairing or overwriting it.
- The normal list selection refers to a **task**, not a wrapped display row. Keep indices valid after deletion, reordering, clearing, and undo.
- A task's section comes from its `section` field, compared case-insensitively. Typed input sets it when the text starts with a word followed by `": "` (`task_prefix`, via `split_section`); stored text is never re-parsed for a section. Stored sections must be one word without spaces, colons, or control characters. `App::view` (`SectionView`) filters what is shown; the selection is always a full-list index of a **visible** task. Navigation, counts, `J`/`K`, and search work over visible tasks; in `[*]` they must behave exactly as without sections. `settled_view` falls back to `[*]` when the tab vanished and follows a selection that left the tab to its own section; mutations and undo/redo resolve it against the new tasks before saving (so the saved tab is the shown tab), and loading applies it through `settle_view`. The active tab is persisted as an optional `section` key in `tasks.json` (absent for `[*]`, `""` for `[-]`). Named tabs follow `App::section_order` (lowercase keys), the single source of the tab order: `settled_section_order` keeps it listing each used section exactly once (new sections append, a section leaves with its last task); it is saved with every mutation and carried in undo snapshots, `rename_section` keeps a renamed key in place, and `move_section` (`H`/`L`, `Alt+P`/`Alt+N`) moves the tab without wrapping or touching tasks (a counted `3L` is one remove/insert, one save, and one undo step). `h`/`l`, `Alt+B`/`Alt+F`, and `s`/`Alt+S` + digit (a transient prefix like a count; any non-digit only cancels it, and it expires like a count, with the event loop waking via `section_jump_expires_in` to hide its digit hint) are Normal-mode only. While adding or editing, a pending section suggestion (`Completion`) exists only while the input is one partial word with the cursor at its end; `Tab`/`Shift+Tab` (and `Ctrl+N`/`Ctrl+P`) cycle it, `Space`/`Enter`/typing/paste accept it, `Backspace` only dismisses it, `Esc`/`Ctrl+G` cancel the whole add/edit, and any other key dismisses it. `E` on a named tab enters `RenamingSection`, drawn as an input inside the active tab; confirming sets `section` on every task in that tab as one undo step ("rename section") and moves the view to the new key. `rename_problem` blocks names taken by another section (a case-only change of the same section is allowed) or invalid as stored sections, and its message is shown even with `--no-notifications`. `` ` `` enters `FuzzySection`, the section picker, drawn as a label-free input in place of `[*]` followed by only the matching tabs (`section_matches`: nucleo fuzzy score, ties and the empty query in tab order, `[-]` matched as "unsectioned", never `[*]`). Its query, matches, and `NavigationCheckpoint` (view, selection, scroll, tab scroll, reveal flag) are transient mode state. The focused match is simply `App::view`: every query edit (detected via `matched_query`, so cursor moves don't count) previews the best match through `show_section`, and `Tab`/`Shift+Tab`/`Ctrl+N`/`Ctrl+P` step through matches with wrapping (`Ctrl+←`/`Ctrl+→` move by word as in every input); with no matches, the last preview stays and only editing or `Esc` act. `Enter` confirms only when `App::view` is one of the matches, keeping the preview as ordinary navigation (no undo entry, saved only on clean exit); `Esc`/`Ctrl+G` restore the checkpoint. Previews never write `tasks.json` (`save_selection` uses the checkpoint), and drawing the picker must not change `tab_scroll` or the checkpoint.
- `tasks.json` stores `{schema_version, tasks, selected, section, section_order}` (current version 5). Unknown or missing `status` values make the file invalid. A `section_order` that does not list each task section exactly once in lowercase makes the file invalid. Duplicate ids make the file invalid. The file must be a JSON object with an integer `schema_version`; `Storage::load_document` removes that field, refuses versions below `OLDEST_SCHEMA_VERSION` (currently 5; older migrations were removed) or above `SCHEMA_VERSION`, and otherwise upgrades the remaining document in memory through `MIGRATIONS` (`MIGRATIONS[n]` upgrades version `OLDEST_SCHEMA_VERSION + n`; currently empty). A format change bumps `SCHEMA_VERSION` and appends one migration step that first parses the previous version strictly into its own typed struct with `deny_unknown_fields`, then builds the next version, never inserting into or overwriting raw fields. An upgraded document sets `needs_migration`, so the current version is written on the next save or clean exit; loading never writes. Reject unsupported, malformed, and failed-migration files with an error, leaving the file untouched. Save resulting selection with mutations and undo/redo; navigation-only focus is saved on clean exit, without persisting temporary search selection.
- `a` appends, `o` inserts below selection, and `O` inserts above selection.
- `j`/`k`/`J`/`K` and their aliases, plus `h`/`l` and `H`/`L`, accept a Vim-style count (`5j`, `3l`, `2H`); counted moves clamp at the ends while plain `j`/`k`/`h`/`l` wrap, and a counted reorder is one undo step. `Ctrl+N`/`Ctrl+P` are normalized to `Down`/`Up` in Normal mode and to `Tab`/`Shift+Tab` in input modes, so they share each mode's next/previous behavior. A pending count is transient, never saved or shown, and any other key, mouse event, or paste discards it; it also expires `PREFIX_TIMEOUT` (2 s) after its last digit, so a stray digit cannot affect a much later motion.
- Undo and redo each hold at most **256 in-memory snapshots** of tasks, section order, selection, and view. A snapshot also records the view its change left behind; undo/redo return to the snapshot's view only while the user is still on that view, so manual tab switches are kept; a kept tab moves the restored selection into it (its first task when the snapshot's selection lies outside it). Mutations pass their target view (e.g. `[*]` when a tab empties) to `replace_with`, which saves it with the tasks and commits it only after the save succeeds. Only successful mutations enter ordinary history and clear redo. Undo/redo transfer snapshots between stacks after saving. Navigation, cancellation, and no-ops leave both histories unchanged.
- Each undo snapshot carries a short action label (e.g. "delete task") that `replace` stores and undo/redo show as "Undid:/Redid: …"; every new mutation must pass one. Routine notifications go through `App::notify` so `--no-notifications` can hide them.
- Undo/redo history is session-only. Never read, write, or delete legacy `history.json` files. Undo/redo still save their resulting task lists to authoritative `tasks.json` before transferring snapshots.

### Storage and failure handling

- Acquire `tasks.lock` before loading and hold the lock for the application's lifetime. Do **not** delete the lock file: all instances must coordinate on the same file.
- Persist each task mutation **before** committing the new in-memory state or consuming undo history. A failed save must not become an accepted mutation.
- Save JSON via a uniquely created temporary file in the same directory, sync it, then replace `tasks.json` by rename. Never truncate/delete the live file first or silently fall back to a non-atomic copy.
- Preserve meaningful errors and leave malformed user data untouched. Be precise about post-rename failures: the replacement may already be visible even when durability reporting fails.
- Respect Linux `XDG_DATA_HOME` (or `~/.local/share`), macOS `~/Library/Application Support`, and Windows `%APPDATA%`. Do not assume Unix-only filesystem or terminal behavior.

### Configuration

- `run()` reads and validates the configuration once with `Config::load`, before taking the instance lock, loading tasks, or setting up the terminal, then applies `CliOptions::resolve` (CLI > TOML > defaults) to the `App`. Any configuration error is fatal and reported on stderr with a nonzero exit; `--help`, `--version`, and `export` return before configuration is read.
- sati does not watch or reload its configuration: edits take effect on the next start. Do not add file watching, polling, or an in-app configuration error display.
- Runtime `t` / `T` theme cycling is session-only and never writes configuration.

### Input, rendering, and terminal lifecycle

- Modes are `Normal`, `Adding`, `Editing`, `Searching`, `RenamingSection`, `FuzzySection`, and `ConfirmClear`. Keep shortcuts mode-specific; accidental key repeats and key releases must not trigger destructive actions. `repeat_allowed` is an allowlist: navigation (`j`/`k`/`h`/`l`, arrows, Page Up/Down, `Tab`, `Alt+B`/`Alt+F`, stepping through matches or suggestions) and text editing repeat; anything that mutates tasks or section order, confirms, or cancels fires once per press.
- Text editing is deliberately **single-line**. Respect Unicode grapheme boundaries when moving, inserting, deleting, pasting, wrapping, and positioning the cursor. Use terminal-cell width, not byte or character count, for layout.
- Task wrapping affects visual-row scrolling, Page Up/Down, mouse hit mapping, and selection visibility. Account for narrow/short windows and tasks taller than the viewport.
- There is no persistent on-screen shortcut legend. Keep contextual hints (such as search's `Tab to cycle` and the section-jump digits) compact, and **do not advertise Page Up/Down on screen**; those are intentionally unlisted convenience shortcuts.
- Preserve raw-mode, alternate-screen, keyboard enhancement, paste, and mouse setup/cleanup. The terminal should be restored on normal exit, errors, and panic paths; unsupported optional features should degrade gracefully.
- Respect the existing CLI flags and subdued styling. The `default` theme inherits terminal foreground/background colors; the 15 named themes supply RGB foregrounds and backgrounds. TOML can override foregrounds, backgrounds, and modifiers for `normal`, `muted`, `completed`, `border`, and `accent`; `--accent` overrides only the resolved accent foreground.
- Apply the resolved normal background through Ratatui cells and, for RGB colors, OSC 11 (including terminal padding). Use OSC 111 to restore the terminal's configured default when Sati changed it. Runtime `t` / `T` theme cycling is session-only, preserves TOML style and CLI accent overrides, and is reset on restart.

## Code style and readability

Write code a human can read top to bottom without decoding it. Readability matters more than saving lines. These rules apply to code you write or change; do not reformat or restructure untouched code just to apply them.

`rustfmt` keeps your line breaks and indentation consistent, but it never adds blank lines. Spacing between ideas is your job.

### Separate ideas with blank lines

- Put a blank line between conceptually different chunks inside a function. A typical mutation has distinct steps: check preconditions, build the new state, persist it, commit it, update view state (selection, scroll, notifications), then return. Each step is a paragraph.
- Never leave a long function as one unbroken block. As a rule of thumb, if roughly 10 to 15 consecutive lines have no blank line, look for the seam where one step ends and the next begins.
- Put a blank line before the final `Ok(...)` or return value, and around multi-line `if`, `match`, `for`, and `while` blocks, unless the block is the only thing in its parent.
- Keep tightly related lines together. Do not insert a blank line between every statement; that is as unreadable as none.
- Always separate top-level items (`fn`, `impl`, `struct`, `enum`, `const`) with exactly one blank line.

Shape to aim for (this is `App::toggle_carryover` with its steps separated):

```rust
fn toggle_carryover(&mut self, storage: &Storage) -> Result<()> {
    let Some(index) = self.list.selected() else {
        return Ok(());
    };

    let (status, label) = match self.tasks[index].status {
        TaskStatus::Pending => (TaskStatus::Carried, "mark carryover"),
        TaskStatus::Carried => (TaskStatus::Pending, "unmark carryover"),
        TaskStatus::Completed => return Ok(()),
    };

    let mut tasks = self.tasks.clone();
    tasks[index].status = status;

    self.replace(tasks, Some(index), label, storage)
}
```

### Keep functions and expressions easy to follow

- Long functions are fine. Do not split a function to make it shorter, to stay under a line count, or because it has several steps. Keep sequential steps together in one function, separated by blank lines and, where useful, short "why" comments, so the reader can follow the whole flow in one place without jumping around the file.
- Extract a helper only to remove notable duplication. Weigh the size of the repeated chunk against how many times it repeats. Two copies of a three-line snippet are fine as they are. A ten-line chunk repeated a dozen times should almost certainly be a helper. Use judgment between those extremes, and when unsure, leave the code inline.
- Do not extract a helper for code that appears only once, to give a step a name, or to reduce nesting or length.
- When you do extract a helper to remove duplication, name it so the call sites read clearly without opening it, keep its parameters few, and place it near its first caller. Use a plain private function, not new traits, generics, builders, macros, or modules.
- Existing long functions (for example `handle_normal_key` and `run`) are not a problem. Do not refactor or split them unless asked. Match the surrounding structure when extending them; for example, a new `match` arm should delegate to a named method if its neighbors do.
- Give intermediate results a name with `let` instead of nesting calls or writing long one-line chains. Break iterator chains one adapter per line.
- Use early returns, guard clauses, and `let ... else` to keep nesting shallow, ideally no more than three levels. Avoid deeply nested closures and `if`/`else` expressions embedded in arguments.
- Use descriptive names (`selected_index`, not `si`). Single-letter names are fine only for tiny, obvious scopes such as a closure parameter or `r, g, b`.
- Replace unexplained numbers and strings with named constants, as `HISTORY_LIMIT` does.
- Prefer straightforward, idiomatic Rust over clever one-liners. Do not use `#[rustfmt::skip]` or shrink code to fit fewer lines.

### Comments

- Comment why, not what. Explain invariants, platform quirks, and ordering constraints that the code cannot say itself, in the style of the existing comments in `src/storage.rs` ("Never truncate the live file...").
- Put a short comment above a non-obvious block rather than at the end of a line. Do not narrate obvious code, and do not leave commented-out code.

### Tests

- Name tests as full sentences describing the behavior (for example `lock_stays_exclusive_across_saves_and_is_released_without_removing_file`).
- Separate arrange, act, and assert with blank lines. Reuse the existing test helpers (such as `Fixture` in `src/storage.rs` tests and `press` in `src/app.rs` tests) instead of repeating setup.
- Keep one behavior per test, and keep test code as readable as production code.

### Before you finish

Reread your diff as a reviewer would. If any hunk looks like a wall of text, add blank lines or a short comment, or rename for clarity. Run `cargo fmt --all` last so the result is rustfmt-clean.

## Development conventions

- Follow `.editorconfig` and `rustfmt.toml`: UTF-8, LF, four-space Rust indentation, Rust 2024 formatting, 100-column width.
- Reuse existing helpers and types instead of introducing frameworks or additional dependencies for small changes.
- Clippy runs with `-D warnings` and `Cargo.toml` warns on `dbg!`, `todo!`, and `unimplemented!`. Do not leave any of them in code.
- Keep error reporting actionable, especially for failures involving saved user data.
- When behavior or options change, update the relevant README documentation and on-screen hints, subject to the deliberate Page Up/Down omission above.
- Keep `Cargo.lock` checked in; review dependency changes for MSRV and Linux/macOS/Windows compatibility.
- Do not automatically commit or push. If asked to commit, use a short Conventional Commit subject (for example, `fix: ...`, `feat: ...`, or `docs: ...`).
- Never add watermarks, attribution, or trailers such as `Co-Authored-By:` or "Generated with ..." to commit messages, pull requests, or code.

## Verification commands

Run the applicable repository checks after code changes. Use `cargo fmt --all` to apply formatting; CI runs the `--check` form.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --all-targets
cargo test --locked
cargo build --release --locked
```

Run a subset of tests while iterating:

```sh
cargo test --locked --bin sati app::tests::name_substring  # unit tests in src/ (no lib target)
cargo test --locked --test locking                         # one integration test file
cargo test --locked -- --exact path::to::test_name         # one exact test
```

When possible, also check the declared MSRV:

```sh
cargo +1.88.0 check --locked --all-targets
```

`.github/workflows/ci.yml` checks Linux, macOS, and Windows; strict Clippy runs on all three, while formatting and Rust 1.88 checks run on Ubuntu. Report commands you could not run, rather than claiming they passed.

## Scope discipline

For a requested change, inspect the involved code paths, implement the smallest coherent fix, review the diff for unrelated changes, and summarize any tradeoffs or platform limitations. If asked for a read-only audit, do not edit files or commit. Do not turn small correctness fixes into broad rewrites.
