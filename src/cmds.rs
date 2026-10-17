//! init, link-group, unlink-group, index, doctor, status, grep, install-opencode.

use crate::ctx::*;
use crate::facts::*;
use crate::fsutil::*;
use crate::{die, outln};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::process::Command;

pub fn set_store_from_home() {
    let mh = memory_home();
    let s = canon_dir(&mh).unwrap_or(mh);
    st(|x| x.store_home = s);
}

fn validate_store_destination(path: &str) {
    if !is_in_store(path) {
        die!("unsafe store destination: {path}");
    }
    let store = crate::ctx::store();
    let mut cur = path.to_string();
    while cur != store {
        if is_link(&cur) {
            die!("unsafe symlink destination: {cur}");
        }
        cur = dirname(&cur);
    }
}

fn ensure_global_gitignore() {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut c = Command::new("git");
    c.args(["config", "--global", "--path", "--get", "core.excludesfile"]);
    let mut f = cap_trim(&mut c).unwrap_or_default();
    if f.is_empty() {
        let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_default();
        let base = if xdg.is_empty() {
            format!("{home}/.config")
        } else {
            xdg
        };
        f = format!("{base}/git/ignore");
    } else if !f.starts_with('/') {
        let mut c = Command::new("git");
        c.args([
            "config",
            "--global",
            "--show-origin",
            "--get",
            "core.excludesfile",
        ]);
        let origin = cap_trim(&mut c).unwrap_or_default();
        let source = origin.split('\t').next().unwrap_or("");
        let source = source.strip_prefix("file:").unwrap_or(source);
        if source.is_empty() {
            die!("cannot resolve global core.excludesfile");
        }
        f = format!("{}/{}", dirname(source), f);
    }
    if f == "~" || f.starts_with("~/") {
        f = format!("{home}{}", &f[1..]);
    }
    if is_file(&f) {
        let c = read_string(&f).unwrap_or_default();
        if c.split('\n').any(|l| l == ".memory/") {
            return;
        }
    }
    mkdir_p(&dirname(&f));
    let existing = fs::read(&f).unwrap_or_default();
    let mut add = String::new();
    if !existing.is_empty() && *existing.last().unwrap() != b'\n' {
        add.push('\n');
    }
    add.push_str(".memory/\n");
    match fs::OpenOptions::new().append(true).create(true).open(&f) {
        Ok(mut h) => {
            let _ = h.write_all(add.as_bytes());
        }
        Err(e) => die!("cannot write {f}: {e}"),
    }
    let rc = run(Command::new("git").args(["config", "--global", "core.excludesfile", &f]));
    if rc != 0 {
        exit_with(rc);
    }
    outln!("added '.memory/' to global gitignore ({f})");
}

pub fn init(args: &[String]) -> i32 {
    let mut workspace = String::new();
    let mut name = String::new();
    let mut name_given = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--name" {
            if name_given {
                die!("--name may be specified only once");
            }
            if args.len() - i < 2 || args[i + 1].is_empty() {
                die!("--name requires a value");
            }
            name = args[i + 1].clone();
            name_given = true;
            i += 2;
        } else if a.starts_with('-') {
            die!("unknown option: {a}");
        } else {
            if !workspace.is_empty() {
                die!("usage: memory init <workspace> [--name <project-name>]");
            }
            workspace = a.clone();
            i += 1;
        }
    }
    let mh = memory_home();
    if workspace.is_empty() {
        eprintln!("existing workspaces:");
        if is_dir(&mh) {
            let items = glob_dir(&mh, |p, _| exists(p));
            if items.is_empty() {
                eprintln!("  (none)");
            }
            for it in items {
                eprintln!("{}", basename(&it));
            }
        } else {
            eprintln!("  (none)");
        }
        eprintln!("usage: memory init <workspace> [--name <project-name>]");
        exit_with(1);
    }
    require_slug("workspace name", &workspace);
    if name_given {
        require_slug("project name", &name);
    }
    mkdir_p(&mh);
    let sh = canon_dir(&mh).unwrap_or_else(|| mh.clone());
    st(|x| {
        x.store_home = sh.clone();
        x.memory_home = sh.clone();
    });
    let store = sh;

    for g in [".memory/project", ".memory/workspace"] {
        if exists_or_link(g) {
            if !is_link(g) {
                die!("{g} exists but is not a symlink");
            }
            if safe_link_target(g).is_none() {
                die!("unsafe symlink: {g} must target an absolute path inside {store}");
            }
        }
    }
    let ws = format!("{store}/{workspace}");
    if is_link(".memory/workspace") {
        let existing = match safe_link_target(".memory/workspace") {
            Some(e) => e,
            None => die!("unsafe symlink: .memory/workspace"),
        };
        if existing != format!("{ws}/workspace") {
            die!(".memory/workspace is bound to {existing}; unlink/rebind it explicitly before initializing '{workspace}'");
        }
    }
    if is_link(".memory/project") {
        let target = match safe_link_target(".memory/project") {
            Some(t) => t,
            None => exit_with(1),
        };
        if !target.starts_with(&format!("{ws}/projects/")) {
            die!(".memory/project is bound outside workspace '{workspace}'");
        }
    }
    let here = pwd();
    let proj;
    if !name_given && is_link(".memory/project") {
        proj = match safe_link_target(".memory/project") {
            Some(p) => p,
            None => die!("unsafe symlink: .memory/project"),
        };
        if !proj.starts_with(&format!("{ws}/projects/")) {
            die!(".memory/project is bound to {proj}, outside workspace '{workspace}'; unlink/rebind it explicitly before initializing '{workspace}'");
        }
    } else {
        if name.is_empty() {
            name = basename(&here);
        }
        require_slug("project name", &name);
        proj = format!("{ws}/projects/{name}");
        validate_store_destination(&format!("{proj}/.project-path"));
        validate_store_destination(&format!("{proj}/INDEX.md"));
        if is_file(&format!("{proj}/.project-path")) {
            let recorded = read_trim(&format!("{proj}/.project-path"));
            if !recorded.is_empty() && is_dir(&recorded) {
                let rc = canon_dir(&recorded).unwrap_or_default();
                if rc != here {
                    die!("project name '{name}' is taken by {recorded}; re-run with --name");
                }
            }
        }
    }
    validate_store_destination(&format!("{ws}/workspace/INDEX.md"));
    validate_store_destination(&format!("{ws}/groups"));
    validate_store_destination(&format!("{ws}/projects"));
    validate_store_destination(&format!("{proj}/INDEX.md"));
    validate_store_destination(&format!("{proj}/.project-path"));
    if !is_dir(&format!("{store}/.git")) {
        let rc = run(Command::new("git")
            .arg("-C")
            .arg(&store)
            .args(["init", "-q"]));
        if rc != 0 {
            exit_with(rc);
        }
    }
    let exclude = format!("{store}/.git/info/exclude");
    if is_dir(&format!("{store}/.git")) {
        mkdir_p(&format!("{store}/.git/info"));
        if is_file(&exclude) {
            let c = read_string(&exclude).unwrap_or_default();
            let kept: Vec<&str> = split_lines(&c)
                .into_iter()
                .filter(|l| *l != "memory-dream.*")
                .collect();
            let joined = kept.join("\n");
            let joined = joined.trim_end_matches('\n');
            write_file(&exclude, &format!("{joined}\n"));
        }
        let pat = ".memory-index.*";
        let has = is_file(&exclude)
            && read_string(&exclude)
                .unwrap_or_default()
                .split('\n')
                .any(|l| l == pat);
        if !has {
            let existing = fs::read(&exclude).unwrap_or_default();
            let mut add = String::new();
            if !existing.is_empty() && *existing.last().unwrap() != b'\n' {
                add.push('\n');
            }
            add.push_str(pat);
            add.push('\n');
            match fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&exclude)
            {
                Ok(mut h) => {
                    let _ = h.write_all(add.as_bytes());
                }
                Err(e) => die!("cannot write {exclude}: {e}"),
            }
        }
    }
    mkdir_p(&format!("{ws}/workspace"));
    mkdir_p(&format!("{ws}/groups"));
    mkdir_p(&format!("{ws}/projects"));
    if !is_file(&format!("{ws}/workspace/INDEX.md")) {
        touch_empty(&format!("{ws}/workspace/INDEX.md"));
    }
    mkdir_p(&proj);
    if !is_file(&format!("{proj}/INDEX.md")) {
        touch_empty(&format!("{proj}/INDEX.md"));
    }
    write_file(&format!("{proj}/.project-path"), &format!("{here}\n"));

    mkdir_p(".memory/groups");
    symlink_force(&proj, ".memory/project");
    symlink_force(&format!("{ws}/workspace"), ".memory/workspace");

    for g in glob_dir(".memory/groups", |_, _| true) {
        if !is_link(&g) {
            continue;
        }
        let target = match safe_link_target(&g) {
            Some(t) => t,
            None => die!("unsafe symlink: {g} must target an absolute path inside {store}"),
        };
        if !exists(&target) {
            mkdir_p(&target);
            touch_empty(&format!("{target}/INDEX.md"));
        }
    }
    ensure_global_gitignore();
    outln!("initialized: project memory at {proj}");
    0
}

pub fn link_group(args: &[String]) -> i32 {
    if args.len() != 1 {
        die!("usage: memory link-group <group>");
    }
    let group = &args[0];
    require_slug("group name", group);
    if !is_link(".memory/workspace") {
        die!("not initialized; run: memory init <workspace>");
    }
    let mh = memory_home();
    if !is_dir(&mh) {
        die!("not initialized; run: memory init <workspace>");
    }
    set_store_from_home();
    let store = crate::ctx::store();
    let ws_dir = match safe_link_target(".memory/workspace") {
        Some(w) => w,
        None => {
            die!("unsafe symlink: .memory/workspace must target an absolute path inside {store}")
        }
    };
    if basename(&ws_dir) != "workspace" || dirname(&dirname(&ws_dir)) != store {
        die!("invalid workspace symlink: .memory/workspace");
    }
    let ws = dirname(&ws_dir);
    let gdir = format!("{ws}/groups/{group}");
    let link = format!(".memory/groups/{group}");
    if exists_or_link(&link) {
        if !is_link(&link) {
            die!("{link} exists but is not a symlink");
        }
        let existing = match safe_link_target(&link) {
            Some(e) => e,
            None => die!("unsafe symlink: {link}"),
        };
        if existing != gdir {
            die!("{link} is bound to a different group target");
        }
    }
    validate_store_destination(&format!("{gdir}/INDEX.md"));
    mkdir_p(&gdir);
    if !is_file(&format!("{gdir}/INDEX.md")) {
        touch_empty(&format!("{gdir}/INDEX.md"));
    }
    mkdir_p(".memory/groups");
    symlink_force(&gdir, &link);
    outln!("linked group '{group}'");
    0
}

pub fn unlink_group(args: &[String]) -> i32 {
    if args.len() != 1 {
        die!("usage: memory unlink-group <group>");
    }
    let group = &args[0];
    require_slug("group name", group);
    let link = format!(".memory/groups/{group}");
    if !is_link(&link) {
        die!("group '{group}' is not linked");
    }
    if fs::remove_file(&link).is_err() {
        exit_with(1);
    }
    outln!("unlinked group '{group}' (its memory stays in the store)");
    0
}

// ------------------------------------------------------------------ index

fn rand_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ ((std::process::id() as u64) << 32)
        ^ CTR.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::SeqCst);
    let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut s = String::new();
    for _ in 0..6 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.push(chars[(x % chars.len() as u64) as usize] as char);
    }
    s
}

fn mktemp_in(level: &str) -> Option<String> {
    for _ in 0..100 {
        let p = format!("{level}/.memory-index.{}", rand_suffix());
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&p)
        {
            Ok(_) => return Some(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Rebuilds INDEX.md for each (level, label) pair: render all, then move into place.
pub fn publish_indexes(levels: &[String], labels: &[String]) {
    st(|s| s.index_temps.clear());
    let own_handlers = st(|s| s.lock_file.is_none());
    if own_handlers {
        install_signal_handlers();
    }
    let mut temps: Vec<String> = vec![];
    for (level, label) in levels.iter().zip(labels) {
        if let Ok(rd) = fs::read_dir(level) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with(".memory-index.") {
                    if let Ok(m) = e.metadata() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        if m.is_file() && now - m.mtime() > 600 {
                            let _ = fs::remove_file(e.path());
                        }
                    }
                }
            }
        }
        let temp = match mktemp_in(level) {
            Some(t) => t,
            None => die!("cannot create temporary index in {level}"),
        };
        st(|s| s.index_temps.push(temp.clone()));
        let (content, warns) = render_index(level, label);
        for w in warns {
            eprintln!("{w}");
        }
        if fs::write(&temp, content).is_err() {
            die!("failed to render index for {label}");
        }
        temps.push(temp);
        check_signal();
    }
    for ((level, label), temp) in levels.iter().zip(labels).zip(&temps) {
        if fs::rename(temp, format!("{level}/INDEX.md")).is_err() {
            die!("failed to publish index for {label}");
        }
        let n = count_index_entries(&format!("{level}/INDEX.md"));
        outln!("indexed {label} ({n} facts)");
    }
    st(|s| s.index_temps.clear());
    if own_handlers {
        reset_signal_handlers();
    }
}

pub fn index(args: &[String]) -> i32 {
    if !args.is_empty() {
        die!("usage: memory index");
    }
    if !is_dir(".memory") {
        die!("no .memory/ here (run 'memory init <workspace>' first)");
    }
    let mh = memory_home();
    if !is_dir(&mh) {
        die!("no store at {mh} (run 'memory init' first)");
    }
    set_store_from_home();
    if !exists_or_link(".memory/project") && !exists_or_link(".memory/workspace") {
        die!("no usable linked levels");
    }
    let mut levels = vec![];
    let mut labels = vec![];
    for link in [".memory/project", ".memory/workspace"] {
        let target = match safe_link_target(link) {
            Some(t) => t,
            None => die!("invalid required memory link: {link}"),
        };
        if !is_dir(&target) {
            die!("broken required memory link: {link}");
        }
        levels.push(target);
        labels.push(link.to_string());
    }
    for link in glob_dir(".memory/groups", |_, _| true) {
        if !exists_or_link(&link) {
            continue;
        }
        let target = match safe_link_target(&link) {
            Some(t) => t,
            None => die!("invalid group memory link: {link}"),
        };
        if !is_dir(&target) {
            die!("broken group memory link: {link}");
        }
        levels.push(target);
        labels.push(link);
    }
    for (l, lab) in levels.iter().zip(&labels) {
        if is_dir(&format!("{l}/INDEX.md")) {
            die!("invalid index destination: {lab}/INDEX.md is a directory");
        }
    }
    publish_indexes(&levels, &labels);
    0
}

// ------------------------------------------------------------------ doctor

fn doctor_project_hints() -> Vec<String> {
    let mut out = vec![];
    let store = crate::ctx::store();
    for ws in glob_subdirs(&store) {
        for p in glob_subdirs(&format!("{ws}/projects")) {
            let pp = if is_file(&format!("{p}/.project-path")) {
                read_trim(&format!("{p}/.project-path"))
            } else {
                String::new()
            };
            if pp.is_empty() || !is_dir(&pp) {
                continue;
            }
            if let Some(linked) = safe_link_target(&format!("{pp}/.memory/project")) {
                if !linked.is_empty() && linked != p {
                    out.push(format!(
                        "PROJECT PATH HINT (possible collision): {p} records {pp}, which links elsewhere"
                    ));
                }
            }
        }
    }
    out
}

/// Returns Err(problem line) when the link is unusable.
pub fn check_link(link: &str) -> Result<(), String> {
    if !exists_or_link(link) {
        return Err(format!("MISSING LINK: {link}"));
    }
    if !is_link(link) {
        return Err(format!("NOT A SYMLINK: {link}"));
    }
    let target = readlink(link).unwrap_or_default();
    if !target.starts_with('/') {
        return Err(format!(
            "UNSAFE SYMLINK: {link} -> {target} (relative target)"
        ));
    }
    let resolved = match canonical_entry(&target) {
        Some(r) => r,
        None => {
            return Err(format!(
                "BROKEN: {link} -> {target} (target directory does not exist)"
            ))
        }
    };
    if !is_in_store(&resolved) {
        return Err(format!(
            "UNSAFE SYMLINK: {link} -> {target} (outside {})",
            crate::ctx::store()
        ));
    }
    if !exists(link) {
        return Err(format!("BROKEN: {link} -> {target}"));
    }
    Ok(())
}

fn wiki_target_exists(slug: &str) -> bool {
    let mut levels = vec![
        ".memory/project".to_string(),
        ".memory/workspace".to_string(),
    ];
    levels.extend(glob_dir(".memory/groups", |_, _| true));
    levels.iter().any(|l| is_file(&format!("{l}/{slug}.md")))
}

fn doctor_warnings() -> Vec<String> {
    let mut out = vec![];
    if !is_dir(".memory") || !is_dir(&crate::ctx::store()) {
        return out;
    }
    let mut levels = vec![
        ".memory/project".to_string(),
        ".memory/workspace".to_string(),
    ];
    levels.extend(glob_dir(".memory/groups", |_, _| true));
    for level in levels {
        if !is_link(&level) || !is_dir(&level) || !is_file(&format!("{level}/INDEX.md")) {
            continue;
        }
        for f in glob_md(&level) {
            let base = basename(&f);
            if base == "INDEX.md" {
                continue;
            }
            let desc = frontmatter_value(&f, "description");
            let n = desc.chars().count();
            if n > 120 {
                out.push(format!(
                    "LONG DESCRIPTION ({n} chars > 120): {level}/{base}"
                ));
            }
            let listed = index_description(&format!("{level}/INDEX.md"), &base);
            if !desc.is_empty() && !listed.is_empty() && listed != desc {
                out.push(format!(
                    "INDEX DESCRIPTION DRIFT: {level}/{base} (index says '{listed}'; run 'memory index')"
                ));
            }
            for slug in wiki_links(&f) {
                if !wiki_target_exists(&slug) {
                    out.push(format!("DANGLING WIKI LINK: {level}/{base} -> [[{slug}]]"));
                }
            }
        }
    }
    out
}

fn doctor_problems() -> Vec<String> {
    let mut out = vec![];
    if !is_dir(".memory") {
        return out;
    }
    let store = crate::ctx::store();
    if !is_dir(&store) {
        out.push(format!(
            "STORE MISSING: {store} (restore it or set AGENT_MEMORY_HOME)"
        ));
        return out;
    }
    let mut levels = vec![];
    for link in [".memory/project", ".memory/workspace"] {
        match check_link(link) {
            Ok(()) => levels.push(link.to_string()),
            Err(e) => out.push(e),
        }
    }
    for link in glob_dir(".memory/groups", |_, _| true) {
        if !is_link(&link) {
            continue;
        }
        match check_link(&link) {
            Ok(()) => levels.push(link),
            Err(e) => out.push(e),
        }
    }
    if levels.is_empty() {
        return out;
    }
    if let (Some(pt), Some(wt)) = (
        safe_link_target(".memory/project"),
        safe_link_target(".memory/workspace"),
    ) {
        if dirname(&dirname(&pt)) != dirname(&wt) {
            out.push(format!(
                "WORKSPACE MISMATCH: .memory/project belongs to workspace '{}' but .memory/workspace to '{}'",
                basename(&dirname(&dirname(&pt))),
                basename(&dirname(&wt))
            ));
        }
    }
    for level in levels {
        let idx = format!("{level}/INDEX.md");
        if !is_file(&idx) {
            out.push(format!("MISSING INDEX: {idx}"));
            continue;
        }
        out.extend(check_index(&idx));
        let listed_files = index_files(&idx);
        for f in glob_md(&level) {
            let base = basename(&f);
            if base == "INDEX.md" {
                continue;
            }
            if !listed_files.contains(&base) {
                out.push(format!("NOT IN INDEX: {level}/{base}"));
            }
            out.extend(check_frontmatter(&f));
        }
        for name in listed_files {
            if !is_file(&format!("{level}/{name}")) {
                out.push(format!("MISSING FILE: {level}/{name} (listed in INDEX.md)"));
            }
        }
    }
    out
}

pub fn doctor(args: &[String]) -> i32 {
    if !args.is_empty() {
        die!("usage: memory doctor");
    }
    if !is_dir(".memory") {
        outln!(
            "no .memory/ in {}; nothing to check (run 'memory init <workspace>' to set up memory)",
            pwd()
        );
        return 0;
    }
    let mh = memory_home();
    let s = if is_dir(&mh) {
        canon_dir(&mh).unwrap_or_else(|| mh.clone())
    } else {
        mh
    };
    st(|x| x.store_home = s);
    let problems = doctor_problems();
    let hints = doctor_project_hints();
    let warnings = doctor_warnings();
    if !hints.is_empty() {
        outln!("project-path hints (not orphan detection):");
        outln!("{}", hints.join("\n"));
    }
    if !warnings.is_empty() {
        outln!("warnings (not failures):");
        outln!("{}", warnings.join("\n"));
    }
    if !problems.is_empty() {
        outln!("{}", problems.join("\n"));
        die!("problems found");
    }
    outln!("OK");
    0
}

// ------------------------------------------------------------------ status

fn count_facts(dir: &str) -> usize {
    glob_md(dir)
        .iter()
        .filter(|f| basename(f) != "INDEX.md")
        .count()
}

pub fn status(args: &[String]) -> i32 {
    if !args.is_empty() {
        die!("usage: memory status");
    }
    if !is_dir(".memory") {
        die!("no .memory/ here (run 'memory init <workspace>' first)");
    }
    let mh = memory_home();
    if !is_dir(&mh) {
        die!("store missing: {mh} (set AGENT_MEMORY_HOME or restore it)");
    }
    set_store_from_home();
    let store = crate::ctx::store();
    let broken = |l: &str| -> ! { die!("unsafe or broken symlink: {l} (run 'memory doctor')") };
    let ws_dir = match safe_link_target(".memory/workspace") {
        Some(w) if is_dir(&w) => w,
        _ => broken(".memory/workspace"),
    };
    let proj = match safe_link_target(".memory/project") {
        Some(p) if is_dir(&p) => p,
        _ => broken(".memory/project"),
    };
    let ws = dirname(&ws_dir);
    outln!("store:      {store}");
    outln!(
        "workspace:  {}  ({} facts)",
        basename(&ws),
        count_facts(&ws_dir)
    );
    outln!(
        "project:    {}  ({} facts)  -> {proj}",
        basename(&proj),
        count_facts(&proj)
    );
    let mut linked = " ".to_string();
    for g in glob_dir(".memory/groups", |_, _| true) {
        if !is_link(&g) {
            continue;
        }
        let name = basename(&g);
        match safe_link_target(&g) {
            Some(t) if is_dir(&t) => {
                outln!("group:      {name}  ({} facts)", count_facts(&t));
                linked.push_str(&format!("{name} "));
            }
            _ => outln!("group:      {name}  (UNSAFE OR BROKEN LINK — run 'memory doctor')"),
        }
    }
    let mut other = String::new();
    for g in glob_subdirs(&format!("{ws}/groups")) {
        let name = basename(&g);
        if !linked.contains(&format!(" {name} ")) {
            other.push_str(&format!(" {name}"));
        }
    }
    if !other.is_empty() {
        outln!("not linked: {}  (memory link-group <group>)", &other[1..]);
    }
    let top = cap_trim(&mut git_plain(&["rev-parse", "--show-toplevel"])).unwrap_or_default();
    let is_top = !top.is_empty() && canon_dir(&top).is_some_and(|t| t == store);
    if is_top {
        let (_, o) = capture(
            &mut git_plain(&["status", "--porcelain", "--untracked-files=all"]),
            true,
        );
        let uncommitted = o.split('\n').filter(|l| !l.is_empty()).count();
        let last = cap_trim(&mut git_plain(&[
            "log",
            "-1",
            "--format=%cd",
            "--date=short",
        ]))
        .unwrap_or_else(|| "never".to_string());
        if uncommitted == 0 {
            outln!("backup:     last {last}, store clean");
        } else {
            outln!(
                "backup:     last {last}, {uncommitted} uncommitted path(s) — run 'memory backup'"
            );
        }
    } else {
        outln!("backup:     unavailable (store is not a git repository)");
    }
    0
}

// ------------------------------------------------------------------ grep

pub fn grep(args: &[String]) -> i32 {
    if args.is_empty() {
        die!("usage: memory grep <pattern> [extra grep options]");
    }
    if !is_dir(".memory") {
        die!("no .memory/ here (run 'memory init <workspace>' first)");
    }
    let mh = memory_home();
    if !is_dir(&mh) {
        die!("store missing: {mh} (set AGENT_MEMORY_HOME or restore it)");
    }
    set_store_from_home();
    let pattern = &args[0];
    let mut dirs = vec![];
    let mut cands = vec![
        ".memory/project".to_string(),
        ".memory/workspace".to_string(),
    ];
    for d in glob_dir(".memory/groups", |_, _| true) {
        if is_link(&d) {
            cands.push(d);
        }
    }
    for d in cands {
        match safe_link_target(&d) {
            Some(t) if is_dir(&t) => dirs.push(format!("{d}/")),
            _ => die!("unsafe or broken symlink: {d} (run 'memory doctor')"),
        }
    }
    let mut c = Command::new("grep");
    c.args(["-r", "-n", "-i", "--exclude=.project-path"]);
    c.args(&args[1..]);
    c.arg("--").arg(pattern).args(&dirs);
    let rc = run(&mut c);
    match rc {
        0 => {}
        1 => outln!("no matches for '{pattern}'"),
        _ => die!("grep failed (exit {rc})"),
    }
    0
}

// ------------------------------------------------------------------ install

pub fn install_opencode(args: &[String]) -> i32 {
    if !args.is_empty() {
        die!("usage: memory install-opencode");
    }
    eprintln!(
        "memory: install-opencode is deprecated; prefer `opencode plugin add github:dot-opencode/memory#release`"
    );
    let skill = match crate::skill_dir() {
        Some(s) => s,
        None => die!("cannot resolve CLI path"),
    };
    let source = format!("{skill}/opencode/agent-memory.js");
    if !is_file(&source) {
        die!("OpenCode plugin not found at {source}");
    }
    let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_default();
    let base = if xdg.is_empty() {
        format!("{}/.config", std::env::var("HOME").unwrap_or_default())
    } else {
        xdg
    };
    let config_dir = format!("{base}/opencode/plugins");
    let dest = format!("{config_dir}/agent-memory.js");
    mkdir_p(&config_dir);
    if is_link(&dest) {
        let current = canonical_path(&dest).unwrap_or_default();
        if current == source {
            outln!("OpenCode plugin already installed: {dest}");
            return 0;
        }
        if fs::remove_file(&dest).is_err() {
            exit_with(1);
        }
    } else if exists(&dest) {
        die!("refusing to overwrite non-symlink: {dest}");
    }
    if std::os::unix::fs::symlink(&source, &dest).is_err() {
        exit_with(1);
    }
    outln!("installed OpenCode plugin: {dest} (restart OpenCode to load it)");
    0
}
