# Contributing to sati

Thanks for your interest in sati. It is meant to stay a small, dependable checklist rather than grow into a task-management platform, so its scope is deliberately limited: due dates, tags, priorities, reminders, projects, and synchronization are out of scope.

Contributions that are especially welcome:

- Bug fixes.
- Portability improvements for Linux, macOS, Windows, and different terminals.
- Refinements that make existing features easier or more pleasant to use.
- Documentation corrections.

## Before you start

For a significant feature or an architectural change, please open an issue to discuss it before writing code. That way we can agree on whether it fits sati before you spend time on it. Small fixes can go straight to a pull request.

## Building and checking

sati needs Rust 1.89 or newer. Build and run it with Cargo, using a scratch data directory so experiments don't touch your real checklist:

```sh
cargo build
cargo run -- --data-dir target/sati-dev
```

Before opening a pull request, run the same checks as CI:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The tests use their own temporary directories. CI also builds release binaries on Linux, macOS, and Windows, and checks Rust 1.89 compatibility on Linux.

## Pull requests

Keep each pull request small and focused on one change. In its description, explain what changed and why, how you tested it, and any change in user-visible behavior. If the change affects documented behavior or a shortcut, update `docs/` or the in-app `?` reference to match.

[AGENTS.md](AGENTS.md) describes sati's architecture, invariants, and engineering conventions in more detail, and is worth reading before a larger change.
