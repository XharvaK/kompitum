//! `kom run --contain` and the hidden shell behind it.

use super::*;
use windows_sys::Win32::System::Threading::{CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};

/// Another core in the same state and config folders, under its own session.
/// Dropped before its `Session`, so the folders stay until the core is stopped.
struct Second {
    name: String,
    root: PathBuf,
}

impl Second {
    fn new(s: &Session) -> Second {
        static N: AtomicU32 = AtomicU32::new(0);
        Second { name: format!("{}-more-{}", s.name, N.fetch_add(1, Ordering::Relaxed)), root: s.root.clone() }
    }

    fn out(&self, args: &[&str]) -> std::process::Output {
        cmd_at(Path::new(BIN), &self.name, &self.root).args(args).output().unwrap()
    }
}

impl Drop for Second {
    fn drop(&mut self) {
        let _ = self.out(&["server", "stop"]);
    }
}

/// A PowerShell script that starts a second session's core from inside a
/// terminal, then waits to be killed.
fn start_a_core(s: &Session, second: &Second) -> String {
    s.file(
        "start-a-core.ps1",
        &format!(
            "Remove-Item Env:KOMPITUM_TERMINAL_ID -ErrorAction SilentlyContinue\r\n\
             Remove-Item Env:KOMPITUM_SOCKET -ErrorAction SilentlyContinue\r\n\
             $env:KOMPITUM_SESSION = '{}'\r\n\
             & '{}' server start | Out-Null\r\n\
             Start-Sleep 120\r\n",
            second.name, BIN
        ),
    )
}

fn second_core_pid(second: &Second) -> u64 {
    let mut pid = 0;
    wait_until("a core in the second session", || {
        let out = second.out(&["server", "status"]);
        if !out.status.success() {
            return false;
        }
        pid = serde_json::from_slice::<Value>(&out.stdout).unwrap()["pid"].as_u64().unwrap();
        true
    });
    pid
}

#[test]
fn a_core_started_inside_a_contained_terminal_ends_with_it() {
    let s = Session::new();
    let second = Second::new(&s);
    let script = start_a_core(&s, &second);
    let t = s.bg(&["--contain", "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    let pid = second_core_pid(&second);
    assert!(alive(pid));
    s.json(&["kill", &t]);
    wait_until("the contained core gone", || !alive(pid));
}

#[test]
fn a_core_started_inside_a_plain_terminal_outlives_it() {
    use std::os::windows::process::CommandExt;
    // A worker's job forbids leaving, so a plain terminal can't show the
    // difference there: say so and leave the rest to a machine that allows it.
    match Command::new(real_cmd()).args(["/c", "exit"]).creation_flags(CREATE_BREAKAWAY_FROM_JOB).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => panic!("cmd /c exit without breakaway failed: {out:?}"),
        Err(e) => {
            eprintln!("this test runs inside a job that forbids leaving, as a worker's does: {e}");
            return;
        }
    }
    let s = Session::new();
    let second = Second::new(&s);
    let script = start_a_core(&s, &second);
    let t = s.bg(&["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    let pid = second_core_pid(&second);
    s.json(&["kill", &t]);
    std::thread::sleep(Duration::from_secs(3));
    assert!(alive(pid), "a plain terminal's job let the core go, and it stays gone only after a stop");
    let stopped = second.out(&["server", "stop"]);
    assert!(stopped.status.success(), "{}", String::from_utf8_lossy(&stopped.stderr));
    wait_until("the stopped core gone", || !alive(pid));
}

/// kompitum-shell started the way a detached program starts a shell: no console
/// of its own, and a command line whose quotes are cmd's business.
fn detached_shell(s: &Session, command: &str) -> std::process::Output {
    use std::os::windows::process::CommandExt;
    Command::new(env!("CARGO_BIN_EXE_kompitum-shell"))
        .current_dir(&s.root)
        .env("KOMPITUM_COMSPEC", real_cmd())
        .stdin(std::process::Stdio::null())
        .raw_arg(command)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .output()
        .unwrap()
}

#[test]
fn a_detached_shell_has_a_hidden_console_and_passes_its_exit_code() {
    let s = Session::new();
    let probe = s.file(
        "shell-console.ps1",
        r#"Add-Type 'using System; using System.Runtime.InteropServices; public static class ShellProbe {
            [DllImport("kernel32.dll")] public static extern uint GetConsoleCP();
            [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
            [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h); }'
$h = [ShellProbe]::GetConsoleWindow()
[Console]::WriteLine("console " + ([ShellProbe]::GetConsoleCP() -ne 0) + " window " + [ShellProbe]::IsWindowVisible($h))
"#,
    );
    let out = detached_shell(
        &s,
        &format!(r#"/d /s /c "powershell -NoProfile -ExecutionPolicy Bypass -File "{probe}" & exit /b 7""#),
    );
    assert_eq!(out.status.code(), Some(7), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
    assert!(text.contains("console true"), "{text}");
    assert!(text.contains("window false"), "{text}");
}

#[test]
fn killing_the_shell_alone_ends_the_command_below_it() {
    use std::os::windows::process::CommandExt;
    let s = Session::new();
    let pid_file = s.root.join("shell-child.pid");
    let child =
        s.file("shell-child.ps1", &format!("$PID | Out-File -Encoding ascii '{}'; Start-Sleep 60", pid_file.display()));
    let mut shell = Command::new(env!("CARGO_BIN_EXE_kompitum-shell"));
    shell
        .current_dir(&s.root)
        .env("KOMPITUM_COMSPEC", real_cmd())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .raw_arg(format!(r#"/d /s /c "powershell -NoProfile -ExecutionPolicy Bypass -File "{child}"""#))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    let mut running = shell.spawn().unwrap();
    wait_until("the command below started", || {
        std::fs::read_to_string(&pid_file).is_ok_and(|pid| !pid.trim().is_empty())
    });
    let pid: u64 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    running.kill().unwrap(); // the shell alone, not a tree kill
    let _ = running.wait();
    wait_until("the command below gone", || !alive(pid));
}

#[test]
fn a_contained_terminal_gets_the_hidden_shell_as_comspec() {
    let s = Session::new();
    let written = s.root.join("comspec.txt");
    let script = s.file(
        "comspec.ps1",
        &format!("[IO.File]::WriteAllLines('{0}', @($env:COMSPEC, $env:KOMPITUM_COMSPEC))", written.display()),
    );
    let t = s.bg(&["--contain", "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script]);
    wait_until("its COMSPEC", || std::fs::read_to_string(&written).is_ok_and(|text| text.lines().count() == 2));
    let text = std::fs::read_to_string(&written).unwrap();
    let mut lines = text.lines();
    let comspec = Path::new(lines.next().unwrap());
    assert_eq!(comspec.file_name().and_then(|name| name.to_str()), Some("cmd.exe"), "{text}");
    let folder = comspec.parent().unwrap();
    assert!(folder.file_name().unwrap().to_string_lossy().starts_with("shell-"), "{text}");
    assert!(folder.starts_with(s.root.join("state")), "{text}");
    assert!(comspec.is_file(), "{text}");
    assert_eq!(
        std::fs::read(comspec).unwrap(),
        std::fs::read(env!("CARGO_BIN_EXE_kompitum-shell")).unwrap(),
        "the copy is kompitum-shell"
    );
    assert_eq!(
        lines.next().unwrap().to_ascii_lowercase(),
        real_cmd().to_ascii_lowercase(),
        "the real shell is remembered: {text}"
    );
    assert_eq!(s.json(&["get", &t])["contain"], true);
}

#[test]
fn a_shell_line_in_a_contained_terminal_still_draws_on_its_screen() {
    let s = Session::new();
    // As a contained terminal itself: COMSPEC is the hidden shell, which can't
    // sit on a pseudo console, and KOMPITUM_COMSPEC names the real cmd.
    let out = cmd_at(Path::new(BIN), &s.name, &s.root)
        .env("COMSPEC", env!("CARGO_BIN_EXE_kompitum-shell"))
        .env("KOMPITUM_COMSPEC", real_cmd())
        .args(["run", "--contain", "--timeout", "20", "echo shell-line-marker & exit 5"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("shell-line-marker"), "{text}");
    assert_eq!(out.status.code(), Some(5), "{text}{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_contained_terminal_is_refused_when_the_shell_is_missing() {
    let s = Session::new();
    let exe = installed(&s);
    assert!(!s.root.join(r"inst\kompitum-shell.exe").exists());
    let out = kom_at(&s, &exe, &["run", "--contain", "cmd", "/c", "echo hi"]);
    assert!(!out.status.success(), "a contained terminal started without its shell");
    let error: Value =
        serde_json::from_slice(&out.stderr).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    assert_eq!(error["error"]["kind"], "refused", "{error}");
    assert!(error["error"]["message"].as_str().unwrap().contains("kompitum-shell.exe"), "{error}");
    let list = kom_at(&s, &exe, &["--json", "ls"]);
    assert!(list.status.success(), "{}", String::from_utf8_lossy(&list.stderr));
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(listed["terminals"].as_array().map(Vec::len), Some(0), "a refused run left a terminal: {listed}");
}

/// The limits of the job a terminal's program sits in, as that program reads them.
fn limits_of_its_job(s: &Session, contain: bool) -> u32 {
    let written = s.root.join(format!("job-limits-{contain}.txt"));
    let script = s.file(
        &format!("job-limits-{contain}.ps1"),
        &format!(
            r#"Add-Type 'using System; using System.Runtime.InteropServices; public static class JobProbe {{
            [DllImport("kernel32.dll")] public static extern bool QueryInformationJobObject(IntPtr job, int kind, byte[] info, int size, IntPtr got); }}'
$info = New-Object byte[] 144
if (-not [JobProbe]::QueryInformationJobObject([IntPtr]::Zero, 9, $info, 144, [IntPtr]::Zero)) {{ exit 1 }}
[IO.File]::WriteAllText('{}', [BitConverter]::ToUInt32($info, 16).ToString())
"#,
            written.display()
        ),
    );
    let mut args = vec!["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script.as_str()];
    if contain {
        args.insert(0, "--contain");
    }
    s.bg(&args);
    let mut limits = 0;
    wait_until("its job's limits", || {
        std::fs::read_to_string(&written).is_ok_and(|text| text.trim().parse().map(|value| limits = value).is_ok())
    });
    limits
}

// The two tests above can't tell the terminals apart inside a job that forbids
// leaving, as a worker's and Claude's app's do. This one reads the job itself.
#[test]
fn only_a_plain_terminal_lets_a_program_break_away() {
    use windows_sys::Win32::System::JobObjects::{JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
    let s = Session::new();
    let plain = limits_of_its_job(&s, false);
    let contained = limits_of_its_job(&s, true);
    assert_ne!(plain & JOB_OBJECT_LIMIT_BREAKAWAY_OK, 0, "{plain:#x}");
    assert_eq!(contained & JOB_OBJECT_LIMIT_BREAKAWAY_OK, 0, "{contained:#x}");
    assert_ne!(plain & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, 0, "{plain:#x}");
    assert_ne!(contained & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, 0, "{contained:#x}");
}
