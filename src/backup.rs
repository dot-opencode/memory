//! `memory backup` and the structured commit message.

use crate::cmds::set_store_from_home;
use crate::ctx::*;
use crate::fsutil::*;
use crate::{die, outln};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

#[derive(Default)]
struct Ops {
    // op -> scope -> items
    items: BTreeMap<(usize, String), Vec<String>>,
    scopes: BTreeMap<String, ()>,
    index_scopes: BTreeMap<String, ()>,
    total: [usize; 4],
}

const OPS: [&str; 4] = ["add", "update", "rename", "remove"];

struct Class {
    scope: String,
    slug: String,
    is_index: bool,
}

fn classify(path: &str) -> Class {
    let arr: Vec<&str> = path.split('/').collect();
    let n = arr.len();
    let (scope, file) = if n == 4 && arr[1] == "projects" && arr[3].ends_with(".md") {
        (arr[2].to_string(), arr[3])
    } else if n == 4 && arr[1] == "groups" && arr[3].ends_with(".md") {
        (format!("group:{}", arr[2]), arr[3])
    } else if n == 3 && arr[1] == "workspace" && arr[2].ends_with(".md") {
        (format!("workspace:{}", arr[0]), arr[2])
    } else {
        return Class {
            scope: "other".into(),
            slug: path.to_string(),
            is_index: false,
        };
    };
    Class {
        scope,
        slug: file.strip_suffix(".md").unwrap_or(file).to_string(),
        is_index: file == "INDEX.md",
    }
}

impl Ops {
    fn count(&self, op: usize, scope: &str) -> usize {
        self.items
            .get(&(op, scope.to_string()))
            .map_or(0, |v| v.len())
    }
    fn join_first(&self, op: usize, scope: &str, k: usize) -> String {
        self.items
            .get(&(op, scope.to_string()))
            .map(|v| v[..k.min(v.len())].join(", "))
            .unwrap_or_default()
    }
    fn full_clause(&self, scope: &str) -> String {
        let mut parts = vec![];
        for (i, name) in OPS.iter().enumerate() {
            let c = self.count(i, scope);
            if c > 0 {
                parts.push(format!("{name} {}", self.join_first(i, scope, c)));
            }
        }
        parts.join("; ")
    }
    fn counts_clause(&self, scope: &str) -> String {
        let mut parts = vec![];
        for (i, name) in OPS.iter().enumerate() {
            let c = self.count(i, scope);
            if c > 0 {
                parts.push(format!("{name} {c}"));
            }
        }
        parts.join(", ")
    }
    fn total_items(&self, scope: &str) -> usize {
        (0..4).map(|i| self.count(i, scope)).sum()
    }
    fn truncated_clause(&self, scope: &str, k: usize) -> String {
        let mut remaining = k;
        let mut shown = [0usize; 4];
        for (i, s) in shown.iter_mut().enumerate() {
            *s = self.count(i, scope).min(remaining);
            remaining -= *s;
        }
        let omitted = self.total_items(scope) as i64 - k as i64;
        let mut parts = vec![];
        for (i, name) in OPS.iter().enumerate() {
            if shown[i] > 0 {
                parts.push(format!("{name} {}", self.join_first(i, scope, shown[i])));
            }
        }
        let mut s = parts.join("; ");
        if omitted > 0 {
            s = if s.is_empty() {
                format!("+{omitted} more")
            } else {
                format!("{s} +{omitted} more")
            };
        }
        s
    }
    fn single_scope_subject(&self, scope: &str) -> String {
        let prefix = format!("memory({scope}): ");
        let full = format!("{prefix}{}", self.full_clause(scope));
        if full.chars().count() <= 72 {
            return full;
        }
        let total = self.total_items(scope);
        let mut k = total as i64 - 1;
        while k >= 1 {
            let cand = format!("{prefix}{}", self.truncated_clause(scope, k as usize));
            if cand.chars().count() <= 72 {
                return cand;
            }
            k -= 1;
        }
        format!("{prefix}{}", self.counts_clause(scope))
    }
}

fn backup_commit_message() -> String {
    let mh = memory_home();
    let (_, out) = capture(
        Command::new("git")
            .arg("-C")
            .arg(&mh)
            .args(["diff", "--cached", "--name-status", "-M"]),
        true,
    );
    let ts = cap_trim(Command::new("date").arg("+%Y-%m-%dT%H:%M:%S")).unwrap_or_default();
    let mut o = Ops::default();
    for line in out.split('\n') {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let mut code = f[0]
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        if code == "R" && f.len() >= 3 {
            let n = classify(f[2]);
            let old = classify(f[1]);
            if n.is_index {
                o.index_scopes.insert(n.scope, ());
                continue;
            }
            o.scopes.insert(n.scope.clone(), ());
            o.items
                .entry((2, n.scope))
                .or_default()
                .push(format!("{}->{}", old.slug, n.slug));
            o.total[2] += 1;
            continue;
        }
        if code != "A" && code != "M" && code != "D" {
            code = "M".into();
        }
        let c = classify(f.get(1).copied().unwrap_or(""));
        if c.is_index {
            o.index_scopes.insert(c.scope, ());
            continue;
        }
        let op = match code.as_str() {
            "A" => 0,
            "D" => 3,
            _ => 1,
        };
        o.scopes.insert(c.scope.clone(), ());
        o.items.entry((op, c.scope)).or_default().push(c.slug);
        o.total[op] += 1;
    }
    let scopes: Vec<String> = o.scopes.keys().cloned().collect();
    if scopes.is_empty() {
        let idx: Vec<String> = o.index_scopes.keys().cloned().collect();
        let subject = if idx.len() == 1 {
            format!("memory({}): reindex", idx[0])
        } else {
            format!("memory: reindex {} scopes", idx.len())
        };
        return format!("{subject}\n\nbackup {ts}");
    }
    let subject = if scopes.len() == 1 {
        o.single_scope_subject(&scopes[0])
    } else {
        let mut parts = vec![];
        for (i, name) in OPS.iter().enumerate() {
            if o.total[i] > 0 {
                parts.push(format!("{name} {}", o.total[i]));
            }
        }
        format!("memory: {} in {} scopes", parts.join(", "), scopes.len())
    };
    let mut msg = format!("{subject}\n\n");
    for s in &scopes {
        msg.push_str(&format!("{s}: {}\n", o.full_clause(s)));
    }
    msg.push_str(&format!("backup {ts}"));
    msg
}

/// A persistent failure marker is published BEFORE attempting a commit.
/// 0 = committed, 2 = no changes, 3 = staging/commit failure (marker retained).
pub fn commit_failed(rc: &str, detail: &str) -> i32 {
    let gd = store_gitdir().unwrap_or_default();
    let marker = format!("{gd}/memory-dream.failed");
    let mut content = format!("{rc}\n");
    if !detail.is_empty() {
        content.push_str(&format!("{detail}\n"));
    }
    if fs::write(&marker, content).is_err() {
        return 3;
    }
    let m1 = format!("memory: dream FAILED (exit {rc}) — review");
    if store_retry(&["add", "-A"]) != 0 {
        return 3;
    }
    if run_quiet_err(&mut store_git(&["diff", "--cached", "--quiet"])) == 0 {
        let _ = fs::remove_file(&marker);
        return 2;
    }
    let mut args = vec!["commit", "-q", "-m", m1.as_str()];
    if !detail.is_empty() {
        args.push("-m");
        args.push(detail);
    }
    if store_retry(&args) != 0 {
        return 3;
    }
    let _ = fs::remove_file(&marker);
    0
}

pub fn run_quiet_err(c: &mut Command) -> i32 {
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    run(c)
}

pub fn backup(args: &[String]) -> i32 {
    let mut push = false;
    match args.len() {
        0 => {}
        1 => {
            if args[0] != "--push" {
                die!("usage: memory backup [--push]");
            }
            push = true;
        }
        _ => die!("usage: memory backup [--push]"),
    }
    let mh = memory_home();
    if !is_dir(&format!("{mh}/.git")) {
        die!("no store at {mh} (run 'memory init' first)");
    }
    set_store_from_home();
    let store = crate::ctx::store();
    let mut own_lock = false;
    if st(|s| s.lock_file.is_none()) {
        let mut attempts: u64 = 0;
        let w = std::env::var("AGENT_MEMORY_BACKUP_LOCK_WAIT").unwrap_or_default();
        let w = if w.is_empty() { "10".to_string() } else { w };
        if !w.bytes().all(|c| c.is_ascii_digit()) {
            die!("invalid AGENT_MEMORY_BACKUP_LOCK_WAIT");
        }
        let wait: u64 = w.parse().unwrap_or(u64::MAX);
        while !lock_acquire() {
            if attempts >= wait {
                eprintln!(
                    "memory: backup busy; request NOT saved, retry backup (exit 75). {}",
                    writer_lock_recovery_help()
                );
                return 75;
            }
            attempts += 1;
            sleep_secs(1.0);
            check_signal();
        }
        own_lock = true;
    }
    let gd = store_gitdir().unwrap_or_default();
    let marker = format!("{gd}/memory-dream.failed");
    if is_file(&marker) {
        let c = read_string(&marker).unwrap_or_default();
        let (first, rest) = match c.split_once('\n') {
            Some((a, b)) => (a.to_string(), b.trim_end_matches('\n').to_string()),
            None => (c.clone(), String::new()),
        };
        let status = commit_failed(&first, &rest);
        if status == 3 {
            die!("FAILED review commit could not be created; failure marker retained");
        }
        if status == 0 {
            outln!("backed up FAILED review; undo with: git -C '{store}' revert HEAD");
        }
    }
    if store_retry(&["add", "-A"]) != 0 {
        die!("git add failed in {store}");
    }
    if run_quiet_err(&mut store_git(&["diff", "--cached", "--quiet"])) == 0 {
        outln!("nothing to back up");
    } else {
        let msg = backup_commit_message();
        if store_retry(&["commit", "-q", "-m", &msg]) != 0 {
            die!("commit failed in {store} — is git user.name / user.email configured?");
        }
        outln!("backed up");
    }
    if push {
        if run_quiet_err(&mut store_git(&["remote", "get-url", "origin"])) != 0 {
            die!("no remote configured (git -C {store} remote add origin <url>)");
        }
        if run_quiet_err(&mut store_git(&["symbolic-ref", "-q", "HEAD"])) != 0 {
            die!("store is on a detached HEAD; check out a branch (git -C {store} switch <branch>) before 'backup --push'");
        }
        let mut c = store_git(&["push", "-q", "origin", "HEAD"]);
        let ssh = std::env::var("GIT_SSH_COMMAND").unwrap_or_default();
        c.env(
            "GIT_SSH_COMMAND",
            if ssh.is_empty() {
                "ssh -oBatchMode=yes".to_string()
            } else {
                ssh
            },
        );
        let rc = run(&mut c);
        if rc != 0 {
            exit_with(rc);
        }
    }
    if own_lock {
        release_lock();
    }
    0
}
