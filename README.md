# Kompitum

`kom` runs programs and coding agents in terminals that stay alive after you leave.
Agents can start other agents, send them work, and hear back; you can attach to any terminal and take over.

Core draws nothing. Use it from your shell, an agent, or your own app.

This is a fork of [herdr-core](https://github.com/herdrdev/herdr-core). So far it is herdr-core with new names.
Kompitum's own parts still live in its Python repository and come over as [the plan](audit/SETTLE.md) says: the worker loop, the vault, the MCP front, and then the app.

## Where it stands

- The rename is done and it builds on Windows, debug and release, with no code changes. Clippy is clean on the Windows code and on the Linux code.
- On Windows the whole suite passes, 156 tests with real cores and terminals among them, and none of it opened a window. One test is skipped because it writes to the real user environment.
- Claude Code runs in an `kom` terminal on its own login, with Kompitum's `cwmcp` and its workers under it, and nothing it started opened a window. `kom` follows its state, down to the command a permission dialog is asking about. So far that is one test run on Windows.
- The Unix suite, 273 tests and most of what checks the core's logic, has only been type-checked since the rename. We have no Linux machine or CI for it yet.
- There are no releases yet. `kom update` and the install scripts point at `https://xharvak.github.io/kompitum`, this repository's GitHub Pages, which holds nothing until the first release, so they fail loudly. Build from source and copy the binary.
- macOS builds are not planned: Windows and Linux are what we run.

## Run it and come back

```sh
kom run cargo test                  # stream output and wait for it
kom run --bg --name web bun dev     # start it and return
kom ls                             # see live terminals
kom attach web                     # take over; Ctrl+\ detaches
kom read web --lines 100            # read without attaching
```

Closing your viewer does not end the program.
`Ctrl+\` means the byte that chord sends, so where `\` needs AltGr it is another key: on Turkish Q, `Ctrl` with the key left of `Z`. Set `detach_key` in the config to use a different chord.
A waiting run checks in after 180 seconds by default, leaving the program running.
It prints the terminal's address so you can read it or wait again.

For a terminal you want to keep even if its program is lost, use `kom run --keep`.
Recovery starts a fresh shell, never reruns the task.
A normal exit or an explicit kill still ends it.

## Default shell

With no program, `kom run --bg` starts the machine's shell: `$SHELL` (or
`/bin/sh`) on Unix, PowerShell 7 on Windows if available, else Windows PowerShell.
Override it in `~/.config/kompitum-core/config.toml` (Windows:
`%APPDATA%\kompitum-core\config.toml`, or `$XDG_CONFIG_HOME/kompitum-core/config.toml`
when set):

```toml
shell = ["nu"]
# Windows Git Bash (literal strings keep backslashes):
# shell = ['C:\Program Files\Git\bin\bash.exe', '--login', '-i']
```

The list is an executable followed by separate arguments, without extra path
quoting. It must contain a nonempty executable. Changes apply to the next new
shell or `keep` recovery, including recovery after a restart; no restart is
needed. Running terminals, explicit programs, and quoted command lines keep
their behavior. A bad setting reports a config error; a shell that cannot start
reports `spawn_failed`. Recovery stays pending and retries, with errors in
`core.log` (config) or `holder.log` (spawn); it never silently falls back to another shell.

## Agents run agents

Give an agent `kom --skill`. It prints the agent docs in one read, including how to delegate, wait, report back, and clean up.

[Templates](docs/templates.md) give an agent a command and a role.
After saving the reviewer example from that page:

```sh
kom templates
kom run --bg --name review reviewer "Review src/x.rs. Read-only."
kom send review "Focus on the error path."
kom read review --lines 100
```

Agents started inside an kom terminal become its children.
They can report back with `kom send` and wait for their children with `kom wait --children all`.
A terminal started with `kom run --person` is a person's: only `kom attach` types into it, `--force` included. News of its children waits for its next `kom wait`.
The parent gets notes when a child needs attention or ends.
Children end with their parent unless started with `--outlive-parent`.

## What ends with a terminal

Everything a program starts ends with its terminal, detached or not. To keep a daemon running, give it a terminal of its own: `kom run --bg --no-parent --name proxy node proxy.js`.
On Windows a program can also leave by asking to break away from the terminal's job, which `detached` in Node and Bun does not do.

`kom run --contain` shuts that door, for work where a kill must be total. Nothing can break away, and on Windows a shell started through `COMSPEC` gets a hidden console, so a detached program below it opens no window.

## Every machine

```sh
kom machine add my-ssh-host --name box
kom run --on box --cwd /srv/app cargo test
kom attach box/t3
```

Use the address returned by your run, not the example's `box/t3`.
Programs and files stay on that machine. Your local core brings its terminals into the same list.
[Machines](docs/machines.md) covers SSH, custom connections, remote files, and reconnecting.

## Status comes from the program

An agent can say it is working, done, blocked, or in error through OSC 7501, the Program Status Protocol.
Core stores what it reports. It does not guess from screen text or recognize agent brands.
A program that does not report status has none.

Use `kom get review` for a snapshot or `kom events` to follow changes.

## Build your own client

`kom schema` prints the request/response API as OpenRPC with JSON Schema.
[The API docs](schema/README.md) cover connecting, terminal streams, events, and status.
Use `kom connect` to carry the protocol over stdin and stdout, including through SSH.

## Docs

- [Commands](docs/commands.md): generated from the binary's help.
- [Machines](docs/machines.md): work across machines.
- [Templates](docs/templates.md): reusable agent roles.
- [Security](docs/security.md): access, state, and updates.
- [API](schema/README.md): build a client.

For a quick lookup, run `kom --help` or a command's help, such as `kom run --help`.
For agents, use `kom --skill`.

## Build from source

You need Rust and Zig 0.16, and on Windows the MSVC C++ build tools. Upstream's CI pins Rust 1.95.0; 1.96 builds it too. If Zig isn't on your PATH, set `ZIG` to its full path.

```sh
git submodule update --init --depth 1 vendor/ghostty
cargo build --release --locked
```

The program is `target/release/kompitum-core` (`kompitum-core.exe` on Windows). The installers put it on your PATH as `kom`; until we have releases, copy it there yourself.
`cargo build` puts `kompitum-shell.exe` beside it too. `kom run --contain` needs it on Windows, so copy it along.
On Windows it needs Microsoft's ConPTY in a `conpty` folder beside it, and the tests need the same in `target\debug`:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fetch-conpty.ps1 -Dest target\release
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fetch-conpty.ps1 -Dest target\debug
cargo test --locked
```

With [just](https://github.com/casey/just), `just check` runs the format check, clippy and the suite, `just docs` refreshes the command reference, and `just install` installs this checkout and restarts your core. Running terminals keep running.

[FORK.md](FORK.md) says how this relates to upstream and how to take an upstream release. [AGENTS.md](AGENTS.md) holds the rules for working here.

License: [Apache-2.0](LICENSE). `kom --licenses` prints the included third-party licenses.
After a dependency change, regenerate them with `just notices`.
