//! Shared black-box harness for the `memory` CLI integration tests.
//!
//! Mirrors `setup()` and the helper functions of the former Bash suite `tests/run.sh`.
//! Each test file does `mod common; use common::*;` and starts with
//! `let mut t = T::new();`.
//!
//! # Model
//! * `T` owns an isolated temp dir (`t.tmp`, canonical, removed on drop) with
//!   `t.home` (fake `$HOME` incl. global git config), `t.store`
//!   (`AGENT_MEMORY_HOME`, not created yet) and `t.proj` (`tmp/code/some-api`,
//!   the default cwd, i.e. Bash's `$PROJ`). `t.root` is the repo root.
//! * The test process's own env/cwd are NEVER touched. Every child gets its
//!   env set explicitly: `AGENT_MEMORY_HOME`, `HOME`, `PWD` (= cwd, like a
//!   shell's `cd`), `AGENT_MEMORY_BACKUP_LOCK_WAIT=0`, `GIT_CONFIG_NOSYSTEM=1`,
//!   `AGENT_MEMORY_SKILL_DIR`, `AGENT_MEMORY_SELF`, `MEMORY_BIN`; and
//!   `XDG_CONFIG_HOME`, `AGENT_MEMORY_DREAM_MODEL`, `AGENT_MEMORY_TODAY`,
//!   `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` are removed. `PATH` is the real
//!   PATH with `t.path_dirs` prepended.
//! * Persistent per-test env (the Bash `export`): `t.set_env(k, v)`,
//!   `t.unset_env(k)`. Persistent PATH prefix (Bash `PATH="$TMP/bin:$PATH"`):
//!   `t.prepend_path(dir)`. Per-command overrides: `Cmd::env/env_remove`.
//!
//! # Running commands
//! * `t.memory(&["init","work"])` -> `Cmd` (cwd defaults to `t.proj`);
//!   `t.cmd("git", &[..])` runs any other program with the same env (use it for
//!   `git config --global ...`; `t.git(&[..])` = `git -C <store>`).
//!   `Cmd`: `.cwd(path)`, `.env(k,v)`, `.env_remove(k)`, `.stdin(bytes)`,
//!   `.run() -> Out`, `.ok()` (run, assert exit 0), `.code(n)` (run, assert code).
//! * `Out { code, stdout, stderr, desc }`: `.combined()` (stdout+stderr, the
//!   `2>&1` equivalent), `.so()` (stdout w/o trailing newlines, like `$(...)`),
//!   `.ok()`, `.code(n)` (both return self for chaining), `.assert_contains(s)`
//!   / `.assert_not_contains(s)` on the combined text.
//!
//! # Assertions (all `#[track_caller]`, failure messages show both sides)
//! `assert_eq_str(actual, expected)`, `assert_contains(hay, needle)`,
//! `assert_not_contains`, `assert_link(link, target)` (target's parent is
//! canonicalised like the Bash helper), `assert_file`, `assert_dir`,
//! `assert_absent`, `cat(path)` (file text minus trailing newlines, like
//! `$(cat f)`), `count_nonempty_lines(path)` (`grep -c .`).
//!
//! # Fixtures (same names as Bash)
//! `write_fact_only(dir, slug, type, desc)`, `write_fact` (also appends the
//! INDEX line), `fill_index(index, n)`, `dfact(dir, slug, type, desc, created,
//! Option<updated>)`, `extract_cursor_body(rule) -> Option<Vec<u8>>`,
//! `t.write(path, text)` (creates parents), `t.write_exec(path, text)`.
//! Dream: `t.dream_fixture()` (sets `AGENT_MEMORY_TODAY=2026-09-30`, fills
//! `t.ds/dp1/dp2/dw`, inits `some-api` and `other-api`), `t.other_proj()`,
//! `t.dream_reindex()`, `t.install_opencode_stub()` (sets `STUB_LOG=t.stub_log`,
//! prepends `tmp/bin`; per-command knobs `STUB_EDIT`, `STUB_EXIT`),
//! `t.dream_launch_fixture()`, `t.commit_count()`,
//! `t.install_backup_calling_stub()`, `t.dream_template_fixture()` (copy of the
//! skill in `t.tskill`, `t.tmem` = its bin/memory; to exercise the copy pass
//! `.env("AGENT_MEMORY_SKILL_DIR", t.tskill)` to a command),
//! `t.install_db_stub()` (`STUB_TOOLS`), `t.dream_db_fixture() -> bool` (false
//! when `sqlite3` is missing: return early), `t.install_acting_stub()`
//! (`STUB_ACT` shell snippet), `FACT_BODY`,
//! `t.assert_escape_is_failed_review(act, expected_path)`.
//!
//! Shell stubs are plain scripts written into `tmp/bin` (fixtures, not part of
//! the binary under test).

#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

pub const MEMORY_BIN: &str = env!("CARGO_BIN_EXE_memory");

/// Held for writing while a fixture script is open for write, and for reading
/// around every `spawn()`, so a concurrent fork can never inherit a writable
/// fd of a script that is about to be executed (ETXTBSY).
static SPAWN_LOCK: RwLock<()> = RwLock::new(());
static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub const FACT_BODY: &str =
    "---\ndescription: escaped\ntype: decision\ncreated: 2026-09-04\n---\nbody\n";

// ---------------------------------------------------------------- assertions

#[track_caller]
pub fn fail(msg: &str) -> ! {
    panic!("assertion failed: {msg}");
}

#[track_caller]
pub fn assert_eq_str(actual: impl AsRef<str>, expected: impl AsRef<str>) {
    let (a, e) = (actual.as_ref(), expected.as_ref());
    if a != e {
        fail(&format!("expected {e:?}, got {a:?}"));
    }
}

#[track_caller]
pub fn assert_contains(hay: impl AsRef<str>, needle: impl AsRef<str>) {
    let (h, n) = (hay.as_ref(), needle.as_ref());
    if !h.contains(n) {
        fail(&format!("no {n:?} in: {h}"));
    }
}

#[track_caller]
pub fn assert_not_contains(hay: impl AsRef<str>, needle: impl AsRef<str>) {
    let (h, n) = (hay.as_ref(), needle.as_ref());
    if h.contains(n) {
        fail(&format!("unexpected {n:?} in: {h}"));
    }
}

#[track_caller]
pub fn assert_file(p: impl AsRef<Path>) {
    let p = p.as_ref();
    if !p.is_file() {
        fail(&format!("missing file: {}", p.display()));
    }
}

#[track_caller]
pub fn assert_dir(p: impl AsRef<Path>) {
    let p = p.as_ref();
    if !p.is_dir() {
        fail(&format!("missing directory: {}", p.display()));
    }
}

/// Nothing at all (not even a dangling symlink) exists at `p`.
#[track_caller]
pub fn assert_absent(p: impl AsRef<Path>) {
    let p = p.as_ref();
    if fs::symlink_metadata(p).is_ok() {
        fail(&format!("unexpectedly present: {}", p.display()));
    }
}

#[track_caller]
pub fn assert_link(link: impl AsRef<Path>, target: impl AsRef<Path>) {
    let (link, target) = (link.as_ref(), target.as_ref());
    match fs::symlink_metadata(link) {
        Ok(m) if m.file_type().is_symlink() => {}
        _ => fail(&format!("{} is not a symlink", link.display())),
    }
    let parent = target.parent().unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let expected = parent.join(target.file_name().expect("target has a file name"));
    let actual = fs::read_link(link).unwrap();
    if actual != expected {
        fail(&format!(
            "symlink {}: expected {:?}, got {:?}",
            link.display(),
            expected.display().to_string(),
            actual.display().to_string()
        ));
    }
}

/// `$(cat file)`: file text without trailing newlines.
#[track_caller]
pub fn cat(p: impl AsRef<Path>) -> String {
    let p = p.as_ref();
    let s = fs::read(p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
    String::from_utf8_lossy(&s)
        .trim_end_matches('\n')
        .to_string()
}

/// `grep -c . file`: number of non-empty lines.
#[track_caller]
pub fn count_nonempty_lines(p: impl AsRef<Path>) -> usize {
    let p = p.as_ref();
    let s = fs::read(p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
    String::from_utf8_lossy(&s)
        .lines()
        .filter(|l| !l.is_empty())
        .count()
}

// ------------------------------------------------------------------ fixtures

fn fact_text(slug: &str, ty: &str, desc: &str) -> String {
    format!("---\ndescription: {desc}\ntype: {ty}\ncreated: 2026-09-04\n---\nbody of {slug}\n")
}

/// Fact file without an INDEX line.
pub fn write_fact_only(dir: &Path, slug: &str, ty: &str, desc: &str) {
    fs::write(dir.join(format!("{slug}.md")), fact_text(slug, ty, desc)).unwrap();
}

/// Fact file plus an INDEX.md line appended.
pub fn write_fact(dir: &Path, slug: &str, ty: &str, desc: &str) {
    write_fact_only(dir, slug, ty, desc);
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("INDEX.md"))
        .unwrap();
    writeln!(f, "- [{slug}]({slug}.md) — {ty}: {desc}").unwrap();
}

/// Append `count` generated entries `f001..` to an INDEX.md.
pub fn fill_index(index: &Path, count: usize) {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(index)
        .unwrap();
    for i in 1..=count {
        writeln!(f, "- [f{i:03}](f{i:03}.md) — gotcha: entry {i}").unwrap();
    }
}

/// Fact with explicit created/updated dates (no INDEX line).
pub fn dfact(dir: &Path, slug: &str, ty: &str, desc: &str, created: &str, updated: Option<&str>) {
    let mut s = format!("---\ndescription: {desc}\ntype: {ty}\ncreated: {created}\n");
    if let Some(u) = updated {
        if !u.is_empty() {
            s.push_str(&format!("updated: {u}\n"));
        }
    }
    s.push_str(&format!("---\nbody of {slug}\n"));
    fs::write(dir.join(format!("{slug}.md")), s).unwrap();
}

/// Body of a Cursor rule (everything after the closing `---` of the
/// frontmatter), byte-exact. `None` when there is no frontmatter.
pub fn extract_cursor_body(rule: &Path) -> Option<Vec<u8>> {
    let data = fs::read(rule).unwrap();
    let mut pos = 0usize;
    let mut first = true;
    while pos < data.len() {
        let end = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| pos + i);
        let line = &data[pos..end.unwrap_or(data.len())];
        let next = end.map(|e| e + 1).unwrap_or(data.len());
        if first {
            if line != b"---" {
                return None;
            }
            first = false;
        } else if line == b"---" {
            return Some(data[next..].to_vec());
        }
        pos = next;
    }
    None
}

// ---------------------------------------------------------------- Out / Cmd

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub desc: String,
}

impl Out {
    /// stdout followed by stderr (the `2>&1` equivalent for assertions).
    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
    /// stdout without trailing newlines (like `$(...)`).
    pub fn so(&self) -> String {
        self.stdout.trim_end_matches('\n').to_string()
    }
    #[track_caller]
    pub fn code(self, n: i32) -> Self {
        if self.code != n {
            fail(&format!(
                "`{}`: expected exit {n}, got {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                self.desc, self.code, self.stdout, self.stderr
            ));
        }
        self
    }
    #[track_caller]
    pub fn ok(self) -> Self {
        self.code(0)
    }
    #[track_caller]
    pub fn assert_contains(&self, needle: &str) -> &Self {
        let c = self.combined();
        if !c.contains(needle) {
            fail(&format!("`{}`: no {needle:?} in output: {c}", self.desc));
        }
        self
    }
    #[track_caller]
    pub fn assert_not_contains(&self, needle: &str) -> &Self {
        let c = self.combined();
        if c.contains(needle) {
            fail(&format!(
                "`{}`: unexpected {needle:?} in output: {c}",
                self.desc
            ));
        }
        self
    }
}

pub struct Cmd<'a> {
    t: &'a T,
    prog: OsString,
    args: Vec<OsString>,
    cwd: PathBuf,
    env: Vec<(OsString, Option<OsString>)>,
    stdin: Option<Vec<u8>>,
}

impl<'a> Cmd<'a> {
    pub fn cwd(mut self, p: impl AsRef<Path>) -> Self {
        self.cwd = p.as_ref().to_path_buf();
        self
    }
    pub fn env(mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> Self {
        self.env.push((k.as_ref().into(), Some(v.as_ref().into())));
        self
    }
    pub fn env_remove(mut self, k: impl AsRef<OsStr>) -> Self {
        self.env.push((k.as_ref().into(), None));
        self
    }
    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }
    pub fn run(self) -> Out {
        let t = self.t;
        let mut c = Command::new(&self.prog);
        c.args(&self.args).current_dir(&self.cwd);
        t.apply_env(&mut c, &self.cwd);
        for (k, v) in &self.env {
            match v {
                Some(v) => c.env(k, v),
                None => c.env_remove(k),
            };
        }
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        c.stdin(if self.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let desc = format!(
            "{} {}",
            Path::new(&self.prog)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            self.args
                .iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut child = {
            let _g = SPAWN_LOCK.read().unwrap();
            c.spawn()
                .unwrap_or_else(|e| panic!("cannot spawn `{desc}`: {e}"))
        };
        if let Some(data) = &self.stdin {
            let mut si = child.stdin.take().unwrap();
            let _ = si.write_all(data);
        }
        let o = child.wait_with_output().unwrap();
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            desc,
        }
    }
    /// Run and assert exit status 0.
    #[track_caller]
    pub fn ok(self) -> Out {
        self.run().ok()
    }
    /// Run and assert a specific exit status.
    #[track_caller]
    pub fn code(self, n: i32) -> Out {
        self.run().code(n)
    }
}

// ------------------------------------------------------------------------ T

pub struct T {
    pub root: PathBuf,
    pub tmp: PathBuf,
    pub home: PathBuf,
    pub store: PathBuf,
    pub proj: PathBuf,
    pub skill: PathBuf,
    /// `AGENT_MEMORY_SELF` default.
    pub self_path: PathBuf,
    env: Vec<(String, Option<String>)>,
    pub path_dirs: Vec<PathBuf>,
    // dream fixture state
    pub ds: PathBuf,
    pub dp1: PathBuf,
    pub dp2: PathBuf,
    pub dw: PathBuf,
    pub stub_log: PathBuf,
    pub tskill: PathBuf,
    pub tmem: PathBuf,
}

impl T {
    pub fn new() -> T {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let tmp = base.join(format!("memtest-{}-{n}-{nanos}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let home = tmp.join("home");
        let proj = tmp.join("code/some-api");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&proj).unwrap();
        let skill = root.join("skills/agent-memory");
        let t = T {
            self_path: skill.join("bin/memory"),
            skill,
            root,
            store: tmp.join("store"),
            home,
            proj,
            stub_log: tmp.join("stub"),
            tskill: tmp.join("skill"),
            tmem: tmp.join("skill/bin/memory"),
            ds: PathBuf::new(),
            dp1: PathBuf::new(),
            dp2: PathBuf::new(),
            dw: PathBuf::new(),
            env: Vec::new(),
            path_dirs: Vec::new(),
            tmp,
        };
        for args in [
            ["config", "--global", "user.email", "test@example.com"],
            ["config", "--global", "user.name", "Test"],
            ["config", "--global", "init.defaultBranch", "main"],
        ] {
            t.cmd("git", &args).cwd(&t.tmp).ok();
        }
        t
    }

    fn apply_env(&self, c: &mut Command, cwd: &Path) {
        c.env("AGENT_MEMORY_HOME", &self.store)
            .env("HOME", &self.home)
            .env("PWD", cwd)
            .env("AGENT_MEMORY_BACKUP_LOCK_WAIT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("AGENT_MEMORY_SKILL_DIR", &self.skill)
            .env("AGENT_MEMORY_SELF", &self.self_path)
            .env("MEMORY_BIN", MEMORY_BIN)
            // Anything that runs the skill launcher (bin/memory) gets the binary under test.
            .env("AGENT_MEMORY_BIN", MEMORY_BIN);
        for k in [
            "XDG_CONFIG_HOME",
            "AGENT_MEMORY_DREAM_MODEL",
            "AGENT_MEMORY_TODAY",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
        ] {
            c.env_remove(k);
        }
        for (k, v) in &self.env {
            match v {
                Some(v) => c.env(k, v),
                None => c.env_remove(k),
            };
        }
        if !self.path_dirs.is_empty() {
            let mut parts: Vec<PathBuf> = self.path_dirs.iter().rev().cloned().collect();
            if let Some(p) = std::env::var_os("PATH") {
                parts.extend(std::env::split_paths(&p));
            }
            c.env("PATH", std::env::join_paths(parts).unwrap());
        }
    }

    /// Persistent env for all later commands (Bash `export K=V`).
    pub fn set_env(&mut self, k: &str, v: impl AsRef<OsStr>) {
        self.env.retain(|(kk, _)| kk != k);
        self.env
            .push((k.into(), Some(v.as_ref().to_string_lossy().into_owned())));
    }
    /// Persistent removal (Bash `unset K`).
    pub fn unset_env(&mut self, k: &str) {
        self.env.retain(|(kk, _)| kk != k);
        self.env.push((k.into(), None));
    }
    /// Put `dir` in front of PATH for all later commands.
    pub fn prepend_path(&mut self, dir: impl AsRef<Path>) {
        self.path_dirs.push(dir.as_ref().to_path_buf());
    }

    /// Run the binary under test.
    pub fn memory<S: AsRef<OsStr>>(&self, args: &[S]) -> Cmd<'_> {
        self.cmd(MEMORY_BIN, args)
    }
    /// Run any program with the isolated environment.
    pub fn cmd<S: AsRef<OsStr>>(&self, prog: impl AsRef<OsStr>, args: &[S]) -> Cmd<'_> {
        Cmd {
            t: self,
            prog: prog.as_ref().into(),
            args: args.iter().map(|a| a.as_ref().into()).collect(),
            cwd: self.proj.clone(),
            env: Vec::new(),
            stdin: None,
        }
    }
    /// `git -C <store> ...`
    pub fn git<S: AsRef<OsStr>>(&self, args: &[S]) -> Cmd<'_> {
        let mut full: Vec<OsString> = vec!["-C".into(), self.store.clone().into()];
        full.extend(args.iter().map(|a| a.as_ref().to_os_string()));
        self.cmd("git", &full)
    }

    /// Path relative to the default project dir (`t.proj`), e.g. `.memory/project`.
    pub fn p(&self, rel: &str) -> PathBuf {
        self.proj.join(rel)
    }

    pub fn write(&self, p: impl AsRef<Path>, text: &str) {
        let p = p.as_ref();
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(p, text).unwrap();
    }

    pub fn write_exec(&self, p: impl AsRef<Path>, text: &str) {
        let p = p.as_ref();
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).unwrap();
        }
        let _g = SPAWN_LOCK.write().unwrap();
        fs::write(p, text).unwrap();
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
    }

    // ------------------------------------------------------------- dream

    /// Workspace "work" with projects some-api (`t.proj`) and other-api.
    pub fn dream_fixture(&mut self) {
        self.set_env("AGENT_MEMORY_TODAY", "2026-09-30");
        self.ds = self.store.join("work");
        self.dp1 = self.ds.join("projects/some-api");
        self.dp2 = self.ds.join("projects/other-api");
        self.dw = self.ds.join("workspace");
        self.memory(&["init", "work"]).ok();
        fs::create_dir_all(self.other_proj()).unwrap();
        self.memory(&["init", "work"]).cwd(self.other_proj()).ok();
    }

    pub fn other_proj(&self) -> PathBuf {
        self.tmp.join("code/other-api")
    }

    pub fn dream_reindex(&self) {
        self.memory(&["index"]).ok();
        self.memory(&["index"]).cwd(self.other_proj()).ok();
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.tmp.join("bin")
    }

    /// Fake `opencode` recording pwd/args/config/db/status; knobs STUB_EDIT, STUB_EXIT.
    pub fn install_opencode_stub(&mut self) {
        fs::create_dir_all(&self.stub_log).unwrap();
        let log = self.stub_log.clone();
        self.set_env("STUB_LOG", log);
        self.write_exec(self.bin_dir().join("opencode"), OPENCODE_STUB);
        let b = self.bin_dir();
        self.prepend_path(b);
    }

    /// Store with a committed baseline that contains stale-fact.
    pub fn dream_launch_fixture(&mut self) {
        self.dream_fixture();
        dfact(
            &self.dp1,
            "stale-fact",
            "decision",
            "an outdated decision",
            "2026-09-04",
            None,
        );
        self.dream_reindex();
        self.memory(&["backup"]).ok();
        self.install_opencode_stub();
    }

    pub fn commit_count(&self) -> usize {
        let o = self.git(&["log", "--oneline"]).ok();
        o.stdout.lines().count()
    }

    /// Stub that, like the plugin after an agent step, runs `memory backup` mid-run.
    pub fn install_backup_calling_stub(&self) {
        self.write_exec(self.bin_dir().join("opencode"), BACKUP_CALLING_STUB);
    }

    pub fn dream_template_fixture(&mut self) {
        self.dream_launch_fixture();
        self.cmd(
            "cp",
            &[
                OsStr::new("-R"),
                self.skill.as_os_str(),
                self.tskill.as_os_str(),
            ][..],
        )
        .ok();
        self.write(
            self.tskill.join("dream/opencode.json"),
            "{\n  \"store\": \"__STORE__\",\n  \"dirs\": [\n    \"__PROJECT_DIR__\",\n  ],\n  \"web\": \"x\", \"_tag\": \"__WEB_ONLY__\"\n  \"end\": 1\n}\n",
        );
    }

    /// Stub writing a fake OpenCode session db; STUB_TOOLS = `cmd=ok|denied ...`.
    pub fn install_db_stub(&self) {
        self.write_exec(self.bin_dir().join("opencode"), DB_STUB);
    }

    /// `false` when sqlite3 is unavailable (the caller should return early).
    pub fn dream_db_fixture(&mut self) -> bool {
        if Command::new("sqlite3").arg("--version").output().is_err() {
            eprintln!("  (skipped: sqlite3 not found)");
            return false;
        }
        self.dream_launch_fixture();
        self.install_db_stub();
        true
    }

    /// Stub that runs `$STUB_ACT` (shell snippet, cwd = store) then exits 0.
    pub fn install_acting_stub(&self) {
        self.write_exec(self.bin_dir().join("opencode"), ACTING_STUB);
    }

    #[track_caller]
    pub fn assert_escape_is_failed_review(&self, act: &str, expected: &str) {
        let marker = |t: &T| {
            t.git(&["rev-parse", "-q", "--verify", "refs/memory-dream/last"])
                .run()
                .so()
        };
        let before = marker(self);
        let out = self.memory(&["dream"]).env("STUB_ACT", act).run().code(1);
        out.assert_contains("run not counted");
        out.assert_contains(expected);
        assert_contains(
            self.git(&["log", "-1", "--format=%s"]).ok().so(),
            "dream FAILED",
        );
        assert_contains(self.git(&["log", "-1", "--format=%B"]).ok().so(), expected);
        assert_eq_str(self.git(&["status", "--porcelain"]).ok().so(), "");
        assert_eq_str(marker(self), before);
        assert_absent(self.store.join(".git/memory-dream.lock"));
        // Nothing is deleted automatically.
        if fs::symlink_metadata(self.store.join(expected)).is_err() {
            fail(&format!("unexpected path was deleted: {expected}"));
        }
    }
}

impl Drop for T {
    fn drop(&mut self) {
        // Tests may leave read-only dirs behind; make everything removable.
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("u+rwx")
            .arg(&self.tmp)
            .output();
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

// ------------------------------------------------------------------- stubs

const OPENCODE_STUB: &str = r#"#!/usr/bin/env bash
pwd -P > "$STUB_LOG/pwd"
printf '%s\n' "$@" > "$STUB_LOG/args"
printf '%s' "${OPENCODE_CONFIG_CONTENT:-}" > "$STUB_LOG/config"
printf '%s' "${OPENCODE_DB:-}" > "$STUB_LOG/db"
if [ -e "${OPENCODE_DB:-}" ] || [ -e "${OPENCODE_DB:-}-wal" ] || [ -e "${OPENCODE_DB:-}-shm" ]; then
  echo yes > "$STUB_LOG/db-existed"
else
  echo no > "$STUB_LOG/db-existed"
fi
git status --porcelain --untracked-files=all > "$STUB_LOG/status-at-run"
git log --oneline | wc -l | tr -d ' ' > "$STUB_LOG/commits-at-run"
if [ -n "${STUB_EDIT:-}" ]; then
  printf -- '---\ndescription: merged replacement\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' \
    > work/projects/some-api/merged-fact.md
fi
exit "${STUB_EXIT:-0}"
"#;

const BACKUP_CALLING_STUB: &str = r#"#!/usr/bin/env bash
printf -- '---\ndescription: mid-run edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' \
  > work/projects/some-api/mid-run.md
"$MEMORY_BIN" backup > "$STUB_LOG/backup-out" 2>&1
[ -d .git/memory-dream.lock ] && echo held > "$STUB_LOG/lock"
printf '%s\n' "$@" > "$STUB_LOG/args"
exit "${STUB_EXIT:-0}"
"#;

const DB_STUB: &str = r#"#!/usr/bin/env bash
rm -f "$OPENCODE_DB"
sqlite3 "$OPENCODE_DB" "create table session_message (id text primary key, session_id text, type text, seq integer, time_created integer, time_updated integer, data text);"
seq=0
for spec in $STUB_TOOLS; do
  seq=$((seq + 1))
  cmd="${spec%%=*}"; state="${spec#*=}"; cmd="${cmd//+/ }"
  if [ "$state" = ok ]; then
    part="{\"type\":\"tool\",\"name\":\"shell\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"$cmd\"}}}"
  else
    part="{\"type\":\"tool\",\"name\":\"shell\",\"state\":{\"status\":\"error\",\"input\":{\"command\":\"$cmd\"},\"error\":{\"type\":\"permission.rejected\",\"message\":\"Permission denied: shell\"}}}"
  fi
  sqlite3 "$OPENCODE_DB" "insert into session_message values ('m$seq','s','assistant',$seq,0,0,'{\"content\":[$part]}');"
done
exit 0
"#;

const ACTING_STUB: &str = r#"#!/usr/bin/env bash
eval "$STUB_ACT"
exit 0
"#;
