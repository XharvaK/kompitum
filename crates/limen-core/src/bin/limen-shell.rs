//! limen-shell: the COMSPEC of a contained terminal.
//!
//! Some programs start cmd.exe detached, so cmd has no console, and the first
//! console program below it makes a new, visible one. This program is built for
//! the GUI subsystem, so it starts fine with no console. It runs the real cmd
//! with the same arguments under a new hidden console that every program below
//! it inherits, passes its own standard handles through, and exits with cmd's
//! exit code.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    std::process::exit(shell::run());
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod shell {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::PathBuf;
    use std::ptr::null;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Environment::GetCommandLineW;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, GetExitCodeProcess, INFINITE, PROCESS_INFORMATION,
        ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
    };

    /// The real cmd: `LIMEN_COMSPEC` (the terminal's), else Windows' own.
    fn real_shell() -> PathBuf {
        std::env::var_os("LIMEN_COMSPEC").filter(|value| !value.is_empty()).map(PathBuf::from).unwrap_or_else(|| {
            let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
            PathBuf::from(root).join(r"System32\cmd.exe")
        })
    }

    /// Everything after the program's own name, exactly as the parent wrote it
    /// (cmd's `/s /c` rules depend on the exact quotes, so nothing is parsed or
    /// re-quoted).
    fn arguments(line: &[u16]) -> &[u16] {
        let quote = u16::from(b'"');
        let mut at = 0;
        if line.first() == Some(&quote) {
            at = line[1..].iter().position(|&c| c == quote).map_or(line.len(), |close| close + 2);
        } else {
            while at < line.len() && line[at] != u16::from(b' ') && line[at] != u16::from(b'\t') {
                at += 1;
            }
        }
        while at < line.len() && (line[at] == u16::from(b' ') || line[at] == u16::from(b'\t')) {
            at += 1;
        }
        &line[at..]
    }

    pub fn run() -> i32 {
        let line = unsafe {
            let at = GetCommandLineW();
            std::slice::from_raw_parts(at, (0..).take_while(|&i| *at.add(i) != 0).count())
        };
        let mut command = vec![u16::from(b'"')];
        command.extend(real_shell().as_os_str().encode_wide());
        command.push(u16::from(b'"'));
        let rest = arguments(line);
        if !rest.is_empty() {
            command.push(u16::from(b' '));
            command.extend_from_slice(rest);
        }
        command.push(0);

        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        // Its own three standard handles, so a caller's pipes still carry cmd's output.
        startup.dwFlags = STARTF_USESTDHANDLES;
        startup.hStdInput = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        startup.hStdOutput = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        startup.hStdError = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let started = unsafe {
            CreateProcessW(
                null(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                CREATE_NO_WINDOW | CREATE_SUSPENDED,
                null(),
                null(),
                &startup,
                &mut process,
            )
        };
        if started == 0 {
            return 9009; // cmd's own "not recognized" code
        }
        // This job only adds to the terminal's own: if it can't be made, that
        // one still ends everything below this process, so carry on.
        let job = unsafe { keep_in_job(process.hProcess) };
        if unsafe { ResumeThread(process.hThread) } == u32::MAX {
            unsafe { TerminateProcess(process.hProcess, 1) };
            return 9009;
        }
        let mut code = 0u32;
        unsafe {
            WaitForSingleObject(process.hProcess, INFINITE);
            GetExitCodeProcess(process.hProcess, &mut code);
            CloseHandle(process.hThread);
            CloseHandle(process.hProcess);
        }
        // Closing the job (as this process ends) kills whatever cmd left running.
        drop(job);
        code as i32
    }

    /// The job that ends what cmd started when this process alone ends. Its
    /// failure is ignored, and it stays open until `run` returns.
    unsafe fn keep_in_job(process: windows_sys::Win32::Foundation::HANDLE) -> Option<OwnedHandle> {
        unsafe {
            let job = CreateJobObjectW(null(), null());
            if job.is_null() {
                return None;
            }
            let job = OwnedHandle::from_raw_handle(job as RawHandle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 {
                return None;
            }
            if AssignProcessToJobObject(job.as_raw_handle() as _, process) == 0 {
                return None;
            }
            Some(job)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::arguments;

        fn wide(text: &str) -> Vec<u16> {
            text.encode_utf16().collect()
        }

        fn rest(line: &str) -> String {
            String::from_utf16(arguments(&wide(line))).unwrap()
        }

        #[test]
        fn arguments_are_everything_after_the_program_name_exactly_as_written() {
            assert_eq!(rest(r#""C:\dir\limen-shell.exe" /d /s /c "echo hi""#), r#"/d /s /c "echo hi""#);
            assert_eq!(rest(r"C:\dir\limen-shell.exe /d /s /c exit"), "/d /s /c exit");
            assert_eq!(rest("limen-shell.exe\t/d\t/c\texit"), "/d\t/c\texit");
            assert_eq!(rest(r"C:\dir\limen-shell.exe"), "", "no arguments");
            assert_eq!(rest(r#""C:\dir\limen-shell.exe /d /s"#), "", "an unclosed quote");
        }
    }
}
