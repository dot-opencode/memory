//! Dream env validation, backup contention, templates, sqlite sandbox checks,
//! escape/failed-review, symlink refusal and remaining init/backup tests.
//! Ported from the former Bash suite tests/run.sh.

mod common;
use common::*;

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

// ------------------------------------------------------------ local helpers

/// `"$TMEM" <args>`: the binary under test pointed at the skill copy.
fn tm<'a>(t: &'a T, args: &[&str]) -> Cmd<'a> {
    t.memory(args).env("AGENT_MEMORY_SKILL_DIR", &t.tskill)
}

fn sp(p: &Path) -> String {
    p.display().to_string()
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// Poll `cond` every 100ms, at most `tries` times.
fn wait_for(tries: usize, cond: impl Fn() -> bool) {
    let mut i = 0;
    while !cond() && i < tries {
        sleep(Duration::from_millis(100));
        i += 1;
    }
}

fn file_nonempty(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

fn ps_lstart(pid: u32) -> String {
    let o = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn pid_alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Background `memory <args>` (with the dream fixture env) writing stdout+stderr to `out`.
fn spawn_memory(t: &T, args: &[&str], envs: &[(&str, &Path)], out: &Path) -> Child {
    let f = fs::File::create(out).unwrap();
    let mut c = Command::new(MEMORY_BIN);
    c.args(args)
        .current_dir(&t.proj)
        .env("AGENT_MEMORY_HOME", &t.store)
        .env("HOME", &t.home)
        .env("PWD", &t.proj)
        .env("AGENT_MEMORY_BACKUP_LOCK_WAIT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("AGENT_MEMORY_SKILL_DIR", &t.skill)
        .env("AGENT_MEMORY_SELF", &t.self_path)
        .env("MEMORY_BIN", MEMORY_BIN)
        .env("AGENT_MEMORY_TODAY", "2026-09-30")
        .env("STUB_LOG", &t.stub_log);
    for k in [
        "XDG_CONFIG_HOME",
        "AGENT_MEMORY_DREAM_MODEL",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
    ] {
        c.env_remove(k);
    }
    if !t.path_dirs.is_empty() {
        let mut parts: Vec<PathBuf> = t.path_dirs.iter().rev().cloned().collect();
        if let Some(p) = std::env::var_os("PATH") {
            parts.extend(std::env::split_paths(&p));
        }
        c.env("PATH", std::env::join_paths(parts).unwrap());
    }
    for (k, v) in envs {
        c.env(k, v);
    }
    c.stdin(Stdio::null())
        .stdout(f.try_clone().unwrap())
        .stderr(f);
    c.spawn().unwrap()
}

fn real_git(t: &T) -> String {
    t.cmd("sh", &["-c", "command -v git"]).ok().so()
}

fn has_sqlite3() -> bool {
    Command::new("sqlite3").arg("--version").output().is_ok()
}

fn ref_exists(t: &T, real: Option<&str>) -> bool {
    let args = [
        "-C",
        t.store.to_str().unwrap(),
        "rev-parse",
        "-q",
        "--verify",
        "refs/memory-dream/last",
    ];
    t.cmd(real.unwrap_or("git"), &args).run().code == 0
}

fn head(t: &T) -> String {
    t.git(&["rev-parse", "HEAD"]).ok().so()
}

fn find_symlinks(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if e.file_name() == ".git" {
            continue;
        }
        let ft = e.file_type().unwrap();
        if ft.is_symlink() {
            out.push(p);
        } else if ft.is_dir() {
            find_symlinks(&p, out);
        }
    }
}

/// `env -i HOME PATH AGENT_MEMORY_HOME AGENT_MEMORY_TODAY [setting] memory <args>`.
fn run_clean(t: &T, setting: &str, args: &[&str]) -> String {
    let mut c = Command::new(MEMORY_BIN);
    c.env_clear()
        .env("HOME", &t.home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("AGENT_MEMORY_HOME", &t.store)
        .env("AGENT_MEMORY_TODAY", "2026-09-30")
        .current_dir(&t.proj)
        .args(args);
    if !setting.is_empty() {
        let (k, v) = setting.split_once('=').unwrap();
        c.env(k, v);
    }
    let o = c.output().unwrap();
    assert!(
        o.status.success(),
        "env -i memory {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

// -------------------------------------------------------------------- tests

#[test]
fn dream_timeout_flag_and_env_validation() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let out = t.memory(&["dream", "--timeout", "abc"]).run().code(1);
    out.assert_contains("usage: memory dream");
    let out = t
        .memory(&["dream"])
        .env("AGENT_MEMORY_DREAM_TIMEOUT", "x")
        .run()
        .code(1);
    out.assert_contains("invalid dream timeout");
    t.memory(&["dream", "--timeout", "5"]).ok();
    assert_file(t.stub_log.join("args"));
}

#[test]
fn dream_signal_kills_child_and_releases_lock() {
    let mut t = T::new();
    t.dream_template_fixture();
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
printf -- '---\ndescription: interrupted edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/interrupted.md
echo $$ > "$STUB_LOG/pid"
exec sleep 60
"#,
    );
    let pidf = t.stub_log.join("pid");
    let outf = t.stub_log.join("dream-out");
    let mut dream = spawn_memory(
        &t,
        &["dream"],
        &[
            ("AGENT_MEMORY_DREAM_KILL_GRACE", Path::new("1")),
            ("AGENT_MEMORY_SKILL_DIR", &t.tskill),
        ],
        &outf,
    );
    wait_for(600, || file_nonempty(&pidf));
    if !file_nonempty(&pidf) {
        let _ = dream.kill();
        let _ = dream.wait();
        fail(&format!("opencode did not become ready: {}", cat(&outf)));
    }
    let cpid = cat(&pidf);
    Command::new("kill")
        .args(["-TERM", &dream.id().to_string()])
        .status()
        .unwrap();
    let _ = dream.wait();
    if pid_alive(&cpid) {
        fail("child survived SIGTERM of dream");
    }
    if exists(&t.store.join(".git/memory-dream.lock")) {
        fail("lock left after SIGTERM");
    }
    assert_contains(
        t.git(&["log", "-1", "--format=%s"]).ok().so(),
        "dream FAILED",
    );
    assert_eq_str(t.git(&["status", "--porcelain"]).ok().so(), "");
}

#[test]
fn dream_lock_is_taken_before_pending_backup() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let mut live = Command::new("sleep").arg("30").spawn().unwrap();
    let h = head(&t);
    dfact(
        &t.dp1,
        "pending-fact",
        "convention",
        "pending",
        "2026-09-30",
        None,
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    t.write(
        t.store.join(".git/memory-dream.lock"),
        &format!("{}\n{}\n{}\n", live.id(), ps_lstart(live.id()), now),
    );
    let out = t.memory(&["dream", "--if-changed"]).run();
    let _ = live.kill();
    let _ = live.wait();
    out.code(1).assert_contains("already running");
    assert_eq_str(head(&t), h);
}

#[test]
fn dream_live_writer_never_expires_by_age() {
    let mut t = T::new();
    t.dream_template_fixture();
    let lock = t.store.join(".git/memory-dream.lock");
    fs::create_dir(&lock).unwrap();
    let me = std::process::id();
    t.write(
        lock.join("owner"),
        &format!("{}\n{}\n1\n", me, ps_lstart(me)),
    );
    tm(&t, &["dream", "--timeout", "480"]).run().code(1);
    if !lock.join("owner").is_file() {
        fail("live old lock removed");
    }
    fs::remove_file(lock.join("owner")).unwrap();
    fs::remove_dir(&lock).unwrap();
}

#[test]
fn backup_ignores_user_hooks_and_nothing_to_back_up_in_empty_store() {
    let t = T::new();
    let empty = t.tmp.join("empty-store");
    fs::create_dir(&empty).unwrap();
    t.cmd("git", &["init", "-q", &sp(&empty)]).ok();
    t.cmd("git", &["config", "--global", "user.useConfigOnly", "true"])
        .ok();
    t.cmd("git", &["config", "--global", "--unset", "user.email"])
        .ok();
    let out = t.memory(&["backup"]).env("AGENT_MEMORY_HOME", &empty).ok();
    assert_eq_str(out.so(), "nothing to back up");
    t.cmd(
        "git",
        &["config", "--global", "user.email", "test@example.com"],
    )
    .ok();
    t.memory(&["init", "work"]).ok();
    let hooks = t.store.join(".git/hooks");
    fs::create_dir_all(&hooks).unwrap();
    t.write_exec(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n");
    write_fact(
        &t.p(".memory/project"),
        "a-fact",
        "context",
        "something learned",
    );
    let out = t.memory(&["backup"]).ok();
    assert_contains(out.stdout, "backed up");
}

#[test]
fn backup_retries_while_index_lock_exists() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    write_fact(
        &t.p(".memory/project"),
        "a-fact",
        "context",
        "something learned",
    );
    let lock = t.store.join(".git/index.lock");
    fs::write(&lock, "").unwrap();
    let h = std::thread::spawn(move || {
        sleep(Duration::from_secs(2));
        let _ = fs::remove_file(lock);
    });
    let out = t.memory(&["backup"]).ok();
    assert_contains(out.stdout, "backed up");
    h.join().unwrap();
}

#[test]
fn backup_push_refuses_detached_head() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    write_fact(
        &t.p(".memory/project"),
        "a-fact",
        "context",
        "something learned",
    );
    t.memory(&["backup"]).ok();
    let remote = t.tmp.join("remote.git");
    t.cmd("git", &["init", "-q", "--bare", &sp(&remote)]).ok();
    t.git(&["remote", "add", "origin", &sp(&remote)]).ok();
    t.git(&["checkout", "-q", "--detach"]).ok();
    t.memory(&["backup", "--push"])
        .run()
        .code(1)
        .assert_contains("detached HEAD");
    t.git(&["checkout", "-q", "main"]).ok();
    t.memory(&["backup", "--push"]).ok();
    let r = t
        .cmd(
            "git",
            &["-C", &sp(&remote), "rev-parse", "-q", "--verify", "main"],
        )
        .run();
    if r.code != 0 {
        fail("push did not reach the remote");
    }
}

#[test]
fn dream_if_changed_workspace_scope() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.memory(&["dream", "--workspace", "work"]).ok();
    let hw = t.store.join("home/workspace");
    fs::create_dir_all(&hw).unwrap();
    dfact(
        &hw,
        "other-ws",
        "convention",
        "another workspace",
        "2026-09-30",
        None,
    );
    let _ = fs::remove_file(t.stub_log.join("args"));
    let out = t
        .memory(&["dream", "--if-changed", "--workspace", "work"])
        .ok();
    assert_contains(&out.stdout, "skipping");
    if exists(&t.stub_log.join("args")) {
        fail("a change in another workspace triggered the run");
    }
}

#[test]
fn init_rejects_project_link_from_other_workspace() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_file(t.p(".memory/workspace")).unwrap();
    t.memory(&["init", "other"])
        .run()
        .code(1)
        .assert_contains("outside workspace 'other'");
}

#[test]
fn doctor_reports_workspace_mismatch() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let ow = t.store.join("other/workspace");
    fs::create_dir_all(&ow).unwrap();
    fs::write(ow.join("INDEX.md"), "").unwrap();
    let link = t.p(".memory/workspace");
    fs::remove_file(&link).unwrap();
    symlink(
        fs::canonicalize(&t.store).unwrap().join("other/workspace"),
        &link,
    )
    .unwrap();
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("WORKSPACE MISMATCH");
}

#[test]
fn dangling_link_check_survives_invalid_utf8() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    write_fact(
        &t.p(".memory/project"),
        "a-fact",
        "context",
        "something learned",
    );
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(t.p(".memory/project/a-fact.md"))
            .unwrap();
        f.write_all(b"bad \xff\xfe bytes [[missing-one]]\n")
            .unwrap();
    }
    assert_contains(
        t.memory(&["doctor"]).run().combined(),
        "DANGLING WIKI LINK: .memory/project/a-fact.md -> [[missing-one]]",
    );
    assert_contains(
        t.memory(&["dream", "--report"]).run().combined(),
        "dangling link",
    );
}

#[test]
fn symlinked_facts_are_skipped() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    write_fact(
        &t.p(".memory/project"),
        "real-fact",
        "context",
        "a real one",
    );
    t.write(
        t.tmp.join("outside.md"),
        "---\ndescription: outside\ntype: context\ncreated: 2026-09-04\n---\nx\n",
    );
    symlink(
        t.tmp.join("outside.md"),
        t.p(".memory/project/linked-fact.md"),
    )
    .unwrap();
    t.memory(&["index"]).ok();
    let idx = cat(t.p(".memory/project/INDEX.md"));
    assert_not_contains(&idx, "linked-fact");
    assert_contains(&idx, "real-fact");
    assert_contains(
        t.memory(&["dream", "--report"]).run().combined(),
        "symlinked fact ignored",
    );
}

#[test]
fn init_excludes_scratch_files_from_store_git() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["init", "work"]).ok();
    let ex = cat(t.store.join(".git/info/exclude"));
    let lines: Vec<&str> = ex.lines().collect();
    if !lines.contains(&".memory-index.*") {
        fail("index temp pattern missing");
    }
    if lines.contains(&"memory-dream.*") {
        fail("broad dream pattern retained");
    }
    assert_eq_str(
        lines
            .iter()
            .filter(|l| **l == ".memory-index.*")
            .count()
            .to_string(),
        "1",
    );
    fs::write(t.p(".memory/project/.memory-index.AbCdEf"), "").unwrap();
    let st = t
        .git(&["status", "--porcelain", "--untracked-files=all"])
        .ok()
        .stdout;
    assert_eq_str(
        st.lines()
            .filter(|l| l.contains("memory-index"))
            .count()
            .to_string(),
        "0",
    );
}

#[test]
fn stale_index_temp_files_are_removed_on_publish() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let f = t.p(".memory/project/.memory-index.old123");
    fs::write(&f, "").unwrap();
    t.cmd("touch", &["-t", "202001010000", &sp(&f)]).ok();
    t.memory(&["index"]).ok();
    if exists(&f) {
        fail("stale temp file kept");
    }
}

#[test]
fn home_unset_gives_clear_error() {
    let t = T::new();
    let out = t
        .memory(&["status"])
        .env_remove("HOME")
        .env_remove("AGENT_MEMORY_HOME")
        .run()
        .code(1);
    out.assert_contains("HOME is not set");
}

#[test]
fn dream_template_project_dir_and_web_tokens() {
    let mut t = T::new();
    t.dream_template_fixture();
    let out = tm(&t, &["dream", "--print-config"]).ok().so();
    assert_not_contains(&out, "__PROJECT_DIR__");
    assert_not_contains(&out, "__WEB_ONLY__");
    assert_contains(
        &out,
        format!("\"{}\",", sp(&fs::canonicalize(&t.proj).unwrap())),
    );
    assert_contains(
        &out,
        format!("\"{}\",", sp(&fs::canonicalize(t.other_proj()).unwrap())),
    );
    assert_not_contains(&out, "_tag");
    let out = tm(&t, &["dream", "--print-config", "--web"]).ok().so();
    assert_contains(&out, "\"_tag\": \"\"");
    assert_not_contains(&out, "__WEB_ONLY__");
    // Missing directories are skipped; a workspace scope limits the list.
    fs::remove_dir_all(t.other_proj()).unwrap();
    let out = tm(&t, &["dream", "--print-config"]).ok().so();
    assert_not_contains(&out, "other-api");
    fs::create_dir_all(t.store.join("home/projects/p")).unwrap();
    fs::create_dir_all(t.tmp.join("code/home-proj")).unwrap();
    t.write(
        t.store.join("home/projects/p/.project-path"),
        &format!("{}\n", sp(&t.tmp.join("code/home-proj"))),
    );
    let out = tm(&t, &["dream", "--print-config", "--workspace", "work"])
        .ok()
        .so();
    assert_not_contains(&out, "home-proj");
    let out = tm(&t, &["dream", "--print-config", "--workspace", "home"])
        .ok()
        .so();
    assert_contains(&out, "home-proj");
    assert_not_contains(&out, "some-api");
}

#[test]
fn dream_template_without_project_dirs_deletes_line() {
    let mut t = T::new();
    t.dream_template_fixture();
    fs::remove_dir_all(t.other_proj()).unwrap();
    fs::remove_dir_all(&t.proj).unwrap();
    let out = tm(&t, &["dream", "--print-config"]).cwd(&t.tmp).ok().so();
    assert_not_contains(&out, "__PROJECT_DIR__");
    assert_contains(&out, "\"dirs\": [");
}

#[test]
fn dream_run_disables_project_config() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
printf '%s %s\n' "${OPENCODE_CONFIG_PROJECT_DISABLE:-}" "${OPENCODE_DISABLE_PROJECT_CONFIG:-}" > "$STUB_LOG/env"
"#,
    );
    t.memory(&["dream"]).ok();
    assert_eq_str(cat(t.stub_log.join("env")), "1 1");
}

#[test]
fn dream_prompt_explains_restricted_git_commands() {
    let mut t = T::new();
    t.dream_fixture();
    let out = t.memory(&["dream", "--print-config"]).ok().so();
    assert_contains(
        &out,
        "Git history and content-reading commands are disabled",
    );
    assert_contains(&out, "git -C <dir> status --short");
}

#[test]
fn dream_first_run_is_full_then_incremental() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let args = t.stub_log.join("args");
    // No previous run: the whole store is in scope.
    let out = t.memory(&["dream", "--print-config"]).ok().so();
    assert_not_contains(&out, "Incremental run");
    t.memory(&["dream"]).ok();
    assert_not_contains(cat(&args), "Incremental run");
    // Nothing changed since: only hygiene.
    let out = t.memory(&["dream", "--print-config"]).ok().so();
    assert_contains(&out, "Incremental run");
    assert_contains(&out, "No fact changed since then");
    // Changed, new (uncommitted) and deleted facts; INDEX.md does not count.
    dfact(
        &t.dp1,
        "fresh-one",
        "convention",
        "a fresh convention",
        "2026-09-30",
        None,
    );
    dfact(
        &t.dw,
        "fresh-two",
        "gotcha",
        "a fresh gotcha",
        "2026-09-30",
        None,
    );
    t.git(&["rm", "-q", "work/projects/some-api/stale-fact.md"])
        .ok();
    t.memory(&["backup"]).ok();
    dfact(
        &t.dp1,
        "untracked-one",
        "decision",
        "an untracked decision",
        "2026-09-30",
        None,
    );
    let out = t.memory(&["dream", "--print-config"]).ok().so();
    assert_contains(&out, "Only these 3 facts were added or changed since then:");
    assert_contains(&out, "- work/projects/some-api/fresh-one.md");
    assert_contains(&out, "- work/projects/some-api/untracked-one.md");
    assert_contains(&out, "- work/workspace/fresh-two.md");
    assert_not_contains(&out, "stale-fact.md");
    assert_not_contains(&out, "INDEX.md\n");
    // --full ignores the marker.
    let out = t.memory(&["dream", "--print-config", "--full"]).ok().so();
    assert_not_contains(&out, "Incremental run");
    t.memory(&["dream", "--full"]).ok();
    assert_not_contains(cat(&args), "Incremental run");
}

#[test]
fn dream_incremental_is_per_workspace() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.memory(&["dream"]).ok();
    // A workspace run has its own marker, so the first one is full.
    let out = t
        .memory(&["dream", "--print-config", "--workspace", "work"])
        .ok()
        .so();
    assert_not_contains(&out, "Incremental run");
    t.memory(&["dream", "--workspace", "work"]).ok();
    let hw = t.store.join("home/workspace");
    fs::create_dir_all(&hw).unwrap();
    dfact(
        &hw,
        "other-ws",
        "convention",
        "another workspace",
        "2026-09-30",
        None,
    );
    let out = t
        .memory(&["dream", "--print-config", "--workspace", "work"])
        .ok()
        .so();
    assert_contains(&out, "No fact changed since then");
    let out = t.memory(&["dream", "--print-config"]).ok().so();
    assert_contains(&out, "- home/workspace/other-ws.md");
}

#[test]
fn description_length_counts_characters_without_locale() {
    let mut t = T::new();
    t.dream_fixture();
    // 76 characters, 136 bytes: under launchd (no LANG) bash used to count bytes.
    dfact(
        &t.dp1,
        "cyrillic",
        "gotcha",
        "Падающий ARK HAL-плагин может скрыть все аудиоустройства, включая встроенные",
        "2026-09-30",
        None,
    );
    t.dream_reindex();
    for setting in ["", "LANG=C", "LC_ALL=C", "LC_ALL=POSIX"] {
        let out = run_clean(&t, setting, &["dream", "--report"]);
        assert_not_contains(&out, "long description");
        let out = run_clean(&t, setting, &["doctor"]);
        assert_not_contains(&out, "LONG DESCRIPTION");
    }
    // A really long one is still reported, with its length in characters.
    dfact(
        &t.dp1,
        "cyrillic",
        "gotcha",
        &"щ".repeat(121),
        "2026-09-30",
        None,
    );
    t.dream_reindex();
    let out = run_clean(&t, "", &["dream", "--report"]);
    assert_contains(&out, "long description (121 chars)");
}

#[test]
fn dream_rejects_full_with_report() {
    let mut t = T::new();
    t.dream_fixture();
    t.memory(&["dream", "--report", "--full"])
        .run()
        .code(1)
        .assert_contains("usage: memory dream");
}

#[test]
fn init_migrates_broad_exclude_and_keeps_legitimate_fact() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let ex = t.store.join(".git/info/exclude");
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&ex).unwrap();
        f.write_all(b"memory-dream.*\nkeep-me\n").unwrap();
    }
    t.memory(&["init", "work"]).ok();
    let text = cat(&ex);
    let lines: Vec<&str> = text.lines().collect();
    if lines.contains(&"memory-dream.*") {
        fail("broad exclude kept");
    }
    if !lines.contains(&"keep-me") {
        fail("unrelated exclude removed");
    }
    write_fact(
        &t.p(".memory/project"),
        "memory-dream",
        "context",
        "legitimate fact",
    );
    t.memory(&["backup"]).ok();
    t.git(&[
        "ls-files",
        "--error-unmatch",
        "work/projects/some-api/memory-dream.md",
    ])
    .ok();
}

#[test]
fn init_rejects_new_symlink_destinations_before_writes() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let outside = t.tmp.join("outside");
    fs::create_dir(&outside).unwrap();
    let empty = |p: &Path| fs::read_dir(p).unwrap().count() == 0;
    symlink(&outside, t.store.join("escape")).unwrap();
    let newp = t.tmp.join("code/new");
    fs::create_dir(&newp).unwrap();
    t.memory(&["init", "escape"]).cwd(&newp).run().code(1);
    assert!(empty(&outside), "outside not empty");
    symlink(&outside, t.store.join("work/projects/evil")).unwrap();
    t.memory(&["init", "work", "--name", "evil"])
        .cwd(&newp)
        .run()
        .code(1);
    symlink(&outside, t.store.join("work/groups/evil")).unwrap();
    t.memory(&["link-group", "evil"]).run().code(1);
    assert!(empty(&outside), "outside not empty");
}

#[test]
fn init_rejects_symlink_metadata_and_index() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let outside = t.tmp.join("outside");
    fs::write(&outside, "unchanged\n").unwrap();
    fs::remove_file(t.p(".memory/project/.project-path")).unwrap();
    symlink(&outside, t.p(".memory/project/.project-path")).unwrap();
    t.memory(&["init", "work"]).run().code(1);
    assert_eq_str(cat(&outside), "unchanged");
    fs::remove_file(t.p(".memory/project/.project-path")).unwrap();
    fs::remove_file(t.p(".memory/workspace/INDEX.md")).unwrap();
    symlink(&outside, t.p(".memory/workspace/INDEX.md")).unwrap();
    t.memory(&["init", "work"]).run().code(1);
    assert_eq_str(cat(&outside), "unchanged");
}

#[test]
fn init_mismatch_has_no_new_workspace_side_effect() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_file(t.p(".memory/workspace")).unwrap();
    let _ = t.memory(&["init", "other"]).run();
    if exists(&t.store.join("other")) {
        fail("mismatch created workspace");
    }
}

#[test]
fn dream_template_literal_paths_and_controls() {
    let mut t = T::new();
    t.dream_template_fixture();
    let literal = t.tmp.join("code/__HOME__-__STORE__");
    fs::create_dir(&literal).unwrap();
    t.write(t.dp1.join(".project-path"), &format!("{}\n", sp(&literal)));
    let out = tm(&t, &["dream", "--print-config"]).ok().so();
    assert_contains(&out, sp(&literal));
    let tab = t.tmp.join("code/tab\tpath");
    fs::create_dir(&tab).unwrap();
    t.write(t.dp1.join(".project-path"), &format!("{}\n", sp(&tab)));
    let out = tm(&t, &["dream", "--print-config"]).ok().so();
    assert_not_contains(&out, "tab\tpath");
    let cr = t.tmp.join("code/cr\rpath");
    fs::create_dir(&cr).unwrap();
    let safe = t.tmp.join("code/safe-name");
    symlink(&cr, &safe).unwrap();
    t.write(t.dp1.join(".project-path"), &format!("{}\n", sp(&safe)));
    let out = tm(&t, &["dream", "--print-config"]).ok().so();
    assert_not_contains(&out, "cr\rpath");
}

#[test]
fn failed_dream_commit_failure_survives_next_backup() {
    let mut t = T::new();
    t.dream_template_fixture();
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
printf -- '---\ndescription: failed partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
git config user.useConfigOnly true
git config --global --unset user.email
git config --global user.useConfigOnly true
exit 7
"#,
    );
    let out = tm(&t, &["dream"]).run().code(1);
    out.assert_contains("could not be created");
    out.assert_not_contains("no changes to review");
    assert_file(t.store.join(".git/memory-dream.failed"));
    t.git(&["config", "user.email", "test@example.com"]).ok();
    let out = tm(&t, &["backup"]).ok();
    assert_contains(&out.stdout, "FAILED review");
    assert_eq_str(
        t.git(&["log", "-1", "--format=%s"]).ok().so(),
        "memory: dream FAILED (exit 7) — review",
    );
    if exists(&t.store.join(".git/memory-dream.failed")) {
        fail("marker retained after commit");
    }
}

#[test]
fn backups_share_atomic_writer_lock() {
    let mut t = T::new();
    t.dream_template_fixture();
    let rg = real_git(&t);
    t.set_env("REAL_GIT", &rg);
    t.write_exec(
        t.bin_dir().join("git"),
        r#"#!/usr/bin/env bash
for arg in "$@"; do
  if [ "$arg" = add ] && [ ! -f "$STUB_LOG/release" ]; then
    : > "$STUB_LOG/adding"
    while [ ! -f "$STUB_LOG/release" ]; do sleep 0.1; done
  fi
done
exec "$REAL_GIT" "$@"
"#,
    );
    dfact(&t.dp1, "pending", "decision", "pending", "2026-09-04", None);
    let adding = t.stub_log.join("adding");
    let release = t.stub_log.join("release");
    std::thread::scope(|s| {
        let writer = s.spawn(|| tm(&t, &["backup"]).run());
        wait_for(300, || exists(&adding));
        assert_file(&adding);
        let busy = tm(&t, &["backup"]).run();
        if busy.code == 0 {
            fail("busy backup reported success");
        }
        busy.assert_contains("request NOT saved");
        tm(&t, &["dream"]).run().code(1);
        fs::write(&release, "").unwrap();
        writer.join().unwrap();
    });
    assert_eq_str(t.git(&["status", "--porcelain"]).ok().so(), "");
}

#[test]
fn dream_success_marker_updated_under_writer_lock() {
    let mut t = T::new();
    t.dream_template_fixture();
    let rg = real_git(&t);
    t.set_env("REAL_GIT", &rg);
    t.write_exec(
        t.bin_dir().join("git"),
        r#"#!/usr/bin/env bash
for arg in "$@"; do
  if [ "$arg" = update-ref ]; then
    if [ -d "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ]; then
      echo held > "$STUB_LOG/ref-lock"
    else
      echo released > "$STUB_LOG/ref-lock"
    fi
  fi
done
exec "$REAL_GIT" "$@"
"#,
    );
    tm(&t, &["dream"]).ok();
    assert_eq_str(cat(t.stub_log.join("ref-lock")), "held");
    assert_eq_str(
        t.git(&["rev-parse", "refs/memory-dream/last"]).ok().so(),
        head(&t),
    );
}

#[test]
fn timeout_kills_stubborn_grandchildren() {
    let mut t = T::new();
    t.dream_template_fixture();
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
bash -c 'trap "" TERM; bash -c '\''trap "" TERM; echo $$ > "$STUB_LOG/grandchild"; while :; do sleep 1; done'\'' & echo $$ > "$STUB_LOG/child"; wait' &
while [ ! -f "$STUB_LOG/grandchild" ]; do sleep 0.1; done
echo $$ > "$STUB_LOG/pid"
wait
"#,
    );
    let _ = tm(&t, &["dream"])
        .env("AGENT_MEMORY_DREAM_TIMEOUT_SECS", "2")
        .env("AGENT_MEMORY_DREAM_KILL_GRACE", "1")
        .run();
    for f in ["child", "grandchild"] {
        assert_file(t.stub_log.join(f));
        let pid = cat(t.stub_log.join(f));
        let o = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !(state.is_empty() || state.contains('Z')) {
            fail(&format!("{f} still live after timeout: {pid} {state}"));
        }
    }
    if exists(&t.store.join(".git/memory-dream.lock")) {
        fail("timeout retained writer lock");
    }
}

#[test]
fn dream_policy_paths_reject_globs() {
    let mut t = T::new();
    t.dream_template_fixture();
    for name in ["app?", "app*", "app[12]"] {
        let d = t.tmp.join("code").join(name);
        fs::create_dir(&d).unwrap();
        t.write(t.dp1.join(".project-path"), &format!("{}\n", sp(&d)));
        let out = tm(&t, &["dream", "--print-config"]).ok();
        assert_not_contains(&out.stdout, sp(&d));
        assert_contains(&out.stderr, "skipping unsafe");
    }
    let bad_home = t.tmp.join("home?");
    fs::create_dir(&bad_home).unwrap();
    let out = tm(&t, &["dream", "--print-config"])
        .env("HOME", &bad_home)
        .run()
        .code(1);
    out.assert_contains("unsafe JSON/policy");
}

#[test]
fn successful_dream_persistence_failure_keeps_failed_marker() {
    let mut t = T::new();
    t.dream_template_fixture();
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
printf -- '---\ndescription: successful partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
git config --global --unset user.email
git config --global user.useConfigOnly true
exit 0
"#,
    );
    let out = tm(&t, &["dream"]).run();
    if out.code == 0 {
        fail("save failure returned success");
    }
    out.assert_contains("final persistence failed");
    assert_file(t.store.join(".git/memory-dream.failed"));
    t.cmd(
        "git",
        &["config", "--global", "user.email", "test@example.com"],
    )
    .ok();
    let out = tm(&t, &["backup"]).ok();
    assert_contains(&out.stdout, "FAILED review");
    assert_contains(
        t.git(&["log", "-1", "--format=%s"]).ok().so(),
        "dream FAILED",
    );
    assert_eq_str(t.git(&["status", "--porcelain"]).ok().so(), "");
}

#[test]
fn dream_update_ref_failure_keeps_failed_marker() {
    let mut t = T::new();
    t.dream_template_fixture();
    let rg = real_git(&t);
    t.set_env("REAL_GIT", &rg);
    t.write_exec(
        t.bin_dir().join("git"),
        r#"#!/usr/bin/env bash
for arg in "$@"; do if [ "$arg" = update-ref ]; then exit 9; fi; done
exec "$REAL_GIT" "$@"
"#,
    );
    let out = tm(&t, &["dream"]).run();
    if out.code == 0 {
        fail("ref failure returned success");
    }
    assert_file(t.store.join(".git/memory-dream.failed"));
    out.assert_contains("final persistence failed");
    if ref_exists(&t, Some(&rg)) {
        fail("advanced failed marker");
    }
}

#[test]
fn successful_dream_index_and_add_failure_keep_marker() {
    let mut t = T::new();
    t.dream_template_fixture();
    let rg = real_git(&t);
    t.set_env("REAL_GIT", &rg);
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
printf -- '---\ndescription: partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
if [ "$FAIL_SAVE" = index ]; then
  # The binary publishes INDEX.md with rename(2); a read-only level dir makes it fail.
  chmod 555 work/projects/some-api
else
  : > "$STUB_LOG/fail-add"
fi
"#,
    );
    t.write_exec(
        t.bin_dir().join("git"),
        r#"#!/usr/bin/env bash
if [ -f "$STUB_LOG/fail-add" ]; then
  for arg in "$@"; do if [ "$arg" = add ]; then exit 8; fi; done
fi
exec "$REAL_GIT" "$@"
"#,
    );
    for mode in ["index", "add"] {
        let out = tm(&t, &["dream"])
            .env("FAIL_SAVE", mode)
            .env("FAIL_BIN", t.bin_dir())
            .run();
        if out.code == 0 {
            fail(&format!("{mode} save failure returned success"));
        }
        assert_file(t.store.join(".git/memory-dream.failed"));
        out.assert_contains("final persistence failed");
        let lvl = t.store.join("work/projects/some-api");
        let _ = fs::set_permissions(&lvl, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_file(t.stub_log.join("fail-add"));
        tm(&t, &["backup"]).ok();
    }
}

#[test]
fn backup_after_add_contention_commits_newer_edit() {
    let mut t = T::new();
    t.dream_template_fixture();
    let rg = real_git(&t);
    t.set_env("REAL_GIT", &rg);
    t.write_exec(
        t.bin_dir().join("git"),
        r#"#!/usr/bin/env bash
"$REAL_GIT" "$@" || exit $?
for arg in "$@"; do
  if [ "$arg" = add ] && [ ! -f "$STUB_LOG/release" ]; then
    : > "$STUB_LOG/after-add"
    while [ ! -f "$STUB_LOG/release" ]; do sleep 0.1; done
  fi
done
"#,
    );
    dfact(
        &t.dp1,
        "contention",
        "decision",
        "version A",
        "2026-09-04",
        None,
    );
    let after_add = t.stub_log.join("after-add");
    let release = t.stub_log.join("release");
    std::thread::scope(|s| {
        let first = s.spawn(|| tm(&t, &["backup"]).run());
        wait_for(300, || exists(&after_add));
        assert_file(&after_add);
        dfact(
            &t.dp1,
            "contention",
            "decision",
            "version B",
            "2026-09-04",
            None,
        );
        let second = s.spawn(|| {
            tm(&t, &["backup"])
                .env("AGENT_MEMORY_BACKUP_LOCK_WAIT", "10")
                .run()
        });
        fs::write(&release, "").unwrap();
        first.join().unwrap();
        second.join().unwrap();
    });
    assert_eq_str(t.git(&["status", "--porcelain"]).ok().so(), "");
    assert_contains(
        t.git(&["show", "HEAD:work/projects/some-api/contention.md"])
            .ok()
            .so(),
        "version B",
    );
}

#[test]
fn dream_all_tools_denied_is_a_failed_run() {
    let mut t = T::new();
    if !t.dream_db_fixture() {
        return;
    }
    let out = t
        .memory(&["dream"])
        .env("STUB_TOOLS", "x=denied y=denied")
        .run()
        .code(1);
    out.assert_contains("sandbox denied the agent's tools; run not counted");
    out.assert_contains("0 tool call(s) succeeded, 2 denied");
    if ref_exists(&t, None) {
        fail("success ref advanced after a broken sandbox");
    }
    if exists(&t.store.join(".git/memory-dream.lock")) {
        fail("lock left behind");
    }
}

#[test]
fn dream_denied_first_report_call_is_a_failed_run() {
    let mut t = T::new();
    if !t.dream_db_fixture() {
        return;
    }
    let out = t
        .memory(&["dream"])
        .env("STUB_TOOLS", "/x/memory+dream+--report+now=denied other=ok")
        .run()
        .code(1);
    out.assert_contains("run not counted");
    if ref_exists(&t, None) {
        fail("success ref advanced although the report was denied");
    }
}

#[test]
fn dream_other_denials_are_reported_but_not_fatal() {
    let mut t = T::new();
    if !t.dream_db_fixture() {
        return;
    }
    let out = t
        .memory(&["dream"])
        .env(
            "STUB_TOOLS",
            "/x/memory+dream+--report+now=ok probe=denied probe2=denied",
        )
        .run();
    assert_contains(
        out.combined(),
        "2 tool call(s) were denied by the sandbox (1 succeeded)",
    );
    assert_eq_str(
        t.git(&["rev-parse", "refs/memory-dream/last"]).ok().so(),
        head(&t),
    );
}

#[test]
fn dream_unreadable_session_db_warns_and_still_succeeds() {
    let mut t = T::new();
    t.dream_launch_fixture(); // default stub writes no database
    let out = t.memory(&["dream"]).run();
    if has_sqlite3() {
        assert_contains(out.combined(), "cannot inspect the session database");
    }
    assert_eq_str(
        t.git(&["rev-parse", "refs/memory-dream/last"]).ok().so(),
        head(&t),
    );
}

#[test]
fn dream_signal_prints_interrupted_notice() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
printf -- '---\ndescription: interrupted edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/interrupted.md
echo $$ > "$STUB_LOG/pid"
exec sleep 60
"#,
    );
    let pidf = t.stub_log.join("pid");
    let outf = t.stub_log.join("dream-out");
    let mut dream = spawn_memory(
        &t,
        &["dream"],
        &[("AGENT_MEMORY_DREAM_KILL_GRACE", Path::new("1"))],
        &outf,
    );
    wait_for(600, || file_nonempty(&pidf));
    if !file_nonempty(&pidf) {
        let _ = dream.kill();
        let _ = dream.wait();
    }
    assert_file(&pidf);
    Command::new("kill")
        .args(["-TERM", &dream.id().to_string()])
        .status()
        .unwrap();
    let _ = dream.wait();
    assert_contains(cat(&outf), "opencode run interrupted (exit 143)");
    assert_contains(cat(&outf), "committed as FAILED review");
}

#[test]
fn dream_new_project_directory_is_a_failed_review() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.install_acting_stub();
    t.assert_escape_is_failed_review(
        &format!("mkdir work/projects/brand-new && printf -- '{FACT_BODY}' > work/projects/brand-new/x.md"),
        "work/projects/brand-new/x.md",
    );
}

#[test]
fn dream_new_group_directory_is_a_failed_review() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.install_acting_stub();
    t.assert_escape_is_failed_review(
        &format!("mkdir -p work/groups/brand-new && printf -- '{FACT_BODY}' > work/groups/brand-new/x.md"),
        "work/groups/brand-new/x.md",
    );
}

#[test]
fn dream_nested_hidden_and_non_markdown_files_are_failed_reviews() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.install_acting_stub();
    t.assert_escape_is_failed_review(
        &format!("mkdir work/projects/some-api/deeper && printf -- '{FACT_BODY}' > work/projects/some-api/deeper/x.md"),
        "work/projects/some-api/deeper/x.md",
    );
    t.assert_escape_is_failed_review(
        "printf x > work/projects/some-api/.hidden.md",
        "work/projects/some-api/.hidden.md",
    );
    t.assert_escape_is_failed_review(
        "printf x > work/projects/some-api/notes.txt",
        "work/projects/some-api/notes.txt",
    );
}

#[test]
fn dream_deleted_fact_is_a_failed_review_without_deleting_more() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.install_acting_stub();
    let out = t
        .memory(&["dream"])
        .env("STUB_ACT", "rm work/projects/some-api/stale-fact.md")
        .run()
        .code(1);
    out.assert_contains("work/projects/some-api/stale-fact.md");
    assert_contains(
        t.git(&["log", "-1", "--format=%s"]).ok().so(),
        "dream FAILED",
    );
}

#[test]
fn dream_staged_rename_is_reported_as_plain_delete_and_add() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.install_acting_stub();
    // A staged rename of an almost identical file: git would pair them as "R".
    let out = t
        .memory(&["dream"])
        .env(
            "STUB_ACT",
            "git mv work/projects/some-api/stale-fact.md work/projects/some-api/renamed-fact.md",
        )
        .run()
        .code(1);
    out.assert_contains("run not counted");
    out.assert_contains("\n  work/projects/some-api/stale-fact.md\n");
    out.assert_not_contains(" -> ");
    assert_contains(
        t.git(&["log", "-1", "--format=%B"]).ok().so(),
        "- work/projects/some-api/stale-fact.md",
    );
    assert_eq_str(t.git(&["status", "--porcelain"]).ok().so(), "");
    if ref_exists(&t, None) {
        fail("ref advanced");
    }
}

#[test]
fn dream_allowed_writes_in_existing_levels_still_succeed() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.install_acting_stub();
    let act = format!(
        "printf -- '{FACT_BODY}' > work/projects/some-api/new-fact.md; printf -- '{FACT_BODY}' > work/workspace/ws-fact.md"
    );
    let out = t.memory(&["dream"]).env("STUB_ACT", &act).run();
    assert_not_contains(out.combined(), "run not counted");
    assert_eq_str(
        t.git(&["rev-parse", "refs/memory-dream/last"]).ok().so(),
        head(&t),
    );
    assert_contains(cat(t.dp1.join("INDEX.md")), "new-fact");
}

#[test]
fn dream_refuses_to_run_with_symlinks_in_the_store() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let outside = t.tmp.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, t.dp1.join("escape")).unwrap();
    let h = head(&t);
    let out = t.memory(&["dream"]).run().code(1);
    out.assert_contains("store contains symlinks");
    out.assert_contains(&sp(&t.dp1.join("escape")));
    if exists(&t.stub_log.join("args")) {
        fail("opencode launched with a symlink in the store");
    }
    assert_eq_str(head(&t), h);
    if exists(&t.store.join(".git/memory-dream.lock")) {
        fail("lock left behind");
    }
    // A symlinked fact is caught too.
    fs::remove_file(t.dp1.join("escape")).unwrap();
    symlink(&outside, t.dp1.join("linked-fact.md")).unwrap();
    t.memory(&["dream"])
        .run()
        .code(1)
        .assert_contains("linked-fact.md");
}

#[test]
fn legit_init_and_link_group_leave_no_symlinks_in_the_store() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["link-group", "shared"]).ok();
    let second = t.tmp.join("code/second");
    fs::create_dir(&second).unwrap();
    t.memory(&["init", "work"]).cwd(&second).ok();
    t.memory(&["link-group", "shared"]).cwd(&second).ok();
    let mut found = Vec::new();
    find_symlinks(&t.store, &mut found);
    assert_eq_str(
        found.iter().map(|p| sp(p)).collect::<Vec<_>>().join("\n"),
        "",
    );
    let m = fs::symlink_metadata(t.p(".memory/project")).unwrap();
    if !m.file_type().is_symlink() {
        fail("the .memory links must stay in the project directory");
    }
}
