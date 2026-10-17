//! Core CLI tests: usage, init, link-group, doctor, index.
//! Ported from the former Bash suite tests/run.sh.

mod common;
use common::*;

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};

const DASH_FACT: &str = "- [a-fact](a-fact.md) — decision: ";

#[test]
fn integration_snippets_exist_and_agree() {
    let t = T::new();
    let r = &t.root;
    let s = r.join("integrations/agent-memory.snippet.md");
    let c = r.join("integrations/cursor/agent-memory.mdc");
    assert_file(&s);
    assert_file(&c);
    assert_file(r.join("integrations/README.md"));
    let readme = cat(r.join("README.md"));
    if !readme.contains("OpenCode V2") {
        fail("README does not identify the V2 plugin integration");
    }
    if !readme.contains("restart OpenCode") {
        fail("README lacks the required restart instruction");
    }
    let snippet = cat(&s);
    for key in [
        ".memory/project/INDEX.md",
        ".memory/groups/*/INDEX.md",
        ".memory/workspace/INDEX.md",
        "memory index",
        "memory grep",
        "never run `memory init`",
        "Never store secrets",
        "description:",
        "type:",
    ] {
        if !snippet.contains(key) {
            fail(&format!("snippet lacks: {key}"));
        }
    }
    let cursor = fs::read_to_string(&c).unwrap();
    assert_eq_str(cursor.lines().next().unwrap(), "---");
    if !cursor.lines().any(|l| l == "alwaysApply: true") {
        fail("cursor rule is not alwaysApply");
    }
    let body =
        extract_cursor_body(&c).unwrap_or_else(|| fail("could not extract cursor rule body"));
    if body != fs::read(&s).unwrap() {
        fail("cursor rule body differs from snippet");
    }
}

#[test]
fn package_json_version_matches_cargo_toml() {
    let t = T::new();
    let cargo = cat(t.root.join("Cargo.toml"));
    let cargo_version = cargo
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|v| v.split('"').next())
        .unwrap_or_else(|| fail("no version in Cargo.toml"))
        .to_string();
    let pkg = cat(t.root.join("package.json"));
    let needle = format!("\"version\": \"{cargo_version}\"");
    if !pkg.contains(&needle) {
        fail(&format!("package.json lacks {needle}"));
    }
}

#[test]
fn integration_cursor_body_detects_final_newline_drift() {
    let t = T::new();
    let s = t.tmp.join("snippet.md");
    let c = t.tmp.join("cursor.mdc");
    fs::write(&s, "body\n").unwrap();
    fs::write(
        &c,
        "---\ndescription: fixture\nalwaysApply: true\n---\nbody\n\n",
    )
    .unwrap();
    let body = extract_cursor_body(&c).unwrap();
    if body == fs::read(&s).unwrap() {
        fail("cursor body comparison ignored final-newline drift");
    }
}

#[test]
fn backup_explains_commit_failure() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    write_fact(
        &t.p(".memory/project"),
        "a-fact",
        "context",
        "something learned",
    );
    t.cmd("git", &["config", "--global", "--unset", "user.email"])
        .ok();
    // make git refuse to guess an identity
    t.cmd("git", &["config", "--global", "user.useConfigOnly", "true"])
        .ok();
    let out = t.memory(&["backup"]).run().code(1);
    out.assert_contains("user.email");
}

#[test]
fn usage_without_args() {
    let t = T::new();
    t.memory::<&str>(&[])
        .run()
        .code(1)
        .assert_contains("usage:");
}

#[test]
fn unknown_command() {
    let t = T::new();
    t.memory(&["frobnicate"])
        .run()
        .code(1)
        .assert_contains("unknown command");
}

#[test]
fn version_flag() {
    let t = T::new();
    assert_eq_str(t.memory(&["--version"]).ok().so(), "memory 0.1.0");
    assert_eq_str(t.memory(&["-v"]).ok().so(), "memory 0.1.0");
    t.memory(&["--version", "extra"])
        .run()
        .code(1)
        .assert_contains("usage: memory --version");
}

#[test]
fn init_creates_structure() {
    let t = T::new();
    let out = t.memory(&["init", "work"]).ok();
    out.assert_contains("initialized");
    assert_link(
        t.p(".memory/project"),
        t.store.join("work/projects/some-api"),
    );
    assert_link(t.p(".memory/workspace"), t.store.join("work/workspace"));
    assert_dir(t.p(".memory/groups"));
    assert_file(t.store.join("work/projects/some-api/INDEX.md"));
    assert_file(t.store.join("work/workspace/INDEX.md"));
    assert_eq_str(
        cat(t.store.join("work/projects/some-api/.project-path")),
        fs::canonicalize(&t.proj).unwrap().to_str().unwrap(),
    );
    assert_dir(t.store.join(".git"));
}

#[test]
fn init_respects_name_flag() {
    let t = T::new();
    t.memory(&["init", "work", "--name", "custom-name"]).ok();
    assert_link(
        t.p(".memory/project"),
        t.store.join("work/projects/custom-name"),
    );
}

#[test]
fn rejects_path_traversal_names() {
    let t = T::new();
    t.memory(&["init", "../evil"])
        .run()
        .code(1)
        .assert_contains("invalid workspace name");
    t.memory(&["init", "work"]).ok();
    t.memory(&["link-group", "../../x"])
        .run()
        .code(1)
        .assert_contains("invalid group name");
}

#[test]
fn relative_memory_home_creates_absolute_links() {
    let mut t = T::new();
    t.set_env("AGENT_MEMORY_HOME", "relative-store");
    t.memory(&["init", "work"]).ok();
    assert_link(
        t.p(".memory/project"),
        t.proj.join("relative-store/work/projects/some-api"),
    );
    assert_link(
        t.p(".memory/workspace"),
        t.proj.join("relative-store/work/workspace"),
    );
}

#[test]
fn init_adds_global_gitignore() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let f = t
        .cmd("git", &["config", "--global", "core.excludesfile"])
        .ok()
        .so();
    let text = fs::read_to_string(&f).unwrap_or_default();
    if !text.lines().any(|l| l == ".memory/") {
        fail("global gitignore lacks .memory/");
    }
}

#[test]
fn init_without_arg_lists_workspaces() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let out = t.memory(&["init"]).run().code(1);
    out.assert_contains("work");
    out.assert_contains("usage");
}

#[test]
fn init_is_idempotent_and_repairs() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_file(t.p(".memory/project")).unwrap(); // simulate a broken/deleted symlink
    t.memory(&["init", "work"]).ok();
    assert_link(
        t.p(".memory/project"),
        t.store.join("work/projects/some-api"),
    );
}

#[test]
fn init_name_collision() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let other = t.tmp.join("elsewhere/some-api");
    fs::create_dir_all(&other).unwrap();
    t.memory(&["init", "work"])
        .cwd(&other)
        .run()
        .code(1)
        .assert_contains("--name");
    t.memory(&["init", "work", "--name", "some-api-2"])
        .cwd(&other)
        .ok();
    assert_link(
        other.join(".memory/project"),
        t.store.join("work/projects/some-api-2"),
    );
}

#[test]
fn reinit_after_rename_keeps_binding() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let renamed = t.tmp.join("code/renamed-api");
    fs::rename(&t.proj, &renamed).unwrap();
    t.memory(&["init", "work"]).cwd(&renamed).ok();
    // still bound to the original store dir, no second dir created
    assert_link(
        renamed.join(".memory/project"),
        t.store.join("work/projects/some-api"),
    );
    if t.store.join("work/projects/renamed-api").is_dir() {
        fail("created a duplicate store dir");
    }
    // backlink updated to the new location
    assert_eq_str(
        cat(renamed.join(".memory/project/.project-path")),
        fs::canonicalize(&renamed).unwrap().to_str().unwrap(),
    );
}

#[test]
fn init_via_symlinked_path_is_not_a_collision() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let alias = t.tmp.join("alias");
    symlink(t.tmp.join("code"), &alias).unwrap();
    let cwd = alias.join("some-api");
    t.memory(&["init", "work", "--name", "some-api"])
        .cwd(&cwd)
        .ok();
    assert_link(
        cwd.join(".memory/project"),
        t.store.join("work/projects/some-api"),
    );
}

#[test]
fn init_rebinds_when_recorded_path_does_not_exist_here() {
    let t = T::new();
    let d = t.store.join("work/projects/some-api");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("INDEX.md"), "").unwrap();
    fs::write(d.join(".project-path"), "/home/other-machine/some-api\n").unwrap();
    t.memory(&["init", "work"]).ok();
    assert_link(t.p(".memory/project"), &d);
    assert_eq_str(
        cat(t.p(".memory/project/.project-path")),
        fs::canonicalize(&t.proj).unwrap().to_str().unwrap(),
    );
}

#[test]
fn reinit_with_different_workspace_is_rejected() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["init", "home"])
        .run()
        .code(1)
        .assert_contains("unlink/rebind");
}

#[test]
fn link_group() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["link-group", "spines"]).ok();
    assert_link(
        t.p(".memory/groups/spines"),
        t.store.join("work/groups/spines"),
    );
    assert_file(t.store.join("work/groups/spines/INDEX.md"));
}

#[test]
fn link_multiple_groups() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["link-group", "spines"]).ok();
    t.memory(&["link-group", "python-services"]).ok();
    assert_link(
        t.p(".memory/groups/spines"),
        t.store.join("work/groups/spines"),
    );
    assert_link(
        t.p(".memory/groups/python-services"),
        t.store.join("work/groups/python-services"),
    );
}

#[test]
fn link_group_requires_init() {
    let t = T::new();
    t.memory(&["link-group", "spines"])
        .run()
        .code(1)
        .assert_contains("memory init");
}

#[test]
fn unlink_group() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["link-group", "spines"]).ok();
    t.memory(&["unlink-group", "spines"]).ok();
    if fs::symlink_metadata(t.p(".memory/groups/spines")).is_ok() {
        fail("symlink still present");
    }
    if !t.store.join("work/groups/spines").is_dir() {
        fail("group memory was deleted");
    }
}

#[test]
fn unlink_group_not_linked() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["unlink-group", "nope"])
        .run()
        .code(1)
        .assert_contains("not linked");
}

#[test]
fn doctor_ok() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["doctor"]).ok().assert_contains("OK");
}

#[test]
fn doctor_broken_symlink() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_dir_all(t.store.join("work/projects/some-api")).unwrap();
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("BROKEN");
}

#[test]
fn doctor_flags_plain_project_or_workspace() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_file(t.p(".memory/project")).unwrap();
    fs::create_dir(t.p(".memory/project")).unwrap();
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("NOT A SYMLINK: .memory/project");
}

#[test]
fn doctor_without_memory_dir_says_so() {
    let t = T::new();
    let out = t.memory(&["doctor"]).ok(); // exit 0: not an error state
    out.assert_contains("no .memory/");
    out.assert_not_contains("OK");
}

#[test]
fn doctor_no_valid_levels_does_not_crash() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_file(t.p(".memory/project")).unwrap();
    fs::remove_file(t.p(".memory/workspace")).unwrap();
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("MISSING LINK: .memory/project");
    out.assert_contains("MISSING LINK: .memory/workspace");
    out.assert_not_contains("unbound variable");
    out.assert_not_contains("panicked");
}

#[test]
fn doctor_store_missing() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_dir_all(&t.store).unwrap();
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("STORE MISSING: ");
    out.assert_not_contains("UNSAFE");
}

#[test]
fn doctor_unsafe_symlink_outside_store() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let outside = t.tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::remove_file(t.p(".memory/project")).unwrap();
    symlink(&outside, t.p(".memory/project")).unwrap();
    let store = fs::canonicalize(&t.store).unwrap();
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("UNSAFE SYMLINK: .memory/project");
    out.assert_contains(&format!("(outside {})", store.display()));
}

#[test]
fn doctor_relative_symlink_is_unsafe() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    fs::remove_file(t.p(".memory/project")).unwrap();
    symlink("../somewhere", t.p(".memory/project")).unwrap();
    let out = t.memory(&["doctor"]).run().code(1);
    out.assert_contains("UNSAFE SYMLINK: .memory/project");
    out.assert_contains("relative");
}

#[test]
fn doctor_file_not_in_index() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/some-fact.md"),
        "---\ndescription: a fact\ntype: gotcha\ncreated: 2026-07-07\n---\nbody\n",
    );
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("NOT IN INDEX");
}

#[test]
fn doctor_index_entry_without_file() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/INDEX.md"),
        "- [ghost](ghost.md) — gotcha: long gone\n",
    );
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("MISSING FILE");
}

#[test]
fn doctor_flags_malformed_index_entry() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/INDEX.md"),
        "this is not an index entry\n",
    );
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("MALFORMED INDEX ENTRY");
}

#[test]
fn doctor_accepts_hyphen_and_en_dash_in_index() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    write_fact_only(&p, "a-fact", "gotcha", "hyphen line");
    write_fact_only(&p, "b-fact", "gotcha", "en dash line");
    t.write(
        p.join("INDEX.md"),
        "- [a-fact](a-fact.md) - gotcha: hyphen line\n- [b-fact](b-fact.md) – gotcha: en dash line\n",
    );
    t.memory(&["doctor"]).ok().assert_contains("OK");
}

#[test]
fn doctor_accepts_valid_updated_field() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/a-fact.md"),
        "---\ndescription: has an updated date\ntype: decision\ncreated: 2026-07-01\nupdated: 2026-09-04\n---\nbody\n",
    );
    t.write(
        t.p(".memory/project/INDEX.md"),
        &format!("{DASH_FACT}has an updated date\n"),
    );
    t.memory(&["doctor"]).ok().assert_contains("OK");
}

#[test]
fn doctor_flags_updated_before_created() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/a-fact.md"),
        "---\ndescription: time travel\ntype: decision\ncreated: 2026-09-04\nupdated: 2026-01-01\n---\nbody\n",
    );
    t.write(
        t.p(".memory/project/INDEX.md"),
        &format!("{DASH_FACT}time travel\n"),
    );
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("updated 2026-01-01 is before created 2026-09-04");
}

#[test]
fn doctor_flags_invalid_updated_format() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/a-fact.md"),
        "---\ndescription: bad updated\ntype: decision\ncreated: 2026-09-04\nupdated: yesterday\n---\nbody\n",
    );
    t.write(
        t.p(".memory/project/INDEX.md"),
        &format!("{DASH_FACT}bad updated\n"),
    );
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("invalid updated date");
}

#[test]
fn doctor_flags_crlf_frontmatter() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/crlf.md"),
        "---\r\ndescription: windows file\r\ntype: gotcha\r\ncreated: 2026-09-04\r\n---\r\nbody\r\n",
    );
    t.write(
        t.p(".memory/project/INDEX.md"),
        "- [crlf](crlf.md) — gotcha: windows file\n",
    );
    t.memory(&["doctor"]).run().code(1).assert_contains("CRLF");
}

#[test]
fn index_generates_sorted_index_from_frontmatter() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    write_fact_only(&p, "zeta-fact", "gotcha", "last by slug");
    write_fact_only(&p, "alpha-fact", "decision", "first by slug");
    t.memory(&["index"])
        .ok()
        .assert_contains("indexed .memory/project (2 facts)");
    assert_eq_str(
        cat(p.join("INDEX.md")),
        "- [alpha-fact](alpha-fact.md) — decision: first by slug\n- [zeta-fact](zeta-fact.md) — gotcha: last by slug",
    );
    t.memory(&["doctor"]).ok().assert_contains("OK");
}

#[test]
fn index_covers_groups_and_workspace() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.memory(&["link-group", "spines"]).ok();
    write_fact_only(
        &t.p(".memory/groups/spines"),
        "shared",
        "convention",
        "shared thing",
    );
    write_fact_only(
        &t.p(".memory/workspace"),
        "editor",
        "context",
        "user prefers vim",
    );
    t.memory(&["index"]).ok();
    assert_contains(
        cat(t.p(".memory/groups/spines/INDEX.md")),
        "[shared](shared.md) — convention: shared thing",
    );
    assert_contains(
        cat(t.p(".memory/workspace/INDEX.md")),
        "[editor](editor.md) — context: user prefers vim",
    );
}

#[test]
fn index_skips_unusable_facts_and_doctor_still_reports_them() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    write_fact_only(&p, "good", "gotcha", "fine");
    t.write(p.join("naked.md"), "no frontmatter at all\n");
    t.memory(&["index"])
        .ok()
        .assert_contains("skipping .memory/project/naked.md");
    assert_eq!(count_nonempty_lines(p.join("INDEX.md")), 1);
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("NOT IN INDEX: .memory/project/naked.md");
}

#[test]
fn index_requires_memory_dir() {
    let t = T::new();
    t.memory(&["index"])
        .run()
        .code(1)
        .assert_contains("no .memory/");
}

#[test]
fn index_rejects_unsafe_project_link_without_writing_external_store() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let outside = t.tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("INDEX.md"), "external sentinel\n").unwrap();
    fs::remove_file(t.p(".memory/project")).unwrap();
    symlink(&outside, t.p(".memory/project")).unwrap();
    t.memory(&["index"])
        .run()
        .code(1)
        .assert_contains(".memory/project");
    assert_eq_str(cat(outside.join("INDEX.md")), "external sentinel");
}

#[test]
fn index_does_not_follow_preexisting_temp_symlink() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    write_fact_only(&p, "safe", "gotcha", "safe fact");
    let outside = t.tmp.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("sentinel"), "external sentinel\n").unwrap();
    symlink(outside.join("sentinel"), p.join("INDEX.md.tmp")).unwrap();
    t.memory(&["index"]).ok();
    assert_eq_str(cat(outside.join("sentinel")), "external sentinel");
}

#[test]
fn index_skips_invalid_frontmatter_type_and_created_values() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    write_fact_only(&p, "good", "gotcha", "good fact");
    write_fact_only(&p, "bad-type", "banana", "bad type");
    t.write(
        p.join("bad-created.md"),
        "---\ndescription: bad date\ntype: gotcha\ncreated: today\n---\nbody\n",
    );
    t.write(p.join("no-frontmatter.md"), "no frontmatter\n");
    let out = t.memory(&["index"]).ok();
    out.assert_contains("skipping .memory/project/bad-type.md");
    out.assert_contains("skipping .memory/project/bad-created.md");
    out.assert_contains("skipping .memory/project/no-frontmatter.md");
    assert_eq_str(
        cat(p.join("INDEX.md")),
        "- [good](good.md) — gotcha: good fact",
    );
}

#[test]
fn index_skips_mixed_crlf_frontmatter() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    write_fact_only(&p, "good", "gotcha", "good fact");
    t.write(
        p.join("mixed-crlf.md"),
        "---\ndescription: windows fact\r\ntype: gotcha\ncreated: 2026-09-04\n---\nbody\n",
    );
    t.memory(&["index"])
        .ok()
        .assert_contains("skipping .memory/project/mixed-crlf.md");
    assert_eq_str(
        cat(p.join("INDEX.md")),
        "- [good](good.md) — gotcha: good fact",
    );
}

#[test]
fn doctor_and_index_reject_duplicate_updated_fields() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(
        t.p(".memory/project/a-fact.md"),
        "---\ndescription: duplicate updated\ntype: decision\ncreated: 2026-09-04\nupdated: yesterday\nupdated: 2026-09-04\n---\nbody\n",
    );
    t.write(
        t.p(".memory/project/INDEX.md"),
        &format!("{DASH_FACT}duplicate updated\n"),
    );
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("invalid updated date");
    t.memory(&["index"])
        .ok()
        .assert_contains("skipping .memory/project/a-fact.md");
    assert_eq!(count_nonempty_lines(t.p(".memory/project/INDEX.md")), 0);
}

#[test]
fn doctor_checks_exact_index_membership() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    write_fact_only(&p, "a-fact", "gotcha", "missing from index");
    write_fact_only(&p, "other", "gotcha", "mentions ](a-fact.md)");
    t.write(
        p.join("INDEX.md"),
        "- [other](other.md) — gotcha: mentions ](a-fact.md)\n",
    );
    t.memory(&["doctor"])
        .run()
        .code(1)
        .assert_contains("NOT IN INDEX: .memory/project/a-fact.md");
}

#[test]
fn index_fails_when_no_usable_linked_level_exists() {
    let t = T::new();
    fs::create_dir_all(&t.store).unwrap();
    fs::create_dir_all(t.p(".memory/groups")).unwrap();
    t.memory(&["index"])
        .run()
        .code(1)
        .assert_contains("no usable linked levels");
}

/// Rust-meaningful equivalent of the Bash `sort` failure test: an index
/// render/publish failure must leave existing INDEX.md files intact.  Here the
/// workspace level directory is made read-only, so its index cannot be written.
#[test]
fn index_sort_failure_preserves_existing_indexes() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    let p = t.p(".memory/project");
    let w = t.p(".memory/workspace");
    write_fact_only(&p, "a-fact", "gotcha", "a fact");
    t.write(p.join("INDEX.md"), "project sentinel\n");
    t.write(w.join("INDEX.md"), "workspace sentinel\n");
    let wdir = t.store.join("work/workspace");
    fs::set_permissions(&wdir, fs::Permissions::from_mode(0o555)).unwrap();
    // Running as root bypasses directory permissions; nothing to verify then.
    let writable = fs::File::create(wdir.join(".probe")).is_ok();
    if writable {
        fs::set_permissions(&wdir, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("  (skipped: directory permissions are not enforced)");
        return;
    }
    let out = t.memory(&["index"]).run().code(1);
    fs::set_permissions(&wdir, fs::Permissions::from_mode(0o755)).unwrap();
    if out.combined().trim().is_empty() {
        fail("index failure produced no diagnostic");
    }
    assert_eq_str(cat(p.join("INDEX.md")), "project sentinel");
    assert_eq_str(cat(w.join("INDEX.md")), "workspace sentinel");
}

#[test]
fn index_broken_required_and_group_links_fail_without_publishing() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    t.write(t.p(".memory/project/INDEX.md"), "project sentinel\n");
    fs::remove_file(t.p(".memory/workspace")).unwrap();
    symlink(
        t.store.join("work/missing-workspace"),
        t.p(".memory/workspace"),
    )
    .unwrap();
    t.memory(&["index"])
        .run()
        .code(1)
        .assert_contains("broken required memory link");
    assert_eq_str(cat(t.p(".memory/project/INDEX.md")), "project sentinel");

    fs::remove_file(t.p(".memory/workspace")).unwrap();
    symlink(t.store.join("work/workspace"), t.p(".memory/workspace")).unwrap();
    t.memory(&["link-group", "spines"]).ok();
    fs::remove_file(t.p(".memory/groups/spines")).unwrap();
    symlink(
        t.store.join("work/groups/missing-spines"),
        t.p(".memory/groups/spines"),
    )
    .unwrap();
    t.memory(&["index"])
        .run()
        .code(1)
        .assert_contains("broken group memory link");
    assert_eq_str(cat(t.p(".memory/project/INDEX.md")), "project sentinel");
}

#[test]
fn index_rejects_directory_destinations_before_any_publish() {
    let t = T::new();
    t.memory(&["init", "work"]).ok();
    write_fact_only(&t.p(".memory/project"), "a-fact", "gotcha", "a fact");
    t.write(t.p(".memory/project/INDEX.md"), "project sentinel\n");
    let dest = t.p(".memory/workspace/INDEX.md");
    fs::remove_file(&dest).unwrap();
    fs::create_dir(&dest).unwrap();
    t.memory(&["index"])
        .run()
        .code(1)
        .assert_contains("invalid index destination");
    assert_eq_str(cat(t.p(".memory/project/INDEX.md")), "project sentinel");
    fs::remove_dir(&dest).unwrap();

    let outside = t.tmp.join("outside-index");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &dest).unwrap();
    t.memory(&["index"])
        .run()
        .code(1)
        .assert_contains("invalid index destination");
    assert_eq_str(cat(t.p(".memory/project/INDEX.md")), "project sentinel");
    fs::remove_file(&dest).unwrap();
    fs::remove_dir(&outside).unwrap();
}
