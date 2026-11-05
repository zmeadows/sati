# Agent instructions for sati

## Project intent

`sati` is a small, persistent terminal checklist written in Rust. It aims to feel like a paper checklist: add tasks, cross them off, reorder them, and clear the list when the context changes. Simplicity is a product requirement, not an unfinished feature set.

- Favor clear, small changes over new abstractions, dependencies, settings, or features.
- Keep the interface quiet and keyboard-friendly. Do not add calendars, tags, priorities, reminders, projects, synchronization, or configurable keybindings unless explicitly requested.
- Preserve existing behavior and cross-platform support. Avoid unrelated cleanup while implementing a focused change.
- Read the current code before editing; this file is orientation, not a replacement for the implementation.

## Stack and source layout

- Rust 2024 edition; minimum supported Rust version (MSRV) **1.85** (`Cargo.toml`).
- Ratatui **0.29** with its Crossterm backend. Import Crossterm via **`ratatui::crossterm`**, not an independently versioned direct dependency.
- Serde/JSON for persisted tasks; `fs4` for advisory file locks; `unicode-segmentation` and `unicode-width` for text handling.
- `src/main.rs`: CLI arguments, startup, terminal lifecycle, event loop, and platform-specific terminal capabilities.
- `src/app.rs`: `Task`, application modes, key/mouse dispatch, task mutations, selection, scrolling, and bounded undo history.
- `src/input.rs`: single-line, grapheme-aware `TextInput`; cursor is a UTF-8 byte offset kept at a grapheme boundary.
- `src/ui.rs`: layout, wrapping, drawing, context-sensitive shortcut legends, mouse row mapping, and input cursor placement.
- `src/storage.rs`: platform data paths, exclusive instance lock, JSON loading/validation, and atomic replacement saves.
- `tests/`: existing CLI, data-directory, and cross-process locking checks.

## Behavioral invariants

### Tasks and app state

- A task has only `text` and `completed`; list order is meaningful. Do not add a persistent schema field casually.
- Task text is a nonempty, trimmed, single-line description. Preserve arbitrary valid Unicode; reject corrupt persisted data rather than silently repairing or overwriting it.
- The normal list selection refers to a **task**, not a wrapped display row. Keep indices valid after deletion, reordering, clearing, and undo.
- `--add-to-bottom` changes insertion location; `--completed-at-bottom` groups completed items below incomplete ones. Keep behavior, navigation, and undo coherent under both options.
- Undo holds at most **10 in-memory snapshots** of tasks and selection. Only successful mutations enter history; undo does not create a new history entry. Navigation, cancellation, and no-ops do not create entries.

### Storage and failure handling

- Acquire `tasks.lock` before loading and hold the lock for the application's lifetime. Do **not** delete the lock file: all instances must coordinate on the same file.
- Persist each task mutation **before** committing the new in-memory state or consuming undo history. A failed save must not become an accepted mutation.
- Save JSON via a uniquely created temporary file in the same directory, sync it, then replace `tasks.json` by rename. Never truncate/delete the live file first or silently fall back to a non-atomic copy.
- Preserve meaningful errors and leave malformed user data untouched. Be precise about post-rename failures: the replacement may already be visible even when durability reporting fails.
- Respect Linux `XDG_DATA_HOME` (or `~/.local/share`), macOS `~/Library/Application Support`, and Windows `%APPDATA%`. Do not assume Unix-only filesystem or terminal behavior.

### Input, rendering, and terminal lifecycle

- Modes are `Normal`, `Adding`, `Editing`, and `ConfirmClear`. Keep shortcuts mode-specific; accidental key repeats and key releases must not trigger destructive actions.
- Text editing is deliberately **single-line**. Respect Unicode grapheme boundaries when moving, inserting, deleting, pasting, wrapping, and positioning the cursor. Use terminal-cell width, not byte or character count, for layout.
- Task wrapping affects visual-row scrolling, Page Up/Down, mouse hit mapping, and selection visibility. Account for narrow/short windows and tasks taller than the viewport.
- Keep footer hints contextual and compact. **Do not advertise Page Up/Down in the bottom shortcut legend**; those are intentionally unlisted convenience shortcuts.
- Preserve raw-mode, alternate-screen, keyboard enhancement, paste, and mouse setup/cleanup. The terminal should be restored on normal exit, errors, and panic paths; unsupported optional features should degrade gracefully.
- Respect the existing CLI flags and subdued Gruvbox Material look. The terminal provides the background; `--accent` overrides the selection accent.

## Development conventions

- Follow `.editorconfig` and `rustfmt.toml`: UTF-8, LF, four-space Rust indentation, Rust 2024 formatting, 100-column width.
- Prefer idiomatic, straightforward Rust. Reuse existing helpers and types instead of introducing frameworks or additional dependencies for small changes.
- Keep error reporting actionable, especially for failures involving saved user data.
- When behavior or options change, update the relevant README documentation and on-screen hints, subject to the deliberate Page Up/Down omission above.
- Keep `Cargo.lock` checked in; review dependency changes for MSRV and Linux/macOS/Windows compatibility.
- Do not automatically commit or push. If asked to commit, use a short Conventional Commit subject (for example, `fix: ...`, `feat: ...`, or `docs: ...`).

## Existing verification commands

Run the applicable repository checks after code changes:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --all-targets
cargo test --locked
cargo build --release --locked
```

When possible, also check the declared MSRV:

```sh
cargo +1.85.0 check --locked --all-targets
```

`.github/workflows/ci.yml` checks Linux, macOS, and Windows; formatting and Clippy run on Ubuntu, and the Ubuntu job also checks Rust 1.85. Report commands you could not run, rather than claiming they passed.

## Scope discipline

For a requested change, inspect the involved code paths, implement the smallest coherent fix, review the diff for unrelated changes, and summarize any tradeoffs or platform limitations. If asked for a read-only audit, do not edit files or commit. Do not turn small correctness fixes into broad rewrites.
