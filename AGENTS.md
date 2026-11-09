# Agent instructions for sati

## Purpose and priorities

`sati` is a small, persistent terminal checklist written in Rust. It should feel like a paper checklist: add tasks, cross them off, reorder them, and clear the list when the context changes. Simplicity is an intentional product requirement, not a missing feature set.

Prioritize, in order:

1. Correctness and preservation of user data.
2. Code that is easy to understand, debug, and change.
3. A quiet, responsive, keyboard-friendly interface.
4. Small changes with little ongoing maintenance cost.

Do not introduce calendars, tags, priorities, reminders, projects, synchronization, configurable keybindings, or other product features unless requested. Sections are an intentional part of the existing design. Preserve existing behavior and Linux, macOS, and Windows support unless the task explicitly changes them.

Read the relevant implementation and documentation before editing. This file describes durable constraints and engineering standards; it is not an exhaustive behavior specification. `README.md` documents the user-facing interface, while the source is authoritative for implementation details.

## Code organization

The project uses Rust 2024 with MSRV **1.89**. Consult `Cargo.toml` for dependencies and enabled features; do not assume a new dependency is needed. Import Crossterm through `ratatui::crossterm`, not a separately versioned direct dependency.

- `src/main.rs` — Clap CLI and subcommands, configuration startup order, terminal setup/cleanup, event loop, and the crate-wide `Result` alias.
- `src/model.rs` — `Task`, `TaskStatus`, `SectionView`, task validation and timestamps, task-input parsing, tab ordering, active-view filtering, and section-order/active-view reconciliation.
- `src/app.rs` — modes, application state, input dispatch, task mutations, navigation, undo/redo, and the `ViewportState` that drawing records for input.
- `src/ui.rs` — layout, drawing, wrapped rows, mouse mapping, and the `?` keyboard reference (`HELP_GROUPS`).
- `src/storage.rs` — platform data paths, instance locking, persistent schema, validation, migration, and atomic saving.
- `src/input.rs` — grapheme-aware single-line editing.
- `src/config.rs` — strict TOML configuration and discovery.
- `src/styles.rs`, `src/themes.rs` — style resolution, the session's theme state (`Appearance`), and built-in themes.
- `src/search.rs` — task search and highlight ranges.
- `src/export.rs` — Markdown output and clipboard content.
- `src/status_message.rs` — timed, in-terminal feedback.
- `tests/` — binary-level integration checks; module-local checks live in `#[cfg(test)]` sections, or in `src/<module>/tests.rs` for the larger modules.

Prefer keeping work within an existing module when its responsibility naturally owns the behavior. Create a module only when it represents a substantial, coherent responsibility—not simply because a file is long.

## Engineering principles

### Make the work visible

Write straightforward Rust that a reader can follow in execution order. Prefer concrete types, ordinary functions, `if`, `match`, and loops when they communicate the operation more directly than elaborate iterator chains, closures, or generic machinery. Use concise standard-library expressions when those are clearer; procedural code is a preference, not a prohibition against idiomatic Rust.

Keep related operations close together. Avoid forcing readers through layers of delegation to discover what a command actually does. A reasonably long function with a coherent sequence of steps is often better than a collection of trivial functions. Conversely, length is not a reason to keep unrelated responsibilities together.

Use early returns and `let ... else` when they clarify preconditions. Name important intermediate values and prefer descriptive identifiers over dense, nested expressions. Match on meaningful enums rather than recreating their state with booleans.

### Make abstractions earn their cost

Start with a direct implementation. Extract a function when it removes meaningful duplication, isolates a distinct responsibility or invariant, makes side effects explicit, or materially clarifies its caller. A helper may be worthwhile even with one caller, but not merely to name a trivial step or satisfy a line-count preference.

Do not introduce traits, generics, builders, macros, registries, callback systems, or abstraction layers for hypothetical future needs. Use them when a concrete problem justifies them. Some small duplication is preferable to an abstraction that obscures important differences. Judge a refactoring by the **total** reduction in concepts and effort needed to understand the affected code, not by lines removed.

Prefer the project's existing direct design over a new framework. Do not reflexively generalize a special case; first establish that there is a stable common operation to express.

### Keep state and effects understandable

Make authoritative data obvious. Derive temporary values when practical instead of keeping multiple independently mutable representations synchronized. If caching or duplicated state is warranted, make the invariant and update points clear.

Make writes, mutation, fallible operations, and expensive work apparent at their call sites. Avoid APIs that appear to be simple queries but secretly persist data or mutate unrelated state. Keep the ordering of state transitions explicit, particularly when persistence or undo history is involved.

Use Rust's ownership, enums, and `Result` to express actual constraints. Do not add defensive layers for impossible internal states, but keep validation and error handling at real boundaries: user input, JSON/TOML, the filesystem, the terminal, and platform-specific APIs. Simplification must not weaken reliability.

### Optimize for readers, not a style checklist

Use whitespace to mark meaningful phases of an operation—validation, computation, persistence, state commit, and UI feedback, for example. Keep closely related statements together. Do not follow mechanical blank-line counts, split every expression, or force every function into a prescribed shape. Follow `.editorconfig` and `rustfmt.toml` (UTF-8, LF, four-space Rust indentation, 100-column formatting).

Format genuinely table-like code—static tables, lookup definitions, palettes, and other sequences of structurally similar entries—with exactly one entry per line so rows can be scanned and compared; long lines are acceptable there. Preserve that layout with `#[rustfmt::skip]` on the smallest enclosing item or `let` statement that supports it (attributes on `match` or `for` expressions do not compile on stable), never on unrelated code or whole modules, and leave `rustfmt.toml` alone. Do not flatten procedural code or expressions whose readability depends on line breaks; rustfmt governs everything else.

Comments should explain constraints, invariants, surprising behavior, and ordering requirements—especially the reason behind a non-obvious choice. Do not narrate obvious statements or leave commented-out alternatives. Prefer a readable implementation over a long explanatory comment compensating for opaque code.

Do not introduce unexplained magic values when a named constant would clarify a real meaning. Do not create constants for obvious local literals simply to eliminate literals.

### Improve code without manufacturing churn

Make the smallest **coherent** change, not necessarily the smallest textual diff. A nearby simplification is appropriate when it directly makes the requested change safer or easier to follow. Avoid speculative rewrites, wholesale formatting, or unrelated cleanup. Do not preserve a poor local structure merely because it already exists, but do not turn a focused fix into an architectural project.

Before finishing, read the changed path as a maintainer unfamiliar with it: Can they follow the operation? Identify the source of truth? See the side effects and failure paths? Does each helper or state field justify itself? If a simpler representation is genuinely clearer and equally dependable, prefer it.

## Behavior and reliability contracts

The following are architectural invariants, not a complete shortcut specification. Check the README, current handlers, and the relevant help text before changing interactive behavior.

### Tasks, sections, and selection

- `Task` has a stable UUID `id`, whole-second UTC `created_at`, optional authoritative `section`, body `text`, and one `TaskStatus` (`Pending`, `Completed`, `Carried`). List order matters. Edits and moves retain identity; pasted tasks receive new identities and start pending. The single status enum prevents a task from being both completed and carried.
- Task bodies are nonempty, trimmed, single-line Unicode. Do not silently repair malformed persisted tasks. Use `Task::new` for task input (it splits a leading `Section: ` with `parse_task_input`) and `full_text()` where the full `Section: body` text is needed. A stored task body must not be re-parsed to infer its section.
- Sections are stored in task fields and compared case-insensitively. Stored section names are single words without spaces, colons, or control characters. `App::section_order` is the authoritative tab ordering; it contains each used named section once. A rename preserves tab position, and reorder operations change tab order without reordering tasks.
- `App::view` is the active view: a filter for the current tab, not what is on screen. Selection remains a task index into the full `App::tasks` list that points to a task in the active view (`task_in_active_view`), not a visual row; `ViewportState::task_index_by_row` maps the rendered rows back to task indices. Keep selection, scrolling, and the active view valid after mutations, clearing, renaming, undo/redo, and resizing. The all-tasks view must retain the behavior of a checklist without sections.
- Section-picker previews and search/navigation state are transient. Canceling restores the appropriate checkpoint; previewing must not write `tasks.json` or enter undo history. Section order and active view are reconciled with the proposed tasks (`reconcile_section_order`, `reconcile_active_view`) **before** saving so the persisted view matches the committed one.

### Mutations and history

- Build a proposed new state, **persist it first**, then accept it in memory and update undo/redo history. Never present a failed save as a successful mutation. Use the existing `commit_task_change` / `commit_checklist_change` path unless there is a concrete reason to modify it.
- Undo/redo use bounded (256-entry), session-only snapshots of tasks, section order, selection, and view. Successful mutations produce coherent undo steps and clear redo; navigation, cancellation, and no-ops do not. Undo/redo also persist their result before committing it.
- Maintain the existing distinction between routine status messages (`show_routine_message`) and important errors/validation feedback (`show_important_message`). A disabled routine-status-message option must not hide meaningful errors.

### Durable storage

- `tasks.json` is the authoritative document (`ChecklistDocument`); its current `schema_version` is **1**. The schema includes tasks, selection, the active view (`active_view`, distinct from each task's `section`), and section order. Version 1 is the first public format and the oldest supported; there are no migrations yet. Reject malformed documents, duplicate IDs, invalid statuses or sections, incorrect section order, unknown schema versions, and unsupported old versions without modifying the file.
- Loading validates strictly and does not write. For future schema changes, bump the version and append an explicit, sequential migration to `MIGRATIONS` from a strictly parsed typed previous schema; do not overwrite or patch unknown raw JSON fields. Never edit or reorder a released migration. Each new migration needs a direct test of the step and a `load_checklist` round-trip test of a real old-version file (see the checklist above `MIGRATIONS`). A migrated document is saved on the next save or clean exit, not during loading.
- Hold the exclusive `tasks.lock` for the interactive process lifetime. Never delete the lock file: different processes must coordinate on the same pathname. Each selected data directory is an independent checklist.
- Save using `atomic-write-file`; never hand-roll replacement, truncate the live file first, or fall back to non-atomic copying. Preserve the Unix `0600` save-file policy and private permissions when creating the data directory. Report save errors accurately: failure *after* rename may mean the replacement is already visible, even if `commit()` reported an error.
- Respect platform-specific data paths and an explicitly supplied `--data-dir`; do not assume Unix-only behavior. `export` is read-only, does not take the interactive instance lock, and does not create a data directory or write files.

### CLI and configuration

- Preserve Clap's existing command/option structure, global `--data-dir`, and command-line error behavior. Keep interactive-only options separate from subcommands. `--help`, `--version`, and `export` return without loading interactive configuration.
- Load and validate configuration once, before storage or terminal initialization. Configuration errors are fatal and actionable. Effective option precedence is **CLI > TOML > defaults**. Missing default configuration is allowed; an explicitly named config file must exist. Unknown keys are errors. Do not introduce live reload or automatic config writing.
- Runtime theme changes are session-only and go through `Appearance::set_theme`, which re-resolves the effective styles; do not store theme inputs or resolved styles elsewhere. Preserve existing style override precedence and terminal-default theme semantics.

### Input, rendering, and terminal operation

- Keep shortcuts mode-specific and preserve both Vim-like and Emacs-like aliases. `normalize_key` and `repeat_allowed` define shared dispatch and repeat behavior: navigation/text editing may repeat where intended; mutations, confirmations, cancellation, and reordering must not accidentally run on key repeats or releases. Check all aliases and `HELP_GROUPS` when changing a binding.
- Input is deliberately single-line and grapheme-aware. Cursor movement, editing, wrapping, search highlighting, and mouse hit mapping must handle arbitrary Unicode correctly. Layout uses **terminal-cell width**, not byte count or Unicode scalar count. Handle small viewports and tasks taller than a viewport.
- `?` opens a read-only keyboard reference. Keep `HELP_GROUPS` aligned with actual shortcuts, but do not turn the help list into a dispatch registry. The normal screen has no persistent shortcut legend beyond its optional compact help hint. Page Up/Down and their aliases are intentionally omitted from on-screen shortcut descriptions.
- Rendering may update transient viewport and hit-mapping state needed for subsequent input. Keep those relationships explicit; do not require purely functional rendering at the cost of substantially more machinery.
- All actions must remain keyboard-accessible. Mouse capture follows the resolved option, and no mouse enable/disable sequence should be sent when capture is off. Preserve raw-mode, alternate-screen, keyboard enhancement, bracketed-paste, background-color, and terminal restoration behavior on normal exit, errors, and panic. Optional terminal capabilities should degrade gracefully.
- Preserve the subdued visual design, resolved theme overrides, and OSC 11/111 background application/restoration behavior when touching terminal colors. Use the existing shared enums and `ui::border_type` for style options instead of creating parallel representations.

## Working process

1. Inspect the affected code and its callers before choosing an approach. Determine which data is authoritative, which invariants constrain the change, and what happens when an operation fails.
2. Implement the simplest dependable design that meets the actual request. Reuse existing types and helpers where they help; avoid new dependencies and abstractions without demonstrated value.
3. Review all affected paths, including alternative keybindings, failure handling, persistence, and platform differences where relevant. Update `README.md`, contextual hints, and `HELP_GROUPS` when public behavior changes, respecting deliberate omissions from on-screen help.
4. Review the diff for accidental behavior changes, unnecessary state, new indirection, confusing naming, and unrelated churn. Report meaningful tradeoffs and any checks that could not be run.

Run applicable existing repository checks after changing code (use `cargo fmt --all` to format):

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --all-targets
cargo test --locked
cargo build --release --locked
```

When available, also check the supported Rust baseline:

```sh
cargo +1.89.0 check --locked --all-targets
```

CI covers Linux, macOS, and Windows. Binary-level integration checks can interact with terminal setup: never direct them at the user's actual data directory, and preserve the existing isolation/early-exit conventions.

Keep `Cargo.lock` checked in. Any dependency change must justify its ongoing complexity and consider maintenance quality, MSRV, and cross-platform behavior. Do not leave `dbg!`, `todo!`, or `unimplemented!` in committed code.

Do not commit or push unless asked. If asked to commit, use a short Conventional Commit subject. Do not add generated-by attribution, watermarks, or `Co-Authored-By` trailers. For a read-only audit, do not modify files.
