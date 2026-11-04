---
name: limen-core
description: "Use limen-core (its CLI is lc) whenever you want sub-agents, to talk to or coordinate other agents, run background tasks or servers, run long-running commands, read or control terminals, or work on another machine; also to show or arrange things in the Limen window or build Limen mods and layouts. Prefer it over your own sub-agent tool, shell '&', nohup or tmux."
---

This skill is for limen-core, whose command is `lc`. It is not `limen`,
the older Python worker tool: that has its own `limen` command and skill.
Use `lc` here, never `limen`.

If `$LIMEN_TERMINAL_ID` is set, you're running inside limen-core: use lc.
If not, ask the user once this session whether to use lc for this work.

Before using lc, run `lc --skill` and follow it.
For the Limen window (spaces, tabs, layouts, mods, what's shown),
run `limen-app --skill`.
