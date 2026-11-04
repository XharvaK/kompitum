# Limen

If AGENTS.local.md exists, read it too: the Owner's local rules.

`limen-core` (command `lc`) is a terminal runtime for coding agents: it runs
programs in terminals, keeps them alive, relays raw bytes to any viewer,
reads status from OSC 7501, and lets agents start, talk to, read, wait on and
get notified by other agents. It draws nothing.

This repository began as a fork of
[herdr-core](https://github.com/herdrdev/herdr-core) and is on its way to
holding all of Limen: the worker loop, the vault and its bell, then the app.
[audit/SETTLE.md](audit/SETTLE.md) holds what was decided and the order of
work. [FORK.md](FORK.md) holds how the fork stays in step with upstream.

Read the code. This file only holds what reading won't tell you: the rules,
the traps, and the handful of decisions that look arbitrary until you know
why. The code is authoritative; ask the Owner about conflicting requirements.

`lc --skill` prints the manual in `crates/limen-core/src/SKILL.md`. The
installable skill is the short stub in `skills/limen-core/SKILL.md`: it says
when to use lc and points to `lc --skill`, so it rarely changes.
Keep the manual current when changing agent-facing behavior.

## The rule

Core is a runtime, not a multiplexer. If a feature only makes sense with a
layout or a workspace to return to, it does not belong in the core.

## Limen's rules

- Workers run headless: every child process starts without a console window,
  and nothing opens a window unless the Owner asks.
- Nothing types into an Opus pane, `--force` included. Notes and worker
  reports reach Opus through MCP, never by `lc send`, and a note does not
  start an Opus session unattended.
- Limen runs the real `claude` unmodified. It never reads, stores or routes
  its login, and never calls Anthropic's API on the plan.
- The vault is a doorbell and a pen, never a judge: it shows what is open and
  writes where the writer may write. What to do about a note is the reader's
  call.
- Fail loudly: a failed pull, a refused write or a damaged note is said in
  words, never skipped in silence.
- Never commit keys, tokens, ids, hostnames or IP addresses. Test values that
  look like secrets are built from parts at runtime.
- Upstream's names change only through `scripts/limenize.py`, never by hand.
  Keep every other change to upstream's files small, since each changed line
  can conflict when an upstream release is merged.
- Name features after what they do.
- Friction and upgrade ideas go to `UPGRADES.md` in the Python repository until
  this one takes over from it.

## Rules that aren't in the code

- libghostty-vt comes from upstream Ghostty, at the commit `vendor/ghostty`
  pins.
- Status comes only from OSC 7501. No screen detection, no agent registry,
  no vendor-specific logic in the runtime. This is the one most likely to be
  helpfully added; don't. The worker loop will know its CLIs by name, and that
  knowledge stays in the worker loop.
- Every action reports what was observed, never what was assumed.
- One API; the CLI is one call per command. The schema is generated from one
  method table, so params and results in `api/types.rs` are also CLI args and
  the published schema. Change them and regenerate:
  `LIMEN_UPDATE_SCHEMA=1 cargo test -p limen-core --test m0`.
- Lean, KISS, always. Fewer lines, fewer files, fewer dependencies. No
  abstraction before a second real caller, no config for things nobody asked
  to configure.
- Deciding code stays separate from IO, so it tests without processes.
- Gate per OS only where the OS really behaves differently, and keep it in
  `platform/` (`unix.rs`, `windows.rs`) behind one interface: shared code
  calls `platform::…` and has no `#[cfg]` of its own. A platform quirk gets
  a test on its platform, and other platforms keep their behavior.
- TDD. A test first for every required documented behavior. One good test per
  behavior, not five checking the same thing. A few end-to-end tests with real
  PTYs beat many mocks. Keep the suite fast.
- Do not change documented behavior silently. Behavior changes need the
  Owner's agreement.
- Aim for source files under ~32 KB, hard cap 48 KB because read tools stop
  near 50. Split along a real seam when you next touch one that's over.

## Traps

- Tests and smokes run in isolated sessions (`LIMEN_SESSION`) with scratch XDG
  paths under `/var/tmp`. Never touch the machine's real core, the Python
  Limen or its running workers. Clean up scratch data.
- `LC_*` is the locale namespace and SSH configs forward it, so our
  environment is `LIMEN_*`. The Python Limen owns `LIMEN_VAULT`, `LIMEN_AGENT`,
  `LIMEN_POLL_SECONDS`, `LIMEN_SEEN_FILE`, `LIMEN_LISTEN` and `LIMEN_BELL_*`,
  and the rename script refuses them.
- `just install` replaces the installed binary and restarts core. Running
  terminals are kept.
- Real-agent smokes (Claude Code with haiku, Pi with a cheap model) cost
  money. Ask before running them.
- A dependency change (crates or Ghostty) needs `just notices`; a test fails
  if a crate is missing from the notices.
- `rustfmt.toml` width 120 is the format: `just fmt`, or `cargo fmt --all`.
  `just lint` is fmt, clippy, and clippy for Windows from Linux. `just check`
  is lint plus the whole suite. Run it before proposing a commit.
- `docs/commands.md` is generated, and `tests/cli_docs.rs` checks every `lc`
  command and flag it finds in the README and `docs/`, in prose too.
