//! `memory dream`: sandboxed OpenCode consolidation run with watchdog and locking.

use crate::backup;
use crate::cmds::{publish_indexes, set_store_from_home};
use crate::ctx::*;
use crate::fsutil::*;
use crate::report::collect_levels;
use crate::{die, outln};
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const FOCUS_MAX: usize = 150;

fn json_path_safe(s: &str) -> bool {
    !s.bytes().any(|b| {
        matches!(b, b'"' | b'\\' | b'\n' | b'*' | b'?' | b'[' | b']') || b < 0x20 || b == 0x7f
    })
}

fn project_dirs(workspace: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for (level, rel) in collect_levels(workspace) {
        if !rel.contains("/projects/") {
            continue;
        }
        let mut code = String::new();
        if is_file(&format!("{level}/.project-path")) {
            code = read_trim(&format!("{level}/.project-path"));
        }
        if !json_path_safe(&code) {
            eprintln!("memory: skipping unsafe project path: {code}");
            code.clear();
        }
        if !code.starts_with('/') {
            code.clear();
        }
        if !code.is_empty() && is_dir(&code) && !code.contains(['"', '\\', '\n']) {
            let mut canon = canon_dir(&code).unwrap_or_default();
            if !json_path_safe(&canon) {
                eprintln!("memory: skipping unsafe canonical project path: {canon}");
                canon.clear();
            }
            if !canon.is_empty() && !canon.contains(['"', '\\']) {
                out.push(canon);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

const TOKENS: [&str; 5] = ["STORE", "MEMORY_BIN", "HOME", "PROJECT_DIR", "WEB_ONLY"];

fn render_line(line: &str, dir: &str, store: &str, bin: &str, home: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    'outer: loop {
        let b = rest.as_bytes();
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == b'_' && b[i + 1] == b'_' {
                for t in TOKENS {
                    let full = format!("__{t}__");
                    if rest[i..].starts_with(&full) {
                        let v = match t {
                            "STORE" => store,
                            "HOME" => home,
                            "MEMORY_BIN" => bin,
                            "PROJECT_DIR" => dir,
                            _ => "",
                        };
                        out.push_str(&rest[..i]);
                        out.push_str(v);
                        rest = &rest[i + full.len()..];
                        continue 'outer;
                    }
                }
            }
            i += 1;
        }
        break;
    }
    out.push_str(rest);
    out
}

fn render(tpl: &str, store: &str, bin: &str, home: &str, dirs: &[String], web: bool) -> String {
    let mut out = String::new();
    let body = format!("{tpl}\n");
    for line in body.strip_suffix('\n').unwrap_or(&body).split('\n') {
        if line.contains("__WEB_ONLY__") && !web {
            continue;
        }
        if line.contains("__PROJECT_DIR__") {
            for d in dirs.iter().filter(|d| !d.is_empty()) {
                out.push_str(&render_line(line, d, store, bin, home));
                out.push('\n');
            }
        } else {
            out.push_str(&render_line(line, "", store, bin, home));
            out.push('\n');
        }
    }
    out.trim_end_matches('\n').to_string()
}

fn is_md_fact(p: &str) -> bool {
    p.ends_with(".md") && p != "INDEX.md" && !p.ends_with("/INDEX.md")
}

fn changed_facts(rf: &str, workspace: &str) -> Vec<String> {
    let scope = if workspace.is_empty() {
        String::new()
    } else {
        format!("{workspace}/")
    };
    let mut all = String::new();
    let mut a = vec!["diff", "--name-only", "--diff-filter=d", rf, "--"];
    if !scope.is_empty() {
        a.push(&scope);
    }
    all.push_str(&capture(&mut git_plain(&a), true).1);
    let mut b = vec!["ls-files", "--others", "--exclude-standard", "--"];
    if !scope.is_empty() {
        b.push(&scope);
    }
    all.push_str(&capture(&mut git_plain(&b), true).1);
    let mut v: Vec<String> = all
        .split('\n')
        .filter(|l| is_md_fact(l))
        .map(String::from)
        .collect();
    v.sort();
    v.dedup();
    v
}

fn focus(rf: &str, workspace: &str) -> String {
    let changed = changed_facts(rf, workspace);
    let n = changed.len();
    if n > FOCUS_MAX {
        return String::new();
    }
    let mut s = String::from(
        "Incremental run: the previous run already consolidated the rest of the store.\n",
    );
    if n == 0 {
        s.push_str("No fact changed since then. Handle only the report's hygiene items, skip the rest of step 2 and all of step 3, and finish.\n");
        return s;
    }
    s.push_str(&format!(
        "Only these {n} facts were added or changed since then:\n"
    ));
    for c in &changed {
        s.push_str(&format!("- {c}\n"));
    }
    s.push_str("In step 2, handle hygiene items, and other items only when they involve one of these facts. In step 3, read only the INDEX.md files of the levels holding these facts, plus their groups and workspace, and compare these facts with the rest; do not re-check facts that are not listed.\n");
    s
}

fn slug_ok(s: &str) -> bool {
    valid_slug(s)
}

/// Paths the agent left behind that the sandbox matcher cannot forbid.
fn unexpected_paths() -> String {
    let levels_before = LEVELS_BEFORE.with(|l| l.borrow().clone());
    let (_, out) = capture(
        &mut store_git(&[
            "-c",
            "status.renames=false",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
        ]),
        false,
    );
    let mut res: Vec<String> = vec![];
    for line in out.split('\n') {
        if line.is_empty() {
            continue;
        }
        let code = line.get(..2).unwrap_or("");
        let path = line.get(3..).unwrap_or("");
        let mut ok = false;
        if matches!(code, "??" | " M" | "M " | "A " | "AM" | "MM") {
            let mut level = String::new();
            let parts: Vec<&str> = path.split('/').collect();
            let fact_ok = |f: &str| f.ends_with(".md") && !f.starts_with('.') && !f.is_empty();
            if parts.len() == 4
                && (parts[1] == "projects" || parts[1] == "groups")
                && slug_ok(parts[0])
                && slug_ok(parts[2])
                && fact_ok(parts[3])
            {
                level = format!("{}/{}/{}", parts[0], parts[1], parts[2]);
            } else if parts.len() == 3
                && parts[1] == "workspace"
                && slug_ok(parts[0])
                && fact_ok(parts[2])
            {
                level = format!("{}/workspace", parts[0]);
            }
            if !level.is_empty() && levels_before.contains(&level) {
                ok = true;
            }
        }
        if !ok {
            res.push(path.to_string());
        }
    }
    res.join("\n")
}

thread_local! {
    static LEVELS_BEFORE: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn rebuild_levels(workspace: &str) {
    let mut levels = vec![];
    let mut labels = vec![];
    for (l, rel) in collect_levels(workspace) {
        if is_dir(&format!("{l}/INDEX.md")) {
            eprintln!("memory: not reindexing {rel}: INDEX.md is a directory");
        } else {
            levels.push(l);
            labels.push(rel);
        }
    }
    if levels.is_empty() {
        return;
    }
    quiet(|| publish_indexes(&levels, &labels));
}

pub fn failure_finalize(rc: i32, why: &str) {
    st(|s| s.active = false);
    let store = crate::ctx::store();
    let mut detail = String::new();
    let unexpected: Vec<String> = unexpected_paths()
        .split('\n')
        .filter(|l| !l.is_empty())
        .take(50)
        .map(String::from)
        .collect();
    if !unexpected.is_empty() {
        detail = format!(
            "unexpected paths left by the agent (nothing was deleted; review them):\n{}",
            unexpected
                .iter()
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        eprintln!("memory: unexpected paths left by the agent (not deleted):");
        for p in &unexpected {
            eprintln!("  {p}");
        }
    }
    let gd = store_gitdir().unwrap_or_default();
    let _ = fs::write(format!("{gd}/memory-dream.failed"), format!("{rc}\n"));
    let ws = st(|s| s.workspace.clone());
    rebuild_levels(&ws);
    let status = backup::commit_failed(&rc.to_string(), &detail);
    match status {
        0 => eprintln!("memory: {why}; changes committed as FAILED review; undo with: git -C '{store}' revert HEAD"),
        2 => eprintln!("memory: {why}; the store has no changes to review"),
        _ => eprintln!("memory: {why}; FAILED review commit could not be created; changes remain uncommitted. Failure marker retained; fix git and run memory backup (then git -C '{store}' revert HEAD)."),
    }
}

/// Sets tools_ok/tools_denied. 0 sane, 1 sandbox broke the run, 2 cannot inspect.
fn sandbox_check(db: &str) -> i32 {
    st(|s| {
        s.tools_ok = "0".into();
        s.tools_denied = "0".into();
    });
    if !which("sqlite3") || !is_file(db) {
        return 2;
    }
    let sql = "
    WITH t AS (
      SELECT m.seq AS seq, j.key AS k,
             json_extract(j.value, '$.name') AS name,
             json_extract(j.value, '$.state.status') AS status,
             json_extract(j.value, '$.state.input.command') AS cmd,
             (coalesce(json_extract(j.value, '$.state.error.type'), '') = 'permission.rejected'
              OR coalesce(json_extract(j.value, '$.state.error.message'), '') LIKE 'Permission denied%') AS denied
      FROM session_message m, json_each(json_extract(m.data, '$.content')) j
      WHERE m.type = 'assistant' AND json_extract(j.value, '$.type') = 'tool')
    SELECT (SELECT count(*) FROM t WHERE status = 'completed'),
           (SELECT count(*) FROM t WHERE denied = 1),
           coalesce((SELECT status || ':' || denied FROM t
                     WHERE name = 'shell' AND cmd LIKE '%dream --report%'
                     ORDER BY seq, k LIMIT 1), '');
  ";
    let mut c = Command::new("sqlite3");
    c.args(["-readonly", "-separator", "|", db, sql]);
    let (rc, out) = capture(&mut c, false);
    if rc != 0 {
        return 2;
    }
    let out = out.trim_end_matches('\n').to_string();
    if out.is_empty() {
        return 2;
    }
    let head = |s: &str| s.split('|').next().unwrap_or("").to_string();
    let tail = |s: &str| match s.split_once('|') {
        Some((_, r)) => r.to_string(),
        None => s.to_string(),
    };
    let ok = head(&out);
    let rest = tail(&out);
    let denied = head(&rest);
    let report = tail(&rest);
    st(|s| {
        s.tools_ok = ok.clone();
        s.tools_denied = denied.clone();
    });
    let both = format!("{ok}{denied}");
    if both.is_empty() || !both.bytes().all(|b| b.is_ascii_digit()) {
        return 2;
    }
    if ok.parse::<u64>().unwrap_or(0) == 0 {
        return 1;
    }
    if report.ends_with(":1") {
        return 1;
    }
    0
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn find_symlinks(dir: &str, out: &mut Vec<String>) {
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut names: Vec<(String, fs::FileType)> = rd
        .flatten()
        .filter_map(|e| {
            e.file_type()
                .ok()
                .map(|t| (e.file_name().to_string_lossy().into_owned(), t))
        })
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    for (n, t) in names {
        if n == ".git" {
            continue;
        }
        let p = format!("{dir}/{n}");
        if t.is_symlink() {
            out.push(p);
        } else if t.is_dir() {
            find_symlinks(&p, out);
        }
    }
}

pub fn dream(args: &[String]) -> i32 {
    let usage = "usage: memory dream [--report [--stale-days N]] [--workspace <ws>] [--model <provider/model>] [--if-changed] [--full] [--web] [--timeout <minutes>] [--print-config]";
    let mut report = false;
    let mut stale_days: Option<i64> = None;
    let mut workspace = String::new();
    let mut model = std::env::var("AGENT_MEMORY_DREAM_MODEL").unwrap_or_default();
    let (mut print_config, mut model_flag, mut if_changed, mut full, mut web) =
        (false, false, false, false, false);
    let mut timeout_min = std::env::var("AGENT_MEMORY_DREAM_TIMEOUT").unwrap_or_default();
    if timeout_min.is_empty() {
        timeout_min = "120".into();
    }
    let mut timeout_flag = false;
    let n = args.len();
    let mut i = 0;
    while i < n {
        match args[i].as_str() {
            "--report" => {
                report = true;
                i += 1;
            }
            "--print-config" => {
                print_config = true;
                i += 1;
            }
            "--if-changed" => {
                if_changed = true;
                i += 1;
            }
            "--full" => {
                full = true;
                i += 1;
            }
            "--web" => {
                web = true;
                i += 1;
            }
            "--timeout" => {
                if n - i < 2 || !all_digits(&args[i + 1]) {
                    die!("{usage}");
                }
                timeout_min = args[i + 1].clone();
                timeout_flag = true;
                i += 2;
            }
            "--stale-days" => {
                if n - i < 2 || !all_digits(&args[i + 1]) {
                    die!("{usage}");
                }
                let v: i64 = args[i + 1].trim_start_matches('0').parse().unwrap_or(
                    if args[i + 1].bytes().all(|b| b == b'0') {
                        0
                    } else {
                        i64::MAX
                    },
                );
                if v <= 0 {
                    die!("{usage}");
                }
                stale_days = Some(v);
                i += 2;
            }
            "--workspace" => {
                if n - i < 2 {
                    die!("{usage}");
                }
                workspace = args[i + 1].clone();
                i += 2;
            }
            "--model" => {
                if n - i < 2 || args[i + 1].is_empty() {
                    die!("{usage}");
                }
                model = args[i + 1].clone();
                model_flag = true;
                i += 2;
            }
            _ => die!("{usage}"),
        }
    }
    if report {
        if model_flag || print_config || if_changed || full || web || timeout_flag {
            die!("{usage}");
        }
    } else if stale_days.is_some() {
        die!("{usage}");
    }
    let stale_days = stale_days.unwrap_or(180);
    if !all_digits(&timeout_min) {
        die!("invalid dream timeout '{timeout_min}': use a whole number of minutes");
    }
    let timeout_min_n: u64 = timeout_min.trim_start_matches('0').parse().unwrap_or(0);
    if timeout_min_n == 0 {
        die!("{usage}");
    }

    let mh = memory_home();
    if !is_dir(&mh) {
        die!("no store at {mh} (run 'memory init' first)");
    }
    set_store_from_home();
    let store = crate::ctx::store();
    if !workspace.is_empty() {
        require_slug("workspace name", &workspace);
        if !is_dir(&format!("{store}/{workspace}")) {
            die!("no such workspace in the store: {workspace}");
        }
    }
    if report {
        crate::report::report(stale_days, &workspace);
        return 0;
    }
    if !is_dir(&format!("{store}/.git")) {
        die!("store at {store} is not a git repository (run 'memory init' first)");
    }
    if !print_config && !which("opencode") {
        die!("opencode not found on PATH");
    }

    let self_path = crate::self_path();
    if !json_path_safe(&self_path) {
        die!("cannot run dream: CLI path has unsafe JSON/policy metacharacters");
    }
    let script = match canonical_path(&self_path) {
        Some(s) => s,
        None => die!("cannot resolve CLI path"),
    };
    let skill = match crate::skill_dir() {
        Some(s) => s,
        None => die!("cannot resolve CLI path"),
    };
    let assets = format!("{skill}/dream");
    if !is_file(&format!("{assets}/opencode.json")) || !is_file(&format!("{assets}/prompt.md")) {
        die!("dream assets not found in {assets}");
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for bad in [&mh, &store, &home, &script] {
        if !json_path_safe(bad) {
            die!("cannot run dream: path contains control characters or unsafe JSON/policy metacharacters (quote, backslash, newline, glob)");
        }
        if bad.contains(['"', '\\', '\n']) {
            die!("cannot run dream: path contains a quote, backslash or newline: {bad}");
        }
    }
    let dirs = project_dirs(&workspace);
    let config_t = read_trim(&format!("{assets}/opencode.json"));
    let prompt_t = read_trim(&format!("{assets}/prompt.md"));
    let config = render(&config_t, &store, &script, &home, &dirs, web);
    let mut prompt = render(&prompt_t, &store, &script, &home, &dirs, web);
    if !workspace.is_empty() {
        prompt.push_str(&format!("\n\nWorkspace scope: {workspace}"));
    }

    let mut last_ref = "refs/memory-dream/last".to_string();
    if !workspace.is_empty() {
        last_ref = format!("{last_ref}-{workspace}");
    }
    let last =
        cap_trim(&mut git_plain(&["rev-parse", "-q", "--verify", &last_ref])).unwrap_or_default();
    if !full && !last.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(focus(&last, &workspace).trim_end_matches('\n'));
    }

    if print_config {
        outln!("{config}\n---\n{prompt}");
        return 0;
    }

    let mut links = vec![];
    find_symlinks(&store, &mut links);
    links.truncate(20);
    if !links.is_empty() {
        let list: Vec<String> = links.iter().map(|l| format!("  {l}")).collect();
        die!("cannot run dream: the store contains symlinks, which the agent's path rules would follow; remove them first:\n{}", list.join("\n"));
    }

    if !lock_acquire() {
        die!(
            "another writer or memory dream is already running on this store (pid {}); manual recovery: {}",
            lock_holder(),
            writer_lock_recovery_help()
        );
    }
    let gitdir = match store_gitdir() {
        Some(g) => g,
        None => die!("cannot locate the git directory of {store}"),
    };

    let (_, pending) = capture(
        &mut git_plain(&["status", "--porcelain", "--untracked-files=all"]),
        true,
    );
    if !pending.trim_end_matches('\n').is_empty() {
        quiet(|| backup::backup(&[]));
        outln!("dream: committed pending store changes first");
    }
    let pre =
        cap_trim(&mut git_plain(&["rev-parse", "-q", "--verify", "HEAD"])).unwrap_or_default();

    if if_changed && !pre.is_empty() && !last.is_empty() {
        let scope = if workspace.is_empty() {
            String::new()
        } else {
            format!("{workspace}/")
        };
        let mut a = vec!["diff", "--name-only", last.as_str(), pre.as_str(), "--"];
        if !scope.is_empty() {
            a.push(&scope);
        }
        let (_, o) = capture(&mut git_plain(&a), true);
        if !o.split('\n').any(is_md_fact) {
            release_lock();
            outln!("dream: no fact changed since the last run — skipping");
            return 0;
        }
    }

    let db = format!("{gitdir}/memory-dream.db");
    let flag = format!("{gitdir}/memory-dream.timeout");
    for p in [
        db.clone(),
        format!("{db}-wal"),
        format!("{db}-shm"),
        flag.clone(),
    ] {
        let _ = fs::remove_file(p);
    }
    let secs: f64 = match std::env::var("AGENT_MEMORY_DREAM_TIMEOUT_SECS") {
        Ok(v) if !v.is_empty() => v.parse().unwrap_or(0.0),
        _ => (timeout_min_n * 60) as f64,
    };

    let before: Vec<String> = collect_levels("").into_iter().map(|(_, r)| r).collect();
    LEVELS_BEFORE.with(|l| *l.borrow_mut() = before);
    st(|s| {
        s.active = true;
        s.workspace = workspace.clone();
    });

    let mut cmd = Command::new("opencode");
    cmd.args(["run", "--standalone", "--agent", "memory-dream"]);
    if !model.is_empty() {
        cmd.args(["--model", &model]);
    }
    cmd.arg(&prompt)
        .current_dir(&store)
        .env("AGENT_MEMORY_HOME", &store)
        .env("OPENCODE_CONFIG_CONTENT", &config)
        .env("OPENCODE_DB", &db)
        .env("OPENCODE_CONFIG_PROJECT_DISABLE", "1")
        .env("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
        .process_group(0);

    let mut rc: i32;
    let mut timed_out = false;
    match cmd.spawn() {
        Err(e) => {
            rc = if e.kind() == std::io::ErrorKind::NotFound {
                127
            } else {
                126
            };
        }
        Ok(child) => {
            let pid = child.id() as i32;
            std::mem::forget(child);
            st(|s| s.child_pid = pid);
            let cancel = Arc::new(AtomicBool::new(false));
            let c2 = cancel.clone();
            let flag2 = flag.clone();
            let grace = kill_grace();
            let h = std::thread::spawn(move || {
                let sleep_cancellable = |total: f64| -> bool {
                    let mut left = total;
                    while left > 0.0 {
                        if c2.load(Ordering::SeqCst) {
                            return false;
                        }
                        let step = left.min(0.05);
                        sleep_secs(step);
                        left -= step;
                    }
                    !c2.load(Ordering::SeqCst)
                };
                if !sleep_cancellable(secs) {
                    return;
                }
                let _ = fs::write(&flag2, b"");
                kill_group(pid, libc::SIGTERM);
                if !sleep_cancellable(grace) {
                    return;
                }
                kill_group(pid, libc::SIGKILL);
            });
            st(|s| s.watchdog = Some((cancel, h)));
            loop {
                let mut status = 0;
                let r = unsafe { libc::waitpid(pid, &mut status, 0) };
                if r == -1 {
                    if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                        check_signal();
                        continue;
                    }
                    rc = 127;
                    break;
                }
                rc = if libc::WIFEXITED(status) {
                    libc::WEXITSTATUS(status)
                } else {
                    128 + libc::WTERMSIG(status)
                };
                break;
            }
            check_signal();
            if exists(&flag) {
                timed_out = true;
                if let Some((_, h)) = st(|s| s.watchdog.take()) {
                    let _ = h.join();
                }
                let _ = fs::remove_file(&flag);
            } else {
                stop_watchdog();
                kill_group(pid, libc::SIGTERM);
                if group_alive(pid) {
                    sleep_secs(kill_grace());
                    kill_group(pid, libc::SIGKILL);
                }
            }
            st(|s| s.child_pid = 0);
        }
    }

    if rc != 0 || timed_out {
        if rc == 0 {
            rc = 1;
        }
        let mut why = format!("opencode run failed (exit {rc})");
        if timed_out {
            why = format!("opencode run timed out after {timeout_min_n} minute(s) (exit {rc})");
        }
        failure_finalize(rc, &why);
        release_lock();
        return 1;
    }

    if !unexpected_paths().is_empty() {
        failure_finalize(
            1,
            "the agent wrote outside the allowed fact paths; run not counted",
        );
        release_lock();
        return 1;
    }

    let sandbox_status = sandbox_check(&db);
    let (ok, denied) = st(|s| (s.tools_ok.clone(), s.tools_denied.clone()));
    match sandbox_status {
        1 => {
            failure_finalize(1, &format!("sandbox denied the agent's tools; run not counted ({ok} tool call(s) succeeded, {denied} denied)"));
            release_lock();
            return 1;
        }
        2 => eprintln!("memory: dream: cannot inspect the session database (sqlite3 or db missing); sandbox not verified"),
        _ => {}
    }

    rebuild_levels(&workspace);
    quiet(|| backup::backup(&[]));
    let final_sha =
        if backup::run_quiet_err(&mut store_git(&["rev-parse", "-q", "--verify", "HEAD"])) == 0 {
            cap_trim(&mut store_git(&["rev-parse", "--verify", "HEAD"])).unwrap_or_default()
        } else {
            let (_, o) = capture(
                &mut store_git(&["status", "--porcelain", "--untracked-files=all"]),
                true,
            );
            if !o.trim_end_matches('\n').is_empty() {
                die!("cannot resolve final dream commit");
            }
            String::new()
        };
    if !final_sha.is_empty() {
        let rc = run(&mut store_git(&["update-ref", &last_ref, &final_sha]));
        if rc != 0 {
            exit_with(rc);
        }
    }
    st(|s| s.active = false);
    release_lock();

    let base = if pre.is_empty() {
        let mut c = Command::new("git");
        c.arg("-C")
            .arg(&store)
            .args(["hash-object", "-t", "tree", "/dev/null"]);
        cap_trim(&mut c).unwrap_or_default()
    } else {
        pre.clone()
    };
    let has_head =
        backup::run_quiet_err(&mut git_plain(&["rev-parse", "-q", "--verify", "HEAD"])) == 0;
    let stat = if has_head {
        capture(&mut git_plain(&["diff", "--stat", &base, "HEAD"]), true).1
    } else {
        String::new()
    };
    if has_head && !stat.trim_end_matches('\n').is_empty() {
        let since = if pre.is_empty() {
            "the start".to_string()
        } else {
            cap_trim(&mut git_plain(&["rev-parse", "--short", &pre])).unwrap_or_default()
        };
        outln!("dream: changes since {since}:");
        let mut c = git_plain(&["diff", "--stat", &base, "HEAD"]);
        c.stdin(Stdio::null());
        run(&mut c);
    } else {
        outln!("dream: no changes");
    }
    if denied.parse::<i64>().unwrap_or(0) != 0 {
        outln!("dream: {denied} tool call(s) were denied by the sandbox ({ok} succeeded)");
    }
    outln!("dream: session transcript in {db} (latest run only)");
    0
}
