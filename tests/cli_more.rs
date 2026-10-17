//! doctor / index / status / grep / backup / install-opencode tests.
//! Ported from the former Bash suite tests/run.sh.

mod common;
use common::*;

use std::fs;
use std::os::unix::fs::symlink;

const PROJ_IDX: &str = ".memory/project/INDEX.md";

fn init(t: &T) {
    t.memory(&["init", "work"]).ok();
}

fn last_subject(t: &T) -> String {
    t.git(&["log", "-1", "--format=%s"]).ok().so()
}

fn last_body(t: &T) -> String {
    t.git(&["log", "-1", "--format=%b"]).ok().so()
}

fn plugin_target(t: &T) -> std::path::PathBuf {
    t.root.join("skills/agent-memory/opencode/agent-memory.js")
}

#[test]
fn doctor_and_index_reject_duplicate_description() {
    let t = T::new();
    init(&t);
    t.write(
        t.p(".memory/project/duplicate.md"),
        "---\ndescription:\ndescription: valid description\ntype: gotcha\ncreated: 2026-09-04\n---\nbody\n",
    );
    t.write(
        t.p(PROJ_IDX),
        "- [duplicate](duplicate.md) — gotcha: valid description\n",
    );
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("missing or duplicate description");
    let out = t.memory(&["index"]).run();
    out.assert_contains("skipping .memory/project/duplicate.md");
    assert_eq!(count_nonempty_lines(t.p(PROJ_IDX)), 0);
}

#[test]
fn doctor_and_index_reject_duplicate_type() {
    let t = T::new();
    init(&t);
    t.write(
        t.p(".memory/project/duplicate.md"),
        "---\ndescription: valid description\ntype: banana\ntype: gotcha\ncreated: 2026-09-04\n---\nbody\n",
    );
    t.write(
        t.p(PROJ_IDX),
        "- [duplicate](duplicate.md) — gotcha: valid description\n",
    );
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("missing or invalid type");
    let out = t.memory(&["index"]).run();
    out.assert_contains("skipping .memory/project/duplicate.md");
    assert_eq!(count_nonempty_lines(t.p(PROJ_IDX)), 0);
}

#[test]
fn doctor_and_index_reject_duplicate_created() {
    let t = T::new();
    init(&t);
    t.write(
        t.p(".memory/project/duplicate.md"),
        "---\ndescription: valid description\ntype: gotcha\ncreated: yesterday\ncreated: 2026-09-04\n---\nbody\n",
    );
    t.write(
        t.p(PROJ_IDX),
        "- [duplicate](duplicate.md) — gotcha: valid description\n",
    );
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("missing or invalid created date");
    let out = t.memory(&["index"]).run();
    out.assert_contains("skipping .memory/project/duplicate.md");
    assert_eq!(count_nonempty_lines(t.p(PROJ_IDX)), 0);
}

#[test]
fn doctor_and_index_reject_invalid_calendar_dates() {
    let t = T::new();
    init(&t);
    t.write(
        t.p(".memory/project/bad-created.md"),
        "---\ndescription: non-leap day\ntype: gotcha\ncreated: 2026-02-29\n---\nbody\n",
    );
    t.write(
        t.p(".memory/project/bad-month.md"),
        "---\ndescription: bad month\ntype: gotcha\ncreated: 2026-13-01\n---\nbody\n",
    );
    t.write(
        t.p(".memory/project/bad-updated.md"),
        "---\ndescription: bad updated day\ntype: gotcha\ncreated: 2024-02-29\nupdated: 2025-02-29\n---\nbody\n",
    );
    t.write(
        t.p(".memory/project/leap-day.md"),
        "---\ndescription: valid leap day\ntype: gotcha\ncreated: 2024-02-29\nupdated: 2024-02-29\n---\nbody\n",
    );
    t.memory(&["index"]).ok();
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("invalid created date");
    out.assert_contains("invalid updated date");
    let out = t.memory(&["index"]).run();
    out.assert_contains("skipping .memory/project/bad-created.md");
    out.assert_contains("skipping .memory/project/bad-month.md");
    out.assert_contains("skipping .memory/project/bad-updated.md");
    assert_eq_str(
        cat(t.p(PROJ_IDX)),
        "- [leap-day](leap-day.md) — gotcha: valid leap day",
    );
}

#[test]
fn doctor_warns_on_long_description_but_passes() {
    let t = T::new();
    init(&t);
    let long = "word ".repeat(30);
    write_fact_only(&t.p(".memory/project"), "wordy", "gotcha", &long);
    t.memory(&["index"]).ok();
    let out = t.memory(&["doctor"]).ok();
    out.assert_contains("warnings (not failures):");
    out.assert_contains("LONG DESCRIPTION (150 chars > 120): .memory/project/wordy.md");
    out.assert_contains("OK");
}

#[test]
fn doctor_warns_on_index_description_drift() {
    let t = T::new();
    init(&t);
    write_fact_only(&t.p(".memory/project"), "a-fact", "gotcha", "new wording");
    t.write(
        t.p(PROJ_IDX),
        "- [a-fact](a-fact.md) — gotcha: old wording\n",
    );
    let out = t.memory(&["doctor"]).ok();
    out.assert_contains("INDEX DESCRIPTION DRIFT: .memory/project/a-fact.md");
    out.assert_contains("memory index");
    out.assert_contains("OK");
}

#[test]
fn doctor_warns_on_dangling_wiki_link_and_accepts_cross_level() {
    let t = T::new();
    init(&t);
    t.memory(&["link-group", "spines"]).ok();
    write_fact_only(
        &t.p(".memory/groups/spines"),
        "shared-topology",
        "reference",
        "shared topology",
    );
    t.write(
        t.p(".memory/project/a-fact.md"),
        "---\ndescription: links elsewhere\ntype: context\ncreated: 2026-09-04\n---\nSee [[shared-topology]] (exists in the group) and [[does-not-exist]].\n",
    );
    t.memory(&["index"]).ok();
    let out = t.memory(&["doctor"]).ok();
    out.assert_contains("DANGLING WIKI LINK: .memory/project/a-fact.md -> [[does-not-exist]]");
    if out.stdout.contains("[[shared-topology]]") {
        fail(&format!(
            "cross-level link wrongly reported: {}",
            out.stdout
        ));
    }
    out.assert_contains("OK");
}

#[test]
fn doctor_checks_group_levels_too() {
    let t = T::new();
    init(&t);
    t.memory(&["link-group", "spines"]).ok();
    t.write(
        t.p(".memory/groups/spines/shared-fact.md"),
        "---\ndescription: shared\ntype: convention\ncreated: 2026-07-07\n---\nbody\n",
    );
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("NOT IN INDEX");
    out.assert_contains("spines");
}

#[test]
fn doctor_bad_frontmatter() {
    let t = T::new();
    init(&t);
    t.write(
        t.p(".memory/project/bad-fact.md"),
        "---\ndescription: has a bogus type\ntype: banana\n---\nbody\n",
    );
    t.write(
        t.p(PROJ_IDX),
        "- [bad-fact](bad-fact.md) — has a bogus type\n",
    );
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("invalid type");
}

#[test]
fn doctor_missing_frontmatter() {
    let t = T::new();
    init(&t);
    t.write(
        t.p(".memory/project/naked-fact.md"),
        "just a body, no frontmatter\n",
    );
    t.write(t.p(PROJ_IDX), "- [naked-fact](naked-fact.md) — naked\n");
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("FRONTMATTER");
}

#[test]
fn doctor_does_not_call_stale_project_path_an_orphan() {
    let t = T::new();
    init(&t);
    let dead = t.store.join("work/projects/dead");
    fs::create_dir_all(&dead).unwrap();
    fs::write(dead.join("INDEX.md"), "").unwrap();
    fs::write(
        dead.join(".project-path"),
        format!("{}\n", t.tmp.join("gone-forever").display()),
    )
    .unwrap();
    // informational only — still exit 0
    let out = t.memory(&["doctor"]).ok();
    out.assert_contains("OK");
}

#[test]
fn rejects_extra_arguments() {
    let t = T::new();
    let out = t.memory(&["init", "work", "extra"]).run().code(1);
    out.assert_contains("usage: memory init");
    let out = t.memory(&["doctor", "extra"]).run().code(1);
    out.assert_contains("usage: memory doctor");
    init(&t);
    let out = t.memory(&["link-group", "one", "two"]).run().code(1);
    out.assert_contains("usage: memory link-group");
}

#[test]
fn status_shows_levels_counts_and_unlinked_groups() {
    let t = T::new();
    init(&t);
    t.memory(&["link-group", "spines"]).ok();
    let platform = t.store.join("work/groups/platform");
    fs::create_dir_all(&platform).unwrap();
    fs::write(platform.join("INDEX.md"), "").unwrap();
    write_fact_only(&t.p(".memory/project"), "a-fact", "gotcha", "a");
    write_fact_only(&t.p(".memory/project"), "b-fact", "gotcha", "b");
    write_fact_only(&t.p(".memory/groups/spines"), "s-fact", "convention", "s");
    let out = t.memory(&["status"]).ok();
    let store = fs::canonicalize(&t.store).unwrap();
    out.assert_contains(&format!("store:      {}", store.display()));
    out.assert_contains("workspace:  work  (0 facts)");
    out.assert_contains("project:    some-api  (2 facts)");
    out.assert_contains("group:      spines  (1 facts)");
    out.assert_contains("not linked: platform");
    out.assert_contains("backup:     last never");
}

#[test]
fn status_reports_backup_state() {
    let t = T::new();
    init(&t);
    write_fact_only(&t.p(".memory/project"), "a-fact", "gotcha", "a");
    t.memory(&["backup"]).ok();
    let out = t.memory(&["status"]).ok();
    let today = t.cmd("date", &["+%Y-%m-%d"]).ok().so();
    out.assert_contains(&format!("backup:     last {today}, store clean"));
    write_fact_only(&t.p(".memory/project"), "b-fact", "gotcha", "b");
    let out = t.memory(&["status"]).ok();
    out.assert_contains("1 uncommitted path(s) — run 'memory backup'");
}

#[test]
fn status_requires_init() {
    let t = T::new();
    let out = t.memory(&["status"]).run().code(1);
    out.assert_contains("no .memory/");
}

#[test]
fn grep_searches_bodies_across_levels_following_symlinks() {
    let t = T::new();
    init(&t);
    t.memory(&["link-group", "spines"]).ok();
    t.write(
        t.p(".memory/project/pool.md"),
        "---\ndescription: pool limit\ntype: gotcha\ncreated: 2026-09-04\n---\npgbouncer in transaction mode returns stale statements above 50 connections.\n",
    );
    t.write(
        t.p(".memory/groups/spines/topology.md"),
        "---\ndescription: shared topology\ntype: reference\ncreated: 2026-09-04\n---\nAll services share one PgBouncer in front of the primary.\n",
    );
    t.memory(&["index"]).ok();
    let out = t.memory(&["grep", "pgbouncer"]).ok();
    out.assert_contains(".memory/project/pool.md:6:pgbouncer in transaction mode");
    out.assert_contains(".memory/groups/spines/topology.md:6:All services share one PgBouncer");
}

#[test]
fn grep_no_match_exits_zero_with_message() {
    let t = T::new();
    init(&t);
    let out = t.memory(&["grep", "nothing-here-at-all"]).run().code(0);
    assert_eq_str(out.so(), "no matches for 'nothing-here-at-all'");
}

#[test]
fn grep_excludes_project_path_file() {
    let t = T::new();
    init(&t);
    // the path is recorded in .project-path
    let out = t.memory(&["grep", "some-api"]).ok();
    if out.stdout.contains(".project-path") {
        fail(&format!("matched .project-path: {}", out.stdout));
    }
}

#[test]
fn grep_requires_pattern() {
    let t = T::new();
    init(&t);
    let out = t.memory(&["grep"]).run().code(1);
    out.assert_contains("usage: memory grep");
}

#[test]
fn status_rejects_unsafe_and_broken_required_links() {
    let t = T::new();
    init(&t);
    fs::remove_file(t.p(".memory/workspace")).unwrap();
    symlink(
        t.store.join("work/missing-workspace"),
        t.p(".memory/workspace"),
    )
    .unwrap();
    let out = t.memory(&["status"]).run().code(1);
    out.assert_contains("unsafe or broken symlink: .memory/workspace (run 'memory doctor')");

    fs::remove_file(t.p(".memory/workspace")).unwrap();
    symlink(t.store.join("work/workspace"), t.p(".memory/workspace")).unwrap();
    let outside = t.tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::remove_file(t.p(".memory/project")).unwrap();
    symlink(&outside, t.p(".memory/project")).unwrap();
    let out = t.memory(&["status"]).run().code(1);
    out.assert_contains("unsafe or broken symlink: .memory/project (run 'memory doctor')");
}

#[test]
fn status_labels_unsafe_and_broken_group_links() {
    let t = T::new();
    init(&t);
    t.memory(&["link-group", "shared"]).ok();
    let outside = t.tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    write_fact_only(&outside, "secret", "gotcha", "outside secret");
    fs::remove_file(t.p(".memory/groups/shared")).unwrap();
    symlink(&outside, t.p(".memory/groups/shared")).unwrap();
    symlink(
        t.store.join("work/groups/missing"),
        t.p(".memory/groups/missing"),
    )
    .unwrap();
    let out = t.memory(&["status"]).ok();
    out.assert_contains("group:      shared  (UNSAFE OR BROKEN LINK — run 'memory doctor')");
    out.assert_contains("group:      missing  (UNSAFE OR BROKEN LINK — run 'memory doctor')");
    let o = &out.stdout;
    if o.contains("outside secret")
        || o.contains(&outside.display().to_string())
        || o.contains("shared  (1 facts)")
    {
        fail(&format!("disclosed unsafe group data: {o}"));
    }
}

#[test]
fn status_reports_backup_unavailable_without_git() {
    let t = T::new();
    init(&t);
    fs::remove_dir_all(t.store.join(".git")).unwrap();
    let out = t.memory(&["status"]).ok();
    out.assert_contains("backup:     unavailable (store is not a git repository)");
}

#[test]
fn status_does_not_use_parent_git_repository() {
    let mut t = T::new();
    let parent = t.tmp.join("parent");
    fs::create_dir_all(&parent).unwrap();
    t.cmd("git", &["init", "-q"]).cwd(&parent).ok();
    fs::write(parent.join("README.md"), "parent repository\n").unwrap();
    t.cmd("git", &["add", "README.md"]).cwd(&parent).ok();
    t.cmd("git", &["commit", "-q", "-m", "parent commit"])
        .cwd(&parent)
        .ok();
    let store = parent.join("store");
    t.set_env("AGENT_MEMORY_HOME", &store);
    init(&t);
    fs::remove_dir_all(store.join(".git")).unwrap();
    let out = t.memory(&["status"]).ok();
    out.assert_contains("backup:     unavailable (store is not a git repository)");
}

#[test]
fn status_counts_untracked_when_git_hides_them() {
    let t = T::new();
    init(&t);
    t.memory(&["backup"]).ok();
    t.git(&["config", "status.showUntrackedFiles", "no"]).ok();
    write_fact_only(
        &t.p(".memory/project"),
        "untracked",
        "gotcha",
        "still must count",
    );
    let out = t.memory(&["status"]).ok();
    out.assert_contains("1 uncommitted path(s) — run 'memory backup'");
}

#[test]
fn grep_rejects_unsafe_and_broken_required_links() {
    let t = T::new();
    init(&t);
    let outside = t.tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::remove_file(t.p(".memory/project")).unwrap();
    symlink(&outside, t.p(".memory/project")).unwrap();
    let out = t.memory(&["grep", "needle"]).run().code(1);
    out.assert_contains("unsafe or broken symlink: .memory/project (run 'memory doctor')");

    fs::remove_file(t.p(".memory/project")).unwrap();
    symlink(
        t.store.join("work/projects/some-api"),
        t.p(".memory/project"),
    )
    .unwrap();
    fs::remove_file(t.p(".memory/workspace")).unwrap();
    symlink(
        t.store.join("work/missing-workspace"),
        t.p(".memory/workspace"),
    )
    .unwrap();
    let out = t.memory(&["grep", "needle"]).run().code(1);
    out.assert_contains("unsafe or broken symlink: .memory/workspace (run 'memory doctor')");
}

#[test]
fn grep_rejects_unsafe_and_broken_group_links() {
    let t = T::new();
    init(&t);
    t.memory(&["link-group", "shared"]).ok();
    let outside = t.tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.md"), "external needle\n").unwrap();
    fs::remove_file(t.p(".memory/groups/shared")).unwrap();
    symlink(&outside, t.p(".memory/groups/shared")).unwrap();
    let out = t.memory(&["grep", "needle"]).run().code(1);
    out.assert_contains("unsafe or broken symlink: .memory/groups/shared (run 'memory doctor')");

    fs::remove_file(t.p(".memory/groups/shared")).unwrap();
    symlink(
        t.store.join("work/groups/missing"),
        t.p(".memory/groups/shared"),
    )
    .unwrap();
    let out = t.memory(&["grep", "needle"]).run().code(1);
    out.assert_contains("unsafe or broken symlink: .memory/groups/shared (run 'memory doctor')");
}

#[test]
fn grep_uses_lowercase_r_to_avoid_nested_symlinks() {
    let mut t = T::new();
    init(&t);
    let fake_bin = t.tmp.join("fake-bin");
    let grep_args = t.tmp.join("grep-args");
    t.set_env("GREP_ARGS", &grep_args);
    t.write_exec(
        fake_bin.join("grep"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$GREP_ARGS\"\nexit 1\n",
    );
    t.prepend_path(&fake_bin);
    let out = t.memory(&["grep", "needle"]).run().code(0);
    assert_eq_str(out.so(), "no matches for 'needle'");
    let args = cat(&grep_args);
    assert_contains(&args, "-r");
    if args.contains("-R") {
        fail(&format!("grep followed nested symlinks: {args}"));
    }
}

#[test]
fn backup_commits_store() {
    let t = T::new();
    init(&t);
    t.write(
        t.p(".memory/project/a-fact.md"),
        "---\ndescription: something learned\ntype: context\ncreated: 2026-07-07\n---\nbody\n",
    );
    t.memory(&["backup"]).ok();
    let n = t.git(&["rev-list", "--count", "HEAD"]).ok().so();
    assert_eq_str(n, "1");
    let st = t.git(&["status", "--porcelain"]).ok();
    if st.stdout.lines().any(|l| !l.is_empty()) {
        fail("store has uncommitted changes");
    }
}

#[test]
fn backup_first_commit_in_fresh_store() {
    // A fresh store has no HEAD yet; `git diff --cached --name-status` still
    // works (diffs against the empty tree) and classifies paths correctly.
    let t = T::new();
    init(&t);
    write_fact_only(&t.p(".memory/project"), "a-fact", "context", "something");
    t.memory(&["backup"]).ok();
    let n = t.git(&["rev-list", "--count", "HEAD"]).ok().so();
    assert_eq_str(n, "1");
    let msg = last_subject(&t);
    // init's .project-path write lands in the same first commit as the fact,
    // under the fallback "other" scope, so this is a multi-scope subject.
    assert_contains(&msg, "add");
    assert_contains(&msg, "2 scopes");
}

#[test]
fn backup_single_scope_add_update_remove_subject() {
    let t = T::new();
    init(&t);
    let d = t.p(".memory/project");
    write_fact_only(&d, "a-fact", "context", "a");
    write_fact_only(&d, "b-fact", "context", "b");
    t.memory(&["backup"]).ok();
    // Distinct content keeps git's rename detection from pairing the removed
    // and added files together.
    t.write(
        d.join("c-fact.md"),
        "---\ndescription: a brand new unrelated fact about something else entirely\ntype: reference\ncreated: 2026-09-04\n---\nthis body is deliberately unlike anything else in the store so it is\nnever mistaken for a rename of another file\n",
    );
    let mut a = fs::read_to_string(d.join("a-fact.md")).unwrap();
    a.push_str("more\n"); // update
    fs::write(d.join("a-fact.md"), a).unwrap();
    fs::remove_file(d.join("b-fact.md")).unwrap(); // remove
    t.memory(&["backup"]).ok();
    assert_eq_str(
        last_subject(&t),
        "memory(some-api): add c-fact; update a-fact; remove b-fact",
    );
}

#[test]
fn backup_multiple_scopes_subject_and_body() {
    let t = T::new();
    init(&t);
    t.memory(&["link-group", "grp1"]).ok();
    t.memory(&["backup"]).ok();
    write_fact_only(&t.p(".memory/project"), "a-fact", "context", "a");
    write_fact_only(&t.p(".memory/groups/grp1"), "g-fact", "context", "g");
    t.memory(&["backup"]).ok();
    assert_eq_str(last_subject(&t), "memory: add 2 in 2 scopes");
    let body = last_body(&t);
    assert_contains(&body, "group:grp1: add g-fact");
    assert_contains(&body, "some-api: add a-fact");
    assert_contains(&body, "backup 20");
}

#[test]
fn backup_rename_shows_slug_arrow() {
    let t = T::new();
    init(&t);
    write_fact_only(&t.p(".memory/project"), "a-fact", "context", "a");
    t.memory(&["backup"]).ok();
    fs::rename(
        t.p(".memory/project/a-fact.md"),
        t.p(".memory/project/a-fact-renamed.md"),
    )
    .unwrap();
    t.memory(&["index"]).ok();
    t.memory(&["backup"]).ok();
    assert_eq_str(
        last_subject(&t),
        "memory(some-api): rename a-fact->a-fact-renamed",
    );
}

#[test]
fn backup_index_only_reindex_subject() {
    let t = T::new();
    init(&t);
    write_fact_only(&t.p(".memory/project"), "a-fact", "context", "a");
    t.memory(&["backup"]).ok();
    t.memory(&["index"]).ok(); // only INDEX.md changes
    t.memory(&["backup"]).ok();
    assert_eq_str(last_subject(&t), "memory(some-api): reindex");
    assert_contains(last_body(&t), "backup 20");
}

#[test]
fn backup_long_list_truncates_within_72_chars() {
    let t = T::new();
    init(&t);
    t.memory(&["backup"]).ok();
    for i in 1..=20 {
        write_fact_only(
            &t.p(".memory/project"),
            &format!("fact-{i}-with-a-long-slug-name"),
            "context",
            &format!("d {i}"),
        );
    }
    t.memory(&["backup"]).ok();
    let subj = last_subject(&t);
    if subj.chars().count() > 72 {
        fail(&format!("subject exceeds 72 chars: {subj}"));
    }
    assert_contains(&subj, "memory(some-api): add");
    assert_contains(&subj, "more");
}

#[test]
fn backup_single_long_slug_falls_back_to_counts() {
    let t = T::new();
    init(&t);
    t.memory(&["backup"]).ok();
    write_fact_only(
        &t.p(".memory/project"),
        "single-leg-then-two-leg-exchange-reimport-with-extra-words",
        "context",
        "d",
    );
    t.memory(&["backup"]).ok();
    assert_eq_str(last_subject(&t), "memory(some-api): add 1");
}

#[test]
fn backup_nothing_to_do() {
    let t = T::new();
    init(&t);
    t.memory(&["backup"]).ok();
    let out = t.memory(&["backup"]).ok();
    out.assert_contains("nothing to back up");
}

#[test]
fn backup_push_without_remote_fails() {
    let t = T::new();
    init(&t);
    let out = t.memory(&["backup", "--push"]).run().code(1);
    out.assert_contains("remote");
}

#[test]
fn install_opencode_creates_plugin_symlink() {
    let t = T::new();
    let out = t.memory(&["install-opencode"]).ok();
    out.assert_contains("opencode plugin add github:dot-opencode/memory#release");
    assert_link(
        t.home.join(".config/opencode/plugins/agent-memory.js"),
        plugin_target(&t),
    );
}

#[test]
fn install_opencode_is_idempotent() {
    let t = T::new();
    t.memory(&["install-opencode"]).ok();
    let out = t.memory(&["install-opencode"]).ok();
    out.assert_contains("already installed");
}

#[test]
fn install_opencode_replaces_legacy_symlink_in_xdg_config() {
    let mut t = T::new();
    let xdg = t.tmp.join("xdg");
    t.set_env("XDG_CONFIG_HOME", &xdg);
    fs::create_dir_all(xdg.join("opencode/plugins")).unwrap();
    let legacy = t.tmp.join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("agent-memory.js"), "").unwrap();
    symlink(
        legacy.join("agent-memory.js"),
        xdg.join("opencode/plugins/agent-memory.js"),
    )
    .unwrap();
    t.memory(&["install-opencode"]).ok();
    assert_link(
        xdg.join("opencode/plugins/agent-memory.js"),
        plugin_target(&t),
    );
    assert_absent(t.home.join(".config/opencode/plugins/agent-memory.js"));
}

#[test]
fn install_opencode_replaces_dangling_symlink() {
    let t = T::new();
    let plugins = t.home.join(".config/opencode/plugins");
    fs::create_dir_all(&plugins).unwrap();
    symlink(
        t.tmp.join("moved/agent-memory.js"),
        plugins.join("agent-memory.js"),
    )
    .unwrap();
    t.memory(&["install-opencode"]).ok();
    assert_link(plugins.join("agent-memory.js"), plugin_target(&t));
}

#[test]
fn install_opencode_does_not_write_config_or_presets() {
    let mut t = T::new();
    let xdg = t.tmp.join("xdg");
    t.set_env("XDG_CONFIG_HOME", &xdg);
    fs::create_dir_all(xdg.join("opencode")).unwrap();
    fs::write(xdg.join("opencode/opencode.json"), "config sentinel\n").unwrap();
    fs::write(
        xdg.join("opencode/oh-my-opencode-slim.json"),
        "preset sentinel\n",
    )
    .unwrap();
    t.memory(&["install-opencode"]).ok();
    assert_eq_str(cat(xdg.join("opencode/opencode.json")), "config sentinel");
    assert_eq_str(
        cat(xdg.join("opencode/oh-my-opencode-slim.json")),
        "preset sentinel",
    );
}

#[test]
fn install_opencode_refuses_regular_file() {
    let t = T::new();
    let plugins = t.home.join(".config/opencode/plugins");
    fs::create_dir_all(&plugins).unwrap();
    fs::write(plugins.join("agent-memory.js"), "keep me\n").unwrap();
    let out = t.memory(&["install-opencode"]).run().code(1);
    out.assert_contains("refusing to overwrite");
    assert_eq_str(cat(plugins.join("agent-memory.js")), "keep me");
}
