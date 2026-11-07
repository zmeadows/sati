# Agent instructions for sati

## Project intent

`sati` is a small, persistent terminal checklist written in Rust. It aims to feel like a paper checklist: add tasks, cross them off, reorder them, and clear the list when the context changes. Simplicity is a product requirement, not an unfinished feature set.

- Favor clear, small changes over new abstractions, dependencies, settings, or features.
- Keep the interface quiet and keyboard-friendly. Do not add calendars, tags, priorities, reminders, projects, synchronization, or configurable keybindings unless explicitly requested.
- Preserve existing behavior and cross-platform support. Avoid unrelated cleanup while implementing a focused change.
- Read the current code before editing; this file is orientation, not a replacement for the implementation.

## Stack and source layout

- Rust 2024 edition; minimum supported Rust version (MSRV) **1.88** (`Cargo.toml`).
- Ratatui **0.30.2** with its Crossterm backend (default features off; see `Cargo.toml` for the enabled set). Import Crossterm via **`ratatui::crossterm`**, not an independently versioned direct dependency.
- Serde/JSON for persisted tasks; `toml` for optional configuration; `fs4` for advisory file locks; `unicode-segmentation` and `unicode-width` for text handling.
- `src/main.rs`: CLI arguments (hand-parsed, no CLI framework), `export` subcommand dispatch, startup, terminal lifecycle, event loop, and platform-specific terminal capabilities.
- `src/app.rs`: `Task`, application modes, key/mouse dispatch, task mutations, selection, scrolling, and bounded undo history.
- `src/input.rs`: single-line, grapheme-aware `TextInput`; cursor is a UTF-8 byte offset kept at a grapheme boundary.
- `src/ui.rs`: layout, wrapping, drawing, context-sensitive shortcut legends, mouse row mapping, and input cursor placement.
- `src/storage.rs`: platform data paths, exclusive instance lock, JSON loading/validation, and atomic replacement saves.
- `src/config.rs`: TOML config discovery (`$XDG_CONFIG_HOME`/`~/.config`, macOS Application Support, `%APPDATA%`) and parsing. Unknown keys and invalid files are errors; sati never writes config files. Precedence is CLI > TOML > defaults; contradictory CLI flag pairs are rejected.
- `src/themes.rs` / `src/styles.rs`: built-in theme palettes and the resolved `Styles` (theme + TOML overrides + `--accent`).
- `src/search.rs`: search matching and highlight ranges for `Searching` / `SearchResults`.
- `src/export.rs`: Markdown rendering shared by `sati export [-o PATH] [--force]` and the `c` OSC 52 clipboard copy. Export runs without taking the instance lock (it works while the TUI is open) and refuses to overwrite sati's own state files or existing files without `--force`.
- `src/notification.rs`: timed footer notifications (e.g. theme name after `t` / `T`).
- Unit tests live in `#[cfg(test)] mod tests` inside each module; `tests/` holds integration tests that run the binary (`cli`, `config`, `directories`, `export`, `locking`). Integration tests isolate data/config by overriding `HOME`, `XDG_DATA_HOME`, and `APPDATA`; never point tests at the real user data directory.

## Behavioral invariants

### Tasks and app state

- A task has `text`, `completed`, and optional `carried` (default false); list order is meaningful. Completed tasks cannot be carried. `m` toggles carryover on unfinished tasks; `D` preserves carried tasks.
- Task text is a nonempty, trimmed, single-line description. Preserve arbitrary valid Unicode; reject corrupt persisted data rather than silently repairing or overwriting it.
- The normal list selection refers to a **task**, not a wrapped display row. Keep indices valid after deletion, reordering, clearing, and undo.
- `tasks.json` stores `{tasks, selected}`. Temporarily accept legacy arrays and migrate on the next save or clean exit. Save resulting selection with mutations and undo/redo; navigation-only focus is saved on clean exit, without persisting temporary search selection. Remap focus by original index during startup grouping.
- `a` appends, `o` inserts below selection, and `O` inserts above selection. With `--completed-at-bottom`, new tasks stay in the incomplete section. Keep insertion, navigation, and undo coherent with this grouping.
- Undo and redo each hold at most **256 in-memory snapshots** of tasks and selection. Only successful mutations enter ordinary history and clear redo. Undo/redo transfer snapshots between stacks after saving. Navigation, cancellation, and no-ops leave both histories unchanged.
- Undo/redo history is session-only. Never read, write, or delete legacy `history.json` files. Undo/redo still save their resulting task lists to authoritative `tasks.json` before transferring snapshots.

### Storage and failure handling

- Acquire `tasks.lock` before loading and hold the lock for the application's lifetime. Do **not** delete the lock file: all instances must coordinate on the same file.
- Persist each task mutation **before** committing the new in-memory state or consuming undo history. A failed save must not become an accepted mutation.
- Save JSON via a uniquely created temporary file in the same directory, sync it, then replace `tasks.json` by rename. Never truncate/delete the live file first or silently fall back to a non-atomic copy.
- Preserve meaningful errors and leave malformed user data untouched. Be precise about post-rename failures: the replacement may already be visible even when durability reporting fails.
- Respect Linux `XDG_DATA_HOME` (or `~/.local/share`), macOS `~/Library/Application Support`, and Windows `%APPDATA%`. Do not assume Unix-only filesystem or terminal behavior.

### Input, rendering, and terminal lifecycle

- Modes are `Normal`, `Adding`, `Editing`, `Searching`, `SearchResults`, and `ConfirmClear`. Keep shortcuts mode-specific; accidental key repeats and key releases must not trigger destructive actions.
- Text editing is deliberately **single-line**. Respect Unicode grapheme boundaries when moving, inserting, deleting, pasting, wrapping, and positioning the cursor. Use terminal-cell width, not byte or character count, for layout.
- Task wrapping affects visual-row scrolling, Page Up/Down, mouse hit mapping, and selection visibility. Account for narrow/short windows and tasks taller than the viewport.
- Keep footer hints contextual and compact. **Do not advertise Page Up/Down in the bottom shortcut legend**; those are intentionally unlisted convenience shortcuts.
- Preserve raw-mode, alternate-screen, keyboard enhancement, paste, and mouse setup/cleanup. The terminal should be restored on normal exit, errors, and panic paths; unsupported optional features should degrade gracefully.
- Respect the existing CLI flags and subdued styling. The `default` theme inherits terminal foreground/background colors; the 15 named themes supply RGB foregrounds and backgrounds. TOML can override foregrounds, backgrounds, and modifiers for `normal`, `muted`, `completed`, `border`, and `accent`; `--accent` overrides only the resolved accent foreground.
- Apply the resolved normal background through Ratatui cells and, for RGB colors, OSC 11 (including terminal padding). Use OSC 111 to restore the terminal's configured default when Sati changed it. Runtime `t` / `T` theme cycling is session-only and preserves TOML style and CLI accent overrides.

## Code style and readability

Write code a human can read top to bottom without decoding it. Readability matters more than saving lines. These rules apply to code you write or change; do not reformat or restructure untouched code just to apply them.

`rustfmt` keeps your line breaks and indentation consistent, but it never adds blank lines. Spacing between ideas is your job.

### Separate ideas with blank lines

- Put a blank line between conceptually different chunks inside a function. A typical mutation has distinct steps: check preconditions, build the new state, persist it, commit it, update view state (selection, scroll, notifications), then return. Each step is a paragraph.
- Never leave a long function as one unbroken block. As a rule of thumb, if roughly 10 to 15 consecutive lines have no blank line, look for the seam where one step ends and the next begins.
- Put a blank line before the final `Ok(...)` or return value, and around multi-line `if`, `match`, `for`, and `while` blocks, unless the block is the only thing in its parent.
- Keep tightly related lines together. Do not insert a blank line between every statement; that is as unreadable as none.
- Always separate top-level items (`fn`, `impl`, `struct`, `enum`, `const`) with exactly one blank line.

Shape to aim for (this is `App::toggle` with its steps separated):

```rust
fn toggle(&mut self, index: usize, storage: &Storage) -> Result<()> {
    let mut tasks = self.tasks.clone();
    tasks[index].completed = !tasks[index].completed;
    tasks[index].carried = false;

    let selected = if self.completed_at_bottom {
        let task = tasks.remove(index);
        let boundary = completion_boundary(&tasks);
        tasks.insert(boundary, task);
        boundary
    } else {
        index
    };

    self.replace(tasks, Some(selected), storage)?;
    self.reveal_selection = selected != index;

    Ok(())
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
