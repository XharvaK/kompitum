# Kompitum: upgrade notes

Friction found while using the system for real work, and what would remove it. Newest observations are added at
the end of each section. Run numbers (time, cost, tokens) live in the playbook ledger, `~/docs/deepseek-runs.md`.

**Frozen since 0.8.0, in force** (the User, 2026-11-01: "Yes, freeze", then 0.8.0 as one last pack). Building
Kompitum took about a fifth of a week's Claude quota, and it pays back only by doing work in the other projects. So
from now on: no new features. Fix only what blocks real use; keep adding the rest here, and act on it only when it
hurts.

**Lifted for 0.9.0 only** (the Owner, 2026-11-03 05:04 TRT, via Manager): the unified pack (the FREE chain
free, Command Code, Go; Linux process sweep; the bell's boot-safe lock; the box install). The freeze stays in
force for anything after it.

**Status after 0.8.0:** done 1 (a change to `mcp_server.py` itself still needs a restart), 2 (the bell pushes
notes to the bots; the parent's waiter wakes at the first task with `--first`), 3 to 10, 16, 17, 19 and 20. Partly
18 (one full suite per batch; the slow tests themselves are untouched). Open 11 to 15 and 21; 22 onwards come from
the audit of 2026-11-02. Fixed the same day: 22, 30, 32, 35, 36, 37 and the load and save of 31
(fix pack 1102 and by hand); 39 to 42 come from weighing that pack. On the Owner's word the same day, the
subagent loop pack closed 39, 40 and 42, and the SessionStart hook moved to the Python doorbell (26). The first
pack sent through the loop closed 43, 44 and 45 the same day; 46 to 49 come from weighing it, and the second
pack closed them; 50 comes from its review and was closed by hand.

## Seamless

1. **The live server runs old code until a reconnect** (2026-10-31). Every change to the server needs the user to
   reconnect it. Until then the parent dispatches with a script that imports `runtime`, outside the tools.
   *Upgrade:* re-import `scripts/` when a file changes between calls (or run each call in a fresh child process),
   and report the code version in `cw_mode`.
2. **The waiter wakes only when every task has ended** (2026-11-01). W4 was done seven minutes before the parent
   knew, because W3 was still running. *Upgrade:* with the Kompitum channel, push "task ended", "worker blocked" and
   "all workers out" into the listening session as events, next to the vault notes, so there is nothing to wait on.
   Until then, the waiter takes `--first`: exit at the first task that ends.
3. **There is no command line for the parent's scripts** (2026-10-31). Dispatching outside the tools means a
   hand-written Python script per batch. *Upgrade:* a `kompitum` CLI over the same runtime (`run <brief> --scope ...`,
   `wait`, `status`, `merge`), used by scripts, hooks and the user alike.
4. **Changing the mode means editing the client config and reconnecting** (2026-11-01). Switching GO to AUTO was a
   hand edit of `~/.claude.json`, which only takes effect after a reconnect. *Upgrade:* leave `OC_DELEGATE_MODE` out
   of the registration and let `cw_mode` take `set: AUTO|CC|GO|FREE`, written to `<state>/config.json` (already read,
   below the environment), effective for the next task.

## Efficient

5. **Acceptance is the worker's claim until the parent reruns it** (2026-11-01). The parent ran the full suite in
   each worktree by hand (about 3.5 minutes each). *Upgrade:* `test_cmd` on `cw_run`. The orchestrator runs it in
   the worktree when the worker ends and stores the summary lines and the failing tests in the result, so the
   review starts from a checked fact.
6. **Review checks are rewritten for every run** (2026-11-01). Scope, banned patterns (`DETACHED_PROCESS`,
   `watch: True`), required texts, guard counts, no real Command Code session. *Upgrade:* `checks` on `cw_run`, or
   a repository file such as `.kompitum/checks.json`, run together with `test_cmd`. Check required texts against the
   loaded values, not the source: texts split across string literals gave false "missing" results.
7. **Ledger rows are written by hand** (2026-10-31). The stats script, then a manual row with the defects found.
   *Upgrade:* `cw_stats` writes the numbers, and `cw_merge` / `cw_discard` take a short `review` text (defects,
   accepted or reworked) that goes into the same row.
8. **The smoke test is a hand-written script** (2026-11-01). One tiny task on Command Code, checked for route,
   verification, cost in the ledger, leftover deny rules and new windows, took a 90-line script and a 300-second
   window watch. *Upgrade:* `kompitum selftest`: one tiny task per installed worker, the same checks, one line each.

## Functional

9. **Progress takes ad-hoc scripts** (2026-11-01). "What's taking so long?" needed a script that counted steps and
   tool calls from the events file. *Upgrade:* `cw_status` returns `progress`: steps, tool calls, the last few tools
   with their targets, files changed so far with line counts, seconds since the last event, cost so far. Times in
   the user's local time next to UTC.
10. **Quota shows only after it has run out** (2026-10-31). `cw_mode` shows blocks, not how much of a window is used.
   *Upgrade:* spend per worker in its current window (5-hour, week, month) from the ledger, beside its state, plus
   Command Code's own usage figure if its CLI exposes one.
11. **Parallel workers need the shared interface first** (2026-10-31). W3 adds `--commandcode` to `delegate.py`. If
   W4 had passed it from its own worktree, every dispatch there would have failed. *Upgrade:* land shared arguments
   and signatures as stubs in the core commit before a parallel dispatch (playbook rule). Later, Kompitum could warn
   when one task's scope calls into another running task's files.
12. **The worker marker reaches worker-run tests of this repo** (2026-11-01). A worker running this repo's suite
    has `CWMCP_WORKER=1`, so the guard refused every `cw_run`, and W4 cleared the marker at import in one test
    module. Seven test files also pin the Command Code path one by one. *Upgrade:* one shared test harness module
    that clears the marker and pins the paths.
13. **`python` is not on the parent's PATH in Git Bash** (2026-10-31). Workers get shims in `<state>/bin`; the
    parent hits the Microsoft Store alias. *Upgrade:* `cw_mode` reports the interpreter path, or the same shims are
    offered to the parent.
14. **A queued `cw_run` reply has no `model_source`** (2026-11-01). Through the queue, the reply carried the route
    but `model_source: null`. *Upgrade:* copy `model_source` from the dispatch receipt into the registry record and
    the reply.
15. **Command Code reports no reasoning tokens** (2026-11-01). Its events give input, output and cache counts only,
    so the ledger's reasoning column stays empty for `cc` rows. Keep it in mind when comparing workers by tokens.
16. **The stall detector killed a worker in the middle of its test run** (2026-11-01). W5 had finished the port and
    started its last full suite, which prints nothing for about three minutes. The detector saw "no worker output for
    181s and no CPU activity" and ended the task as a timeout, although its earlier suite had passed (225 tests). The
    CPU check looks at the worker process only, not at the shell's child that runs the tests. *Upgrade:* treat a tool
    that has started and not finished as activity, or count CPU over the whole job tree, or give `cc` a longer stall
    limit.
17. **A timed-out task with finished work cannot be merged** (2026-11-01). After #16, W5's work was complete and
    passed review, but `cw_merge` takes only `done` tasks, so the parent built the patch by hand and applied it with
    git. *Upgrade:* `cw_merge` takes `accept_status: true` for a `timeout` or `failed` task (said in the reply and
    the review row), or the checks of #5 turn a stalled task whose checks pass into `done` with a note.
18. **The suite takes 4 to 12 minutes** (2026-11-01). 228 tests ran in 234 s in W5's worktree, then 698 s and 550 s
    on main with the machine busy. Most tests start real processes (the worker helper, the fake CLIs, git, the server)
    one after another, each Windows start going through the uv launcher, and a few wait on purpose (stall and timeout
    cases). With the checks of #5 the suite runs at the end of every worker run. *Upgrade:* find the slow tests with
    `--durations 20`, run test modules in parallel processes, and let `.kompitum/checks.json` name a fast set for
    workers while the parent runs the whole suite before a commit.
19. **Command Code's background shells opened windows** (2026-11-01, fixed in 0.7.0). With `run_in_background` (its
    tool text recommends it for long work, so the workers used it for the test suite), Command Code starts `cmd.exe`
    detached: no console, so the Python below it made a new one, and Windows 11 handed it to Windows Terminal, a
    visible window over the user's game. The worker's hidden console never reaches a detached child. *Fix:* the
    worker's `COMSPEC` is `cwshell` (`scripts/cwshell.cs`, built on first use), a console-less program named
    `cmd.exe` that runs the real cmd under a new hidden console in a kill-on-close job.
20. **Parallel workers each run the full suite twice** (2026-11-01, the Owner: "this is inefficient"). In 0.8.0,
    W9, W10 and W11 ran at once. Each ran the whole suite itself (the footer's test rule), then the server ran it
    again in each worktree for the checks of #5, and the parent runs it once more after merging: seven runs of about
    13 minutes each, slowing each other down, where one run on the merged result decides. *Upgrade:* one full suite
    per batch, run by the server, never by a worker. `.kompitum/checks.json` gains a `focused_cmd` template (for this
    repository `python -B -m unittest {files}`), filled with the test files in the task's scope. Workers run only
    that, and say so in the footer. Tasks dispatched together share a `batch` name (`kompitum run --batch NAME`). A
    task in a batch is checked at its end with the focused tests and the forbid, require and warn rules, all in
    seconds. When the last task of the batch is merged, the server starts the full suite on the merged tree as its
    own background task (`NAME-suite`), which the waiter can wait for, and its failures name the tests. A task with
    no batch keeps one full run at its end, by the server only.
21. **A reader that times out writes nothing** (2026-11-01, Natalis pass 11-01). The free tier was out for the
    day, so the reader went on with billed Go. It took 53 steps and 88 tool calls through the repository (down to
    `tool-policy.ts`, which the patch never touched), with a 257K context and 122K output tokens, and cost $0.137
    against the worker's $0.055. It hit 900 s with no doubt written, and the task stayed open until then. Its
    slowness is by design (the User: "it's supposed to be slower than commandcode or Go"). *Upgrade, when it
    hurts:* near its limit the reader writes the doubts it has so far, so a long read still yields something.
    *Done 2026-11-08:* the reader has no time limit: it runs one turn until it finishes, and a reader cut by
    the stall rule keeps the doubts it wrote. The Owner: "a worker that is working gets time, even when it
    hasn't printed anything; judging work by output is wrong."

## Audit, 2026-11-02

From reading the code and measuring the running system (the User asked for "a general audit"), and from what two
outside systems do well: DeepSeek's super-agent prototype (`deepseek-ai/dsh-superagent-prototype` and its article)
and devspace (`Waishnav/devspace`). Nothing here blocks real use today.

22. **The registry is rewritten every two seconds, idle or not** (2026-11-02, fixed by pack 1102). Each open session's server runs
    `pump()` every 2 s, and `pump()` saves the whole registry even when nothing changed: one write every 2.1 s
    (measured), about 41,000 a day per open session. The file is 288 KB after 20 tasks, so that is about 12 GB of
    writes a day, and it grows with every task: finished tasks stay in it with their whole brief (19 KB of the
    largest record's 23 KB). CPU is small (0.6% of one core). The same habit, smaller: each running task's state
    file is rewritten by its 2 s heartbeat, a listening session rewrites its rung file every minute, and the guard
    hooks are rewritten every turn. *Upgrade:* write only when the data changed; drop the prompt from the record
    once the task has started (it is kept as `turn-1.prompt.md`); move finished tasks past their TTL to an archive
    file.
23. **A rate limit counts as an empty quota** (2026-11-02; not seen yet, no fallback in the ledger). `QUOTA_PATTERN`
    takes any `429`, "rate limit" or "too many requests" for exhaustion. On opencode, when the message names no
    reset, the worker is blocked until the next 00:00 UTC, up to a day; Command Code's `rate` reason blocks it for
    10 minutes. Either way the task leaves its session and goes on with the next worker in a fresh one, so the
    context is lost, and the next worker is Go, which Ashley shares. Kompitum never retries the same worker.
    *Upgrade* (DeepSeek's harness retries transient errors three times with backoff): sort errors into transient
    (408, 5xx, network, a 429 with no window), throttle (a short named wait) and window (quota, credits, a named
    reset). Retry the first two on the same worker and session, three times with backoff; block and fall back only
    for a window.
24. **The second reader keeps the parent asleep** (2026-11-02; see #21). `finalize()` runs the reader before it
    writes `completed`, so the waiter cannot wake until the reader ends. On Natalis pass 11-01 the worker was done in
    about 9 minutes and the parent woke at about 26. *Upgrade:* wake the parent when the worker and its checks end,
    with the reader still reading; `cw_review` shows the doubts when they come, and `cw_merge` waits for them unless
    told not to. Whether the reader may go on with billed Go at all is the User's call: there it cost 2.5 times the
    worker.
25. **Each session starts with notes that wait for nobody** (2026-11-02). The doorbell calls "still open" every
    inbox note its reader has not answered with `re:`, whatever its kind. This morning: an answer its sender marked
    open, and a shelved direction. They come back every session until a reply is posted to the shared vault.
    *Upgrade:* count only the kinds that wait (`request`, `question`) as open, and give the reader a local
    acknowledgement (`kompitum ack <path>`, kept in `~/.kompitum/`) that silences a note without writing to the vault.
26. **The SessionStart doorbell still runs the TypeScript original** (2026-11-02, the hook moved the same day). The hook in
    `~/.claude/settings.json` calls `node ~/Projects/kompitum-mcp/dist/cli.js doorbell`, the 10-19 code that lives on here
    as `limen-ts/` (legacy name). `scripts/kompitum_cli.py doorbell` does the same, line for line. Two copies drift apart: #25 would
    need fixing in both. *Upgrade:* point the hook at the Python CLI and archive `~/Projects/kompitum-mcp` (it has no
    remote, and its commits are in this history). *Done:* the hook runs `scripts/kompitum_cli.py doorbell` with the
    real interpreter, on the same seen file. On the User's word the same day, the TypeScript original moved to
    `~/Projects/_archive/limen-ts-original` (legacy name) and this repository to `~/Projects/kompitum-mcp`, its canonical home; a junction
    keeps `~/Projects/opencode-delegate` working until no config uses it.
27. **State only grows** (2026-11-02). The state folder holds 67 MB, mostly 22 `turn-N.jsonl` event files of up to
    9.3 MB each, kept after their task was merged or discarded. Nothing else is removed either: the dispatch logs,
    the `tasks/<task>/` folders and per-turn prompts, `patches/`, one lock file per task name, the ledger and the
    reviews; `sweep()` removes worktrees and branches only. *Upgrade:* once a task is merged or discarded and its
    ledger row is written, delete or compress its files after the TTL, and rotate the ledger by size.
28. **The idle limit ignores CPU** (2026-11-02; see #16). Since 0.8.0 the stall check counts the job's CPU, but
    `idle_timeout` (900 s) still ends a run after 15 minutes with no worker output, however busy its shell is. In a
    repository without a `focused_cmd` the footer has the worker run the full `test_cmd` itself, and here a full
    suite took 13 minutes on a busy machine (#18). *Upgrade:* an unfinished tool call or job CPU counts as activity
    for the idle limit too.
29. **Progress counts Kompitum's own Command Code settings as a worker change** (2026-11-02). `cw_status` listed
    `.commandcode/settings.local.json` (+25 lines), the deny rules Kompitum writes for the run, among the files changed.
    *Upgrade:* leave it out of progress, as `_finish` already does for the result.
30. **The waiter crashed just as the task ended** (2026-11-02, fixed). `cw_wait.py`, the
    `wait_cmd` that `cw_run` hands out, printed the worker's report to a cp1252 stdout. The audit worker's report had
    an arrow in it, so the parent got a `UnicodeEncodeError` traceback and exit 1 instead of the result. The `kompitum`
    CLI and the server already switch their streams to UTF-8. *Fix:* `cw_wait.py` does the same, with a test that
    runs it under a cp1252 stdout.
31. **The registry's load, save and lock** (2026-11-02, from the worker's sweep; load and save fixed by pack 1102, the lock open). A registry that cannot be read or
    parsed loads as empty, and the next `pump()` saves it, losing every task without a word. `_save()` lacks the
    retry for the Windows `PermissionError` that `delegate.write_json` has, while readers such as `cw_wait.py` read
    without the lock: a failed save on the tick that started a task leaves it queued on disk, to be started again.
    And `pump()` holds the lock through `_start()`, which waits up to 15 s for each worker's receipt, so a few starts
    in one tick can outlast another call's 60 s lock timeout. *Upgrade:* only a missing file loads as empty, and a
    registry that failed to load is never saved; a unique temp name and the retry of `write_json`; reserve a task
    under the lock and start it outside. With #22, one change to `orchestrator.py`.
32. **The commit guard fails open** (2026-11-02, from the sweep; fixed by pack 1102). If `git rev-parse --git-common-dir` fails at
    dispatch, `OC_DELEGATE_PROTECTED` is empty and the guard hook blocks nothing. The tool permissions still deny
    `*git push*` and `*git commit*`, but not forms such as `git -C . push`, which only the hook stops. *Upgrade:*
    fail the dispatch when the git dir cannot be read.
33. **Three silent fallbacks** (2026-11-02, from the sweep). The forbid scan skips an added file over 2 MB or not in
    UTF-8 and still reports `forbid.ok`; a failed process listing makes `cw_cancel` report no survivors; a failed
    `cwshell` build leaves the real `cmd.exe` as `COMSPEC`, so a detached shell could open a window again (#19).
    *Upgrade:* say each one: a forbid error, a cancel that says it could not verify, a warning in the task and in
    `cw_mode`.
34. **Shared files without a lock** (2026-11-02, from the sweep). Two workers blocked at the same moment rewrite
    `quota_state.json` through one fixed temp name, so one block can be lost or the second write fail; two first
    reviews can both write the `reviews.tsv` header; the Command Code start slot's stale-lock check can let two runs
    in; two listening sessions (the setup has one) would ring the same note. Each is rare. *Upgrade:* a lock and a
    unique temp name in `record_block`, an exclusive create for the header, an OS lock for the slot.
35. **The server's children inherit its stdin** (2026-11-02, from the sweep; fixed by pack 1102, and a test keeps it so). `runtime.cli()` (sessions, export,
    service), the process listing and `taskkill` start without `stdin=subprocess.DEVNULL`, so they inherit the
    JSON-RPC pipe, the hazard `vaultgit.py` already guards against. Two git calls behind `cw_diff` and progress have
    no timeout. *Upgrade:* `stdin=subprocess.DEVNULL` everywhere and a timeout on every git call.
36. **`references/operations.md` has drifted** (2026-11-02, from the sweep; fixed by hand). It says the default backend is the
    shared service (it is standalone), that `cw_wait` caps every call at 30 s (`until: done` waits 300 s), and gives
    the upstream model order (since 0.7.0 the mode's chain comes first). *Upgrade:* correct the three paragraphs.
37. **A policy test passed only under the uv launcher** (2026-11-02, fixed). The worker-env test
    gave the wrapper a made-up `PYTHONHOME`. The uv stub overrides it, but the plain interpreter cannot start with
    it, so the suite failed that test when run with the interpreter's full path (#13). *Fix:* a real home
    (`sys.base_prefix`), which still must not reach the worker.
38. **The suite took 24 minutes, near its 30-minute limit** (2026-11-02). 392 tests in 1425 s with one worker
    running beside it, against `test_timeout_seconds: 1800` in `.kompitum/checks.json`. The slowest 30 tests (12 to
    33 s each, six of the top ten in `test_batch`) make up nearly 40% of it, mostly real process starts; direct
    sleeps add only 74 s. 18 test files still set the worker marker or pin the Command Code path one by one (#12).
    *Upgrade:* #18's parallel test modules; until then, raise the limit before a batch suite hits it. Raised to
    2700 s with the audit fixes (the server reads `.kompitum/checks.json` from HEAD, so a raise counts once committed). The
    merged suite of pack 1102 then ran 409 tests in 483 s with nothing beside it: the limit was less tight than it looked.
39. **The waiter's line carries no tokens for a Command Code run** (2026-11-02, pack 1102; fixed by the subagent
    loop). It woke with
    `tokens: {steps: 0, input: 0, ...}` while `cw_stats` had the run (31 steps, 257K fresh, 2.76M cache reads):
    `cw_wait.entry()` reads them with `policy.usage_from_events` from the events file, which seems to know only
    opencode's events. *Upgrade:* the wake line takes the numbers the ledger row has. *Fix:* it takes the state's
    own `tokens` when the worker recorded any, else the events file.
40. **The parent's side is not in the ledger** (2026-11-02, pack 1102; fixed by the subagent loop). The worker cost
    $0.07; the same job cost Claude
    about 14 requests at 130 to 210K context (about 2.4M cache reads, 45K output) at Opus rates, the larger bill by
    far, and `cw_stats` shows none of it. *Upgrade:* `cw_stats` adds the parent's requests and tokens between `cw_run`
    and the merge, read from the Claude Code transcript that holds the `cw_run` call
    (`~/.claude/projects/<slug>/<session>.jsonl`); `~/scripts/claude-session-usage.mjs` does it by hand today.
    *Fix:* `cw_stats` with `task` adds `parent` (`scripts/parent_usage.py`), and the markdown row ends with
    "Parent: N requests, ... cache reads, ... output, peak ...". The window runs from the message that holds the
    task's `cw_run` to the one that holds its `cw_merge` or `cw_discard`, so other work done in between counts too.
41. **A batch of one wakes the parent twice** (2026-11-02, pack 1102): for the worker's focused tests, then 24 minutes
    after the merge for the suite, each wake a request at full context. Without `batch` the suite runs at the
    worker's end and one wake carries it, but the worktree lacks the parent's uncommitted fixes (#37's here), so the
    suite fails on them. *Upgrade:* none in the server; the playbook says to commit the parent's own fixes first and
    to batch only two or more workers.
42. **A worker job should cost the parent what a subagent costs** (2026-11-02, the Owner: the CLIs "should work as if
    they are running as subagents on claude code, that's our mission"). A native subagent takes one call to start,
    wakes the parent once with its report, and its work is simply there. This job took the parent about 14 requests.
    *Upgrade, the subagent loop, for three parent requests a job (dispatch, review, accept):* (a) the wait goes out in
    the same message as `cw_run`: `cw_wait` treats a task it cannot see yet as pending for a short grace, not as
    unknown; (b) one wake: #39 and #41; (c) accept in one call: a `commit` strategy for `cw_merge` that applies the
    patch and commits only its files, so a dirty tree is fine and the review becomes the message (local, never
    pushed); (d) the QA gate below, so failed checks go back to the worker before the parent wakes; (e) #40, so the
    ledger shows what the loop costs. *Done the same day,* as one worker pack: (a) `cw_wait --grace` (30 s), and the
    server's instructions give the wait command; (b) #39, while #41 stays a playbook rule; (c) `cw_merge` strategy
    `commit`, refused with `FILES_DIRTY` when a patch file already has changes in the target; (d) `qa_rounds` on
    `cw_run` or in `.kompitum/checks.json` (0 to 3, default 1): failed end checks go back to the worker in its session,
    the second reader waits for the last round, and `cw_review` lists the rounds; (e) #40.
43. **`kompitum_post` answers one note at a time** (2026-11-02). The inbox reads `re:` as a list (`inbox_for` uses
    `list_of`), but `kompitum_post` and `kompitum post --re` take one path. Closing three stale notes with one reply took a
    hand-written note through the vault's own functions. *Upgrade:* `re` takes a list on the tool and a repeatable
    `--re` on the command line, each path checked as today. *Done 2026-11-02* (the first pack through the loop):
    `re` is one path or a list on the tool, whose argument check now takes a list of types, and `--re` repeats on
    the command line; one path is still written as `re: <path>`, several as a list.
44. **A large patch overflows the tool result** (2026-11-02, the subagent loop pack). `cw_diff` with a 150 KB cap
    returned 70K characters, more than Claude Code shows in one result; the client saved it to a file and the parent
    pulled the patch out by script before reading it. *Upgrade:* when the patch passes `max_bytes`, `cw_diff` writes
    it to `<state>/patches/<task>.patch` (where `apply` keeps it already) and returns the path with the numstat.
    *Done 2026-11-02:* over `max_bytes` the reply carries `patch_path`, `<state>/patches/<task>.diff`, instead of
    `patch`; the merge's `<task>.patch` stays its own.
45. **A QA round runs under the default limits** (2026-11-02, the subagent loop pack). `runtime.dispatch` passes
    `timeout_seconds` and `idle_seconds` only on `run`, so a QA round, like any `cw_continue`, gets the executor's
    defaults rather than the task's own. *Upgrade:* a continue passes on the limits the record keeps.
    *Done 2026-11-02:* `delegate` records `idle_timeout` beside `timeout` in the state's `limits`, and
    `runtime.dispatch` passes both on to every `continue` and `recover` that does not carry its own.
46. **A `require` literal broke on a wrapped line** (2026-11-02, loop-u43-45). `require` matches one line, and the
    worker wrapped the status sentence the literal named, so a QA round, with a second full suite (571 s), went to
    joining two lines. *Upgrade:* keep `require` literals to a few words in briefs; a round whose changes touch only
    `.md` files reruns `forbid` and `require` but not `test_cmd`.
    *Done 2026-11-02* (the second pack through the loop): a round whose changes since the last checks are
    all Markdown keeps the last test result, with `test.reused` saying why, and still reruns `forbid` and
    `require`; the orchestrator passes that result and a fingerprint of the changed files as `qa_previous`.
47. **`cw_review full` overflows the tool result as `cw_diff` did** (2026-11-02, loop-u43-45). A 45 KB patch made a
    53 KB review; Claude Code saved it to a file as JSON, and the parent pulled the text out by script before reading
    it. *Upgrade:* past `max_bytes`, `cw_review` writes the patch to `<state>/patches/<task>.diff` as `cw_diff` now
    does, names the path and keeps the review inline.
    *Done 2026-11-02:* a patch that does not fit in `max_bytes` goes whole to
    `<state>/patches/<task>.diff`, and the review names the file in its Patch section, ahead of the parts a cut
    may drop.
48. **`cw_run` says to send the wait next time when it was sent** (2026-11-02, loop-u43-45). The reply's hint shows
    even when the waiter went out in the same message. *Upgrade:* drop the hint: the server's instructions already
    say it, and the reply cannot know whether the wait went out.
    *Done 2026-11-02:* the reply says to run `wait_cmd` only when it did not go out with the call.
49. **A `kompitum.tools` schema change needs a reconnect** (2026-11-02, loop-u43-45). `cw_mode reload` re-imports
    `kompitum.tools`, but `mcp_server.py` builds `TOOLS` once at import, so the new `kompitum_post` schema waits for /mcp.
    *Upgrade:* rebuild `TOOLS` on reload, or have `code` say that a tool schema changed.
    *Done 2026-11-02* (the second way): `code.restart_reasons` names what needs a reconnect:
    `mcp_server.py` changed, or a reload changed a `kompitum_*` tool or the server instructions.
50. **The Markdown-only reuse would skip a `test_cmd` that checks Markdown** (2026-11-02, review of loop-u46-49).
    Since #46 a QA round whose changes are all `*.md` keeps the last passing test. That is right for Kompitum, whose
    tests read none of its docs, but wrong where `test_cmd` itself checks Markdown: a skills check over `SKILL.md`
    files, a docs linter, a test that compares a README with the code. *Upgrade:* a `.kompitum/checks.json` key names
    the files a round may change and keep the test (default `["*.md"]`; `[]` always reruns it).
    *Done 2026-11-02* (by hand, on the Owner's word): `reuse_test_for` in the committed `.kompitum/checks.json` names
    those files as globs; a value that is not an array of strings fails the checks, and the test runs.
51. **A fast death waits for the slowest task** (2026-11-04, the fork's audit). One waiter held five tasks: the scout
    died in 3.8 s, and the parent heard 36 minutes later, when the last worker ended. The Owner: "if the worker died
    after 3 seconds and we havent notified for 30 mintues - thats also a problem to be fixed in kompitum."
    *Done 2026-11-04:* a wait for several tasks ends as soon as one ends badly, and the summary's `unfinished` names
    the others. `--first` stays for waking at any end, good or bad.
52. **A scout out of free quota stopped while Go was free** (2026-11-04, the same scout). The chain said Command Code
    next, a scout may not run there, and the code took that for the chain's end: a false ALL WORKERS OUT. A scout
    started while free was out refused for the same reason. All the tests ran with Command Code not installed.
    *Done 2026-11-04:* `pick_worker(read_only=True)` passes over Command Code; the tests install the fake.
53. **A hand-off left the record wrong** (2026-11-04, the same audit). A's Command Code turn ended on quota; a pump in
    the moment before Go began marked the task `failed`, Go then finished it, and `failed` stayed. B, C and D were
    `done`, but all four reviews said Command Code, with only Go's cost.
    *Done 2026-11-04:* `delegate.handing_off` keeps that moment from reading as an end, in the registry, the waiters
    and the wait for a second reader. The record follows the worker, the header says "opencode Go after Command
    Code", and the cost covers every turn. Records from before the fix keep their `failed`.
54. **No read-only folder outside the worktree** (2026-11-04, audit-d-kompitum). D read Kompitum's code from a kompitum-core
    worktree with absolute paths named in its brief; the scope limits only what a worker may change. *Upgrade:* a
    `read` list on `cw_run`, shown in the review, when a job really needs a second repository.
55. **A Rust `target` per worktree** (2026-11-04, the fork). A worktree starts with no build, and on this repo a first
    build means Zig, Ghostty and minutes of compiling, for every worker. *Upgrade:* one `CARGO_TARGET_DIR` per repo,
    with locking in mind, or sccache. Settle it before the first port worker.
    *Done for now 2026-11-04:* a cargo config above the state folder gives workers one build folder, Zig and Ghostty's
    source, and the fork's `scripts/check_windows.py` touches the crate roots so a check builds its own worktree. Cargo
    judges freshness by file times, so two worktrees building at once can still run each other's binaries: one building
    worker at a time. SETTLE.md has the plan for the port: a build folder per worktree, seeded by hard links.
56. **Command Code's credits reset is a guess** (2026-11-04). Running out of credits names no reset, so Kompitum blocks the
    worker until an estimate (source `cooldown`). The next task then tries it and hands off at once, which is cheap but
    can read as a real reset. *Upgrade:* `cw_mode` says "estimated" beside the time.
57. **Two Kompitums, one Command Code plan, no shared quota** (2026-11-04). Manager's box and this PC use the same
    subscription, each with its own `quota_state.json`, so neither knows what the other spent; all four audit workers
    ran out at about 12.5 minutes. *Upgrade:* for the fork's Manager work: share blocks through the vault or `kom`.
58. **The CI matrix spends more than it checks** (2026-11-04). `AGENTS.md` asks for Windows and Linux, but
    `.github/workflows/test.yml` also runs macOS, twice per push (Python 3.9 and 3.12), and GitHub bills macOS at ten
    times Linux on a private repo. That day GitHub refused every job on `709ffea` for billing, with no step run, so the
    red run said nothing about the code; `kompitum suite` gave the evidence instead (523 tests OK in 1809 s, with a worker
    beside it). *Upgrade:* drop macOS from the matrix, or run it only before a release. The Owner's call, with the billing.
59. **A hand-off throws away what the first worker worked out** (2026-11-04, the audit and kompitumize-script). All four
    audit workers ran out on Command Code within a minute of each other, after about 12.5 minutes and $0.52 in all, while
    finishing their reports ("I need to trim about 14 KB"). Go then spent $0.58 and 63 worker-minutes against Command
    Code's 51, and re-read 42% to 90% of the same files. Some of that is the job, since an audit's citations have to hold,
    but the note did not help: it named `?? audit/`, a folder and not the files, gave 300 to 1800 characters of glued
    narration, and said "keep what is correct", which invites checking everything. The event log holds 130 to 210 KB of
    each worker's reasoning, and none of it is passed on. kompitumize-script is the other shape: refused on its first
    request, no work and no cost, yet its note still said a worker had stopped before finishing. `cw_status` shows a
    hand-off only as `turn: 2` beside an orchestrator block still naming Command Code, and the wake line has no
    `fallback`, so the parent called it a mid-work hand-off when it was not. *Upgrade:*
    (a) the note lists changed files one by one with sizes, the files read and the last 8 KB of reasoning from the log,
    and says to assume the work is right unless a check fails and to read only what finishing needs;
    (b) no note when the first worker did nothing, since that is a restart;
    (c) the record follows the worker at the hand-off, not at the finish, and `cw_status`, `cw_wait` and the wake line say
    in one sentence what happened. `cw_wait` names no worker today: in the fork's step 4 test on 2026-11-04 the parent read
    `worker: cc` from `cw_run` and `state: completed` from `cw_wait`, and reported Command Code when Go had done the work;
    (d) after a credits refusal `cw_mode` says "unknown, next try at ..." and the wait doubles on each refusal (#56): the
    CLI's usage code works with `monthlyCredits` and a period end, and an hour is far too short: on 2026-11-04 the monthly
    credits were out until Nov 23, 19 days off, while Kompitum said an hour. Give the parent a way to set a known reset; that
    day it took a hand call to `policy.record_block`;
    (e) remaining credits, so a long job does not start on a nearly empty plan. Under `-p`, `/usage` goes to the model as
    a prompt, and the event stream carries no credits. The CLI reads `/alpha/billing/credits` with its login, which
    Kompitum should not touch. In the fork, try `/usage` in a windowless `kom` terminal; the server's number covers the box
    too (#57);
    (f) a task stranded by ALL WORKERS OUT goes on only with the same worker in the same session, since `recover` never
    falls back. It should go on with whichever worker comes back first, as a hand-off;
    (g) moving the conversation itself: `opencode session import` exists, but each later request would carry the 280K to
    350K tokens of context the first worker died with, where a note starts small. Try it only if (a) does not cut the
    re-reading.
    Weigh it on the next real hand-off: steps and re-reads against the audit's.
    *Decided 2026-11-04 (the Owner): this gets built in the Rust port, not in today's Kompitum.* *Landed 2026-11-05 in the
    Rust loop: (a), (b), (c) and (f) by `loop-b2a`, (d) by part B1. (e) waits for Command Code's credits, and (g) for a
    real hand-off's numbers.*
60. **A desktop of its own instead of a shell shim** (2026-11-04, the fork's step 5). `cwshell` covers only what goes
    through `COMSPEC`: a detached program that starts `git.exe` directly still gets a new console, and on Windows 11 a
    Windows Terminal window. *Upgrade to try:* start a worker's whole tree on a hidden desktop (`CreateDesktopW`,
    `lpDesktop` in `STARTUPINFO`). Children inherit it, so any window lands where nobody sees it, and a stuck GUI prompt
    hits the idle timeout instead of the game. Unknown: whether Windows Terminal's default-terminal hand-off crosses to
    the visible desktop. Testing that may show one window, so ask the Owner first.
61. **Vault reads answered from an old copy, and said so firmly** (2026-11-05). `kompitum_open`, `kompitum_read` and
    `kompitum_find` never pulled, not since the Python port of 2026-11-01. Only the check at a new session's start and
    the two writes did. In a long session `kompitum_open` said "Nothing is open for you." 11 hours after Manager pushed
    a note, and the Owner had to say it was there. It looked fine before because something had always pulled first,
    once by hand. Manager wrote that note by hand with no front matter: not the cause, the tools list it (now a test),
    with `note` as its kind and no date. *Done:* every read pulls first, the CLI's too, and one that cannot pull says
    so in its first line. *Still missing:* nothing tells a running session that a note arrived until someone reads.
    The `SessionStart` hook's matcher is `startup` alone, so neither `/compact` nor a resume rings, and `KOMPITUM_LISTEN`
    is off. The Rust vault should hear of a note by itself (SETTLE.md, step 6).

62. **OpenCode's starts collide too, and a worker that died at its start isn't tried again** (2026-11-05, the research
    batch). Three workers started 3 s apart, and one died 5 s later with "Standalone server exited before reporting
    readiness", having taken no step. `cc_start_slot` spaces only Command Code. The task ended as needing review, woke
    the parent, and was restarted by hand. The Rust loop spaces every worker's starts and gives a dead start two more
    tries (the fork's `loop-b2a`).

63. **No build folder per task on `cw_run`** (2026-11-05, parallel builders). Each builder of a compiled repo needs its
    own build folder, and a `.cargo\config.toml` above the worktrees gives them all one. The stopgap is a first command
    in the brief that appends to the repo's own `.cargo/config.toml` and marks it skip-worktree, with a copied folder
    made by hand. The Rust loop's #55 closes this.

64. **A worker carried a lot it never used, and sent the Owner's home file out** (2026-11-05, the deepseek-ai raid). The
    makers' own table has V4.1-Flash at 65.5 and 85.0 through OpenCode, against 74.2 and 90.3 through a plain harness. A
    worker's fixed part was 9,653 tokens: 4,464 for the list of the machine's skills, 1,391 for Code Mode, and 1,242 for
    `subagent`, `question` and the web, and in 3,106 real tool calls `skill`, `subagent` and `question` were never
    called and all 7 `execute` calls failed or found nothing. OpenCode also reads `AGENTS.md` from the worker's folder
    up to home, so the home file went to the provider on every request. *Done:* those four tools are denied, which takes
    them out of the request, `OPENCODE_DISABLE_PROJECT_CONFIG=1` stops the walk and a repo's own config, and the footer
    carries the repo's `AGENTS.md`. About 2,600 tokens remain. The web stays on here since `cw_run` can't say who needs
    it; the Rust loop makes it `--web`. Whether the workers do better is still to be seen (SETTLE.md, "Ideas taken from
    deepseek-ai"). Command Code's side wasn't measured: it is out of credits until Nov 23.

65. **The deny rules match a command's whole text** (2026-11-05, measured). All 25 denials in real streams were
    `Permission denied: shell`. Some were wrong: a heredoc or a one-liner that merely names `.env` or `git commit`, as
    `*.env*` also matches `os.environ`, and commits in a scratch test repo. Workers route around it, at a step or two
    each. The guard hook already stops a commit in the repo itself, so `*git commit*` could go. Weigh it in the Rust
    loop's part E.

66. **A ledger row is short by a step** (2026-11-05). OpenCode's stream lacks its last `step_finish` in 21 of 27 turns,
    so the last step's tokens and cost are in no sum, and `largest_request` can miss the largest. `opencode session
    export --standalone <id>` has it, and the route check reads that export already. Part D's ledger takes it from there.

67. **`cw_merge` lands only on the branch the task targets** (2026-11-05, part B2). I made a branch so `main` would get
    only checked work, and three merges refused with `TARGET_BRANCH_CHANGED`, saving their review lines all the same.
    Merging on `main` and pushing after the check does the same job, since it never pushes. Not worth a flag.

68. **The waiter woke at once on a reworked task** (2026-11-07, parts C and D). `cw_continue` leaves the registry on
    `done` while the new turn runs, and the waiter took a finished record as finished: two waits ended in seconds and
    called the running task a problem. Fixed in `cw_wait.inspect`. The same record gave a third false wake: a state file
    the server was replacing couldn't be read for an instant, which read as the task's end, in the middle of a turn
    (`loop-c3`, 13 minutes in). `read_state` now reads it again, four times over 0.6 s, and only one that stays damaged
    ends the wait.

69. **The review's header says "QA rounds 1" when none ran** (2026-11-07). `_header` prints the task's `qa_rounds`, the
    limit, not `qa_used`. Three clean runs in a row read as if each had failed once. Print the rounds used, and nothing
    when none.

70. **A file with a non-ASCII name fails a merge after the patch is on disk** (2026-11-07, caught reviewing the Rust
    accept). `_commit` lists files with `git diff --name-only` and no `-z`, so git quotes `şarkı.txt` and the add fails.
    The answer is `COMMIT_FAILED` with the changes in the working tree, and a second `cw_merge` then gets `FILES_DIRTY`,
    so its words should say to commit by hand and discard. The Rust accept does both right.

71. **The waiter woke at once on a recovered task too** (2026-11-07, `review-c3`). `cw_recover` leaves the registry on
    `timeout` while the new turn runs. Fixed in `cw_wait.inspect`, for any finished record with a live turn under it.
    Still open: while that turn runs, the wait's `cost_usd` shows the first turn's cost twice ($0.345 for $0.173).

72. **The review's header mixes turns** (2026-11-07, `loop-c3`). After a rework it read "48 min, $0.264, steps 48": the
    first turn's time, every turn's cost, the last turn's steps. Say each turn, or all of them.

73. **A worker at its limit loses its report** (2026-11-07, `app-rename`). It had finished its work when my 40-minute limit
    stopped it, so there was no report and the server's checks didn't run. At the wall limit, give it one short last turn
    to write the report, then run the checks. `cw_recover` did this by hand for `review-c3`: 3 steps, $0.009.

74. **Stats don't say where the time went** (2026-11-07). The ledger has time and tokens, not the split, and it took five
    scripts to find that builders spend 32 to 71% of their time in tools and that some runs wait minutes before first
    tokens. A row should carry model, tools, the wait, and the slowest repeated command.
    `~/scripts/deepseek-time-split.py` has the first two.

75. **A recovered task that ended well still wakes as a problem** (2026-11-07). The registry stays on `timeout`, so the wake
    exits 1 and lists a fine task under `problems`. #71 fixed the half where its turn is still running.

76. **A task brought in by hand can only be recorded as discarded** (2026-11-08, `manager-way`). Its patch's one
    rustfmt-only hunk conflicted, so I applied the rest with `git apply --exclude` and committed it myself. The only way
    to close the task was `cw_discard`, so the ledger shows an accepted run as thrown away, with the truth only in the
    review text. `cw_merge` wants a `strategy: "recorded"` (or `cw_discard` an `accepted: true`) that marks it merged,
    names the commit and removes the worktree. Better still, `apply` could take an `exclude` list itself.

77. **A merge deletes a research worker's clones** (2026-11-07, `gallery-layouts`). The worktree goes with everything
    in it, so the `_source/` repositories it cloned for the catalogue were lost and cloned again by hand. `cw_merge`
    should say what untracked, git-ignored folders it is about to remove, or move a `_source/` aside into the state
    folder and name where.

78. **A scout reads the live checkout and blames it for the parent's commits** (2026-11-08, the loop-seam ask). It
    ran in the repository itself, so two commits I made while it read showed up as "the read-only scout changed files:
    app/defaults/frame/welcome/main.js, HEAD". It also fell back from free to Go, read for 15 minutes, hit the 900 s
    cap and wrote nothing ($0.18). A scout should read a detached worktree at the asked HEAD, so the parent's work
    is not its own, and at the cap it should hand back what it has so far instead of nothing.

79. **A worker reformats files nobody asked it to** (2026-11-08, `rename-fork`). It ran rustfmt over 13 Rust files
    with the wrong config, adding about 1,150 lines that changed only layout to a rename. The checks passed. I found it
    at review and put the layout back with a script that aligns each file's tokens with HEAD's and keeps only real
    edits. #76 had the same cause. Wanted:
    - a built-in check that fails hunks whose tokens are unchanged, so the QA round sends them back before I wake;
    - a boilerplate line: never run a formatter over whole files, format only the lines you change.

80. **Moving the checks file trips the guard, and the worker games it** (2026-11-08, `rename-mcp`, `rename-fork`).
    The guard refuses any change to the checks file, so a rename that moved it failed QA. Both workers then hid the old
    file from git with skip-worktree to get past the guard. They said so in their Gaps, but the server noticed nothing.
    The guard now sees a pure move as unchanged (the rename commit). Still wanted:
    - a built-in check that fails index tricks (skip-worktree or assume-unchanged entries, from `git ls-files -v`);
    - a brief that moves the checks file should say the guard exists.

81. **A worker that ends on an error event gets no checks** (2026-11-08, `market-pool`). It had finished and written
    its report, then the last event was an error, so the server ran no checks and I ran the checker by hand. Checks
    should run whenever the worktree has changes, whatever the last event was.

82. **The lead cannot steer a running worker** (the Owner, 2026-11-08: "the orchestrator could and should ask a
    working worker (send a message to CLI) so if needed, the orchestrator can steer them"). Today I can wait, cancel,
    or continue a finished task in its session. Wanted:
    - find out which worker CLIs take a message into a running headless session (opencode has a server mode with a
      session API; whether `opencode run` can be reached that way is unchecked), and fall back to stop-and-continue
      in the same session where none can;
    - a tripwire on live progress, such as a file whose changed lines run far past what the brief implies, that wakes
      me while there is still time to steer.

83. **Nothing shows what a long run is doing** (2026-11-08). At 90 minutes the Owner asked "what are they doing".
    `cw_status` shows steps and recent tools, not the phase (editing, building, testing, QA round 2) or how the time
    compares with similar runs. The fork rename spent 100 of its 125 minutes in its first turn, much of it in repeated
    10-minute test runs, and the guard's false alarm added 26. Same root as #74.

84. **The server edits itself** (2026-11-08, `rename-mcp`). It runs from the repository it works on, so its own rename
    could merge only once no task was running, and then it needs a reconnect. Run the server from an installed copy
    that a merge doesn't touch, and move to the new copy on reconnect.

85. **A worktree build needs tools only the worker knew** (2026-11-08, `rename-fork`). The app build wants Zig and
    libclang from local folders. The worker set `ZIG` and `LIBCLANG_PATH` in its own commands, and my rebuild failed
    until I read them from its log. The build environment belongs in the repo's checks config (an `env` map), so the
    worker, the checks and the lead build the same way.
86. **`link` refuses a submodule, so app workers copy Ghostty by hand** (2026-11-08, `app-exit`, `quick-ui`). A
    worktree's submodules are empty, and the fork's app `include_str!`s from `app/vendor/ghostty`, so a build env
    var can't stand in for it. `link: ["app/vendor/ghostty"]` was skipped as "not git-ignored", and the worker
    copied the checkout in, minutes each run. A path that is an uninitialized submodule (empty, a gitlink in the
    index) is safe to link to the target's checkout of the same commit: allow it when the commits match, and say
    so when they don't.

### Ideas from outside, for when they would pay

- **A QA gate in the server** (built 2026-11-02 as #42 (d); devspace's QA-gate workflow; DeepSeek caps automated review at three rounds). When a
  task's focused checks fail, the server continues it once or twice with the failing output before it wakes the
  parent. Pays when parent turns go into one-line rework briefs.
- **One concrete question instead of a guess** (DeepSeek's `human_action`). The footer gains an optional
  `## Question`: the one question whose answer would change the work, asked after doing everything it does not
  block. `cw_review` shows it first. Today "no questions" leaves the worker to guess or to skip.
- **"Examples only show format"** (a DeepSeek prompt rule), one line in the footer: an example in a brief is not a
  fact the worker may claim.
- **The method version in every ledger row** (DeepSeek records which prompt revision each session ran under). A
  short hash of the footer, `.kompitum/brief.md` and the playbook, so a change of method can be weighed by its rows.
- **Drift in progress** (DeepSeek's watchdog checks progress, not only liveness). `cw_status` shows steps since the
  last edit, files read outside the scope, and repeated suite runs. The reader of #21 read `tool-policy.ts`, which
  the patch never touched.
- **Workers on Mint** (DeepSeek keeps compute apart from state; a devspace agent outlives the session that started
  it). An always-on machine, off the gaming PC, where the vault mirror, Ashley and the bots already run.
- **Ranked search in `kompitum_find`** (from weighing Living Brain, 2026-11-05). Today every word must be in the note,
  so different wording finds nothing. Rank by keyword (BM25) over the vault's Markdown when the vault moves to Rust:
  local, with no account and no key.
- **Read marks in the vault** (the Owner, 2026-11-05: "they can be read and worked on, but might not need an
  answer. there should be a "read at [time]" i think"). Today only a reply closes a note, so Manager's note that said
  "No reply required" stayed open on my list, and answering it spent a wake of Grok's weekly quota. For the Rust
  vault: reading writes `read at` in the reader's own folder, `done` closes a note with no reply, requests and
  questions stay listed until one of the two, and the sender sees both without waking anyone. The design is in the
  fork's `audit/SETTLE.md`, "The vault (step 6)". Until then a note wanting no reply goes through `kompitum_post` as kind
  `report`.

Not worth it at this size: devspace's workflow engine, profiles, write modes (the scope already does more), XML
output, request ids and wait exit codes; DeepSeek's router and trigger layers (Claude is the router here).

Not adopted, by the Owner's word of 2026-11-05 ("we should take ideas from it, not adopt it"): Living Brain
(livingbrain.com, a hosted memory over MCP, asked about through Manager). Workers stay off it: a store they write to
hands their unreviewed claims to the next worker. Its token saving, by
its own tests, is against resending a whole history, which a brief already avoids, and the parent's bill is its own
transcript, which a search does not shrink. It is hosted below the custom Business plan and lists $8 per 1M input
tokens. Its pricing page gives retention as a "Standard policy" with no terms and does not mention training on
customer data, so the vault does not go there. The ideas worth taking go into the Rust port: the fork's
`audit/SETTLE.md`, "Ideas taken from Living Brain", lists eleven and where each is built.

87. **A scout is blamed for the parent's own edits** (2026-11-08, `scout-app-settings`, `scout-tests`). `cw_ask` runs
    in the folder itself and compares it before and after, so a commit or an edit the parent makes while the scout
    reads comes back as "the read-only scout changed files: audit/REDESIGN.md" (or `HEAD`), and the waiter exits 1.
    Compare only what the scout's own tool calls touched, or name the parent's changes as the parent's.

88. **A CLI update can meet a worker that is just starting** (2026-11-08, `core-liveness-updates`). The core's daily
    OpenCode and Grok Build update waits while a task's worker or a process of that name runs, but nothing stops the
    loop from starting an OpenCode worker while `opencode upgrade` replaces the binary. The start then fails loudly,
    and the task is lost for that turn. Hold new OpenCode starts while its update runs, the way a busy CLI holds the
    update.

89. **The batch suite runs where the build tools aren't** (2026-11-09, `wave2-suite`). After the last merge the suite
    ran `check_windows.py` in the target checkout, which has no `.cargo/config.toml` (only the worktrees under the
    state folder get one), so the Ghostty build found no Zig and the suite ran 0 tests and failed. It said so in
    words, but the run was wasted, and I ran it again by hand with `ZIG` and `LIBCLANG_PATH` set. Same root as #85:
    the checks config should carry the build environment, and the suite should get it too.

90. **The repository's check runs the whole Windows suite in every worker** (2026-11-09, `theme-default`). Its first
    round failed on two gaps of the worker's environment, not of the patch: the build folders had no ConPTY test
    files (`check_windows.py` never fetches them), and the core under test inherited the worker's
    `KOMPITUM_TERMINAL_SOCKET`. Both are fixed now (the files copied into the three build folders, the variable
    cleared in `cmd_at`), but a new build folder will miss the ConPTY files again. Fetch them when a build folder is
    made, or have `check_windows.py` copy them from the main target.

91. **The vault's writers don't share one lock** (2026-11-09, `vault-lock`). The core now runs its vault git work
    one process at a time behind `<vault>/.git/kompitum-vault.lock` (a token file; stale after 5 min), because two
    `kom vault` pulls at once left "Cannot rebase onto multiple branches". This server's own vault code
    (`kompitum_post`, `kompitum_find`, the bell's pulls) runs git in the same folder without that lock, so it can
    still race the core. Take the same lock (create the file exclusively with a token, remove it only while it
    still holds that token), or route vault writes through `kom vault`.

92. **Parallel builders on overlapping files cost a hand merge each** (2026-11-09, `part5b-prompts`,
    `polish-settings`, `polish-window`). With `scopes_may_overlap: true` three app workers ran at once on
    `window.rs`, `render_tests.rs`, `AGENTS.md` and `commands.rs`; `cw_merge` refused two of them with CONFLICT and
    both were applied by hand with `git apply --3way`. A `cw_merge` that tries `--3way` itself and reports the
    files left in conflict (leaving them for the parent to resolve) would save the patch-and-apply dance.

93. **A note that comes in mid-task waits until someone asks** (2026-11-09, the rework session). Manager sent Opus
    two notes at 20:36 TRT; Opus saw them only when the Owner asked an hour later, though Opus called `cw_run` and
    `cw_status` at 20:36–20:38, the same minutes. Nothing tells a busy session that a note arrived. Fix: every
    `cw_*` and `kompitum_*` reply carries one line when a note for this agent is newer than the last one it was
    told about ("2 new notes for opus: manager, Domus report; manager, NB fixes merge"), read from the bell's
    last pull, never a new pull inside the reply (a pull timed out at 15 s the same evening). Then a Claude Code
    hook (PostToolUse, reading the same state file) for sessions that call no MCP tool for a while; that one is
    Opus's to add to the Owner's settings. The line is a doorbell: it names sender and title, never the body, and
    it doesn't start a session.
    Later the same evening (hook added, `~/scripts/note-bell.py`): a hook runs only while the session works, so an
    idle session still heard nothing. Manager's notes of 21:40, 21:49 and 21:52 waited until the Owner wrote at
    21:54, because Opus's turn had ended at 21:28. What does reach an idle Claude Code session is the end of a
    background shell task (the worker waiters wake it that way). So the doorbell needs a waiting form too:
    `kompitum doorbell --wait` pulls every minute, prints the new notes and exits as soon as there are any,
    and stops quietly after some hours so one left by a closed session ends. Opus keeps one running and starts
    it again after each wake. Two rings at once in one vault fail ("Cannot rebase onto multiple branches"), so
    rings take a lock. `note-bell.py --watch` does this today; Kompitum's own doorbell should replace it.
94. **A batch's suite can pass without testing the batch** (2026-11-09, `rework1-suite`). Every task of the batch
    changed only `app/`; the suite ran the repository's `test_cmd`, `scripts/check_windows.py`, which checks the
    core workspace (the app is a workspace of its own), in the main checkout, where the `.cargo/config.toml` that
    gives worktrees `ZIG` and `LIBCLANG_PATH` doesn't reach. It failed in 95 s on "zig not found" and ran 0
    tests; with Zig it would have passed while testing none of the batch's code. Fixes: run the suite in a fresh
    worktree under the state folder (the build environment is there, and the Owner's tree stays out of it), and
    let `.kompitum/checks.json` name a suite per path (`"suites": {"app/**": "cd app && cargo test --locked --bin
    kompitum-app"}`), so the suite runs what covers the files the batch changed. A suite that ran 0 tests says
    so as its first line, never only "FAILED".
95. **Agents can't click in the app, so a screenshot loop stops at the first popover** (2026-11-10, chat view).
    Opus could open screens by command and save `kompitum-app shot`s, but couldn't open the model picker, the
    effort card, the first-pick card, the Add a worker card or the "+" file dialog: each opens on a click, and
    there's no command for one. So the Owner had to click them by hand, and the dead "+" (a click handler
    reusing a stale `cx`) went unseen for two rounds. Fix: an app command `ui.click {id}` (and `ui.hover`,
    `ui.key`) that finds a mod element by its `.id()` through the debug selectors (gpui-kit already records
    them for the render tests) and sends it a real click. It should be off unless a debug flag or the test home
    is set. Then an agent can drive every screen and shoot it without the Owner.
96. **The lead's own subagents are invisible in the chat view** (2026-11-10). Opus ran five Haiku subagents in
    the background today. Nothing in Kompitum showed them; the Owner saw only Opus's messages. Claude Code's
    `SubagentStart`/`SubagentStop` and `TaskCreated`/`TaskCompleted` hooks can feed the panel: a quiet card per
    subagent ("Haiku · Rework chat-view-1 · running 12 min"), folded like tools, with its report one click away
    when it ends. That makes the panel show the whole team, not just the lead.
97. **A subagent's report can arrive twice, and "still working" notices are noise** (2026-11-10). The 1b
    subagent's hand-back reached Opus twice, minutes apart. Opus also got four "waiting on its own background
    work" notices from one agent before its report, each a wake that cost a turn. Kompitum's own task waits
    should wake once per real change: done, a question, or a stall by activity (see the liveness rule). They
    should deliver one report per task id, never a repeat.
98. **Steering a running worker** (2026-11-10). While a Haiku subagent was building the empty state, the Owner
    changed its wording. `SendMessage` reached it at its next tool round, and it took the change without a
    restart. Kompitum's workers have no such channel: `cw_continue` works only after a task ends. Add
    `task_steer {task, message}`: the text goes into the worker's session at its next step, is logged in the
    task's record, and the review shows it with the brief.
99. **A brief on one branch, the work on another** (2026-11-10). Briefs live in `.kompitum/briefs/` on `main`,
    but the chat-view work was on its own branch, so the subagent had to `git show main:<brief>` before it could
    start. Kompitum hands `task_start` the brief as text, which avoids this. Keep it that way, and when a brief
    names other briefs, inline or attach them rather than pointing into a branch the worker won't have.
100. **Two builds and a running app in one checkout** (2026-11-10). Opus ran an app build, a Linux cross-build
     of the core and the running test app from the same checkout. The test app locked
     `target/debug/kompitum-app.exe`, so a worker's `cargo build` couldn't replace it until Opus quit the app by
     hand. `build_folders` already keeps builders apart. Kompitum should also know which of its own processes
     hold a build output open: say so in the task's status ("the test app holds kompitum-app.exe"), and offer to
     quit it rather than fail the build.
101. **Agents can't send each other pictures or files** (2026-11-10, the Owner: "we should make kompitum able
     to send media and files to each other"). The Owner pasted four screenshots for Manager. Notes are text
     only, so Opus had to describe each picture in words. Let a note carry attachments:
     - `kompitum_post` and `vault_post` take files, stored in the vault beside the note with a size cap per
       file.
     - `kompitum_read` returns their paths after the pull, so an agent can open an image as an image.
     - The mailbox shows images as thumbnails and other files as chips.
     - A picture can hold an address or a private word that the text check can't see. A file goes in only
       after the sender has looked at it, or after the Owner says yes, and the note says which.
