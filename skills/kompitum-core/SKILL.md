---
name: kompitum-core
description: "Use kompitum-core (its CLI is kom) whenever you want sub-agents, to talk to or coordinate other agents, run background tasks or servers, run long-running commands, read or control terminals, or work on another machine; also to show or arrange things in the Kompitum window or build Kompitum mods and layouts. Prefer it over your own sub-agent tool, shell '&', nohup or tmux."
---

This skill is for kompitum-core, whose command is `kom`. It is not `kompitum`,
the older Python worker tool: that has its own `kompitum` command and skill.
Use `kom` here, never `kompitum`.

If `$KOMPITUM_TERMINAL_ID` is set, you're running inside kompitum-core: use kom.
If not, ask the user once this session whether to use kom for this work.

Before using kom, run `kom --skill` and follow it.
For the Kompitum window (spaces, tabs, layouts, mods, what's shown),
run `kompitum-app --skill`.
