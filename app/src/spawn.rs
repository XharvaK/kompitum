//! Starting programs. The app has no console on Windows, so each console
//! program it starts (git, kom, curl) would open a window of its own.

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs a command with its output captured, waiting at most `limit` for it to
/// end. A run that takes too long is killed (the child itself) and says so.
pub fn output_within(mut command: Command, limit: Duration) -> std::io::Result<(Output, bool)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = tx.send(bytes);
        });
        rx
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let start = Instant::now();
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status, false),
            Ok(None) if start.elapsed() >= limit => {
                let _ = child.kill();
                break (child.wait()?, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    };
    // A grandchild can hold the pipes after its parent is gone: don't wait forever for them.
    let deadline = Instant::now() + Duration::from_secs(2);
    let collect = |rx: mpsc::Receiver<Vec<u8>>| {
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
    };
    let output = Output {
        status,
        stdout: collect(stdout),
        stderr: collect(stderr),
    };
    Ok((output, timed_out))
}

/// Ends a process and everything it started.
pub fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = command("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output_of(program: &str, args: &[&str], limit: Duration) -> (Output, bool) {
        let mut command = command(program);
        command.args(args);
        output_within(command, limit).unwrap()
    }

    #[test]
    fn captured_output_comes_back() {
        #[cfg(windows)]
        let (output, timed_out) = output_of(
            "cmd.exe",
            &["/d", "/c", "echo out & echo err 1>&2"],
            Duration::from_secs(5),
        );
        #[cfg(unix)]
        let (output, timed_out) =
            output_of("sh", &["-c", "printf out; printf err >&2"], Duration::from_secs(5));
        assert!(!timed_out);
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "out");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "err");
    }

    #[test]
    fn a_command_that_runs_too_long_is_killed() {
        #[cfg(windows)]
        let (output, timed_out) = output_of(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ],
            Duration::from_secs(1),
        );
        #[cfg(unix)]
        let (output, timed_out) =
            output_of("sh", &["-c", "sleep 30"], Duration::from_millis(200));
        assert!(timed_out, "{output:?}");
    }
}
