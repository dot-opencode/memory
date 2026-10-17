//! Process-wide state, exit/cleanup handling, signals and subprocess helpers.

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct State {
    pub memory_home: String,
    pub store_home: String,
    pub lock_file: Option<String>,
    pub index_temps: Vec<String>,
    pub child_pid: i32,
    pub watchdog: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
    pub active: bool,
    pub workspace: String,
    pub tools_ok: String,
    pub tools_denied: String,
}

static STATE: Mutex<State> = Mutex::new(State {
    memory_home: String::new(),
    store_home: String::new(),
    lock_file: None,
    index_temps: Vec::new(),
    child_pid: 0,
    watchdog: None,
    active: false,
    workspace: String::new(),
    tools_ok: String::new(),
    tools_denied: String::new(),
});

pub static QUIET: AtomicBool = AtomicBool::new(false);
static SIGNALED: AtomicBool = AtomicBool::new(false);

pub fn st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

pub fn store() -> String {
    st(|s| s.store_home.clone())
}
pub fn memory_home() -> String {
    st(|s| s.memory_home.clone())
}

#[macro_export]
macro_rules! outln {
    ($($a:tt)*) => {{
        if !$crate::ctx::QUIET.load(std::sync::atomic::Ordering::SeqCst) {
            use std::io::Write;
            let _ = writeln!(std::io::stdout(), $($a)*);
        }
    }};
}

#[macro_export]
macro_rules! die {
    ($($a:tt)*) => { $crate::ctx::die(&format!($($a)*)) };
}

/// Run `f` with stdout output of `outln!` suppressed (like `>/dev/null`).
pub fn quiet<R>(f: impl FnOnce() -> R) -> R {
    let prev = QUIET.swap(true, Ordering::SeqCst);
    let r = f();
    QUIET.store(prev, Ordering::SeqCst);
    r
}

pub fn die(msg: &str) -> ! {
    eprintln!("memory: {msg}");
    exit_with(1)
}

pub fn exit_with(code: i32) -> ! {
    cleanup(code);
    std::process::exit(code)
}

/// Equivalent of the script's EXIT trap (`cleanup_on_exit`).
pub fn cleanup(code: i32) {
    let (active, lock) = st(|s| (s.active, s.lock_file.is_some()));
    if active && lock {
        let rc = if code != 0 { code } else { 1 };
        if let Some(gd) = store_gitdir() {
            let _ = fs::write(format!("{gd}/memory-dream.failed"), format!("{rc}\n"));
        }
        eprintln!("memory: dream final persistence failed (exit {rc}); FAILED review marker retained. Fix git/index errors and run memory backup; changes must be reviewed.");
    }
    dream_kill_child();
    cleanup_index_temps();
    if st(|s| s.lock_file.is_some()) {
        release_lock();
    }
}

pub fn cleanup_index_temps() {
    let temps = st(|s| std::mem::take(&mut s.index_temps));
    for t in temps {
        let _ = fs::remove_file(t);
    }
}

// ---------------------------------------------------------------- signals

extern "C" fn on_signal(_: libc::c_int) {
    SIGNALED.store(true, Ordering::SeqCst);
}

pub fn install_signal_handlers() {
    for sig in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut sa.sa_mask);
            sa.sa_flags = 0;
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

pub fn reset_signal_handlers() {
    for sig in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
        }
    }
}

pub fn check_signal() {
    if SIGNALED.load(Ordering::SeqCst) {
        dream_signal(143);
    }
}

fn dream_signal(code: i32) -> ! {
    SIGNALED.store(false, Ordering::SeqCst);
    for sig in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        unsafe {
            libc::signal(sig, libc::SIG_IGN);
        }
    }
    dream_kill_child();
    if st(|s| s.active) {
        crate::dream::failure_finalize(code, &format!("opencode run interrupted (exit {code})"));
    }
    release_lock();
    exit_with(1)
}

// ---------------------------------------------------------------- processes

pub fn status_code(s: ExitStatus) -> i32 {
    s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0))
}

/// Run with inherited stdio; returns the exit code (127 when it cannot start).
pub fn run(cmd: &mut Command) -> i32 {
    let rc = match cmd.status() {
        Ok(s) => status_code(s),
        Err(_) => 127,
    };
    check_signal();
    rc
}

/// Run with stdout captured. stderr is discarded unless `show_err`.
pub fn capture(cmd: &mut Command, show_err: bool) -> (i32, String) {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped());
    cmd.stderr(if show_err {
        Stdio::inherit()
    } else {
        Stdio::null()
    });
    let r = match cmd.output() {
        Ok(o) => (
            status_code(o.status),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        ),
        Err(_) => (127, String::new()),
    };
    check_signal();
    r
}

/// Captured stdout like `$(...)`: trailing newlines stripped.
pub fn cap_trim(cmd: &mut Command) -> Option<String> {
    let (rc, out) = capture(cmd, false);
    if rc == 0 {
        Some(out.trim_end_matches('\n').to_string())
    } else {
        None
    }
}

pub fn git_plain(args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(store()).args(args);
    c
}

/// git on the store as a service repo: never prompts, signs, edits or runs hooks.
pub fn store_git(args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .arg("-C")
        .arg(store())
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args);
    c
}

pub fn store_gitdir() -> Option<String> {
    let s = store();
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(&s)
        .args(["rev-parse", "--absolute-git-dir"]);
    cap_trim(&mut c)
}

pub fn sleep_secs(secs: f64) {
    if secs > 0.0 {
        std::thread::sleep(Duration::from_secs_f64(secs));
    }
}

pub fn kill_grace() -> f64 {
    match std::env::var("AGENT_MEMORY_DREAM_KILL_GRACE") {
        Ok(v) if !v.is_empty() => v.parse().unwrap_or(5.0),
        _ => 5.0,
    }
}

pub fn kill_group(pgid: i32, sig: i32) {
    if pgid > 0 {
        unsafe {
            libc::kill(-pgid, sig);
        }
    }
}

pub fn group_alive(pgid: i32) -> bool {
    pgid > 0 && unsafe { libc::kill(-pgid, 0) == 0 }
}

pub fn stop_watchdog() {
    if let Some((cancel, h)) = st(|s| s.watchdog.take()) {
        cancel.store(true, Ordering::SeqCst);
        let _ = h.join();
    }
}

pub fn dream_kill_child() {
    stop_watchdog();
    let pid = st(|s| s.child_pid);
    if pid == 0 {
        return;
    }
    kill_group(pid, libc::SIGTERM);
    sleep_secs(kill_grace());
    kill_group(pid, libc::SIGKILL);
    let mut status = 0;
    unsafe {
        libc::waitpid(pid, &mut status, 0);
    }
    st(|s| s.child_pid = 0);
}

// ---------------------------------------------------------------- writer lock

pub fn ps_lstart(pid: &str) -> String {
    let mut c = Command::new("ps");
    c.args(["-o", "lstart=", "-p", pid]);
    let (rc, out) = capture(&mut c, false);
    if rc != 0 {
        return String::new();
    }
    out.trim_matches(|ch| ch == ' ' || ch == '\n').to_string()
}

pub fn lock_acquire() -> bool {
    if st(|s| s.lock_file.is_some()) {
        return true;
    }
    let gd = store_gitdir().unwrap_or_default();
    let lock = format!("{gd}/memory-dream.lock");
    if fs::create_dir(&lock).is_err() {
        return false;
    }
    st(|s| s.lock_file = Some(lock.clone()));
    install_signal_handlers();
    let pid = std::process::id();
    let start = ps_lstart(&pid.to_string());
    let _ = fs::write(format!("{lock}/owner"), format!("{pid}\n{start}\n"));
    true
}

pub fn release_lock() {
    if let Some(l) = st(|s| s.lock_file.take()) {
        let _ = fs::remove_file(format!("{l}/owner"));
        if fs::remove_dir(&l).is_err() {
            die(&format!("cannot release writer lock {l}"));
        }
    }
    reset_signal_handlers();
}

/// Diagnostic only; acquisition never depends on a check-then-act pid test.
pub fn lock_holder() -> String {
    let gd = store_gitdir().unwrap_or_default();
    let lock = format!("{gd}/memory-dream.lock");
    let pid: String;
    let mut start = String::new();
    if crate::fsutil::is_dir(&lock) {
        let owner = format!("{lock}/owner");
        if !crate::fsutil::is_file(&owner) {
            return "unpublished (do not remove)".to_string();
        }
        let c = fs::read(&owner)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let mut it = c.split('\n');
        pid = it.next().unwrap_or("").to_string();
        if c.contains('\n') {
            start = it.next().unwrap_or("").to_string();
        }
    } else if crate::fsutil::is_file(&lock) {
        let c = fs::read(&lock)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        pid = c.split('\n').next().unwrap_or("").to_string();
    } else {
        return String::new();
    }
    let current = ps_lstart(&pid);
    if !start.is_empty() && current == start {
        pid
    } else {
        format!("{pid}; stale or uncertain, manual recovery required")
    }
}

pub fn writer_lock_recovery_help() -> String {
    let gd = store_gitdir().unwrap_or_default();
    let lock = format!("{gd}/memory-dream.lock");
    let mut s = format!(
        "Writer lock: {lock} (pid {}). Stop ALL memory writers before manual recovery.\n",
        lock_holder()
    );
    if crate::fsutil::is_dir(&lock) {
        s.push_str(&format!("Inspect owner metadata {lock}/owner; verify BOTH PID and process start time with ps -o lstart= -p <PID>. Unpublished/missing metadata is uncertain, not stale. Only after verifying no writer is active, remove owner then rmdir the lock directory."));
    } else {
        s.push_str(&format!("Legacy lock FILE (not directory): inspect {lock} and verify PID/start identity; only after verifying no writer is active remove this file."));
    }
    s
}

/// Run a command, retrying while another git process holds .git/index.lock.
pub fn store_retry(args: &[&str]) -> i32 {
    let gd = store_gitdir().unwrap_or_else(|| format!("{}/.git", store()));
    let mut i = 1;
    loop {
        let rc = run(&mut store_git(args));
        if rc == 0 {
            return 0;
        }
        if !(i < 5 && crate::fsutil::exists(&format!("{gd}/index.lock"))) {
            return rc;
        }
        i += 1;
        sleep_secs(1.0);
        check_signal();
    }
}

pub fn which(cmd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var("PATH").unwrap_or_default();
    for d in path.split(':') {
        let d = if d.is_empty() { "." } else { d };
        let p = format!("{d}/{cmd}");
        if let Ok(m) = fs::metadata(&p) {
            if m.is_file() && m.permissions().mode() & 0o111 != 0 {
                return true;
            }
        }
    }
    false
}
