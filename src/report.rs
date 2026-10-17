//! `memory dream --report`: read-only analysis of the whole store.

use crate::ctx::*;
use crate::facts::*;
use crate::fsutil::*;
use crate::{die, outln};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::process::Command;

const STOPWORDS: &str =
    "the and for with that this from are was not use uses used when into only via per its has have all any but can must should does never always";

/// Every level of the store (optionally one workspace): (canonical path, store-relative name).
pub fn collect_levels(only: &str) -> Vec<(String, String)> {
    let store = crate::ctx::store();
    let mut out = vec![];
    for ws in glob_subdirs(&store) {
        let wsname = basename(&ws);
        if !valid_slug(&wsname) {
            continue;
        }
        if !only.is_empty() && only != wsname {
            continue;
        }
        let mut cands = vec![format!("{ws}/workspace")];
        cands.extend(glob_subdirs(&format!("{ws}/groups")));
        cands.extend(glob_subdirs(&format!("{ws}/projects")));
        for d in cands {
            if !is_dir(&d) || !valid_slug(&basename(&d)) {
                continue;
            }
            let canon = match canon_dir(&d) {
                Some(c) => c,
                None => continue,
            };
            if !is_in_store(&canon) {
                continue;
            }
            let rel = d
                .strip_prefix(&format!("{store}/"))
                .unwrap_or(&d)
                .to_string();
            out.push((canon, rel));
        }
    }
    out
}

fn fact_path(rel: &str, file: &str, code: &str) -> String {
    if code.is_empty() {
        format!("{rel}/{file}")
    } else {
        format!("{rel}/{file} [code: {code}]")
    }
}

fn wiki_exists(ws: &str, slug: &str) -> bool {
    let w = format!("{}/{ws}", crate::ctx::store());
    if is_file(&format!("{w}/workspace/{slug}.md")) {
        return true;
    }
    for kind in ["groups", "projects"] {
        for d in glob_subdirs(&format!("{w}/{kind}")) {
            if is_file(&format!("{d}/{slug}.md")) {
                return true;
            }
        }
    }
    false
}

fn orphan_reason(level: &str, code: &str) -> String {
    if code.is_empty() {
        return ".project-path missing".into();
    }
    if !is_dir(code) {
        return "recorded dir is gone".into();
    }
    let linked = safe_link_target(&format!("{code}/.memory/project")).unwrap_or_default();
    if linked != level {
        ".memory/project no longer links to this project dir".into()
    } else {
        String::new()
    }
}

struct Row {
    rel: String,
    ws: String,
    slug: String,
    typ: String,
    desc: String,
    date: String,
    code: String,
}

fn tokenize(slug: &str, desc: &str) -> Vec<Vec<u8>> {
    let text = format!("{slug} {desc}");
    let lower = text.to_ascii_lowercase();
    let stop: HashSet<&str> = STOPWORDS.split(' ').collect();
    let mut words: Vec<Vec<u8>> = vec![];
    let mut cur: Vec<u8> = vec![];
    let flush = |cur: &mut Vec<u8>, words: &mut Vec<Vec<u8>>| {
        if !cur.is_empty() {
            let w = std::mem::take(cur);
            if w.len() >= 3
                && !stop.contains(String::from_utf8_lossy(&w).as_ref())
                && !words.contains(&w)
            {
                words.push(w);
            }
        }
    };
    for &b in lower.as_bytes() {
        if b.is_ascii_lowercase() || b.is_ascii_digit() || b >= 0x80 {
            cur.push(b);
        } else {
            flush(&mut cur, &mut words);
        }
    }
    flush(&mut cur, &mut words);
    words
}

fn is_proj(rel: &str) -> bool {
    rel.split('/').nth(1) == Some("projects")
}

/// Returns (similar, repeated, shadowed) lines.
fn pairs(rows: &[Row]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let toks: Vec<Vec<Vec<u8>>> = rows.iter().map(|r| tokenize(&r.slug, &r.desc)).collect();
    let path = |r: &Row| fact_path(&r.rel, &format!("{}.md", r.slug), &r.code);
    let (mut sim, mut rep, mut shad) = (vec![], vec![], vec![]);
    for a in 0..rows.len() {
        for b in a + 1..rows.len() {
            if rows[a].ws != rows[b].ws {
                continue;
            }
            let mut shared: BTreeSet<&Vec<u8>> = BTreeSet::new();
            for wa in &toks[a] {
                if toks[b].contains(wa) {
                    shared.insert(wa);
                }
            }
            let n = shared.len();
            let minlen = toks[a].len().min(toks[b].len());
            let similar = n >= 2 && minlen > 0 && (n as f64 / minlen as f64) >= 0.6;
            let repeated =
                is_proj(&rows[a].rel) && is_proj(&rows[b].rel) && rows[a].rel != rows[b].rel;
            if repeated && rows[a].slug == rows[b].slug {
                continue;
            }
            if !similar {
                continue;
            }
            let list = shared
                .iter()
                .map(|w| String::from_utf8_lossy(w).into_owned())
                .collect::<Vec<_>>()
                .join(", ");
            let line = format!(
                "  {} <-> {} (shared: {list})",
                path(&rows[a]),
                path(&rows[b])
            );
            if repeated {
                rep.push(line);
            } else {
                sim.push(line);
            }
        }
    }
    // same slug in several levels
    struct Agg {
        count: usize,
        all_proj: bool,
        levels: String,
    }
    let mut keys: BTreeMap<(String, String), Agg> = BTreeMap::new();
    for r in rows {
        let lvl = r.rel.get(r.ws.len() + 1..).unwrap_or("").to_string();
        let e = keys.entry((r.ws.clone(), r.slug.clone())).or_insert(Agg {
            count: 0,
            all_proj: true,
            levels: String::new(),
        });
        if e.count == 0 {
            e.levels = lvl;
        } else {
            e.levels.push_str(&format!(", {lvl}"));
        }
        e.count += 1;
        if !is_proj(&r.rel) {
            e.all_proj = false;
        }
    }
    for ((ws, slug), a) in keys {
        if a.count < 2 {
            continue;
        }
        if a.all_proj {
            rep.push(format!("  {ws}: {slug} in {} (same slug)", a.levels));
        } else {
            shad.push(format!("  {ws}: {slug} in {}", a.levels));
        }
    }
    (sim, rep, shad)
}

fn emit(header: &str, body: &mut [String]) {
    if body.is_empty() {
        return;
    }
    outln!("{header}");
    body.sort();
    for l in body.iter() {
        outln!("{l}");
    }
}

pub fn today() -> String {
    let t = std::env::var("AGENT_MEMORY_TODAY").unwrap_or_default();
    if !t.is_empty() {
        if !valid_calendar_date(&t) {
            die!("invalid AGENT_MEMORY_TODAY: {t}");
        }
        return t;
    }
    cap_trim(Command::new("date").arg("+%Y-%m-%d")).unwrap_or_default()
}

pub fn report(stale_days: i64, only: &str) {
    let today = today();
    let today_days = days_from_civil(&today);
    let levels = collect_levels(only);
    let mut rows: Vec<Row> = vec![];
    let (mut unusable, mut stale, mut large, mut drift, mut hygiene) =
        (vec![], vec![], vec![], vec![], vec![]);
    for (level, rel) in &levels {
        let ws = rel.split('/').next().unwrap_or("").to_string();
        let mut code = String::new();
        if rel.contains("/projects/") {
            if is_file(&format!("{level}/.project-path")) {
                code = first_line(&format!("{level}/.project-path")).replace('\t', " ");
            }
            let reason = orphan_reason(level, &code);
            if !reason.is_empty() {
                hygiene.push(format!("  orphan project: {rel} ({reason})"));
            }
        }
        let idx = format!("{level}/INDEX.md");
        if is_file(&idx) {
            let n = count_index_entries(&idx);
            if n > 40 {
                large.push(format!("  {rel}/INDEX.md ({n} entries)"));
            }
            let (expected, _) = render_index(level, rel);
            if expected.trim_end_matches('\n') != read_trim(&idx) {
                drift.push(format!("  {rel}/INDEX.md (differs from the facts)"));
            }
        } else {
            drift.push(format!("  {rel}/INDEX.md (missing)"));
        }
        for f in glob_md_all(level) {
            let base = basename(&f);
            if base == "INDEX.md" {
                continue;
            }
            let slug = base.strip_suffix(".md").unwrap_or(&base).to_string();
            let fpath = fact_path(rel, &base, &code);
            if !is_file(&f) {
                unusable.push(format!("  {fpath} — not a regular file"));
                continue;
            }
            if is_link(&f) {
                hygiene.push(format!("  symlinked fact ignored: {fpath}"));
                continue;
            }
            if let Err(reason) = fact_is_usable(&f) {
                let suffix = format!(": {f}");
                let parts: Vec<String> = reason
                    .split('\n')
                    .map(|l| l.strip_suffix(suffix.as_str()).unwrap_or(l).to_string())
                    .collect();
                unusable.push(format!("  {fpath} — {}", parts.join("; ")));
                continue;
            }
            let typ = frontmatter_value(&f, "type");
            let desc = frontmatter_value(&f, "description").replace('\t', " ");
            let created = frontmatter_value(&f, "created");
            let updated = frontmatter_value(&f, "updated");
            let date = if updated.is_empty() { created } else { updated };
            let age = today_days - days_from_civil(&date);
            if age > stale_days {
                stale.push(format!("  {fpath} — {typ}, {age}d since {date}"));
            }
            let dl = desc.chars().count();
            if dl > 120 {
                hygiene.push(format!("  long description ({dl} chars): {fpath}"));
            }
            for w in wiki_links(&f) {
                if !wiki_exists(&ws, &w) {
                    hygiene.push(format!("  dangling link: {fpath} -> [[{w}]]"));
                }
            }
            rows.push(Row {
                rel: rel.clone(),
                ws: ws.clone(),
                slug,
                typ,
                desc,
                date,
                code: code.clone(),
            });
        }
    }
    let (mut similar, mut repeated, mut shadowed) = (vec![], vec![], vec![]);
    if !rows.is_empty() {
        // Pair analysis runs over rows sorted like `LC_ALL=C sort` of the TSV lines.
        rows.sort_by_key(|r| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.rel, r.ws, r.slug, r.typ, r.desc, r.date, r.code
            )
        });
        let (s, r, h) = pairs(&rows);
        similar = s;
        repeated = r;
        shadowed = h;
    }
    emit("unusable facts (fix frontmatter):", &mut unusable.clone());
    emit(
        "similar facts (merge, or check for contradiction):",
        &mut similar.clone(),
    );
    emit(
        "repeated across projects (move shared part to a group or workspace):",
        &mut repeated.clone(),
    );
    emit(
        "same slug in several levels (shadowing):",
        &mut shadowed.clone(),
    );
    emit(
        &format!(
            "stale facts (older than {stale_days} days — verify, update 'updated:', or delete):"
        ),
        &mut stale.clone(),
    );
    emit("large indexes (> 40 entries — merge or prune):", &mut large);
    emit(
        "index drift (run 'memory dream' or 'memory index'):",
        &mut drift,
    );
    emit("hygiene:", &mut hygiene);
    let (a, b, c, d, e) = (
        similar.len(),
        repeated.len(),
        shadowed.len(),
        stale.len(),
        unusable.len(),
    );
    if a + b + c + d + e == 0 {
        outln!("dream: memory is tidy — nothing to consolidate");
    } else {
        outln!("dream: {a} similar, {b} repeated across projects, {c} shadowed, {d} stale, {e} unusable");
    }
}
