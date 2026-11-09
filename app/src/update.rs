//! The app's own updates. An installed app (Linux: `~/.local/share/kompitum-app`,
//! macOS: `Kompitum.app`) checks `xharvak.github.io/kompitum/app/latest.json` a little
//! after it opens and every hour, downloads a newer build for this
//! machine, checks its sha256 and unpacks it next to itself (`<install>.new`).
//! The frame then shows "Restart"; the swap happens on restart or the next
//! quit, or before opening the next window after a crash. The first start
//! after a swap runs `kom update` for the core.
//!
//! A build from source never updates: only `scripts/package` (the release
//! builds) sets `KOMPITUM_APP_RELEASE`. `scripts/install-macos` makes a
//! Kompitum.app too, so the install's shape alone can't tell.
//!
//! The core is checked on the same clock (`core/latest.json`): a newer core
//! of the same generation is offered (also applied after an app update), unless
//! a package manager owns it. Within a generation a newer core only adds.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

const EVERY: Duration = Duration::from_secs(60 * 60);
/// After a failed check or download (offline, a damaged file).
const RETRY: Duration = Duration::from_secs(30 * 60);

/// Where this app is installed: the folder (Linux) or bundle (macOS) an
/// update replaces. Read once: after a swap this process's own file is gone.
fn root() -> Option<PathBuf> {
    static ROOT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    ROOT.get_or_init(find_root).clone()
}

fn find_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    if cfg!(target_os = "macos") {
        // Kompitum.app/Contents/MacOS/Kompitum
        let bundle = dir.parent()?.parent()?;
        (bundle.extension()? == "app").then(|| bundle.to_path_buf())
    } else {
        // The release folder holds the binary and its icon; cargo's target/ doesn't.
        dir.join("kompitum-app.svg")
            .is_file()
            .then(|| dir.to_path_buf())
    }
}

/// The folder or bundle name inside a release tarball.
const PACKED: &str = if cfg!(target_os = "macos") {
    "Kompitum.app"
} else {
    "kompitum-app"
};

fn beside(root: &Path, suffix: &str) -> PathBuf {
    let mut name = root.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    // macOS: Spotlight skips a folder ending in `.noindex`, so a staged
    // `Kompitum.app.new` never shows up in search under that name.
    if cfg!(target_os = "macos") {
        name.push(".noindex");
    }
    root.with_file_name(name)
}

fn target() -> String {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn releases() -> String {
    std::env::var("KOMPITUM_RELEASES_URL")
        .unwrap_or_else(|_| "https://xharvak.github.io/kompitum".into())
}

/// The app download's stage: idle, looking, a version waiting for a restart, or
/// the version this process swapped to (nothing newer can be ready again).
#[derive(Clone, Debug, PartialEq)]
pub enum AppStage {
    Idle,
    Checking,
    Ready(String),
    Applied(String),
}

impl AppStage {
    fn word(&self) -> &'static str {
        match self {
            AppStage::Idle => "idle",
            AppStage::Checking => "checking",
            AppStage::Ready(_) => "ready",
            AppStage::Applied(_) => "applied",
        }
    }

    fn ready(&self) -> Option<String> {
        match self {
            AppStage::Ready(version) => Some(version.clone()),
            _ => None,
        }
    }
}

/// The core's update stage. Its version and owner are facts beside it.
#[derive(Clone, Debug, PartialEq)]
pub enum CoreStage {
    Idle,
    Ready(String),
    Updating(String),
    Failed(String),
}

impl CoreStage {
    fn word(&self) -> &'static str {
        match self {
            CoreStage::Idle => "idle",
            CoreStage::Ready(_) => "ready",
            CoreStage::Updating(_) => "updating",
            CoreStage::Failed(_) => "failed",
        }
    }
}

struct CoreUpdate {
    version: Option<String>,
    /// A package manager that owns it (homebrew, nix, mise): never updated by the app.
    managed: Option<String>,
    stage: CoreStage,
}

/// Every update stage in one place, so they cannot disagree.
struct Updates {
    app: AppStage,
    core: CoreUpdate,
    /// Unix seconds of the last successful check; a failed check clears it.
    checked: Option<u64>,
}

impl Updates {
    /// Only while a pass runs; an applied app says so until it restarts.
    fn begin_check(&mut self) {
        // A ready update stays ready while the next pass looks, so a quit still applies it.
        if matches!(self.app, AppStage::Idle) {
            self.app = AppStage::Checking;
        }
    }

    /// A successful app check: a newer staged release is ready, anything else idle.
    fn app_found(&mut self, found: Option<String>) {
        if matches!(self.app, AppStage::Applied(_)) {
            return;
        }
        self.app = match found {
            Some(version) => AppStage::Ready(version),
            None => AppStage::Idle,
        };
    }

    /// A failed app check: no old ready state survives.
    /// A check that couldn't reach the releases: it says nothing about a staged
    /// update, so a ready one stays ready.
    fn app_failed(&mut self) {
        self.checked = None;
        if matches!(self.app, AppStage::Checking) {
            self.app = AppStage::Idle;
        }
    }

    fn record_check(&mut self, succeeded: bool, now: u64) {
        self.checked = checked_at(succeeded, now);
    }

    fn applied(&mut self, version: &str) {
        self.app = AppStage::Applied(version.to_owned());
    }

    /// Starts a core update unless one already runs. Clears an old error.
    fn start_core_update(&mut self) -> bool {
        let updating = match &self.core.stage {
            CoreStage::Idle | CoreStage::Failed(_) => String::new(),
            CoreStage::Ready(version) => version.clone(),
            CoreStage::Updating(_) => return false,
        };
        self.core.stage = CoreStage::Updating(updating);
        true
    }

    fn finish_core_update(&mut self, result: Result<String, String>) {
        match result {
            Ok(now) => {
                self.core.version = Some(now.trim_start_matches("kom ").to_owned());
                self.core.stage = CoreStage::Idle;
            }
            Err(error) => self.core.stage = CoreStage::Failed(error),
        }
    }

    fn set_core(&mut self, version: Option<String>, managed: Option<String>) {
        if let (Some(have), CoreStage::Ready(ready)) = (&version, &self.core.stage)
            && !newer(ready, have)
        {
            self.core.stage = CoreStage::Idle;
        }
        self.core.version = version;
        self.core.managed = managed;
    }
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            app: AppStage::Idle,
            core: CoreUpdate {
                version: None,
                managed: None,
                stage: CoreStage::Idle,
            },
            checked: None,
        }
    }
}

static UPDATES: Mutex<Updates> = Mutex::new(Updates {
    app: AppStage::Idle,
    core: CoreUpdate {
        version: None,
        managed: None,
        stage: CoreStage::Idle,
    },
    checked: None,
});
/// The changelog parsed once per ready version, not every 500 ms.
static NOTES: Mutex<Option<(Option<String>, Vec<crate::update_notes::Note>)>> = Mutex::new(None);
/// "Check now" wakes the checker early.
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
/// The checker exists (a release, or pointed at a test server).
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn ready() -> Option<String> {
    UPDATES.lock().unwrap().app.ready()
}

/// The core the app runs, from its `server.status`, and a newer release waiting.
#[derive(Clone, Default, serde::Serialize)]
pub struct CoreInfo {
    pub version: Option<String>,
    /// A package manager that owns it (homebrew, nix, mise): never updated by the app.
    pub managed: Option<String>,
    pub ready: Option<String>,
    pub updating: bool,
    pub error: Option<String>,
    /// `idle`, `ready`, `updating` or `failed`.
    pub stage: String,
}

pub fn core() -> CoreInfo {
    let u = UPDATES.lock().unwrap();
    CoreInfo {
        version: u.core.version.clone(),
        managed: u.core.managed.clone(),
        ready: match &u.core.stage {
            CoreStage::Ready(version) => Some(version.clone()),
            _ => None,
        },
        updating: matches!(u.core.stage, CoreStage::Updating(_)),
        error: match &u.core.stage {
            CoreStage::Failed(error) => Some(error.clone()),
            _ => None,
        },
        stage: u.core.stage.word().to_owned(),
    }
}

/// What the connected core says about itself (on connect and every reconnect).
pub fn set_core(version: Option<String>, managed: Option<String>) {
    UPDATES.lock().unwrap().set_core(version, managed);
}

fn notes(version: Option<String>) -> Vec<crate::update_notes::Note> {
    let mut cache = NOTES.lock().unwrap();
    if cache.as_ref().is_none_or(|(cached, _)| cached != &version) {
        let loaded = crate::update_notes::notes(version.as_deref());
        *cache = Some((version, loaded));
    }
    cache.as_ref().unwrap().1.clone()
}

fn checks_app() -> Option<PathBuf> {
    root().filter(|_| {
        option_env!("KOMPITUM_APP_RELEASE").is_some()
            || std::env::var_os("KOMPITUM_RELEASES_URL").is_some()
    })
}

/// A release build checks the core; a source build only when pointed at a test release server.
fn checks_core() -> bool {
    option_env!("KOMPITUM_APP_RELEASE").is_some()
        || std::env::var_os("KOMPITUM_RELEASES_URL").is_some()
}

/// For the settings screen: both programs, what's waiting, when we last looked.
pub fn status() -> serde_json::Value {
    let (app_ready, app_stage, checked, checking) = {
        let u = UPDATES.lock().unwrap();
        (
            u.app.ready(),
            u.app.word(),
            u.checked,
            matches!(u.app, AppStage::Checking),
        )
    };
    serde_json::json!({
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "ready": app_ready,
            "updates": checks_app().is_some(),
            "stage": app_stage,
        },
        "notes": notes(app_ready),
        "core": core(),
        "checked": checked,
        "checks": RUNNING.load(Ordering::Relaxed),
        "checking": checking,
    })
}

pub fn check_now() {
    *WAKE.0.lock().unwrap() = true;
    WAKE.1.notify_all();
}

fn sleep(d: Duration) {
    let woken = WAKE.0.lock().unwrap();
    let (mut woken, _) = WAKE.1.wait_timeout_while(woken, d, |w| !*w).unwrap();
    *woken = false;
}

/// Starts checking in the background: the app if it's an installed release, the core too.
pub fn start() {
    let app = checks_app();
    let core = checks_core();
    if app.is_none() && !core {
        tracing::info!("a build from source: no updates");
        return;
    }
    RUNNING.store(true, Ordering::Relaxed);
    std::thread::spawn(move || {
        sleep(Duration::from_secs(10));
        loop {
            UPDATES.lock().unwrap().begin_check();
            let mut wait = EVERY;
            let mut succeeded = true;
            if let Some(root) = &app {
                match check(root) {
                    Ok(found) => UPDATES.lock().unwrap().app_found(found),
                    Err(e) => {
                        tracing::warn!("update check: {e}");
                        crate::telemetry::count("app_update_failed");
                        wait = RETRY;
                        succeeded = false;
                        UPDATES.lock().unwrap().app_failed();
                    }
                }
            } else {
                // A core-only checker still settles the app stage after its pass.
                UPDATES.lock().unwrap().app_found(None);
            }
            if core && let Err(e) = check_core() {
                tracing::warn!("core update check: {e}");
                wait = RETRY;
                succeeded = false;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            UPDATES.lock().unwrap().record_check(succeeded, now);
            sleep(wait);
        }
    });
}

fn checked_at(succeeded: bool, now: u64) -> Option<u64> {
    succeeded.then_some(now)
}

/// Looks for a newer core of this app's generation.
fn check_core() -> Result<(), String> {
    let (have, managed) = {
        let u = UPDATES.lock().unwrap();
        (u.core.version.clone(), u.core.managed.clone())
    };
    let Some(have) = have else { return Ok(()) };
    if managed.is_some() {
        return Ok(());
    }
    let latest = latest_core()?;
    let same = latest.generation == Some(crate::kom::GENERATION);
    let ready = (same && newer(&latest.version, &have)).then_some(latest.version);
    let mut u = UPDATES.lock().unwrap();
    // A running update or its failure says more than this check does.
    if matches!(u.core.stage, CoreStage::Idle | CoreStage::Ready(_)) {
        u.core.stage = match ready {
            Some(version) => CoreStage::Ready(version),
            None => CoreStage::Idle,
        };
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct CoreRelease {
    version: String,
    generation: Option<u64>,
}

fn latest_core() -> Result<CoreRelease, String> {
    let text = curl_text(&format!("{}/core/latest.json", releases()))?;
    serde_json::from_str(&text).map_err(|e| format!("core/latest.json: {e}"))
}

fn after_update_target<'a>(
    have: &str,
    latest: Option<&'a CoreRelease>,
    generation: u64,
    core_min: &'a str,
) -> Option<&'a str> {
    if newer("0.1.5", have) {
        return None;
    }
    Some(match latest {
        Some(latest) if latest.generation == Some(generation) && newer(&latest.version, have) => {
            &latest.version
        }
        _ => core_min,
    })
}

/// `kom update` (terminals keep running; the app reconnects when the core restarts).
pub fn update_core() {
    if !UPDATES.lock().unwrap().start_core_update() {
        return;
    }
    std::thread::spawn(|| {
        let result = crate::core_bin::update();
        crate::telemetry::count(if result.is_ok() {
            "core_update_ok"
        } else {
            "core_update_failed"
        });
        UPDATES.lock().unwrap().finish_core_update(result);
    });
}

/// Downloads the latest release if it's newer. Ok(Some): a version is ready.
fn check(root: &Path) -> Result<Option<String>, String> {
    check_from(root, &releases())
}

fn check_from(root: &Path, releases: &str) -> Result<Option<String>, String> {
    let path = "app/latest.json";
    let text = curl_text(&format!("{releases}/{path}"))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let version = manifest["version"]
        .as_str()
        .ok_or(format!("{path}: no version"))?;
    if !newer(version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    // Notes are optional: a missing feed must never hold up the update.
    if !crate::update_notes::has(version)
        && let Ok(text) = curl_text(&format!("{releases}/app/{version}/changelog.md"))
    {
        crate::update_notes::cache(&text);
    }
    let staged = beside(root, ".new");
    if staged_version(root).as_deref() == Some(version) {
        return Ok(Some(version.to_owned()));
    }
    let entry = &manifest["targets"][target()];
    let (Some(url), Some(sha)) = (entry["url"].as_str(), entry["sha256"].as_str()) else {
        return Ok(None);
    };
    tracing::info!("downloading Kompitum {version}");
    let work = beside(root, ".download");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let result = unpack(url, sha, &work).and_then(|unpacked| {
        if version_of(&unpacked).as_deref() != Some(version) {
            return Err(format!("the download doesn't run as {version}"));
        }
        let _ = std::fs::remove_file(beside(root, ".new.json"));
        let _ = std::fs::remove_dir_all(&staged);
        std::fs::rename(&unpacked, &staged).map_err(|e| format!("{}: {e}", staged.display()))?;
        // Outside the bundle: writing a receipt must not change its signature.
        std::fs::write(
            beside(root, ".new.json"),
            serde_json::json!({ "version": version }).to_string(),
        )
        .map_err(|e| e.to_string())
    });
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| Some(version.to_owned()))
}

/// Downloads, checks and unpacks a release into `work`; returns the app inside.
fn unpack(url: &str, sha: &str, work: &Path) -> Result<PathBuf, String> {
    let tarball = work.join("app.tar.gz");
    run(crate::spawn::command(system("curl"))
        .args(["-fsSL", "--max-time", "600", "-o"])
        .arg(&tarball)
        .arg(url))?;
    let bytes = std::fs::read(&tarball).map_err(|e| e.to_string())?;
    if sha256(&bytes) != sha.to_ascii_lowercase() {
        return Err("the download is damaged (checksum mismatch)".into());
    }
    run(crate::spawn::command(system("tar"))
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(work))?;
    let app = work.join(PACKED);
    if cfg!(target_os = "macos") {
        // Authority is absent at -dv, even for a Developer ID release.
        let signature = Command::new("/usr/bin/codesign")
            .arg("-dvv")
            .arg(&app)
            .output()
            .map_err(|e| format!("codesign: {e}"))?;
        prepare_macos_bundle(&app, &signature)?;
    }
    Ok(app)
}

fn prepare_macos_bundle(app: &Path, signature: &std::process::Output) -> Result<(), String> {
    if release_signature(
        signature.status.success(),
        &String::from_utf8_lossy(&signature.stderr),
    )? {
        run(Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(app))?;
    } else {
        // Only known dev builds may change the plist and lose their signature.
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{home}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
        let plist = app.join("Contents/Info.plist");
        run(Command::new("/usr/libexec/PlistBuddy")
            .arg("-c")
            .arg(format!("Add :LSEnvironment:PATH string {path}"))
            .arg(&plist))?;
        run(Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(app))?;
    }
    Ok(())
}

fn release_signature(success: bool, signature: &str) -> Result<bool, String> {
    if success
        && signature
            .lines()
            .any(|line| line.starts_with("Authority=Developer ID Application:"))
    {
        Ok(true)
    } else if (success && signature.lines().any(|line| line == "Signature=adhoc"))
        || (!success
            && signature
                .lines()
                .any(|line| line.ends_with(": code object is not signed at all")))
    {
        Ok(false)
    } else {
        Err(format!(
            "cannot determine app signature: {}",
            signature.trim()
        ))
    }
}

/// curl and tar: on Windows its own (System32), since Git's tar, often first
/// on PATH, takes `C:\...` for a remote host.
pub fn system(name: &str) -> PathBuf {
    match std::env::var_os("SystemRoot").filter(|_| cfg!(windows)) {
        Some(root) => PathBuf::from(root)
            .join("System32")
            .join(format!("{name}.exe")),
        None => PathBuf::from(name),
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn curl_text(url: &str) -> Result<String, String> {
    let out = crate::spawn::command(system("curl"))
        .args(["-fsSL", "--max-time", "30", url])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{:?}: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn exe_in(app: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        app.join("Contents/MacOS/Kompitum")
    } else {
        app.join(format!("kompitum-app{}", std::env::consts::EXE_SUFFIX))
    }
}

/// What `kompitum-app --version` says in this folder or bundle (`0.2.0`).
fn version_of(app: &Path) -> Option<String> {
    let out = crate::spawn::command(exe_in(app))
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return None;
    }
    text.trim().strip_prefix("kompitum-app ").map(str::to_owned)
}

/// `a` is a later version than `b` (semver; `0.2.0-rc.1` comes before `0.2.0`).
pub(crate) fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, Option<&str>) {
        let (core, pre) = v
            .trim_start_matches('v')
            .split_once('-')
            .map_or((v.trim_start_matches('v'), None), |(c, p)| (c, Some(p)));
        (
            core.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            pre,
        )
    }
    let ((ca, pa), (cb, pb)) = (parts(a), parts(b));
    match ca.cmp(&cb) {
        std::cmp::Ordering::Equal => match (pa, pb) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
        o => o == std::cmp::Ordering::Greater,
    }
}

/// A receipt is published only after the archive and its executable were verified.
/// Legacy stages without one are left for the normal checker to download again.
fn staged_version(root: &Path) -> Option<String> {
    let staged = beside(root, ".new");
    let receipt = beside(root, ".new.json");
    let version = std::fs::read(&receipt)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["version"].as_str().map(str::to_owned));
    let version = version?;
    if version_of(&staged).as_deref() != Some(&version) {
        tracing::warn!(
            "update: staged executable does not match its verified manifest; discarding it"
        );
        let _ = std::fs::remove_file(receipt);
        let _ = std::fs::remove_dir_all(staged);
        return None;
    }
    Some(version)
}

fn startup_target<'a>(
    running: &str,
    staged: Option<&'a str>,
    relaunched: Option<&str>,
) -> Option<&'a str> {
    if relaunched.is_some() {
        return None;
    }
    staged.filter(|version| newer(version, running))
}

/// Recover a staged hotfix even when the previous window never quit cleanly.
/// The inherited expected version allows only one automatic relaunch.
pub fn apply_on_startup() -> bool {
    let expected = std::env::var("KOMPITUM_APP_UPDATE_EXPECTED").ok();
    if let Some(expected) = &expected {
        // SAFETY: startup runs before any app threads. Do not leak the guard
        // into the core's environment and every terminal it later starts.
        unsafe { std::env::remove_var("KOMPITUM_APP_UPDATE_EXPECTED") };
        if expected != env!("CARGO_PKG_VERSION") {
            tracing::warn!(
                "update: relaunched as {}, expected {expected}; not retrying",
                env!("CARGO_PKG_VERSION")
            );
        }
        return false;
    }
    let Some(root) = checks_app() else {
        return false;
    };
    let staged = staged_version(&root);
    let Some(version) = startup_target(
        env!("CARGO_PKG_VERSION"),
        staged.as_deref(),
        expected.as_deref(),
    ) else {
        return false;
    };
    if !swap(&root, version) {
        return false;
    }
    relaunch_expected(Some(version))
}

/// Puts the downloaded version in place of this one. The running app keeps
/// its files open, so it can go on until it quits. True if it swapped.
/// Windows lets a running program's folder be renamed, not deleted: the old
/// one goes on the next start (`after_update`).
pub fn apply() -> bool {
    let Some(root) = root() else { return false };
    let Some(staged) = staged_version(&root) else {
        return false;
    };
    if ready().as_deref() != Some(&staged)
        || !newer(&staged, env!("CARGO_PKG_VERSION"))
    {
        return false;
    }
    swap(&root, &staged)
}

fn swap(root: &Path, version: &str) -> bool {
    let staged = beside(root, ".new");
    let old = beside(root, ".old");
    let _ = std::fs::remove_dir_all(&old);
    if let Err(e) = std::fs::rename(root, &old) {
        tracing::warn!("update: cannot move {} aside: {e}", root.display());
        return false;
    }
    if let Err(e) = std::fs::rename(&staged, root) {
        tracing::warn!("update: cannot put the new version in place: {e}");
        if let Err(rollback) = std::fs::rename(&old, root) {
            tracing::error!("update: cannot restore the previous version: {rollback}");
        }
        return false;
    }
    let _ = std::fs::remove_file(beside(root, ".new.json"));
    UPDATES.lock().unwrap().applied(version);
    let _ = std::fs::write(marker(), "");
    true
}

/// Whether this build knows how to open itself again (an installed release).
pub fn relaunch_available() -> bool {
    root().is_some()
}

/// Opens the app again once this process has exited (after `apply`).
pub fn relaunch() {
    relaunch_expected(None);
}

#[cfg(windows)]
fn relaunch_expected(expected: Option<&str>) -> bool {
    let Some(root) = root() else { return false };
    let exe = exe_in(&root).to_string_lossy().replace('\'', "''");
    let script = format!(
        "Wait-Process -Id {} -ErrorAction SilentlyContinue; Start-Process '{exe}'",
        std::process::id()
    );
    let mut command = crate::spawn::command("powershell");
    if let Some(expected) = expected {
        command.env("KOMPITUM_APP_UPDATE_EXPECTED", expected);
    }
    let spawned = command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("update: cannot reopen the app: {e}");
            false
        }
    }
}

/// Opens the app again once this process has exited (after `apply`).
#[cfg(unix)]
fn relaunch_expected(expected: Option<&str>) -> bool {
    let Some(root) = root() else { return false };
    // LaunchServices need not preserve the startup guard or an isolated home.
    let open = if cfg!(target_os = "macos") {
        if expected.is_some() {
            "exec \"$1/Contents/MacOS/Kompitum\""
        } else {
            "open \"$1\""
        }
    } else {
        "exec \"$1/kompitum-app\""
    };
    let script = format!("while kill -0 \"$0\" 2>/dev/null; do sleep 0.1; done; {open}");
    let spawned = {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        if let Some(expected) = expected {
            command.env("KOMPITUM_APP_UPDATE_EXPECTED", expected);
        }
        command
            .arg("-c")
            .arg(script)
            .arg(std::process::id().to_string())
            .arg(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
    };
    match spawned {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("update: cannot reopen the app: {e}");
            false
        }
    }
}

/// Left by `apply`: the next start brings the core up to date too.
fn marker() -> PathBuf {
    crate::mods::app_dir().join(".updated")
}

/// The first start after an app update brings the core to the latest release
/// of this app's generation (`kom update --version`, terminals kept). If the
/// release check fails or offers another generation, keep the minimum pin.
/// Cores before 0.1.5 lack `--version`: they keep their same-generation update
/// path. A core a package manager owns is left alone (kom says so).
pub fn after_update() {
    if std::fs::remove_file(marker()).is_err() {
        return;
    }
    if !UPDATES.lock().unwrap().start_core_update() {
        return;
    }
    std::thread::spawn(|| {
        let have = crate::core_bin::version(&crate::core_bin::path()).unwrap_or_default();
        let latest = latest_core()
            .map_err(|e| tracing::warn!("core release check after app update: {e}"))
            .ok();
        let target = after_update_target(
            have.trim_start_matches("kom "),
            latest.as_ref(),
            crate::kom::GENERATION,
            crate::kom::core_min(),
        );
        let result = crate::core_bin::update_to(target);
        if let Ok(now) = &result {
            if let Some(root) = root() {
                let _ = std::fs::remove_dir_all(beside(&root, ".old"));
            }
            tracing::info!("kompitum-core is {now}");
        } else if let Err(e) = &result {
            tracing::warn!("kompitum-core update: {e}");
        }
        UPDATES.lock().unwrap().finish_core_update(result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_check_clears_success_and_recovery_records_a_new_time() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // An isolated release server drops the first connection, then recovers.
        // No environment changes, app window, core, or installed files are involved.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for recover in [false, true] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(request.starts_with(b"GET /app/latest.json HTTP/1.1\r\n"));
                if recover {
                    let body = format!(r#"{{"version":"{}"}}"#, env!("CARGO_PKG_VERSION"));
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                }
            }
        });
        let home =
            std::env::temp_dir().join(format!("kompitum-offline-check-{}", std::process::id()));
        let mut checked = checked_at(true, 100);
        assert_eq!(checked, Some(100));
        let failed = check_from(&home, &url);
        assert!(
            failed.is_err(),
            "a dropped connection must fail: {failed:?}"
        );
        checked = checked_at(failed.is_ok(), 200);
        assert_eq!(checked, None);
        let recovered = check_from(&home, &url);
        assert_eq!(recovered, Ok(None));
        checked = checked_at(recovered.is_ok(), 300);
        assert_eq!(checked, Some(300));
        server.join().unwrap();
    }

    #[test]
    fn startup_applies_only_a_newer_version_and_never_relaunches_twice() {
        assert_eq!(startup_target("0.1.7", Some("0.1.8"), None), Some("0.1.8"));
        for staged in [None, Some("0.1.6"), Some("0.1.7")] {
            assert_eq!(startup_target("0.1.7", staged, None), None);
        }
        for expected in ["0.1.7", "0.1.8", "0.1.9"] {
            assert_eq!(startup_target("0.1.7", Some("0.1.8"), Some(expected)), None);
        }
    }

    #[cfg(unix)]
    #[test]
    fn stages_need_a_receipt_and_a_matching_executable() {
        use std::os::unix::fs::PermissionsExt;
        let work = std::env::temp_dir().join(format!("kompitum-update-{}", std::process::id()));
        let root = work.join(PACKED);
        let staged = beside(&root, ".new");
        let receipt = beside(&root, ".new.json");
        let exe = exe_in(&staged);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "#!/bin/sh\necho kompitum-app 0.1.8\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(staged_version(&root), None);
        assert!(staged.is_dir()); // Legacy stages are left for the checker.
        std::fs::write(&receipt, r#"{"version":"0.1.8"}"#).unwrap();
        assert_eq!(staged_version(&root).as_deref(), Some("0.1.8"));
        std::fs::write(&receipt, r#"{"version":"0.1.9"}"#).unwrap();
        assert_eq!(staged_version(&root), None);
        assert!(!staged.exists());
        assert!(!receipt.exists());
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "corrupt").unwrap();
        std::fs::write(&receipt, r#"{"version":"0.1.8"}"#).unwrap();
        assert_eq!(staged_version(&root), None);
        assert!(!staged.exists());
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn signature_inspection_fails_closed() {
        let release = include_str!("../scripts/fixtures/codesign/developer-id.txt");
        let adhoc = include_str!("../scripts/fixtures/codesign/adhoc.txt");
        let unsigned = include_str!("../scripts/fixtures/codesign/unsigned.txt");
        assert_eq!(release_signature(true, release), Ok(true));
        assert_eq!(release_signature(true, adhoc), Ok(false));
        assert_eq!(release_signature(false, unsigned), Ok(false));
        for (success, output) in [
            (false, release),
            (false, adhoc),
            (true, unsigned),
            (true, ""),
            (false, "codesign: internal error"),
            (true, "Authority=Apple Development: Someone"),
        ] {
            assert!(
                release_signature(success, output)
                    .unwrap_err()
                    .starts_with("cannot determine app signature:")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unknown_signature_does_not_modify_the_bundle() {
        use std::os::unix::process::ExitStatusExt;
        let work =
            std::env::temp_dir().join(format!("kompitum-unknown-signature-{}", std::process::id()));
        let plist = work.join("Contents/Info.plist");
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, "unchanged plist").unwrap();
        let signature = std::process::Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"codesign: internal error".to_vec(),
        };
        assert_eq!(
            prepare_macos_bundle(&work, &signature),
            Err("cannot determine app signature: codesign: internal error".into())
        );
        assert_eq!(std::fs::read_to_string(plist).unwrap(), "unchanged plist");
        assert_eq!(std::fs::read_dir(work.join("Contents")).unwrap().count(), 1);
        std::fs::remove_dir_all(work).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "downloads the current release; runs real codesign on isolated bundles"]
    fn macos_unpack_preserves_release_signature_and_prepares_dev_builds() {
        let version = env!("CARGO_PKG_VERSION");
        let manifest: serde_json::Value = serde_json::from_str(
            &curl_text(&format!("{}/app/{version}/manifest.json", releases())).unwrap(),
        )
        .unwrap();
        let work =
            std::env::temp_dir().join(format!("kompitum-signature-test-{}", std::process::id()));
        let signed = work.join("signed");
        let reference = work.join("reference");
        std::fs::create_dir_all(&signed).unwrap();
        std::fs::create_dir_all(&reference).unwrap();
        let entry = &manifest["targets"][target()];
        let app = unpack(
            entry["url"].as_str().unwrap(),
            entry["sha256"].as_str().unwrap(),
            &signed,
        )
        .unwrap();
        let signature = Command::new("/usr/bin/codesign")
            .arg("-dvv")
            .arg(&app)
            .output()
            .unwrap();
        assert_eq!(
            release_signature(
                signature.status.success(),
                &String::from_utf8_lossy(&signature.stderr)
            ),
            Ok(true)
        );
        run(Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app))
        .unwrap();
        run(Command::new("tar")
            .arg("-xzf")
            .arg(signed.join("app.tar.gz"))
            .arg("-C")
            .arg(&reference))
        .unwrap();
        for file in [
            "Contents/Info.plist",
            "Contents/MacOS/Kompitum",
            "Contents/_CodeSignature/CodeResources",
        ] {
            assert_eq!(
                std::fs::read(app.join(file)).unwrap(),
                std::fs::read(reference.join(PACKED).join(file)).unwrap(),
                "{file} was modified"
            );
        }
        for kind in ["adhoc", "unsigned"] {
            let source = reference.join(PACKED);
            let mut codesign = Command::new("/usr/bin/codesign");
            if kind == "adhoc" {
                codesign.args(["--force", "--sign", "-"]);
            } else {
                codesign.arg("--remove-signature");
            }
            run(codesign.arg(&source)).unwrap();
            let archive = work.join(format!("{kind}.tar.gz"));
            run(Command::new("tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(&reference)
                .arg(PACKED))
            .unwrap();
            let dev = work.join(kind);
            std::fs::create_dir_all(&dev).unwrap();
            let sha = sha256(&std::fs::read(&archive).unwrap());
            let app = unpack(&format!("file://{}", archive.display()), &sha, &dev).unwrap();
            let signature = Command::new("/usr/bin/codesign")
                .arg("-dvv")
                .arg(&app)
                .output()
                .unwrap();
            assert!(signature.status.success());
            let signature = String::from_utf8_lossy(&signature.stderr);
            assert_eq!(release_signature(true, &signature), Ok(false));
            assert!(signature.lines().any(|line| line == "Signature=adhoc"));
            run(Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&app))
            .unwrap();
            let path = Command::new("/usr/libexec/PlistBuddy")
                .args(["-c", "Print :LSEnvironment:PATH"])
                .arg(app.join("Contents/Info.plist"))
                .output()
                .unwrap();
            assert!(path.status.success());
            assert!(String::from_utf8_lossy(&path.stdout).contains("/opt/homebrew/bin"));
        }
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn versions_compare_like_semver() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(newer("1.0.0", "1.0.0-rc.1"));
        assert!(!newer("1.0.0-rc.1", "1.0.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn after_app_update_targets_latest_only_in_our_generation() {
        let release = |version: &str, generation| CoreRelease {
            version: version.into(),
            generation,
        };
        let latest = release("0.1.13", Some(1));
        assert_eq!(
            after_update_target("0.1.12", Some(&latest), 1, "0.1.11"),
            Some("0.1.13")
        );
        for have in ["0.1.13", "0.1.14"] {
            assert_eq!(
                after_update_target(have, Some(&latest), 1, "0.1.11"),
                Some("0.1.11")
            );
        }
        for latest in [
            None,
            Some(release("0.2.0", Some(2))),
            Some(release("0.1.13", None)),
        ] {
            assert_eq!(
                after_update_target("0.1.12", latest.as_ref(), 1, "0.1.11"),
                Some("0.1.11")
            );
            assert_eq!(
                after_update_target("0.1.4", latest.as_ref(), 1, "0.1.11"),
                None
            );
        }
        assert_eq!(
            after_update_target("0.1.4", Some(&latest), 1, "0.1.11"),
            None
        );
        assert_eq!(
            after_update_target("0.1.5", Some(&latest), 1, "0.1.11"),
            Some("0.1.13")
        );
        assert_eq!(
            after_update_target("0.1.5", None, 1, "0.1.11"),
            Some("0.1.11")
        );
    }

    #[test]
    fn an_app_check_is_checking_only_while_a_pass_runs() {
        let mut u = Updates::default();
        assert_eq!(u.app, AppStage::Idle);
        u.begin_check();
        assert_eq!(u.app, AppStage::Checking);
        u.app_found(Some("0.3.0".into()));
        assert_eq!(u.app, AppStage::Ready("0.3.0".into()));
        u.begin_check();
        assert_eq!(u.app, AppStage::Ready("0.3.0".into()), "a ready update stays ready while the next pass looks");
        u.app_found(None);
        assert_eq!(u.app, AppStage::Idle, "nothing newer means nothing ready");
    }

    #[test]
    fn a_failed_check_is_idle_and_clears_the_last_good_time() {
        let mut u = Updates::default();
        u.record_check(true, 100);
        u.begin_check();
        u.app_failed();
        assert_eq!(u.app, AppStage::Idle);
        u.app_found(Some("0.3.0".into()));
        u.app_failed();
        assert_eq!(u.app, AppStage::Ready("0.3.0".into()), "a failed check leaves a staged update ready");
        assert_eq!(u.checked, None);
        u.record_check(false, 200);
        assert_eq!(u.checked, None);
    }

    #[test]
    fn an_applied_app_is_left_alone_by_later_checks() {
        let mut u = Updates::default();
        u.applied("0.3.0");
        u.begin_check();
        u.app_found(Some("0.4.0".into()));
        u.app_failed();
        assert_eq!(u.app, AppStage::Applied("0.3.0".into()));
    }

    #[test]
    fn a_core_update_starts_once_and_clears_an_old_error() {
        let mut u = Updates::default();
        u.finish_core_update(Err("no network".into()));
        assert_eq!(u.core.stage, CoreStage::Failed("no network".into()));
        assert!(u.start_core_update());
        assert_eq!(u.core.stage, CoreStage::Updating(String::new()));
        assert!(!u.start_core_update(), "a second updater is refused");
        u.finish_core_update(Ok("kom 0.2.0".into()));
        assert_eq!(u.core.version.as_deref(), Some("0.2.0"));
        assert_eq!(u.core.stage, CoreStage::Idle);
        assert!(u.start_core_update(), "idle may start again");
        u.finish_core_update(Ok("kom 0.2.1".into()));
        u.core.stage = CoreStage::Ready("0.3.0".into());
        assert!(u.start_core_update());
        assert_eq!(u.core.stage, CoreStage::Updating("0.3.0".into()));
    }

    #[test]
    fn a_core_ready_that_is_not_newer_goes_idle_again() {
        let mut u = Updates::default();
        u.set_core(Some("0.2.0".into()), None);
        u.core.stage = CoreStage::Ready("0.2.0".into());
        u.set_core(Some("0.2.0".into()), Some("brew".into()));
        assert_eq!(u.core.stage, CoreStage::Idle);
        assert_eq!(u.core.managed.as_deref(), Some("brew"));
        u.core.stage = CoreStage::Ready("0.3.0".into());
        u.set_core(Some("0.2.0".into()), None);
        assert_eq!(u.core.stage, CoreStage::Ready("0.3.0".into()), "a newer core stays ready");
        u.core.stage = CoreStage::Updating("0.4.0".into());
        u.set_core(Some("0.2.0".into()), None);
        assert_eq!(
            u.core.stage,
            CoreStage::Updating("0.4.0".into()),
            "a running update is not a check result"
        );
    }

    #[test]
    fn status_keeps_the_settings_keys_and_names_the_stages() {
        let status = status();
        for key in ["ready", "updates", "version"] {
            assert!(status["app"].get(key).is_some(), "app.{key}");
        }
        for key in ["ready", "updating", "error", "version", "managed"] {
            assert!(status["core"].get(key).is_some(), "core.{key}");
        }
        assert!(status["app"]["stage"].is_string());
        assert!(status["core"]["stage"].is_string());
        for key in ["notes", "checked", "checks", "checking"] {
            assert!(status.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn sha256_is_hex() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
