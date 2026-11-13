//! What differs between Unix (Linux, macOS) and Windows, behind one small
//! surface: connections, terminals, process trees, the console, files.
//! Everything above it is shared.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(target_os = "linux")]
pub mod systemd;

#[cfg(target_os = "macos")]
mod macos_bundle;
#[cfg(target_os = "macos")]
mod macos_launchd;
#[cfg(target_os = "macos")]
mod macos_responsibility;
#[cfg(target_os = "macos")]
mod macos_shutdown;

/// Run before threads or runtime setup: a launchd launcher forks the core here.
pub fn daemon_startup() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_bundle::installer_startup();
        macos_responsibility::holder_startup();
        macos_launchd::startup()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// A parsed serve may have come from an older updater's direct spawn.
pub fn prepare_core() {
    #[cfg(target_os = "macos")]
    macos_launchd::prepare_core();
}

/// Daemons must not keep launchd's 256-fd or systemd's 1024-fd soft limit.
pub(crate) fn raise_nofile_limit() {
    #[cfg(unix)]
    if let Err(error) = unix::raise_nofile_limit() {
        eprintln!("cannot raise open file limit; using inherited limit: {error}");
    }
}

pub fn core_executable(exe: &std::path::Path) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        macos_bundle::core_executable(exe)
    }
    #[cfg(not(target_os = "macos"))]
    exe.to_path_buf()
}

pub fn prefer_release_bundle() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::env::current_exe().is_ok_and(|exe| macos_bundle::bundle_destination(&exe))
    }
    #[cfg(not(target_os = "macos"))]
    true
}

pub fn needs_bundle_update() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_bundle::needs_bundle_update()
    }
    #[cfg(not(target_os = "macos"))]
    false
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn legacy_bundle_notice(state: &std::path::Path) {
    #[cfg(target_os = "macos")]
    macos_bundle::legacy_notice(state);
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn verify_release_bundle(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if macos_bundle::source_is_bundle(path) {
        let local = std::env::var("KOMPITUM_RELEASES_URL")
            .is_ok_and(|url| url.trim_end_matches('/') != "https://xharvak.github.io/kompitum");
        macos_bundle::verify(path.parent().unwrap().parent().unwrap().parent().unwrap(), local)?;
    }
    Ok(())
}

/// macOS cores leave the app's process coalition; holders inherit the core's.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn spawn_core_job(cmd: &mut std::process::Command, log: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    if cmd.get_args().next() == Some(std::ffi::OsStr::new("serve")) {
        return macos_launchd::spawn(cmd, log);
    }
    false
}

pub fn update_source(path: &std::path::Path) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    if path.is_dir() {
        return path.join(macos_bundle::EXECUTABLE);
    }
    path.to_path_buf()
}

pub fn bundle_update_hint() -> Option<&'static str> {
    cfg!(target_os = "macos")
        .then_some("use kom update, or --from 'Kompitum Core.app' (its Contents/MacOS/kompitum-core also works)")
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn privacy_upgrade_notice(previous_exe: &std::path::Path) -> &'static str {
    #[cfg(target_os = "macos")]
    if !macos_bundle::source_is_bundle(previous_exe) {
        return "; recreate terminals started before the Kompitum Core.app upgrade if macOS privacy access is broken (they were not stopped)";
    }
    ""
}

/// Keep the early shutdown watch alive for the core's lifetime.
pub fn monitor_host_shutdown() -> impl Sized {
    #[cfg(target_os = "macos")]
    {
        macos_shutdown::monitor()
    }
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
#[cfg(windows)]
mod windows_contain;
#[cfg(windows)]
mod windows_exec;
#[cfg(windows)]
pub use windows_contain::*;
#[cfg(windows)]
pub use windows_exec::*;
#[cfg(windows)]
mod windows_env;
#[cfg(windows)]
mod windows_session;

/// Whether the host is shutting down (Windows also includes logout).
pub fn host_shutdown_in_progress() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};
        // Keep USER32 statically linked: first loading it during shutdown can interrupt exit recording.
        unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) != 0 }
    }
    #[cfg(target_os = "macos")]
    {
        macos_shutdown::in_progress()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}

/// What marks a process as an ssh login's.
pub const SSH_LOGIN: &[&str] = &["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];

/// This process's environment, without what isn't UTF-8.
pub fn utf8_vars() -> impl Iterator<Item = (String, String)> {
    std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
}

/// The core's environment as programs get it: never an ssh login's marks,
/// and on Linux the login session as the user manager has it now.
pub async fn program_env() -> std::collections::BTreeMap<String, String> {
    #[allow(unused_mut)]
    let mut env: std::collections::BTreeMap<String, String> = utf8_vars().collect();
    drop_claude_session(&mut env);
    #[cfg(target_os = "linux")]
    {
        env.retain(|k, _| !systemd::SERVICE.contains(&k.as_str()));
        if let Ok(Some(manager)) = tokio::task::spawn_blocking(systemd::manager_env).await {
            systemd::apply_session(&mut env, &manager);
        }
    }
    #[cfg(windows)]
    {
        env = tokio::task::spawn_blocking(move || {
            if let Err(error) = windows_env::refresh(&mut env) {
                eprintln!("cannot refresh Windows environment; using inherited settings: {error}");
            }
            env
        })
        .await
        .expect("environment reader panicked");
    }
    env.retain(|k, _| !SSH_LOGIN.contains(&k.as_str()));
    drop_unreachable_displays(&mut env);
    env
}

/// What a Claude Code session sets for the programs it starts; `CLAUDECODE` marks them. A core started inside one
/// (by `kom` from Claude's own shell) would hand that session to every terminal, and a Claude started there then takes
/// itself for that session's child and saves no transcript, so the lead can't resume its conversation. Every `CLAUDE*`
/// goes while `CLAUDECODE` is set (the Owner's call, 2026-11-13). On Windows the registry's values are added back
/// afterwards, so a variable the user set for themselves stays.
pub fn drop_claude_session(env: &mut std::collections::BTreeMap<String, String>) {
    if env.keys().any(|key| key.eq_ignore_ascii_case("CLAUDECODE")) {
        env.retain(|key, _| !key.to_ascii_uppercase().starts_with("CLAUDE"));
    }
}

/// 32 random hex digits (ids, socket and folder names).
pub fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    random_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Signal numbers as the holder protocol carries them (the Unix ones; the
/// Windows holder maps them onto its own means).
pub mod signal {
    pub const HUP: i32 = 1;
    pub const INT: i32 = 2;
    pub const KILL: i32 = 9;
    pub const TERM: i32 = 15;
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_claude_session_that_started_the_core_reaches_no_terminal() {
        let vars = |pairs: &[(&str, &str)]| -> std::collections::BTreeMap<String, String> {
            pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
        };
        let mut env = vars(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_CHILD_SESSION", "1"), ("Claude_Pid", "7"), ("PATH", "/bin")]);
        super::drop_claude_session(&mut env);
        assert_eq!(env, vars(&[("PATH", "/bin")]));

        let mut own = vars(&[("CLAUDE_CONFIG_DIR", "/mine"), ("PATH", "/bin")]);
        super::drop_claude_session(&mut own);
        assert_eq!(own.len(), 2, "without a session's mark, nothing goes: {own:?}");
    }
}
