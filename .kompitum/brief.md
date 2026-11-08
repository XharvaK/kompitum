This is `kompitum-core` (command `kom`), a Rust terminal runtime forked from herdr-core. Read `AGENTS.md` first: its rules and traps bind you. Then read what the task names, not the whole crate.

Your shell is Git Bash on Windows. `cmd /c` gets mangled there, and a bare name never finds a `.cmd` file.

Build and test with plain `cargo`, always with `--locked`. A config above this folder sets the build folder, `ZIG` and Ghostty's source, so `vendor/ghostty` stays empty here and you never run `git submodule`. Rebuilding the crate takes a minute or two.

- One Windows test: `cargo test --locked -p kompitum-core --test windows NAME`
- Unit tests: `cargo test --locked -p kompitum-core --lib NAME`
- Everything the server checks: `python scripts/check_windows.py`. About a minute once you have built the crate. Run it once, when the work is done.
- Never run `registry_changes_after_start_are_seen_by_each_new_terminal`: it writes to the real user environment.
- After a change to params in `api/types.rs`, `KOMPITUM_UPDATE_SCHEMA=1 cargo test --locked -p kompitum-core --test schema_file` regenerates the schema, and `KOMPITUM_UPDATE_DOCS=1 cargo test --locked -p kompitum-core --test cli_docs` regenerates `docs/commands.md`.

On Windows nearly every file in `crates/kompitum-core/tests/` is `#![cfg(unix)]` and compiles to nothing. A test of Windows behaviour goes in a file under `tests/windows/`, like `contain.rs`, with a `#[path]` line at the top of `tests/windows.rs`, whose helpers it uses through `use super::*`: that keeps upstream's file small for merging. A test of logic both systems share goes in the `src` file it tests. Unix code can't run here and the check only type-checks it, so read your Unix changes twice.

This is Rust, so where the rules above say `creationflags`, a subprocess gets no window through `platform::no_console`. A pattern or a test that has to name a flag the checks forbid builds it from parts with `concat!`, as `task/checks.rs` does: the forbid rule reads your source too.

Tests start `kom` through a `Session` (see `tests/windows.rs`), which has its own `KOMPITUM_SESSION` and state folders. Never start `kom` or `kompitum-core` any other way: the machine's real core, the Python Kompitum and its workers must stay untouched.
