# Commands

Generated from the built binary's help. Do not edit by hand.
Run `just docs` to refresh it and `just docs-check` to check it.

[Start here](../README.md) for a short tour.

## kom

```text
kom - run programs and coding agents in terminals that stay alive

Usage: kom <command> [options]
       kom               pick a terminal and attach (new… on top)

Terminals
  run <cmd|template> [task]   run it and wait like a shell (--bg: don't wait)
  ls                          list live terminals (--all: finished too)
  get <t>                     show one terminal
  attach [t]                  put this terminal on it (ctrl+\ detaches)
  read <t>                    print its screen and history
  kill <t>                    end its program
  rm <t>                      remove a finished terminal
  label <t> k=v...            set labels
  keep <t> [--off]            replace lost programs with a shell (also run --keep)
                              signal death recovers too; streams end on loss, reattach

Talk to programs
  send <t> <text>             paste text and press enter
  keys <t> <keys...>          press keys: enter, esc, up, ctrl+c
  interrupt <t>               press the stop key, report what happened
  wait <t>                    wait until it stops working (--output X: until it prints X,
                              --children: your helpers). Use it instead of sleep.
  put <t> <file>              send a file beside it, print where it landed

No terminal
  exec [--on m] <cmd>         run it, print exactly what it wrote, exit with its code
                              (-i: feed it what is piped in)
  file read|write|edit [--on m] <path>
                              a file here or there: print it, write what is piped
                              in, or replace exact text (--old X --new Y)

Workers and notes
  status                      agents, tasks, workers and Claude's limits at once
                              (--line: Claude Code's status line)
  task start|wait|review|accept|redo|discard
                              a job for a cheap worker in its own worktree
  workers                     which workers can take a task; --block cc --until 3h
  ledger                      what each worker run cost
  vault open|read|post|done|thread|find
                              notes between agents in the shared vault
  mcp [--as manager]          all of the above as MCP tools, for an agent's client

More
  templates                   list templates
  events                      follow changes live
  server start|stop|status|restart
                              stop parks kept terminals; next core start gives them shells
  update [--version X|--from <file>]   latest release, at least X, or a build
  machine add|ls|login|update|rename|rm   other machines, over ssh or a command
                              (--via 'fly ssh console -a x --pty=false -C')
  connect                     the API over stdin/stdout (ssh box kom connect)
  schema                      the API as JSON Schema, for app builders

<t> is an address like t7 or t1b, or a name. In a terminal, attach, read, get,
kill, rm, wait and interrupt pick one when it's left out.
Options: --json (default when piped; run and read print text), -V --version,
--licenses (what kom includes, and their licenses).
Per command: kom <command> --help
Settings: ~/.config/kompitum-core/config.toml. notes = false: kom types nothing
on its own (no notes to parent agents, no <kom> tags on messages).
shell = ["nu"] overrides the shell for new terminals and keep recovery.

Examples
  kom run --on vps "cd app && npm test"     like ssh, in a terminal that stays
  kom run --bg claude                       Claude Code in a new terminal
  kom attach t3                             take it over
  kom run reviewer "review src/x.rs"        a template with a task

Agent? Run: kom --skill
```

## kom attach

```text
Put this terminal on a core terminal (detach: ctrl+\)

Usage: kom attach [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json               JSON output even in a terminal
      --observe            Watch only: never type or resize
      --history <HISTORY>  Scrollback lines to show first (main screen only) [default: 0]
  -h, --help               Print help
```

## kom connect

```text
Speak the protocol over stdin/stdout (`ssh box kom connect`)

Usage: kom connect [OPTIONS]

Options:
      --json               JSON output even in a terminal
      --session <SESSION>  The kom session to reach, like KOMPITUM_SESSION (one command that reads the same in sh, cmd and PowerShell)
  -h, --help               Print help
```

## kom events

```text
Follow fact changes as they happen (one JSON line each when piped)

Usage: kom events [OPTIONS]

Options:
      --json                       JSON output even in a terminal
      --terminal <TERMINAL>        Only this terminal
      --children-of <CHILDREN_OF>  Only children of this terminal
      --label <K=V>                Only terminals carrying all of these labels (repeatable)
  -h, --help                       Print help
```

## kom exec

```text
Run one program without a terminal, here or on a machine: exactly what it wrote, and its exit code

Usage: kom exec [OPTIONS] <ARGV>...

Arguments:
  <ARGV>...  Program and arguments, run as given (no shell)

Options:
      --json               JSON output even in a terminal
      --on <MACHINE>       Run it on another machine (see `kom machine ls`)
      --cwd <CWD>          Working directory (default: where kom runs; through the API, the core's; with --on, that machine's home)
      --env <K=V>          Set on top of the core's environment, key=value (repeatable)
      --timeout <TIMEOUT>  Seconds before it and everything it started are ended (default 60, at most 600)
  -i, --stdin              Feed what is piped into kom to its stdin
  -h, --help               Print help
```

## kom file

```text
Read, write or edit a file here or on a machine: an agent's hands there

Usage: kom file [OPTIONS] <COMMAND>

Commands:
  read   Print a text file, or lines of it (--offset, --limit)
  write  Write what is piped in as the file, making missing folders
  edit   Replace exact text found once (--old X --new Y, repeatable); all or none
  help   Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom file edit

```text
Replace exact text found once (--old X --new Y, repeatable); all or none

Usage: kom file edit [OPTIONS] --old <OLD> --new <NEW> <PATH>

Arguments:
  <PATH>  The file: absolute, or from `cwd`

Options:
      --json          JSON output even in a terminal
      --on <MACHINE>  On another machine (see `kom machine ls`)
      --cwd <CWD>     Where a relative path starts (default: where kom runs; through the API, the core's; with --on, that machine's home)
      --root <ROOT>   Stay inside this folder: a path or link that leads out is refused. Relative ones start at `cwd`. Default: `cwd`
      --old <OLD>     Exact text found once in the file (repeatable, paired with --new in order)
      --new <NEW>     What replaces it
  -h, --help          Print help
```

## kom file read

```text
Print a text file, or lines of it (--offset, --limit)

Usage: kom file read [OPTIONS] <PATH>

Arguments:
  <PATH>  The file: absolute, or from `cwd`

Options:
      --json             JSON output even in a terminal
      --on <MACHINE>     On another machine (see `kom machine ls`)
      --cwd <CWD>        Where a relative path starts (default: where kom runs; through the API, the core's; with --on, that machine's home)
      --root <ROOT>      Stay inside this folder: a path or link that leads out is refused. Relative ones start at `cwd`. Default: `cwd`
      --offset <OFFSET>  The first line to give, from 1
      --limit <LIMIT>    At most this many lines (default: all, up to 8 MB)
  -h, --help             Print help
```

## kom file write

```text
Write what is piped in as the file, making missing folders

Usage: kom file write [OPTIONS] <PATH>

Arguments:
  <PATH>  The file: absolute, or from `cwd`

Options:
      --json          JSON output even in a terminal
      --on <MACHINE>  On another machine (see `kom machine ls`)
      --cwd <CWD>     Where a relative path starts (default: where kom runs; through the API, the core's; with --on, that machine's home)
      --root <ROOT>   Stay inside this folder: a path or link that leads out is refused. Relative ones start at `cwd`. Default: `cwd`
  -h, --help          Print help
```

## kom get

```text
Show one terminal

Usage: kom get [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom interrupt

```text
Press the stop key and report what happened

Usage: kom interrupt [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json               JSON output even in a terminal
      --timeout <TIMEOUT>  Seconds to watch for an effect (default 10)
      --force              Interrupt even if the program is blocked: the key answers its dialog
  -h, --help               Print help
```

## kom keep

```text
Keep a live terminal after loss (--off disables)

Usage: kom keep [OPTIONS] <TERMINAL>

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
      --off   Keep this live terminal after loss. Existing streams end with the loss reason; reattach after `restarted`
  -h, --help  Print help
```

## kom keys

```text
Press named keys (enter, esc, up, ctrl+c, alt+x, f5, a...)

Usage: kom keys [OPTIONS] <TERMINAL> <KEYS>...

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`
  <KEYS>...   Key names: enter, esc, tab, up, ctrl+c, alt+x, f5, a, ...

Options:
      --force  Press them even if the program is blocked: that answers its dialog
      --json   JSON output even in a terminal
  -h, --help   Print help
```

## kom kill

```text
End a terminal's program (hangup, then escalate)

Usage: kom kill [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom label

```text
Set labels (key=value; key= removes)

Usage: kom label [OPTIONS] <TERMINAL> <K=V>...

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`
  <K=V>...    Labels to set, key=value; `key=` removes one

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom ledger

```text
Worker runs: one row per accepted or discarded task, with what it cost

Usage: kom ledger [OPTIONS]

Options:
      --json         JSON output even in a terminal
      --task <TASK>  Only this task's row, with the orchestrator's own usage for it
      --last <N>     The newest rows to show (default 20)
  -h, --help         Print help
```

## kom ls

```text
List live terminals

Usage: kom ls [OPTIONS]

Options:
      --all          Include finished terminals
      --json         JSON output even in a terminal
      --label <K=V>  Only terminals carrying all of these labels (repeatable)
  -h, --help         Print help
```

## kom machine

```text
Other machines, reached over ssh

Usage: kom machine [OPTIONS] <COMMAND>

Commands:
  add     Add a machine you reach with ssh: kom machine add vps
  ls      List the machines added here
  login   Log in again (a password or code) after the machine says it needs one. On Windows: reconnect once its SSH keys work
  update  Copy this kom there and restart kom there (its terminals keep running)
  rename  Give a machine a new name here
  rm      Forget a machine (its terminals keep running there)
  help    Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom machine add

```text
Add a machine you reach with ssh: kom machine add vps

Usage: kom machine add [OPTIONS] [TARGET]

Arguments:
  [TARGET]  How ssh reaches it: `vps`, `can@10.0.0.7`, `can@10.0.0.7:2222`, a Host from ~/.ssh/config

Options:
      --json               JSON output even in a terminal
      --via <COMMAND>      Reach it with this command instead of ssh: it is given a shell line to run there as its last argument, like ssh (`ssh -p 2222 -i key user@ip`, `fly ssh console -a app --pty=false -C`). It must pass bytes both ways untouched and ask nothing. Needs --name
      --temp               Mark it short-lived (a sandbox for a task), so clients can show it apart from machines that stay. kom treats it like any other; remove it with `kom machine rm`
      --install            Copy this kom there when it has none or one too old, without asking (for scripts; in a terminal you are asked)
      --name <NAME>        What to call it here (default: the target's host)
      --session <SESSION>  The kom session on that machine (default: its default)
  -h, --help               Print help
```

## kom machine login

```text
Log in again (a password or code) after the machine says it needs one. On Windows: reconnect once its SSH keys work

Usage: kom machine login [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>  The machine's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom machine ls

```text
List the machines added here

Usage: kom machine ls [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom machine rename

```text
Give a machine a new name here

Usage: kom machine rename [OPTIONS] <MACHINE> <NEW_NAME>

Arguments:
  <MACHINE>   The machine's name now
  <NEW_NAME>  Its new name. Addresses with the old one stop working

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom machine rm

```text
Forget a machine (its terminals keep running there)

Usage: kom machine rm [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>  The machine's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom machine update

```text
Copy this kom there and restart kom there (its terminals keep running)

Usage: kom machine update [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>  The machine's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom mcp

```text
Serve tasks, workers and the vault as MCP tools on stdin and stdout, for an agent's client

Usage: kom mcp [OPTIONS]

Options:
      --as <AGENT>  The agent this server speaks for (default: KOMPITUM_AGENT, else opus)
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## kom put

```text
Send a local file beside a terminal (on its machine); prints where it landed

Usage: kom put [OPTIONS] <TERMINAL> <FILE>

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`
  <FILE>      The file's name, kept so `.png` stays `.png` (CLI: the local file to send)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom read

```text
Print a terminal's screen and history

Usage: kom read [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json           JSON output even in a terminal
      --lines <LINES>  Lines of history above the screen [default: 0]
  -h, --help           Print help
```

## kom rm

```text
Remove a finished terminal

Usage: kom rm [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom run

```text
Start a program, or a template by name with its task: kom run reviewer "review src/x.rs"

Usage: kom run [OPTIONS] [ARGV]...

Arguments:
  [ARGV]...  Program and arguments, or a template name and its task. With neither argv nor line: that machine's own shell

Options:
      --json               JSON output even in a terminal
      --name <NAME>        Name, unique among live terminals: 1-32 of a-z 0-9 - _ 
      --cwd <CWD>          Working directory (default: where kom runs; through the API, the core's; with --on, that machine's home)
      --on <MACHINE>       Run it on another machine (see `kom machine ls`); its address is `machine/t7`
      --label <K=V>        Labels, key=value (repeatable)
      --parent <PARENT>    Parent terminal (default: the terminal it is started from)
      --no-parent          No parent, even when started from a terminal
      --no-notify          Don't type notes about it into its parent
      --outlive-parent     Keep it running when its parent's program ends (by default it ends too)
      --rm                 Remove it right after it exits (once a waiting parent has the exit code)
      --keep               Replace lost programs with a shell in place, never re-run the original command
      --contain            Nothing this program starts can leave its terminal: no breaking away from its job, and on Windows a shell started through COMSPEC stays hidden. Changes nothing on Unix, where everything carrying the terminal's mark ends with it
      --person             A person works in this terminal: only `kom attach` types into it. `send`, `keys`, `interrupt` and notes are refused, `--force` included. News of its children waits for its next `kom wait`
      --bg                 Start it and come back at once (a server, several workers)
      --timeout <TIMEOUT>  Seconds before a wait checks in, leaving it running (default: config `wait_timeout`, else 180)
  -h, --help               Print help
```

## kom schema

```text
Every API method with its JSON Schema, for app builders

Usage: kom schema [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom send

```text
Paste text into a terminal and press Enter (waits for a person to pause)

Usage: kom send [OPTIONS] <TERMINAL> <TEXT>

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`
  <TEXT>      Pasted, then submitted with Enter

Options:
      --force              Type even if the program is blocked (it may answer its dialog)
      --json               JSON output even in a terminal
      --if <STATUS>        Only type if the status is exactly this [possible values: idle, working]
      --wait               Then wait for the turn this send started
      --timeout <TIMEOUT>  With wait: seconds before it checks in (default: config, else 180)
  -h, --help               Print help
```

## kom server

```text
Start, stop or inspect the core

Usage: kom server [OPTIONS] <COMMAND>

Commands:
  start    
  stop     
  status   
  restart  Restart the core (e.g. after an update); programs keep running
  help     Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom server restart

```text
Restart the core (e.g. after an update); programs keep running

Usage: kom server restart [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom server start

```text
Usage: kom server start [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom server status

```text
Usage: kom server status [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom server stop

```text
Usage: kom server stop [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom status

```text
Everything at one glance: agents, tasks, workers and Claude's limits

Usage: kom status [OPTIONS]

Options:
      --json  JSON output even in a terminal
      --line  Read Claude Code's status line JSON on stdin: keep its limit resets and print one line
  -h, --help  Print help
```

## kom task

```text
Tasks: a worktree, a contained terminal and a worker, with a record

Usage: kom task [OPTIONS] <COMMAND>

Commands:
  start    Start a task: a worktree, a contained terminal and a worker
  ls       List tasks (--all includes discarded)
  show     Show one task: its record, progress, changes and report
  wait     Wait for tasks; prints one JSON line each and a summary, at the end
  cancel   Cancel a queued or running task
  discard  Remove a finished task's worktree and branch, and write its ledger row
  review   The whole review of a task in one read: checks, scope, warns, Gaps, report and the hunks they point at
  accept   Bring a finished task's changes into its repository and commit only those files
  redo     Send a finished task back with a follow-up brief, or answer an asking one; its checks run again
  sweep    Remove what old tasks left; their ledger rows stay
  help     Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom task accept

```text
Bring a finished task's changes into its repository and commit only those files

Usage: kom task accept [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --json                    JSON output even in a terminal
      --review <REVIEW>         What the review decided, kept for the ledger
      --message <MESSAGE>       The commit message (default: the review, else a line naming the task)
      --no-commit               Put the changes in the working tree and make no commit
      --accept-unfinished       Accept a task that failed, timed out or was cancelled, or whose last checks failed
      --accept-scope-violation  Accept files changed outside the task's scope
      --owner-said <TEXT>       The Owner's words that let a manager accept; kept in the record and the ledger row
  -h, --help                    Print help
```

## kom task cancel

```text
Cancel a queued or running task

Usage: kom task cancel [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --json  JSON output even in a terminal
      --yes   Cancel a task another caller started
  -h, --help  Print help
```

## kom task discard

```text
Remove a finished task's worktree and branch, and write its ledger row

Usage: kom task discard [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --json             JSON output even in a terminal
      --review <REVIEW>  What the review decided, kept with the record
      --yes              Discard a task another caller started
  -h, --help             Print help
```

## kom task ls

```text
List tasks (--all includes discarded)

Usage: kom task ls [OPTIONS]

Options:
      --all   Include discarded tasks
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom task redo

```text
Send a finished task back with a follow-up brief, or answer an asking one; its checks run again

Usage: kom task redo [OPTIONS] --brief <FILE> <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --brief <FILE>  The follow-up's or answer's file, or - for stdin
      --json          JSON output even in a terminal
  -h, --help          Print help
```

## kom task review

```text
The whole review of a task in one read: checks, scope, warns, Gaps, report and the hunks they point at

Usage: kom task review [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --full           Add the whole patch; one that does not fit goes to a file the review names
      --json           JSON output even in a terminal
      --max-bytes <N>  The review's size in bytes (default 20000)
  -h, --help           Print help
```

## kom task show

```text
Show one task: its record, progress, changes and report

Usage: kom task show [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom task start

```text
Start a task: a worktree, a contained terminal and a worker

Usage: kom task start [OPTIONS] --brief <FILE> <NAME>

Arguments:
  <NAME>  The task's name (1-32 of a-z 0-9 - _ ., starting with a letter or digit)

Options:
      --brief <FILE>       The brief's file, or - for stdin
      --json               JSON output even in a terminal
      --in <FOLDER>        The folder in the repository (default: where kom runs)
      --scope <GLOB>       Paths the task may change (repeatable); none means the whole repository
      --worker <WORKER>    Which worker: auto (the order), free, or a worker key. Default: what kom workers shows
      --idle <IDLE>        Seconds without output, CPU or I/O before the turn is cut (default 900)
      --timeout <TIMEOUT>  Wall-clock limit in seconds (default: none)
      --test <COMMAND>     The test command the checks run when the worker ends, replacing the repository's
      --forbid <PATTERN>   A regex, or glob::regex, no added line may match (repeatable)
      --require <PATTERN>  A glob::regex some line of the matching files must match after the run (repeatable)
      --warn <PATTERN>     A regex, or glob::regex, whose hits are flagged for the review (repeatable)
      --qa-rounds <N>      How many times failed checks go back to the worker, 0 to 3 (default: the repository's, else 1)
      --web                Let the worker use the web
  -h, --help               Print help
```

## kom task sweep

```text
Remove what old tasks left; their ledger rows stay

Usage: kom task sweep [OPTIONS]

Options:
      --days <N>  Days after its end that a task keeps its files (default 14)
      --json      JSON output even in a terminal
      --dry-run   Say what would go, and remove nothing
  -h, --help      Print help
```

## kom task wait

```text
Wait for tasks; prints one JSON line each and a summary, at the end

Usage: kom task wait [OPTIONS] <NAMES>...

Arguments:
  <NAMES>...  The tasks to wait for

Options:
      --first              Return at the first task that ends or asks, however it did
      --json               JSON output even in a terminal
      --while-stranded     Keep waiting while every worker is out; the task goes on by itself when one comes back
      --timeout <TIMEOUT>  Seconds to wait (default: as long as it takes)
      --grace <GRACE>      Seconds a name not known yet stays pending, for a start still on its way (default 30) [default: 30]
  -h, --help               Print help
```

## kom templates

```text
Templates you can run by name (~/.config/kompitum-core/templates)

Usage: kom templates [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom update

```text
Swap this kom for another build and restart (terminals keep running)

Usage: kom update [OPTIONS]

Options:
      --from <FILE>      A kompitum-core binary to use (default: download the latest release)
      --json             JSON output even in a terminal
      --version <X.Y.Z>  At least this release (`0.1.4`): nothing changes when this kom is already that or newer
      --force            Allow an older version (saved state only moves forward), or a release of another protocol generation
  -h, --help             Print help
```

## kom vault

```text
Messages between agents, and my own memory, in the shared vault: read, post, mark, find, remember

Usage: kom vault [OPTIONS] <COMMAND>

Commands:
  open      The messages open for me, newest first
  read      Read one note by its vault path; this also leaves a read mark
  find      Search the vault: every word must be in the note; the filters narrow it
  post      Write a message into my own folder, then commit and push it
  mark      Set the status of one of my own notes: open, answered or superseded
  done      Close a note for me with no reply, leaving a done mark
  thread    Every message between me and another agent, both ways, oldest first
  changes   The messages added since the last commit this reader saw
  remember  Write one memory note into my own memory folder, then commit and push it
  forget    Mark one of my memory notes superseded, with why, then commit and push it
  memory    What I wake with: standing memories in full, then one line per other memory
  help      Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## kom vault changes

```text
The messages added since the last commit this reader saw

Usage: kom vault changes [OPTIONS]

Options:
      --as <AGENT>  Act as this agent, overriding KOMPITUM_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## kom vault done

```text
Close a note for me with no reply, leaving a done mark

Usage: kom vault done [OPTIONS] <PATH>

Arguments:
  <PATH>  The note to close

Options:
      --json        JSON output even in a terminal
      --why <TEXT>  Why it is closed, kept with the mark
      --as <AGENT>  Act as this agent, overriding KOMPITUM_AGENT
  -h, --help        Print help
```

## kom vault find

```text
Search the vault: every word must be in the note; the filters narrow it

Usage: kom vault find [OPTIONS] [WORDS]...

Arguments:
  [WORDS]...  Words that must all be in the note (case does not matter)

Options:
      --json         JSON output even in a terminal
      --project <P>  Only notes of this project (front matter, or the folder)
      --kind <K>     Only notes of this kind
      --status <S>   Only notes with this status
      --from <A>     Only notes from this agent (front matter, or the folder)
      --limit <N>    The newest this many (default 20) [default: 20]
      --as <AGENT>   Act as this agent, overriding KOMPITUM_AGENT
  -h, --help         Print help
```

## kom vault forget

```text
Mark one of my memory notes superseded, with why, then commit and push it

Usage: kom vault forget [OPTIONS] --why <TEXT> <PATH>

Arguments:
  <PATH>  One of my own memory notes

Options:
      --json        JSON output even in a terminal
      --why <TEXT>  Why it no longer holds; kept in the note
      --as <AGENT>  Act as this agent, overriding KOMPITUM_AGENT
  -h, --help        Print help
```

## kom vault mark

```text
Set the status of one of my own notes: open, answered or superseded

Usage: kom vault mark [OPTIONS] <PATH> <STATUS>

Arguments:
  <PATH>    One of my own notes
  <STATUS>  open, answered or superseded

Options:
      --as <AGENT>  Act as this agent, overriding KOMPITUM_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## kom vault memory

```text
What I wake with: standing memories in full, then one line per other memory

Usage: kom vault memory [OPTIONS]

Options:
      --json         JSON output even in a terminal
      --project <P>  The standing memories, General, and this project only
      --due          The memories due for review instead of the packet, oldest first
      --hook         Print Claude Code's session-start hook JSON and always exit 0
      --as <AGENT>   Act as this agent, overriding KOMPITUM_AGENT
  -h, --help         Print help
```

## kom vault open

```text
The messages open for me, newest first

Usage: kom vault open [OPTIONS]

Options:
      --as <AGENT>  Act as this agent, overriding KOMPITUM_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## kom vault post

```text
Write a message into my own folder, then commit and push it

Usage: kom vault post [OPTIONS] --to <A> --kind <K> --title <T>

Options:
      --json              JSON output even in a terminal
      --to <A>            The agent's folder to write into, or all
      --kind <K>          request, report, decision, tried, question or answer
      --title <T>         One line naming the note
      --body <TEXT>       The body, in Markdown
      --body-file <FILE>  The body's file, or - for standard input
      --project <P>       The project the note belongs to
      --re <PATH>         A note this one answers (repeatable for one answer to several)
      --as <AGENT>        Act as this agent, overriding KOMPITUM_AGENT
  -h, --help              Print help
```

## kom vault read

```text
Read one note by its vault path; this also leaves a read mark

Usage: kom vault read [OPTIONS] <PATH>

Arguments:
  <PATH>  The note's vault path, such as manager/to/opus/2026-11-04_1826_living-brain.md

Options:
      --as <AGENT>  Act as this agent, overriding KOMPITUM_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## kom vault remember

```text
Write one memory note into my own memory folder, then commit and push it

Usage: kom vault remember [OPTIONS] --title <LINE>

Options:
      --json                 JSON output even in a terminal
      --title <LINE>         One line naming the memory; at most 120 characters
      --body <TEXT>          The memory itself, in Markdown; at most 4000 characters
      --body-file <FILE>     The body's file, or - for standard input
      --project <P>          The project the memory belongs to
      --standing             A rule that holds in every session; the packet shows it in full
      --review-after <DATE>  When this memory goes stale (YYYY-MM-DD)
      --replace <PATH>       Rewrite one of my own memory notes in place
      --new                  Write even when a close title already exists
      --as <AGENT>           Act as this agent, overriding KOMPITUM_AGENT
  -h, --help                 Print help
```

## kom vault thread

```text
Every message between me and another agent, both ways, oldest first

Usage: kom vault thread [OPTIONS] <AGENT>

Arguments:
  <AGENT>  The other agent

Options:
      --json        JSON output even in a terminal
      --limit <N>   The newest this many messages (default 50) [default: 50]
      --as <AGENT>  Act as this agent, overriding KOMPITUM_AGENT
  -h, --help        Print help
```

## kom wait

```text
Wait for your children (--children any) or one terminal; a message to you also ends it

Usage: kom wait [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]
          The terminal; with `children`, whose children (default: the caller)

Options:
      --children <CHILDREN>
          Wait on children instead: all = until none is working, any = next news or a message to you.
          
          `all`: until none of the children is working (one that hasn't reported yet counts as working). `any`: the parent's unseen news (a turn ended, it blocked, it hit an error, it exited), else the next child to stop working; for the parent itself, also a message typed into it while it worked (reason `message`). A child with a sent launch task counts as working until its turn advances past delivery, or it reports done, blocked, error, or exits. Startup idle does not complete that task in either mode. Without a launch task, idle still counts as stopped.
          
          [possible values: any, all]

      --json
          JSON output even in a terminal

      --output <OUTPUT>
          For programs, not agents: end when output printed after the wait started contains this text (plain text, escape sequences stripped). Blocked, error or exit end it too

      --timeout <TIMEOUT>
          Seconds before it checks in (default: config `wait_timeout`, else 180)

  -h, --help
          Print help (see a summary with '-h')
```

## kom workers

```text
See which workers can take a task, change the order, or say when one is back

Usage: kom workers [OPTIONS]

Options:
      --json             JSON output even in a terminal
      --mode <MODE>      Which worker new tasks use: auto or a worker key
      --order <KEY,...>  The auto chain, in order (a comma list)
      --add <KEY>        Add a worker, at the end of the auto chain
      --cli <CLI>        The added worker's CLI: codex, command_code or opencode [possible values: codex, command_code, opencode]
      --route <ROUTE>    The added worker's model route
      --label <LABEL>    The added worker's label (default: its key)
      --remove <KEY>     Remove an added worker
      --block <BLOCK>    Mark this worker as out
      --until <UNTIL>    When it is back: 2026-11-23, 2026-11-23 15:36, or a span such as 3h or 2d
      --reason <REASON>  Why, in your words
      --clear <CLEAR>    Say this worker is back now
  -h, --help             Print help
```
