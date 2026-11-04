//! What core refuses: nothing types into a person's terminal, a blocked
//! program takes no typing without `--force`, a program cannot pose as another
//! terminal, and a file call stays inside its root.

use super::*;

/// The error a command failed with, as JSON on stderr.
fn err(s: &Session, args: &[&str]) -> Value {
    let out = cmd(&s.name, &s.root).args(args).output().unwrap();
    assert!(!out.status.success(), "{args:?} should have failed");
    serde_json::from_slice(&out.stderr)
        .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stderr)))
}

/// A PowerShell program that says `ready`, then prints every line it reads.
fn echo_lines(s: &Session, name: &str) -> String {
    s.file(
        name,
        "Write-Host 'ready'\r\n\
         while ($true) {\r\n\
           $l = [Console]::In.ReadLine()\r\n\
           if ($null -eq $l) { break }\r\n\
           [Console]::Out.WriteLine(\"got:$l\")\r\n\
           [Console]::Out.Flush()\r\n\
         }\r\n",
    )
}

#[test]
fn nothing_types_into_a_persons_terminal() {
    let s = Session::new();
    let script = echo_lines(&s, "echo-lines.ps1");
    let t = s.bg(&["--person", "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    s.shows(&t, "ready");
    for args in [
        vec!["send", t.as_str(), "hello"],
        vec!["send", "--force", t.as_str(), "hello"],
        vec!["keys", t.as_str(), "a", "enter"],
        vec!["keys", "--force", t.as_str(), "a", "enter"],
        vec!["interrupt", t.as_str()],
        vec!["interrupt", "--force", t.as_str()],
    ] {
        assert_eq!(err(&s, &args)["error"]["kind"], "refused", "{args:?}");
    }
    assert!(!s.read(&t).contains("got:"), "something was typed into it: {}", s.read(&t));
    assert_eq!(s.json(&["get", &t])["person"], true);

    // A viewer's keys are a person's own: they still arrive.
    let outer = s.bg(&[BIN, "attach", &t]);
    s.shows(&outer, "ready");
    s.typed(&outer, "hello");
    s.shows(&t, "got:hello");

    // Without `--person` the same program takes a send.
    let plain = s.bg(&["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    s.shows(&plain, "ready");
    assert_eq!(s.json(&["get", &plain])["person"], false);
    s.json(&["send", &plain, "hello"]);
    s.shows(&plain, "got:hello");
}

#[test]
fn a_childs_news_is_not_typed_into_a_persons_terminal() {
    let s = Session::new();
    let script = s.file(
        "reports-and-reads.ps1",
        r#"$e = [char]27
[Console]::Out.Write("$e]7501;state=idle$e\")
[Console]::Out.WriteLine("ready")
[Console]::Out.Flush()
while ($true) {
  $l = [Console]::In.ReadLine()
  if ($null -eq $l) { break }
  [Console]::Out.WriteLine("read:$l")
  [Console]::Out.Flush()
}
"#,
    );
    let child_of = |parent: &str| {
        let started = s.json(&["run", "--bg", "--parent", parent, "cmd", "/c", "exit", "0"]);
        started["address"].as_str().unwrap().to_string()
    };

    // Plainly, the note about a child goes into its parent.
    let plain = s.bg(&["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    s.shows(&plain, "ready");
    let kid = child_of(&plain);
    s.shows(&plain, "<lc>");
    assert!(s.read(&plain).contains(&kid), "the note names the child: {}", s.read(&plain));

    // A person's terminal takes no notes: nothing is typed in five seconds.
    let person = s.bg(&["--person", "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    s.shows(&person, "ready");
    let quiet = child_of(&person);
    std::thread::sleep(Duration::from_secs(5));
    assert!(!s.read(&person).contains("<lc>"), "a note was typed into a person's terminal: {}", s.read(&person));

    // Its next wait is how it hears of that child.
    let out = cmd(&s.name, &s.root)
        .env("LIMEN_TERMINAL_ID", &person)
        .args(["--json", "wait", "--children", "any", "--timeout", "20"])
        .output()
        .unwrap();
    let w: Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    let kids = w["children"].as_array().cloned().unwrap_or_default();
    assert!(kids.iter().any(|c| c["terminal"] == quiet.as_str()), "the wait reports the child: {w}");
}

#[test]
fn a_persons_terminal_takes_no_first_message() {
    let s = Session::new();
    let templates = s.root.join("config/limen-core/templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(
        templates.join("reviewer.md"),
        "---\ndescription: r\ncommand: cmd /d /c ping -n 120 127.0.0.1\n---\nYou review code.\n",
    )
    .unwrap();
    let e = err(&s, &["run", "--bg", "--person", "reviewer", "review src/x.rs"]);
    assert_eq!(e["error"]["kind"], "invalid_params", "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("nothing types into it"), "{e}");
    let listed: Value = s.json(&["ls"]);
    assert_eq!(listed["terminals"].as_array().map(Vec::len), Some(0), "a refused run left a terminal: {listed}");
}

#[test]
fn a_send_to_a_blocked_program_goes_in_when_it_leaves_blocked() {
    let s = Session::new();
    let flag = s.root.join("let-it-go");
    let script = s.file(
        "waits-for-a-flag.ps1",
        &format!(
            r#"$e = [char]27
function report($s) {{ [Console]::Out.Write("$e]7501;$s$e\"); [Console]::Out.Flush() }}
report "state=blocked:kind=permission"
[Console]::Out.WriteLine("ready")
[Console]::Out.Flush()
while (-not (Test-Path '{}')) {{ Start-Sleep -Milliseconds 100 }}
report "state=idle"
while ($true) {{
  $l = [Console]::In.ReadLine()
  if ($null -eq $l) {{ break }}
  [Console]::Out.WriteLine("got:$l")
  [Console]::Out.Flush()
}}
"#,
            flag.display()
        ),
    );
    let t = s.bg(&["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    s.shows(&t, "ready");
    wait_until("blocked", || s.json(&["get", &t])["status"]["state"] == "blocked");
    let held = s.json(&["send", &t, "held-one"]);
    assert_eq!(held["queued"], true, "{held}");
    assert_eq!(held["status"], "blocked", "{held}");
    assert!(!s.read(&t).contains("held-one"), "typed while blocked");
    // A status check on a blocked program is still an error, and holds nothing.
    let e = err(&s, &["send", &t, "checked", "--if", "idle"]);
    assert_eq!(e["error"]["kind"], "blocked", "{e}");
    std::fs::write(&flag, "").unwrap();
    s.shows(&t, "got:held-one");
    assert!(!s.read(&t).contains("checked"), "a refused status check was held too");
}

#[test]
fn keys_and_interrupt_refuse_a_blocked_program() {
    let s = Session::new();
    let script = s.file(
        "blocked.ps1",
        r#"$e = [char]27
[Console]::Out.Write("$e]7501;state=blocked:kind=permission$e\")
[Console]::Out.WriteLine("ready")
[Console]::Out.Flush()
while ($true) {
  $l = [Console]::In.ReadLine()
  if ($null -eq $l) { break }
  [Console]::Out.WriteLine("got:$l")
  [Console]::Out.Flush()
}
"#,
    );
    let t = s.bg(&["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    s.shows(&t, "ready");
    wait_until("blocked", || s.json(&["get", &t])["status"]["state"] == "blocked");
    for args in [vec!["keys", t.as_str(), "a", "enter"], vec!["interrupt", t.as_str()]] {
        let out = cmd(&s.name, &s.root).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(9), "{args:?}");
        let e: Value = serde_json::from_slice(&out.stderr).unwrap();
        assert_eq!(e["error"]["kind"], "blocked", "{args:?}: {e}");
        let said = format!("{}{}", e["error"]["message"], e["error"]["hint"]);
        assert!(!said.contains("--force"), "the hint invites --force: {said}");
    }
    assert!(!s.read(&t).contains("got:"), "a refused key reached the program");
    s.json(&["keys", "--force", &t, "a", "enter"]);
    s.shows(&t, "got:a");
}

#[test]
fn a_program_cannot_pose_as_another_terminal() {
    let s = Session::new();
    let a = s.bg(&["cmd", "/c", "ping", "-n", "120", "127.0.0.1"]);
    let b = s.bg(&["cmd", "/c", "ping", "-n", "120", "127.0.0.1"]);
    // A hand-set caller that doesn't own the scope it shows is refused.
    let out = cmd(&s.name, &s.root)
        .env("LIMEN_TERMINAL_ID", &a)
        .env("LIMEN_SCOPE", "wrong")
        .args(["--json", "read", &b])
        .output()
        .unwrap();
    assert!(!out.status.success(), "a posing call was answered: {}", String::from_utf8_lossy(&out.stdout));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["kind"], "denied", "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains(&format!("names {a} as its caller")), "{e}");
    // No scope shown: a call from outside any terminal is taken at its word.
    let out = cmd(&s.name, &s.root).env("LIMEN_TERMINAL_ID", &a).args(["--json", "read", &b]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    // A program in a terminal inherits its scope: naming another terminal gives it away.
    let codes = s.root.join("codes.txt");
    let script = s.file(
        "pose.ps1",
        &format!(
            "$env:LIMEN_TERMINAL_ID = '{a}'\r\n\
             & '{BIN}' read '{b}' | Out-Null\r\n\
             $posed = $LASTEXITCODE\r\n\
             Remove-Item Env:LIMEN_TERMINAL_ID\r\n\
             & '{BIN}' read '{b}' | Out-Null\r\n\
             $plain = $LASTEXITCODE\r\n\
             [IO.File]::WriteAllText('{}', \"$posed,$plain\")\r\n",
            codes.display()
        ),
    );
    s.bg(&["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    wait_until("its exit codes", || std::fs::read_to_string(&codes).is_ok_and(|c| c.lines().count() == 1));
    assert_eq!(std::fs::read_to_string(&codes).unwrap().trim(), "1,0", "a program posed as another terminal");
}

#[test]
fn a_file_call_stays_inside_its_root() {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    let s = Session::new();
    let inside = s.root.join("work");
    let shared = s.root.join("shared");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
    let junction = inside.join("junction");
    let made = Command::new(real_cmd())
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&shared)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(made.status.success(), "mklink: {}", String::from_utf8_lossy(&made.stderr));

    let (cwd, root) = (inside.display().to_string(), s.root.display().to_string());
    let e = err(&s, &["file", "write", "--cwd", &cwd, "junction/x"]);
    assert_eq!(e["error"]["kind"], "outside_root", "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("is outside"), "{e}");
    assert!(!shared.join("x").exists(), "the write went through the junction");

    // With the shared parent as the root, it may reach there.
    let out =
        cmd(&s.name, &s.root).args(["file", "write", "--cwd", &cwd, "--root", &root, "junction/x"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(shared.join("x").exists(), "--root on the shared parent did not reach");

    // Reading and editing stay inside too.
    assert_eq!(err(&s, &["file", "read", "--cwd", &cwd, "junction/x"])["error"]["kind"], "outside_root");
    let edit = err(&s, &["file", "edit", "--cwd", &cwd, "junction/x", "--old", "a", "--new", "b"]);
    assert_eq!(edit["error"]["kind"], "outside_root", "{edit}");
}
