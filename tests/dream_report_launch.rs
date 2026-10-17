//! `memory dream --report` and the dream launcher (fake opencode).
//! Ported from the former Bash suite tests/run.sh.

mod common;
use common::*;

use std::fs;
use std::path::Path;
use std::process::Command;

const SAME: &str = "dream: memory is tidy — nothing to consolidate";

/// stdout of `memory <args>` run in the default cwd, asserting success.
fn run_ok(t: &T, args: &[&str]) -> String {
    t.memory(args).ok().stdout
}

/// `$(cd -P "$PROJ" && pwd -P)`
fn proj_real(t: &T) -> String {
    fs::canonicalize(&t.proj).unwrap().display().to_string()
}

fn store_real(t: &T) -> String {
    fs::canonicalize(&t.store).unwrap().display().to_string()
}

fn report(t: &T) -> String {
    run_ok(t, &["dream", "--report"])
}

/// Local helper: `find DIR -type f -exec cksum {} + | sort` equivalent —
/// sorted (path, content) pairs of every regular file below `dir`.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(d: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                walk(&p, out);
            } else if ft.is_file() {
                // git's detached `maintenance run --auto` (spawned by an earlier
                // commit) creates/removes objects/maintenance.lock in the
                // background; it is not a write by the command under test.
                if p.file_name().is_some_and(|n| n == "maintenance.lock") {
                    continue;
                }
                out.push((p.display().to_string(), fs::read(&p).unwrap()));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, &mut v);
    v.sort();
    v
}

fn assert_snapshot_eq(before: &[(String, Vec<u8>)], after: &[(String, Vec<u8>)], what: &str) {
    if before == after {
        return;
    }
    let diff: Vec<&str> = before
        .iter()
        .chain(after.iter())
        .filter(|e| !before.contains(e) || !after.contains(e))
        .map(|(p, _)| p.as_str())
        .collect();
    fail(&format!("{what}: changed paths {diff:?}"));
}

fn gitout(t: &T, args: &[&str]) -> String {
    t.git(args).ok().so()
}

fn read(p: impl AsRef<Path>) -> String {
    cat(p)
}

// ------------------------------------------------------------------ report

#[test]
fn skill_memory_dream_frontmatter() {
    let t = T::new();
    let f = t.root.join("skills/memory-dream/SKILL.md");
    assert_file(&f);
    let text = fs::read_to_string(&f).unwrap();
    assert_eq_str(text.lines().next().unwrap(), "---");
    if !text.lines().any(|l| l == "name: memory-dream") {
        fail("memory-dream skill has no name: memory-dream");
    }
    if !text
        .lines()
        .any(|l| l.starts_with("description: ") && l.len() > 13)
    {
        fail("memory-dream skill has no description");
    }
    if !text.lines().any(|l| l == "license: MIT") {
        fail("memory-dream skill has no license");
    }
}

#[test]
fn dream_report_tidy_store() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "pool-limit",
        "gotcha",
        "pgbouncer pool breaks above fifty connections",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dw,
        "editor-setup",
        "convention",
        "vim keybindings everywhere",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    assert_eq_str(report(&t).trim_end_matches('\n'), SAME);
}

#[test]
fn dream_report_runs_without_memory_dir() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "pool-limit",
        "gotcha",
        "pgbouncer pool breaks above fifty connections",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    let elsewhere = t.tmp.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    assert_absent(elsewhere.join(".memory"));
    let out = t.memory(&["dream", "--report"]).cwd(&elsewhere).ok().stdout;
    assert_contains(&out, "dream:");
}

#[test]
fn dream_report_similar_within_a_level() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "pgbouncer-pool-limit",
        "gotcha",
        "pgbouncer connection pool breaks above 50 connections",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "pgbouncer-pool-size",
        "gotcha",
        "pgbouncer connection pool size limit fifty",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "deploy-checklist",
        "convention",
        "release steps for staging",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    let out = report(&t);
    let pr = proj_real(&t);
    assert_contains(&out, "similar facts (merge, or check for contradiction):");
    assert_contains(
        &out,
        format!("  work/projects/some-api/pgbouncer-pool-limit.md [code: {pr}] <-> work/projects/some-api/pgbouncer-pool-size.md [code: {pr}] (shared: connection, limit, pgbouncer, pool)"),
    );
    assert_contains(
        &out,
        "dream: 1 similar, 0 repeated across projects, 0 shadowed, 0 stale, 0 unusable",
    );
    assert_not_contains(&out, "deploy-checklist");
}

#[test]
fn dream_report_unrelated_facts_are_not_paired() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "pool-limit",
        "gotcha",
        "pgbouncer pool breaks above fifty connections",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "release-steps",
        "convention",
        "staging deploy needs manual approval",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dw,
        "editor-setup",
        "convention",
        "vim keybindings everywhere",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    let out = report(&t);
    assert_not_contains(&out, "similar facts");
    assert_not_contains(&out, "<->");
}

#[test]
fn dream_report_survives_many_facts() {
    let mut t = T::new();
    t.dream_fixture();
    for i in 1..=320 {
        t.write(
            t.dw.join(format!("n{i}.md")),
            &format!(
                "---\ndescription: note q{i}x\ntype: context\ncreated: 2026-09-01\n---\nsee [[n{}]]\n",
                i + 1
            ),
        );
    }
    let o = t.memory(&["dream", "--report"]).run();
    assert_eq!(o.code, 0, "{}", o.combined());
    assert_contains(
        o.combined(),
        "dangling link: work/workspace/n320.md -> [[n321]]",
    );
}

#[test]
fn dream_report_repeated_across_projects() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "retry-policy",
        "convention",
        "clients retry three times with backoff",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp2,
        "retry-policy",
        "convention",
        "always exponential delays for HTTP calls",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    let out = report(&t);
    assert_contains(
        &out,
        "repeated across projects (move shared part to a group or workspace):",
    );
    assert_contains(
        &out,
        "  work: retry-policy in projects/other-api, projects/some-api (same slug)",
    );
    assert_not_contains(&out, "same slug in several levels");
    assert_not_contains(&out, "similar facts");
    assert_contains(
        &out,
        "dream: 0 similar, 1 repeated across projects, 0 shadowed, 0 stale, 0 unusable",
    );
}

#[test]
fn dream_report_shadowing() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "editor-setup",
        "convention",
        "vim keybindings",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dw,
        "editor-setup",
        "convention",
        "docker compose ports notes",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    let out = report(&t);
    assert_contains(&out, "same slug in several levels (shadowing):");
    assert_contains(&out, "  work: editor-setup in projects/some-api, workspace");
    assert_contains(&out, "1 shadowed");
}

#[test]
fn dream_report_never_compares_across_workspaces() {
    let mut t = T::new();
    t.dream_fixture();
    let hp = t.tmp.join("code/home-proj");
    fs::create_dir_all(&hp).unwrap();
    t.memory(&["init", "home"]).cwd(&hp).ok();
    dfact(
        &t.dp1,
        "pool-limit",
        "gotcha",
        "pgbouncer pool breaks above fifty connections",
        "2026-09-04",
        None,
    );
    dfact(
        &t.store.join("home/workspace"),
        "pool-limit",
        "gotcha",
        "pgbouncer pool breaks above fifty connections",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    t.memory(&["index"]).cwd(&hp).ok();
    let out = report(&t);
    assert_not_contains(&out, "<->");
    assert_not_contains(&out, "pool-limit in");
    assert_contains(&out, "dream: memory is tidy");
}

#[test]
fn dream_report_workspace_filter() {
    let mut t = T::new();
    t.dream_fixture();
    let hp = t.tmp.join("code/home-proj");
    fs::create_dir_all(&hp).unwrap();
    t.memory(&["init", "home"]).cwd(&hp).ok();
    dfact(
        &t.dp1,
        "pgbouncer-pool-limit",
        "gotcha",
        "pgbouncer connection pool breaks above 50 connections",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "pgbouncer-pool-size",
        "gotcha",
        "pgbouncer connection pool size limit fifty",
        "2026-09-04",
        None,
    );
    dfact(
        &t.store.join("home/workspace"),
        "old-note",
        "context",
        "very old note about laptop",
        "2025-01-01",
        None,
    );
    t.dream_reindex();
    t.memory(&["index"]).cwd(&hp).ok();
    let out = run_ok(&t, &["dream", "--report", "--workspace", "home"]);
    assert_contains(&out, "home/workspace/old-note.md");
    assert_not_contains(&out, "pgbouncer");
    let out = run_ok(&t, &["dream", "--report", "--workspace", "work"]);
    assert_contains(&out, "pgbouncer-pool-limit");
    assert_not_contains(&out, "old-note");
    let o = t
        .memory(&["dream", "--report", "--workspace", "nope"])
        .code(1);
    assert_contains(o.combined(), "no such workspace");
    let o = t
        .memory(&["dream", "--report", "--workspace", "Bad_Name"])
        .code(1);
    assert_contains(o.combined(), "invalid workspace name");
    let o = t.memory(&["dream", "--report", "--workspace"]).code(1);
    assert_contains(o.combined(), "usage: memory dream");
}

#[test]
fn dream_report_stale_facts() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "old-decision",
        "decision",
        "chose the queue library long ago",
        "2026-01-01",
        None,
    );
    dfact(
        &t.dp1,
        "fresh-decision",
        "decision",
        "picked the metrics backend",
        "2026-01-01",
        Some("2026-09-01"),
    );
    t.dream_reindex();
    let pr = proj_real(&t);
    let out = report(&t);
    assert_contains(
        &out,
        "stale facts (older than 180 days — verify, update 'updated:', or delete):",
    );
    assert_contains(
        &out,
        format!("  work/projects/some-api/old-decision.md [code: {pr}] — decision, 272d since 2026-01-01"),
    );
    assert_not_contains(&out, "fresh-decision");
    assert_contains(&out, "1 stale");
    let out = run_ok(&t, &["dream", "--report", "--stale-days", "300"]);
    assert_eq_str(out.trim_end_matches('\n'), SAME);
    let out = run_ok(&t, &["dream", "--report", "--stale-days", "10"]);
    assert_contains(&out, "older than 10 days");
    assert_contains(
        &out,
        format!("fresh-decision.md [code: {pr}] — decision, 29d since 2026-09-01"),
    );
}

#[test]
fn dream_report_rejects_bad_arguments() {
    let mut t = T::new();
    t.dream_fixture();
    for args in [
        "--report --stale-days 0",
        "--report --stale-days abc",
        "--report --stale-days -5",
        "--report --stale-days",
        "--report --stale-days 1.5",
        "--report --bogus",
        "--report --model x/y",
        "--report --if-changed",
        "--stale-days 5",
        "--report extra",
    ] {
        let mut full = vec!["dream"];
        full.extend(args.split_whitespace());
        let o = t.memory(&full).code(1);
        assert_contains(o.combined(), "usage: memory dream");
    }
    let o = t
        .memory(&["dream", "--report"])
        .env("AGENT_MEMORY_TODAY", "2026-02-30")
        .code(1);
    assert_contains(o.combined(), "invalid AGENT_MEMORY_TODAY");
}

#[test]
fn dream_report_lists_unusable_facts_without_failing() {
    let mut t = T::new();
    t.dream_fixture();
    t.write(t.dp1.join("broken.md"), "no frontmatter here\n");
    dfact(
        &t.dp1,
        "good-fact",
        "convention",
        "vim keybindings everywhere",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    let out = report(&t);
    assert_contains(&out, "unusable facts (fix frontmatter):");
    assert_contains(&out, "work/projects/some-api/broken.md");
    assert_not_contains(&out, "good-fact");
    assert_contains(&out, "1 unusable");
}

#[test]
fn dream_report_orphan_project() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp2,
        "some-fact",
        "convention",
        "vim keybindings everywhere",
        "2026-09-04",
        None,
    );
    t.memory(&["index"]).cwd(t.other_proj()).ok();
    fs::remove_dir_all(t.other_proj()).unwrap();
    let out = report(&t);
    assert_contains(&out, "hygiene:");
    assert_contains(
        &out,
        "  orphan project: work/projects/other-api (recorded dir is gone)",
    );
    let _ = fs::remove_file(t.dp1.join(".project-path"));
    let out = report(&t);
    assert_contains(
        &out,
        "  orphan project: work/projects/some-api (.project-path missing)",
    );
}

#[test]
fn dream_report_hygiene_long_description_and_dangling_link() {
    let mut t = T::new();
    t.dream_fixture();
    let long = "word ".repeat(40);
    dfact(&t.dp1, "long-one", "gotcha", &long, "2026-09-04", None);
    dfact(
        &t.dp1,
        "linker",
        "convention",
        "vim keybindings everywhere",
        "2026-09-04",
        None,
    );
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(t.dp1.join("linker.md"))
            .unwrap();
        f.write_all(b"see [[gone-fact]] and [[linker]]\n").unwrap();
    }
    t.dream_reindex();
    let out = report(&t);
    assert_contains(&out, "long description (");
    assert_contains(&out, "dangling link: work/projects/some-api/linker.md");
    assert_contains(&out, "-> [[gone-fact]]");
    assert_not_contains(&out, "[[linker]]");
}

#[test]
fn dream_report_index_drift_and_large_index() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "pool-limit",
        "gotcha",
        "pgbouncer pool breaks above fifty connections",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    dfact(
        &t.dp1,
        "unindexed-fact",
        "convention",
        "vim keybindings everywhere",
        "2026-09-04",
        None,
    );
    fs::remove_file(t.dw.join("INDEX.md")).unwrap();
    fill_index(&t.dp2.join("INDEX.md"), 41);
    let out = report(&t);
    assert_contains(&out, "index drift (run 'memory dream' or 'memory index'):");
    assert_contains(
        &out,
        "  work/projects/some-api/INDEX.md (differs from the facts)",
    );
    assert_contains(&out, "  work/workspace/INDEX.md (missing)");
    assert_contains(&out, "large indexes (> 40 entries — merge or prune):");
    assert_contains(&out, "  work/projects/other-api/INDEX.md (41 entries)");
}

#[test]
fn dream_report_writes_nothing() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "pgbouncer-pool-limit",
        "gotcha",
        "pgbouncer connection pool breaks above 50 connections",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "pgbouncer-pool-size",
        "gotcha",
        "pgbouncer connection pool size limit fifty",
        "2026-01-01",
        None,
    );
    dfact(
        &t.dp2,
        "retry-policy",
        "convention",
        "clients retry three times with backoff",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "retry-policy",
        "convention",
        "always exponential delays for HTTP calls",
        "2026-09-04",
        None,
    );
    t.write(t.dp1.join("broken.md"), "broken\n");
    t.dream_reindex();
    dfact(
        &t.dp1,
        "unindexed",
        "convention",
        "vim keybindings everywhere",
        "2026-09-04",
        None,
    );
    let before = snapshot(&t.store);
    let out = report(&t);
    assert_contains(&out, "dream:");
    let after = snapshot(&t.store);
    assert!(after == before, "dream --report modified the store");
}

#[test]
fn dream_report_output_is_deterministic() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "pgbouncer-pool-limit",
        "gotcha",
        "pgbouncer connection pool breaks above 50 connections",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "pgbouncer-pool-size",
        "gotcha",
        "pgbouncer connection pool size limit fifty",
        "2026-01-01",
        None,
    );
    dfact(
        &t.dp2,
        "retry-policy",
        "convention",
        "clients retry three times with backoff",
        "2026-01-02",
        None,
    );
    dfact(
        &t.dp1,
        "retry-policy",
        "convention",
        "always exponential delays for HTTP calls",
        "2026-01-03",
        None,
    );
    t.dream_reindex();
    let a = report(&t);
    let b = t
        .memory(&["dream", "--report"])
        .env("LC_ALL", "en_US.UTF-8")
        .ok()
        .stdout;
    assert_eq_str(b.trim_end_matches('\n'), a.trim_end_matches('\n'));
}

#[test]
fn dream_report_handles_non_ascii_descriptions() {
    let mut t = T::new();
    t.dream_fixture();
    dfact(
        &t.dp1,
        "uvicorn-workers-first",
        "gotcha",
        "воркеры uvicorn падают при перезапуске",
        "2026-09-04",
        None,
    );
    dfact(
        &t.dp1,
        "uvicorn-workers-second",
        "gotcha",
        "воркеры uvicorn падают при перезапуске сервера",
        "2026-09-04",
        None,
    );
    t.dream_reindex();
    let out = report(&t);
    assert_contains(
        &out,
        format!(
            "uvicorn-workers-first.md [code: {}] <-> work/projects/some-api/uvicorn-workers-second.md",
            proj_real(&t)
        ),
    );
    assert_contains(&out, "воркеры");
}

// ---------------------------------------------------------------- launcher

#[test]
fn dream_print_config_renders_without_launching() {
    let mut t = T::new();
    t.dream_fixture();
    let store = store_real(&t);
    // No opencode on PATH: --print-config must not need it.
    let out = t
        .memory(&["dream", "--print-config"])
        .env("PATH", "/usr/bin:/bin")
        .ok()
        .stdout;
    assert_not_contains(&out, "__");
    assert_contains(&out, &store);
    assert_contains(&out, "\"policies\"");
    assert_contains(&out, "memory-dream-check");
    assert_contains(&out, format!("{}/.ssh/*", t.home.display()));
    assert_contains(&out, "edit:*/.git/*");
    assert_contains(
        &out,
        format!(
            "{}/skills/agent-memory/bin/memory dream --report",
            t.root.display()
        ),
    );
    assert_contains(&out, "---");
    assert_not_contains(&out, "Workspace scope:");
    let out = run_ok(&t, &["dream", "--print-config", "--workspace", "work"]);
    assert_contains(&out, "Workspace scope: work");
}

#[test]
fn dream_print_config_is_valid_json() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("  (skipped: node not found)");
        return;
    }
    let mut t = T::new();
    t.dream_fixture();
    let out = run_ok(&t, &["dream", "--print-config"]);
    let mut json = String::new();
    for l in out.lines() {
        if l == "---" {
            break;
        }
        json.push_str(l);
        json.push('\n');
    }
    if json.is_empty() {
        fail("no config part before ---");
    }
    let script = r#"
    const c = JSON.parse(require("fs").readFileSync(0, "utf8"));
    if (c.default_agent !== "memory-dream") throw new Error("default_agent");
    if (!Array.isArray(c.policies) || c.policies[0].effect !== "deny") throw new Error("policies");
    if (c.agents["memory-dream"].mode !== "primary") throw new Error("primary");
    if (c.agents["memory-dream-check"].mode !== "subagent") throw new Error("subagent");
    const check = c.agents["memory-dream-check"].permissions;
    if (check.some((p) => p.action === "edit" && p.effect === "allow")) throw new Error("check agent can edit");
    if (check.some((p) => p.action === "subagent" && p.effect === "allow")) throw new Error("check agent can spawn");
    if (JSON.stringify(c).includes("\"ask\"")) throw new Error("ask effect used");
  "#;
    let o = t
        .cmd("node", &["-e", script])
        .stdin(json.into_bytes())
        .run();
    if o.code != 0 {
        fail(&format!(
            "print-config JSON invalid or wrong shape: {}",
            o.combined()
        ));
    }
}

#[test]
fn dream_rejects_json_unsafe_home() {
    let mut t = T::new();
    t.dream_fixture();
    let bad = t.tmp.join("ho\"me");
    fs::create_dir_all(&bad).unwrap();
    let o = t
        .memory(&["dream", "--print-config"])
        .env("HOME", &bad)
        .code(1);
    assert_contains(o.combined(), "cannot run dream");
}

#[test]
fn dream_requires_opencode_on_path() {
    let mut t = T::new();
    t.dream_fixture();
    if ["/usr/bin", "/bin"]
        .iter()
        .any(|d| Path::new(d).join("opencode").exists())
    {
        eprintln!("  (skipped: opencode installed in a system directory)");
        return;
    }
    let o = t.memory(&["dream"]).env("PATH", "/usr/bin:/bin").code(1);
    assert_contains(o.combined(), "opencode not found on PATH");
}

#[test]
fn dream_requires_git_store() {
    let t = T::new();
    let o = t.memory(&["dream", "--print-config"]).code(1);
    assert_contains(o.combined(), "no store");
    fs::create_dir_all(t.store.join("work")).unwrap();
    let o = t.memory(&["dream", "--print-config"]).code(1);
    assert_contains(o.combined(), "not a git repository");
}

#[test]
fn dream_print_config_writes_nothing() {
    let mut t = T::new();
    t.dream_launch_fixture();
    dfact(
        &t.dp1,
        "pending",
        "convention",
        "uncommitted fact",
        "2026-09-04",
        None,
    );
    let before = snapshot(&t.store);
    t.memory(&["dream", "--print-config"]).ok();
    let after = snapshot(&t.store);
    assert_snapshot_eq(&before, &after, "print-config modified the store");
    assert_absent(t.stub_log.join("args"));
    if gitout(&t, &["status", "--porcelain"]).is_empty() {
        fail("print-config committed the pending change");
    }
}

#[test]
fn dream_run_commits_pending_changes_first() {
    let mut t = T::new();
    t.dream_launch_fixture();
    dfact(
        &t.dp1,
        "pending",
        "convention",
        "uncommitted fact",
        "2026-09-04",
        None,
    );
    let base = t.commit_count();
    t.memory(&["dream"]).ok();
    assert_eq_str(cat(t.stub_log.join("status-at-run")), "");
    assert_eq_str(
        cat(t.stub_log.join("commits-at-run")),
        (base + 1).to_string(),
    );
}

#[test]
fn dream_run_launches_from_store_with_agent_and_config() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.memory(&["dream"]).ok();
    let store = store_real(&t);
    let log = &t.stub_log;
    assert_eq_str(cat(log.join("pwd")), &store);
    let args = fs::read_to_string(log.join("args")).unwrap();
    let head4: Vec<&str> = args.lines().take(4).collect();
    assert_eq_str(
        format!("{} ", head4.join(" ")),
        "run --standalone --agent memory-dream ",
    );
    assert_contains(
        cat(log.join("args")),
        format!("You run unattended over the memory store at {store}"),
    );
    assert_contains(
        cat(log.join("config")),
        "\"default_agent\": \"memory-dream\"",
    );
    assert_not_contains(cat(log.join("config")), "__STORE__");
    assert_not_contains(cat(log.join("args")), "--model");
    assert_eq_str(cat(log.join("db")), format!("{store}/.git/memory-dream.db"));
}

#[test]
fn dream_run_starts_with_a_fresh_untracked_db() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let db = t.store.join(".git/memory-dream.db");
    fs::write(&db, "stale\n").unwrap();
    fs::write(t.store.join(".git/memory-dream.db-wal"), "stale\n").unwrap();
    let out = t.memory(&["dream"]).ok().stdout;
    assert_eq_str(cat(t.stub_log.join("db-existed")), "no");
    assert_contains(&out, "dream: session transcript in ");
    assert_not_contains(cat(t.stub_log.join("status-at-run")), "memory-dream.db");
    assert_not_contains(
        gitout(&t, &["status", "--porcelain", "--untracked-files=all"]),
        "memory-dream.db",
    );
}

#[test]
fn dream_report_and_print_config_keep_the_db() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let db = t.store.join(".git/memory-dream.db");
    fs::write(&db, "keep\n").unwrap();
    t.memory(&["dream", "--report"]).ok();
    assert_file(&db);
    t.memory(&["dream", "--print-config"]).ok();
    assert_file(&db);
    assert_eq_str(cat(&db), "keep");
}

#[test]
fn dream_run_passes_model_and_workspace() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.memory(&["dream", "--model", "example/model-1", "--workspace", "work"])
        .ok();
    let args = cat(t.stub_log.join("args"));
    assert_contains(&args, "--model\nexample/model-1");
    assert_contains(&args, "Workspace scope: work");
}

#[test]
fn dream_run_model_from_env_and_flag_override() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.memory(&["dream"])
        .env("AGENT_MEMORY_DREAM_MODEL", "example/from-env")
        .ok();
    assert_contains(cat(t.stub_log.join("args")), "--model\nexample/from-env");
    t.memory(&["dream", "--model", "example/flag"])
        .env("AGENT_MEMORY_DREAM_MODEL", "example/from-env")
        .ok();
    let args = cat(t.stub_log.join("args"));
    assert_contains(&args, "--model\nexample/flag");
    assert_not_contains(&args, "from-env");
    // The env default must not make --report reject its arguments.
    t.memory(&["dream", "--report"])
        .env("AGENT_MEMORY_DREAM_MODEL", "example/from-env")
        .ok();
}

#[test]
fn dream_run_if_changed_skips_when_store_is_unchanged() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let args_file = t.stub_log.join("args");
    // First run: no record of a previous run, so it runs and records HEAD.
    let out = run_ok(&t, &["dream", "--if-changed"]);
    assert_not_contains(&out, "skipping");
    assert_eq_str(
        gitout(&t, &["rev-parse", "refs/memory-dream/last"]),
        gitout(&t, &["rev-parse", "HEAD"]),
    );
    let _ = fs::remove_file(&args_file);
    let out = run_ok(&t, &["dream", "--if-changed"]);
    assert_contains(&out, "dream: no fact changed since the last run — skipping");
    assert_absent(&args_file);
    // Commits that touch no fact (an index rebuild, a new project) do not count.
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(t.dp1.join("INDEX.md"))
            .unwrap();
        f.write_all(b"- touched\n").unwrap();
    }
    t.memory(&["backup"]).ok();
    t.memory(&["index"]).ok();
    t.memory(&["backup"]).ok();
    let out = run_ok(&t, &["dream", "--if-changed"]);
    assert_contains(&out, "skipping");
    assert_absent(&args_file);
    // A new fact (even uncommitted) makes the next run happen.
    dfact(
        &t.dp1,
        "new-fact",
        "convention",
        "a brand new convention",
        "2026-09-30",
        None,
    );
    let out = run_ok(&t, &["dream", "--if-changed"]);
    assert_not_contains(&out, "skipping");
    assert_file(&args_file);
    // Without the flag it always runs.
    let _ = fs::remove_file(&args_file);
    t.memory(&["dream"]).ok();
    assert_file(&args_file);
    // A failed run does not move the marker.
    let before = gitout(&t, &["rev-parse", "refs/memory-dream/last"]);
    dfact(
        &t.dp1,
        "another-fact",
        "convention",
        "yet another convention",
        "2026-09-30",
        None,
    );
    let _ = t
        .memory(&["dream", "--if-changed"])
        .env("STUB_EXIT", "1")
        .run();
    assert_eq_str(gitout(&t, &["rev-parse", "refs/memory-dream/last"]), before);
}

#[test]
fn dream_run_holds_lock_so_other_backups_wait() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.install_backup_calling_stub();
    let base = t.commit_count();
    t.memory(&["dream"]).env("MEMORY_BIN", MEMORY_BIN).ok();
    assert_contains(cat(t.stub_log.join("backup-out")), "request NOT saved");
    assert_eq_str(cat(t.stub_log.join("lock")), "held");
    // One commit for the whole run, holding the mid-run edit; the lock is gone.
    assert_eq!(t.commit_count(), base + 1);
    let shown = t
        .git(&["show", "--name-only", "--format=", "HEAD"])
        .ok()
        .stdout;
    if !shown.contains("mid-run.md") {
        fail("mid-run edit not in the dream commit");
    }
    assert_absent(t.store.join(".git/memory-dream.lock"));
    dfact(
        &t.dp1,
        "after-run",
        "convention",
        "written after the run",
        "2026-09-30",
        None,
    );
    assert_contains(run_ok(&t, &["backup"]), "backed up");
}

#[test]
fn dream_uncertain_and_stale_locks_require_manual_recovery() {
    let mut t = T::new();
    t.dream_template_fixture();
    let lock = t.store.join(".git/memory-dream.lock");
    let tmem = t.tmem.clone();
    let tskill = t.tskill.clone();
    let tm = |args: &[&str]| {
        t.cmd(&tmem, args)
            .env("AGENT_MEMORY_SKILL_DIR", &tskill)
            .run()
    };
    fs::create_dir(&lock).unwrap();
    let o = tm(&["backup"]);
    assert_ne!(o.code, 0, "busy backup reported success");
    assert_contains(o.combined(), "request NOT saved");
    let o = tm(&["dream"]).code(1);
    assert_contains(o.combined(), "manual recovery");
    assert_dir(&lock);
    fs::write(lock.join("owner"), "999999\nold start\n").unwrap();
    let o = tm(&["backup"]);
    assert_ne!(o.code, 0, "busy backup reported success");
    assert_contains(o.combined(), "request NOT saved");
    assert_file(lock.join("owner"));
    fs::remove_file(lock.join("owner")).unwrap();
    fs::remove_dir(&lock).unwrap();
    tm(&["dream"]).ok();
}

#[test]
fn dream_run_refuses_a_second_concurrent_run() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let live = child.id();
    fs::write(t.store.join(".git/memory-dream.lock"), format!("{live}\n")).unwrap();
    let o = t.memory(&["dream"]).run();
    let _ = child.kill();
    let _ = child.wait();
    o.code(1)
        .assert_contains(&format!("already running on this store (pid {live}"));
    assert_absent(t.stub_log.join("args"));
}

#[test]
fn dream_run_rebuilds_indexes_and_commits() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let base = t.commit_count();
    let out = t.memory(&["dream"]).env("STUB_EDIT", "1").ok().stdout;
    assert_eq!(t.commit_count(), base + 1);
    let idx = read(t.dp1.join("INDEX.md"));
    assert_contains(
        &idx,
        "[merged-fact](merged-fact.md) — decision: merged replacement",
    );
    // The agent cannot delete: the obsolete fact stays for manual removal.
    assert_contains(&idx, "stale-fact");
    assert_eq_str(gitout(&t, &["status", "--porcelain"]), "");
    assert_contains(&out, "dream: changes since");
    assert_contains(&out, "merged-fact.md");
    t.memory(&["doctor"]).ok();
    if !gitout(&t, &["remote"]).is_empty() {
        fail("store unexpectedly has a remote");
    }
}

#[test]
fn dream_run_without_changes_reports_none() {
    let mut t = T::new();
    t.dream_launch_fixture();
    let base = t.commit_count();
    let out = run_ok(&t, &["dream"]);
    assert_eq!(t.commit_count(), base);
    assert_contains(&out, "dream: no changes");
}

#[test]
fn dream_run_failure_commits_separately_and_keeps_marker() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.memory(&["dream"]).ok();
    let marker = gitout(&t, &["rev-parse", "refs/memory-dream/last"]);
    let head = gitout(&t, &["rev-parse", "HEAD"]);
    let o = t
        .memory(&["dream"])
        .env("STUB_EDIT", "1")
        .env("STUB_EXIT", "3")
        .code(1);
    assert_contains(o.combined(), "opencode run failed (exit 3)");
    assert_contains(o.combined(), "revert HEAD");
    assert_eq_str(
        gitout(&t, &["log", "-1", "--format=%s"]),
        "memory: dream FAILED (exit 3) — review",
    );
    if gitout(&t, &["rev-parse", "HEAD"]) == head {
        fail("failed run was not committed");
    }
    assert_eq_str(gitout(&t, &["status", "--porcelain"]), "");
    assert_eq_str(gitout(&t, &["rev-parse", "refs/memory-dream/last"]), marker);
    assert_contains(read(t.dp1.join("INDEX.md")), "merged-fact");
    t.git(&["revert", "--no-edit", "HEAD"]).ok();
    if !t.dp1.join("stale-fact.md").is_file() {
        fail("revert lost a pre-existing fact");
    }
    assert_absent(t.dp1.join("merged-fact.md"));
}

#[test]
fn dream_run_times_out_and_commits_failure() {
    let mut t = T::new();
    t.dream_launch_fixture();
    t.write_exec(
        t.bin_dir().join("opencode"),
        r#"#!/usr/bin/env bash
printf -- '---\ndescription: partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
printf -- '---\ndescription: interrupted edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/interrupted.md
echo $$ > "$STUB_LOG/pid"
exec sleep 60
"#,
    );
    // Under heavy parallel load bash may not even have started the stub within
    // the 1s timeout: the watchdog then kills it before it writes anything, so
    // there is nothing to commit (same in the Bash CLI). Retry only in that case
    // (no pid file = the stub body never ran); the timeouts stay as they were.
    let mut attempts = 0;
    let o = loop {
        attempts += 1;
        let o = t
            .memory(&["dream"])
            .env("AGENT_MEMORY_DREAM_TIMEOUT_SECS", "1")
            .env("AGENT_MEMORY_DREAM_KILL_GRACE", "1")
            .code(1);
        if t.stub_log.join("pid").exists() || attempts >= 10 {
            break o;
        }
    };
    assert_contains(o.combined(), "timed out");
    assert_contains(gitout(&t, &["log", "-1", "--format=%s"]), "dream FAILED");
    let pid = cat(t.stub_log.join("pid"));
    let alive = Command::new("kill")
        .args(["-0", &pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    if alive {
        fail("opencode still running after timeout");
    }
    assert_absent(t.store.join(".git/memory-dream.lock"));
}
