# Commands

Generated from the built binary's help. Do not edit by hand.
Run `just docs` to refresh it and `just docs-check` to check it.

[Start here](../README.md) for a short tour.

## lc

```text
lc - run programs and coding agents in terminals that stay alive

Usage: lc <command> [options]
       lc               pick a terminal and attach (new… on top)

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

More
  templates                   list templates
  events                      follow changes live
  server start|stop|status|restart
                              stop parks kept terminals; next core start gives them shells
  update [--version X|--from <file>]   latest release, at least X, or a build
  machine add|ls|login|update|rename|rm   other machines, over ssh or a command
                              (--via 'fly ssh console -a x --pty=false -C')
  connect                     the API over stdin/stdout (ssh box lc connect)
  schema                      the API as JSON Schema, for app builders

<t> is an address like t7 or t1b, or a name. In a terminal, attach, read, get,
kill, rm, wait and interrupt pick one when it's left out.
Options: --json (default when piped; run and read print text), -V --version,
--licenses (what lc includes, and their licenses).
Per command: lc <command> --help
Settings: ~/.config/limen-core/config.toml. notes = false: lc types nothing
on its own (no notes to parent agents, no <lc> tags on messages).
shell = ["nu"] overrides the shell for new terminals and keep recovery.

Examples
  lc run --on vps "cd app && npm test"     like ssh, in a terminal that stays
  lc run --bg claude                       Claude Code in a new terminal
  lc attach t3                             take it over
  lc run reviewer "review src/x.rs"        a template with a task

Agent? Run: lc --skill
```

## lc attach

```text
Put this terminal on a core terminal (detach: ctrl+\)

Usage: lc attach [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json               JSON output even in a terminal
      --observe            Watch only: never type or resize
      --history <HISTORY>  Scrollback lines to show first (main screen only) [default: 0]
  -h, --help               Print help
```

## lc connect

```text
Speak the protocol over stdin/stdout (`ssh box lc connect`)

Usage: lc connect [OPTIONS]

Options:
      --json               JSON output even in a terminal
      --session <SESSION>  The lc session to reach, like LIMEN_SESSION (one command that reads the same in sh, cmd and PowerShell)
  -h, --help               Print help
```

## lc events

```text
Follow fact changes as they happen (one JSON line each when piped)

Usage: lc events [OPTIONS]

Options:
      --json                       JSON output even in a terminal
      --terminal <TERMINAL>        Only this terminal
      --children-of <CHILDREN_OF>  Only children of this terminal
      --label <K=V>                Only terminals carrying all of these labels (repeatable)
  -h, --help                       Print help
```

## lc exec

```text
Run one program without a terminal, here or on a machine: exactly what it wrote, and its exit code

Usage: lc exec [OPTIONS] <ARGV>...

Arguments:
  <ARGV>...  Program and arguments, run as given (no shell)

Options:
      --json               JSON output even in a terminal
      --on <MACHINE>       Run it on another machine (see `lc machine ls`)
      --cwd <CWD>          Working directory (default: where lc runs; through the API, the core's; with --on, that machine's home)
      --env <K=V>          Set on top of the core's environment, key=value (repeatable)
      --timeout <TIMEOUT>  Seconds before it and everything it started are ended (default 60, at most 600)
  -i, --stdin              Feed what is piped into lc to its stdin
  -h, --help               Print help
```

## lc file

```text
Read, write or edit a file here or on a machine: an agent's hands there

Usage: lc file [OPTIONS] <COMMAND>

Commands:
  read   Print a text file, or lines of it (--offset, --limit)
  write  Write what is piped in as the file, making missing folders
  edit   Replace exact text found once (--old X --new Y, repeatable); all or none
  help   Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc file edit

```text
Replace exact text found once (--old X --new Y, repeatable); all or none

Usage: lc file edit [OPTIONS] --old <OLD> --new <NEW> <PATH>

Arguments:
  <PATH>  The file: absolute, or from `cwd`

Options:
      --json          JSON output even in a terminal
      --on <MACHINE>  On another machine (see `lc machine ls`)
      --cwd <CWD>     Where a relative path starts (default: where lc runs; through the API, the core's; with --on, that machine's home)
      --root <ROOT>   Stay inside this folder: a path or link that leads out is refused. Relative ones start at `cwd`. Default: `cwd`
      --old <OLD>     Exact text found once in the file (repeatable, paired with --new in order)
      --new <NEW>     What replaces it
  -h, --help          Print help
```

## lc file read

```text
Print a text file, or lines of it (--offset, --limit)

Usage: lc file read [OPTIONS] <PATH>

Arguments:
  <PATH>  The file: absolute, or from `cwd`

Options:
      --json             JSON output even in a terminal
      --on <MACHINE>     On another machine (see `lc machine ls`)
      --cwd <CWD>        Where a relative path starts (default: where lc runs; through the API, the core's; with --on, that machine's home)
      --root <ROOT>      Stay inside this folder: a path or link that leads out is refused. Relative ones start at `cwd`. Default: `cwd`
      --offset <OFFSET>  The first line to give, from 1
      --limit <LIMIT>    At most this many lines (default: all, up to 8 MB)
  -h, --help             Print help
```

## lc file write

```text
Write what is piped in as the file, making missing folders

Usage: lc file write [OPTIONS] <PATH>

Arguments:
  <PATH>  The file: absolute, or from `cwd`

Options:
      --json          JSON output even in a terminal
      --on <MACHINE>  On another machine (see `lc machine ls`)
      --cwd <CWD>     Where a relative path starts (default: where lc runs; through the API, the core's; with --on, that machine's home)
      --root <ROOT>   Stay inside this folder: a path or link that leads out is refused. Relative ones start at `cwd`. Default: `cwd`
  -h, --help          Print help
```

## lc get

```text
Show one terminal

Usage: lc get [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc interrupt

```text
Press the stop key and report what happened

Usage: lc interrupt [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json               JSON output even in a terminal
      --timeout <TIMEOUT>  Seconds to watch for an effect (default 10)
      --force              Interrupt even if the program is blocked: the key answers its dialog
  -h, --help               Print help
```

## lc keep

```text
Keep a live terminal after loss (--off disables)

Usage: lc keep [OPTIONS] <TERMINAL>

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
      --off   Keep this live terminal after loss. Existing streams end with the loss reason; reattach after `restarted`
  -h, --help  Print help
```

## lc keys

```text
Press named keys (enter, esc, up, ctrl+c, alt+x, f5, a...)

Usage: lc keys [OPTIONS] <TERMINAL> <KEYS>...

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`
  <KEYS>...   Key names: enter, esc, tab, up, ctrl+c, alt+x, f5, a, ...

Options:
      --force  Press them even if the program is blocked: that answers its dialog
      --json   JSON output even in a terminal
  -h, --help   Print help
```

## lc kill

```text
End a terminal's program (hangup, then escalate)

Usage: lc kill [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc label

```text
Set labels (key=value; key= removes)

Usage: lc label [OPTIONS] <TERMINAL> <K=V>...

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`
  <K=V>...    Labels to set, key=value; `key=` removes one

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc ledger

```text
Worker runs: one row per accepted or discarded task, with what it cost

Usage: lc ledger [OPTIONS]

Options:
      --json         JSON output even in a terminal
      --task <TASK>  Only this task's row, with the orchestrator's own usage for it
      --last <N>     The newest rows to show (default 20)
  -h, --help         Print help
```

## lc ls

```text
List live terminals

Usage: lc ls [OPTIONS]

Options:
      --all          Include finished terminals
      --json         JSON output even in a terminal
      --label <K=V>  Only terminals carrying all of these labels (repeatable)
  -h, --help         Print help
```

## lc machine

```text
Other machines, reached over ssh

Usage: lc machine [OPTIONS] <COMMAND>

Commands:
  add     Add a machine you reach with ssh: lc machine add vps
  ls      List the machines added here
  login   Log in again (a password or code) after the machine says it needs one. On Windows: reconnect once its SSH keys work
  update  Copy this lc there and restart lc there (its terminals keep running)
  rename  Give a machine a new name here
  rm      Forget a machine (its terminals keep running there)
  help    Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc machine add

```text
Add a machine you reach with ssh: lc machine add vps

Usage: lc machine add [OPTIONS] [TARGET]

Arguments:
  [TARGET]  How ssh reaches it: `vps`, `can@10.0.0.7`, `can@10.0.0.7:2222`, a Host from ~/.ssh/config

Options:
      --json               JSON output even in a terminal
      --via <COMMAND>      Reach it with this command instead of ssh: it is given a shell line to run there as its last argument, like ssh (`ssh -p 2222 -i key user@ip`, `fly ssh console -a app --pty=false -C`). It must pass bytes both ways untouched and ask nothing. Needs --name
      --temp               Mark it short-lived (a sandbox for a task), so clients can show it apart from machines that stay. lc treats it like any other; remove it with `lc machine rm`
      --install            Copy this lc there when it has none or one too old, without asking (for scripts; in a terminal you are asked)
      --name <NAME>        What to call it here (default: the target's host)
      --session <SESSION>  The lc session on that machine (default: its default)
  -h, --help               Print help
```

## lc machine login

```text
Log in again (a password or code) after the machine says it needs one. On Windows: reconnect once its SSH keys work

Usage: lc machine login [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>  The machine's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc machine ls

```text
List the machines added here

Usage: lc machine ls [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc machine rename

```text
Give a machine a new name here

Usage: lc machine rename [OPTIONS] <MACHINE> <NEW_NAME>

Arguments:
  <MACHINE>   The machine's name now
  <NEW_NAME>  Its new name. Addresses with the old one stop working

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc machine rm

```text
Forget a machine (its terminals keep running there)

Usage: lc machine rm [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>  The machine's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc machine update

```text
Copy this lc there and restart lc there (its terminals keep running)

Usage: lc machine update [OPTIONS] <MACHINE>

Arguments:
  <MACHINE>  The machine's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc put

```text
Send a local file beside a terminal (on its machine); prints where it landed

Usage: lc put [OPTIONS] <TERMINAL> <FILE>

Arguments:
  <TERMINAL>  The terminal: `t7`, a name, or `machine/t7`
  <FILE>      The file's name, kept so `.png` stays `.png` (CLI: the local file to send)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc read

```text
Print a terminal's screen and history

Usage: lc read [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json           JSON output even in a terminal
      --lines <LINES>  Lines of history above the screen [default: 0]
  -h, --help           Print help
```

## lc rm

```text
Remove a finished terminal

Usage: lc rm [OPTIONS] [TERMINAL]

Arguments:
  [TERMINAL]  The terminal: `t7`, a name, or `machine/t7`

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc run

```text
Start a program, or a template by name with its task: lc run reviewer "review src/x.rs"

Usage: lc run [OPTIONS] [ARGV]...

Arguments:
  [ARGV]...  Program and arguments, or a template name and its task. With neither argv nor line: that machine's own shell

Options:
      --json               JSON output even in a terminal
      --name <NAME>        Name, unique among live terminals: 1-32 of a-z 0-9 - _ 
      --cwd <CWD>          Working directory (default: where lc runs; through the API, the core's; with --on, that machine's home)
      --on <MACHINE>       Run it on another machine (see `lc machine ls`); its address is `machine/t7`
      --label <K=V>        Labels, key=value (repeatable)
      --parent <PARENT>    Parent terminal (default: the terminal it is started from)
      --no-parent          No parent, even when started from a terminal
      --no-notify          Don't type notes about it into its parent
      --outlive-parent     Keep it running when its parent's program ends (by default it ends too)
      --rm                 Remove it right after it exits (once a waiting parent has the exit code)
      --keep               Replace lost programs with a shell in place, never re-run the original command
      --contain            Nothing this program starts can leave its terminal: no breaking away from its job, and on Windows a shell started through COMSPEC stays hidden. Changes nothing on Unix, where everything carrying the terminal's mark ends with it
      --person             A person works in this terminal: only `lc attach` types into it. `send`, `keys`, `interrupt` and notes are refused, `--force` included. News of its children waits for its next `lc wait`
      --bg                 Start it and come back at once (a server, several workers)
      --timeout <TIMEOUT>  Seconds before a wait checks in, leaving it running (default: config `wait_timeout`, else 180)
  -h, --help               Print help
```

## lc schema

```text
Every API method with its JSON Schema, for app builders

Usage: lc schema [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc send

```text
Paste text into a terminal and press Enter (waits for a person to pause)

Usage: lc send [OPTIONS] <TERMINAL> <TEXT>

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

## lc server

```text
Start, stop or inspect the core

Usage: lc server [OPTIONS] <COMMAND>

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

## lc server restart

```text
Restart the core (e.g. after an update); programs keep running

Usage: lc server restart [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc server start

```text
Usage: lc server start [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc server status

```text
Usage: lc server status [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc server stop

```text
Usage: lc server stop [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc status

```text
Everything at one glance: agents, tasks, workers and Claude's limits

Usage: lc status [OPTIONS]

Options:
      --json  JSON output even in a terminal
      --line  Read Claude Code's status line JSON on stdin: keep its limit resets and print one line
  -h, --help  Print help
```

## lc task

```text
Tasks: a worktree, a contained terminal and a worker, with a record

Usage: lc task [OPTIONS] <COMMAND>

Commands:
  start    Start a task: a worktree, a contained terminal and a worker
  ls       List tasks (--all includes discarded)
  show     Show one task: its record, progress, changes and report
  wait     Wait for tasks; prints one JSON line each and a summary, at the end
  cancel   Cancel a queued or running task
  discard  Remove a finished task's worktree and branch, and write its ledger row
  review   The whole review of a task in one read: checks, scope, warns, Gaps, report and the hunks they point at
  accept   Bring a finished task's changes into its repository and commit only those files
  redo     Send a finished task back with a follow-up brief; its checks run again
  sweep    Remove what old tasks left; their ledger rows stay
  help     Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc task accept

```text
Bring a finished task's changes into its repository and commit only those files

Usage: lc task accept [OPTIONS] <TASK>

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

## lc task cancel

```text
Cancel a queued or running task

Usage: lc task cancel [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --json  JSON output even in a terminal
      --yes   Cancel a task another caller started
  -h, --help  Print help
```

## lc task discard

```text
Remove a finished task's worktree and branch, and write its ledger row

Usage: lc task discard [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --json             JSON output even in a terminal
      --review <REVIEW>  What the review decided, kept with the record
      --yes              Discard a task another caller started
  -h, --help             Print help
```

## lc task ls

```text
List tasks (--all includes discarded)

Usage: lc task ls [OPTIONS]

Options:
      --all   Include discarded tasks
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc task redo

```text
Send a finished task back with a follow-up brief; its checks run again

Usage: lc task redo [OPTIONS] --brief <FILE> <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --brief <FILE>  The follow-up's file, or - for stdin
      --json          JSON output even in a terminal
  -h, --help          Print help
```

## lc task review

```text
The whole review of a task in one read: checks, scope, warns, Gaps, report and the hunks they point at

Usage: lc task review [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --full           Add the whole patch; one that does not fit goes to a file the review names
      --json           JSON output even in a terminal
      --max-bytes <N>  The review's size in bytes (default 20000)
  -h, --help           Print help
```

## lc task show

```text
Show one task: its record, progress, changes and report

Usage: lc task show [OPTIONS] <TASK>

Arguments:
  <TASK>  The task's name

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc task start

```text
Start a task: a worktree, a contained terminal and a worker

Usage: lc task start [OPTIONS] --brief <FILE> <NAME>

Arguments:
  <NAME>  The task's name (1-32 of a-z 0-9 - _ ., starting with a letter or digit)

Options:
      --brief <FILE>       The brief's file, or - for stdin
      --json               JSON output even in a terminal
      --in <FOLDER>        The folder in the repository (default: where lc runs)
      --scope <GLOB>       Paths the task may change (repeatable); none means the whole repository
      --worker <WORKER>    Which workers to use, in order: auto is Command Code then opencode Go, free is opencode free then both. Default: what lc workers shows [possible values: auto, cc, go, free]
      --idle <IDLE>        Seconds without output or CPU before the turn is cut (default 900)
      --timeout <TIMEOUT>  Seconds the turn may run (default 3600)
      --test <COMMAND>     The test command the checks run when the worker ends, replacing the repository's
      --forbid <PATTERN>   A regex, or glob::regex, no added line may match (repeatable)
      --require <PATTERN>  A glob::regex some line of the matching files must match after the run (repeatable)
      --warn <PATTERN>     A regex, or glob::regex, whose hits are flagged for the review (repeatable)
      --qa-rounds <N>      How many times failed checks go back to the worker, 0 to 3 (default: the repository's, else 1)
      --web                Let the worker use the web
  -h, --help               Print help
```

## lc task sweep

```text
Remove what old tasks left; their ledger rows stay

Usage: lc task sweep [OPTIONS]

Options:
      --days <N>  Days after its end that a task keeps its files (default 14)
      --json      JSON output even in a terminal
      --dry-run   Say what would go, and remove nothing
  -h, --help      Print help
```

## lc task wait

```text
Wait for tasks; prints one JSON line each and a summary, at the end

Usage: lc task wait [OPTIONS] <NAMES>...

Arguments:
  <NAMES>...  The tasks to wait for

Options:
      --first              Return at the first task that ends, however it ended
      --json               JSON output even in a terminal
      --while-stranded     Keep waiting while every worker is out; the task goes on by itself when one comes back
      --timeout <TIMEOUT>  Seconds to wait (default: as long as it takes)
      --grace <GRACE>      Seconds a name not known yet stays pending, for a start still on its way (default 30) [default: 30]
  -h, --help               Print help
```

## lc templates

```text
Templates you can run by name (~/.config/limen-core/templates)

Usage: lc templates [OPTIONS]

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc update

```text
Swap this lc for another build and restart (terminals keep running)

Usage: lc update [OPTIONS]

Options:
      --from <FILE>      A limen-core binary to use (default: download the latest release)
      --json             JSON output even in a terminal
      --version <X.Y.Z>  At least this release (`0.1.4`): nothing changes when this lc is already that or newer
      --force            Allow an older version (saved state only moves forward), or a release of another protocol generation
  -h, --help             Print help
```

## lc vault

```text
Notes between agents in the shared vault: read, post, mark, find

Usage: lc vault [OPTIONS] <COMMAND>

Commands:
  open     The messages open for me, newest first
  read     Read one note by its vault path; this also leaves a read mark
  find     Search the vault: every word must be in the note; the filters narrow it
  post     Write a message into my own folder, then commit and push it
  mark     Set the status of one of my own notes: open, answered or superseded
  done     Close a note for me with no reply, leaving a done mark
  thread   Every message between me and another agent, both ways, oldest first
  changes  The messages added since the last commit this reader saw
  help     Print this message or the help of the given subcommand(s)

Options:
      --json  JSON output even in a terminal
  -h, --help  Print help
```

## lc vault changes

```text
The messages added since the last commit this reader saw

Usage: lc vault changes [OPTIONS]

Options:
      --as <AGENT>  Act as this agent, overriding LIMEN_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## lc vault done

```text
Close a note for me with no reply, leaving a done mark

Usage: lc vault done [OPTIONS] <PATH>

Arguments:
  <PATH>  The note to close

Options:
      --json        JSON output even in a terminal
      --why <TEXT>  Why it is closed, kept with the mark
      --as <AGENT>  Act as this agent, overriding LIMEN_AGENT
  -h, --help        Print help
```

## lc vault find

```text
Search the vault: every word must be in the note; the filters narrow it

Usage: lc vault find [OPTIONS] [WORDS]...

Arguments:
  [WORDS]...  Words that must all be in the note (case does not matter)

Options:
      --json         JSON output even in a terminal
      --project <P>  Only notes of this project (front matter, or the folder)
      --kind <K>     Only notes of this kind
      --status <S>   Only notes with this status
      --from <A>     Only notes from this agent (front matter, or the folder)
      --limit <N>    The newest this many (default 20) [default: 20]
      --as <AGENT>   Act as this agent, overriding LIMEN_AGENT
  -h, --help         Print help
```

## lc vault mark

```text
Set the status of one of my own notes: open, answered or superseded

Usage: lc vault mark [OPTIONS] <PATH> <STATUS>

Arguments:
  <PATH>    One of my own notes
  <STATUS>  open, answered or superseded

Options:
      --as <AGENT>  Act as this agent, overriding LIMEN_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## lc vault open

```text
The messages open for me, newest first

Usage: lc vault open [OPTIONS]

Options:
      --as <AGENT>  Act as this agent, overriding LIMEN_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## lc vault post

```text
Write a message into my own folder, then commit and push it

Usage: lc vault post [OPTIONS] --to <A> --kind <K> --title <T>

Options:
      --json              JSON output even in a terminal
      --to <A>            The agent's folder to write into, or all
      --kind <K>          request, report, decision, tried, question or answer
      --title <T>         One line naming the note
      --body <TEXT>       The body, in Markdown
      --body-file <FILE>  The body's file, or - for standard input
      --project <P>       The project the note belongs to
      --re <PATH>         A note this one answers (repeatable for one answer to several)
      --as <AGENT>        Act as this agent, overriding LIMEN_AGENT
  -h, --help              Print help
```

## lc vault read

```text
Read one note by its vault path; this also leaves a read mark

Usage: lc vault read [OPTIONS] <PATH>

Arguments:
  <PATH>  The note's vault path, such as manager/to/opus/2026-11-04_1826_living-brain.md

Options:
      --as <AGENT>  Act as this agent, overriding LIMEN_AGENT
      --json        JSON output even in a terminal
  -h, --help        Print help
```

## lc vault thread

```text
Every message between me and another agent, both ways, oldest first

Usage: lc vault thread [OPTIONS] <AGENT>

Arguments:
  <AGENT>  The other agent

Options:
      --json        JSON output even in a terminal
      --limit <N>   The newest this many messages (default 50) [default: 50]
      --as <AGENT>  Act as this agent, overriding LIMEN_AGENT
  -h, --help        Print help
```

## lc wait

```text
Wait for your children (--children any) or one terminal; a message to you also ends it

Usage: lc wait [OPTIONS] [TERMINAL]

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

## lc workers

```text
See which workers can take a task, change the order, or say when one is back

Usage: lc workers [OPTIONS]

Options:
      --json             JSON output even in a terminal
      --mode <MODE>      Which workers new tasks use, in order: auto is Command Code then opencode Go, free is opencode free then both [possible values: auto, cc, go, free]
      --block <BLOCK>    Mark this worker as out [possible values: cc, go, free]
      --until <UNTIL>    When it is back: 2026-11-23, 2026-11-23 15:36, or a span such as 3h or 2d
      --reason <REASON>  Why, in your words
      --clear <CLEAR>    Say this worker is back now [possible values: cc, go, free]
  -h, --help             Print help
```
