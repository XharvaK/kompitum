---
name: herdr-app
description: "Drive the Herdr desktop window: inspect spaces, tabs, panes and focus; show terminals; and customize layouts, keys and mods. Use when the user asks to change what is on screen or how Herdr looks or works. For running and coordinating agents or programs, use hc instead."
---

## Install this skill

Ask your user whether to install this skill for future sessions. Only if they
say yes, save this exact output as `herdr-app/SKILL.md` in your harness's skills
folder, user-level unless they want it per repo. Common folders:
- `~/.agents/skills/` or `<repo>/.agents/skills/` (Codex, pi, Cursor, Gemini CLI, OpenCode, Amp).
- `~/.claude/skills/` or `<repo>/.claude/skills/` (Claude Code).
- Also: `~/.pi/agent/skills/`, `~/.cursor/skills/`, `~/.gemini/skills/`,
  `~/.config/opencode/skills/`, `~/.config/agents/skills/` (Amp).
Check your harness's docs if unsure. Installing is optional; use the skill now.

# herdr-app: drive the person's window

Herdr shows spaces, tabs and panes. herdr-core (`hc`) runs their terminals
and keeps them alive when the window closes. Use `herdr-app` only to change
what the person sees. For starting, reading, messaging, waiting on or cleaning
up agents and programs, read `hc --skill` and use hc.
Your terminal address is `$HC_TERMINAL_ID`; it is not a pane ID.

## Inspect before acting

```sh
herdr-app ls          # spaces > tabs > terminals, including pane IDs
herdr-app status      # current layout, focus, overlays, zoom and file/mod errors
herdr-app commands    # available commands, arguments and descriptions
```

These talk to the running window, not a new app. Do not launch bare
`herdr-app` to inspect it: that opens or raises the window. Do not stop,
signal or restart the person's app or core. A window closing does not kill
its terminals.

## Show something when asked

New spaces and tabs are shown immediately: only create them when asked.
Pick an explicit, useful name for every new space and tab; don't rely on
default names. Use IDs from `ls` rather than guessing or acting on the wrong
focused pane.

```sh
herdr-app do space.new --name api --folder /path/to/api
herdr-app do tab.new --name "fix login" --argv '["claude"]' --task "Look at the login bug"
herdr-app do pane.split --direction right --pane p12
herdr-app do agent.focus --terminal t7
```

For a remote space:

```sh
herdr-app do space.new --name "remote api" --machine vps --folder /srv/api
```

The folder is on that machine; tabs and splits in the space stay there.
`hc machine ls` lists machines. Use `hc run --on vps ...`, `hc exec --on vps ...`
and `hc file ... --on vps ...` for remote work, not your local file tools.

`do` accepts `--key value` arguments (JSON for arrays/objects) or one JSON
object. Use `commands` to confirm the command's exact shape:

```sh
herdr-app do pane.split '{"direction":"right","pane":"p12"}'
```

It prints the created IDs (for example `{ "pane": "...", "terminal": "..." }`)
or `{ "error": "..." }` and exits 1. Read the result, then inspect `ls` or
`status` to confirm. Commands invoked by an agent never open a missing-argument
dialog: supply all required arguments.

A terminal started with hc opts into the window with `herdr.space=<id>`.
`herdr.tab=<id>` chooses its tab; without it, Herdr creates a tab in that space.
Unlabeled workers stay hidden. Do not add labels just to run a background
worker. Use app commands when the
user wants to see a new tab or split, hc for orchestration that need not change
the screen.

## Customize layouts, keys, or mods

First read `~/.config/herdr-app/AGENTS.md` (or `$HERDR_APP_HOME/AGENTS.md` if
set). It is the installed guide to settings, layouts, commands and mods.
Read the generated `.d.ts` files beside it for full API details; do not edit
those files or the generated guide. The app rewrites them on start.

- Home defaults to `~/.config/herdr-app`; `HERDR_APP_HOME` selects another.
- Mods live in `mods/<name>/` with `main.js` and optional `mod.json`.
  They are trusted JavaScript with command and network access.
- Shipped mods belong to Herdr: duplicate with `mod.clone` before editing.
  Shipped layouts likewise use `layout.clone` for a personal copy.
  Discover required arguments with `herdr-app commands`.
- Save to reload. A broken save keeps the old version running and reports
  an error in `status`; a still-working screen is not proof the edit worked.
- Make only the requested change. Don't touch unrelated mods or settings.
- Use theme colors. Do not retain a mod's `cx` across callbacks; each
  callback receives its own. Native animations, not redraw timers, keep
  the window cheap to run. The installed guide has runnable examples.

After every mod change:

```sh
herdr-app check ~/.config/herdr-app/mods/<name>
herdr-app status
```

`check` loads the mod and renders it once. It exits nonzero with the exact
error. Never call a mod done without a passing check. Then confirm it is
shown correctly in the running window; don't claim a visual check you
haven't made. This version has no `herdr-app shot` command.

## Problems and safety

If Herdr misbehaves, `herdr-app feedback "<what happened>"` drafts a report.
The person reviews and sends it; it does not silently send feedback.

Never experiment with pointer/key automation on the person's real display.
When developing the app on Linux, its `scripts/sandbox` provides an isolated
home, core and display: `up`, `do`, `status`, `shot`, then `down` when done.
The sandbox's screenshot helper is not a command in the installed app CLI.
Never stop a server or session you didn't start.
