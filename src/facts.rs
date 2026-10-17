//! Fact files: frontmatter, INDEX.md rendering/parsing and date helpers.

use crate::fsutil::*;

pub fn split_lines(s: &str) -> Vec<&str> {
    let mut v: Vec<&str> = s.split('\n').collect();
    if v.last() == Some(&"") {
        v.pop();
    }
    v
}

/// Value of one flat frontmatter key; empty when the key or frontmatter is absent.
pub fn frontmatter_value(path: &str, key: &str) -> String {
    let content = read_string(path).unwrap_or_default();
    let lines = split_lines(&content);
    if lines.is_empty() || lines[0] != "---" {
        return String::new();
    }
    let prefix = format!("{key}: ");
    for l in &lines[1..] {
        if *l == "---" {
            return String::new();
        }
        if let Some(rest) = l.strip_prefix(prefix.as_str()) {
            return rest.to_string();
        }
    }
    String::new()
}

pub fn valid_calendar_date(date: &str) -> bool {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return false;
    }
    let y: i64 = date[0..4].parse().unwrap_or(0);
    let m: i64 = date[5..7].parse().unwrap_or(0);
    let d: i64 = date[8..10].parse().unwrap_or(0);
    let max = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    d >= 1 && d <= max
}

/// Days-from-civil (Howard Hinnant). Caller validated the date.
pub fn days_from_civil(date: &str) -> i64 {
    let y0: i64 = date[0..4].parse().unwrap_or(0);
    let m: i64 = date[5..7].parse().unwrap_or(1);
    let d: i64 = date[8..10].parse().unwrap_or(1);
    let y = if m <= 2 { y0 - 1 } else { y0 };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn is_date_line(prefix: &str, line: &str) -> bool {
    match line.strip_prefix(prefix) {
        Some(r) => {
            let b = r.as_bytes();
            b.len() == 10
                && b.iter().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        *c == b'-'
                    } else {
                        c.is_ascii_digit()
                    }
                })
        }
        None => false,
    }
}

pub const TYPES: [&str; 5] = ["gotcha", "decision", "convention", "context", "reference"];

/// Problems with a fact's frontmatter, one line each (empty when fine).
pub fn check_frontmatter(f: &str) -> Vec<String> {
    let mut out = vec![];
    let content = read_string(f).unwrap_or_default();
    let first = content.split('\n').next().unwrap_or("");
    if first.ends_with('\r') {
        out.push(format!("BAD FRONTMATTER (CRLF line endings): {f}"));
        return out;
    }
    if first != "---" {
        out.push(format!("BAD FRONTMATTER (no opening ---): {f}"));
        return out;
    }
    let lines = split_lines(&content);
    let mut front: Vec<&str> = vec![];
    let mut found = false;
    for l in &lines[1..] {
        if l.ends_with('\r') {
            out.push(format!("BAD FRONTMATTER (CRLF line endings): {f}"));
            return out;
        }
        if *l == "---" {
            found = true;
            break;
        }
        front.push(l);
    }
    if !found {
        out.push(format!("BAD FRONTMATTER (no closing ---): {f}"));
        return out;
    }
    let count = |p: &str| front.iter().filter(|l| l.starts_with(p)).count();
    let created = frontmatter_value(f, "created");
    if !(count("description:") == 1
        && front.iter().any(|l| {
            l.strip_prefix("description: ")
                .is_some_and(|r| !r.is_empty())
        }))
    {
        out.push(format!(
            "BAD FRONTMATTER (missing or duplicate description): {f}"
        ));
    }
    if !(count("type:") == 1
        && front
            .iter()
            .any(|l| l.strip_prefix("type: ").is_some_and(|t| TYPES.contains(&t))))
    {
        out.push(format!("BAD FRONTMATTER (missing or invalid type): {f}"));
    }
    if !(count("created:") == 1
        && front.iter().any(|l| is_date_line("created: ", l))
        && valid_calendar_date(&created))
    {
        out.push(format!(
            "BAD FRONTMATTER (missing or invalid created date): {f}"
        ));
    }
    let uc = count("updated:");
    if uc > 0 {
        let updated = frontmatter_value(f, "updated");
        if uc != 1
            || !front.iter().any(|l| is_date_line("updated: ", l))
            || !valid_calendar_date(&updated)
        {
            out.push(format!("BAD FRONTMATTER (invalid updated date): {f}"));
            return out;
        }
        if !created.is_empty() && updated.as_str() < created.as_str() {
            out.push(format!(
                "BAD FRONTMATTER (updated {updated} is before created {created}): {f}"
            ));
        }
    }
    out
}

pub fn fact_is_usable(f: &str) -> Result<(), String> {
    let base = basename(f);
    let stem = base.strip_suffix(".md").unwrap_or(&base);
    if !valid_slug(stem) {
        return Err("invalid fact slug".to_string());
    }
    let p = check_frontmatter(f);
    if p.is_empty() {
        Ok(())
    } else {
        Err(p.join("\n"))
    }
}

fn slug_len(b: &[u8]) -> usize {
    if b.is_empty() || !(b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) {
        return 0;
    }
    b.iter()
        .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || **c == b'-')
        .count()
}

/// Parses one INDEX.md entry; returns (label, file).
pub fn parse_entry(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("- [")?;
    let n = slug_len(rest.as_bytes());
    if n == 0 {
        return None;
    }
    let label = &rest[..n];
    let rest = rest[n..].strip_prefix("](")?;
    let n2 = slug_len(rest.as_bytes());
    if n2 == 0 {
        return None;
    }
    let stem = &rest[..n2];
    let rest = rest[n2..].strip_prefix(".md) ")?;
    let rest = ["—", "–", "-"].iter().find_map(|d| rest.strip_prefix(*d))?;
    let rest = rest.strip_prefix(' ')?;
    let ok = TYPES.iter().any(|t| {
        rest.strip_prefix(*t)
            .and_then(|r| r.strip_prefix(": "))
            .is_some_and(|r| !r.is_empty())
    });
    if !ok {
        return None;
    }
    Some((label.to_string(), format!("{stem}.md")))
}

pub fn check_index(index_file: &str) -> Vec<String> {
    let mut out = vec![];
    let content = read_string(index_file).unwrap_or_default();
    let mut seen: Vec<String> = vec![];
    for (i, line) in split_lines(&content).into_iter().enumerate() {
        let nr = i + 1;
        if line.chars().all(|c| c.is_ascii_whitespace() || c == '\x0b') {
            continue;
        }
        match parse_entry(line) {
            None => out.push(format!("MALFORMED INDEX ENTRY: {index_file}:{nr}: {line}")),
            Some((label, file)) => {
                let stem = file.strip_suffix(".md").unwrap_or(&file);
                if label != stem {
                    out.push(format!(
                        "MALFORMED INDEX ENTRY: {index_file}:{nr}: label does not match {file}"
                    ));
                    continue;
                }
                if seen.contains(&file) {
                    out.push(format!("DUPLICATE INDEX ENTRY: {index_file}: {file}"));
                } else {
                    seen.push(file);
                }
            }
        }
    }
    out
}

pub fn index_files(index_file: &str) -> Vec<String> {
    let content = read_string(index_file).unwrap_or_default();
    split_lines(&content)
        .into_iter()
        .filter_map(|l| parse_entry(l).map(|(_, f)| f))
        .collect()
}

/// Description text of the INDEX.md entry for one file.
pub fn index_description(index_file: &str, want: &str) -> String {
    let content = read_string(index_file).unwrap_or_default();
    for l in split_lines(&content) {
        if let Some((_, f)) = parse_entry(l) {
            if f == want {
                return match l.find(": ") {
                    Some(i) => l[i + 2..].to_string(),
                    None => String::new(),
                };
            }
        }
    }
    String::new()
}

/// INDEX.md content for one level plus warnings for skipped facts.
pub fn render_index(level: &str, display: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut warns = vec![];
    for f in glob_md(level) {
        let base = basename(&f);
        if base == "INDEX.md" || is_link(&f) {
            continue;
        }
        if let Err(reason) = fact_is_usable(&f) {
            warns.push(format!("memory: skipping {display}/{base}: {reason}"));
            continue;
        }
        let typ = frontmatter_value(&f, "type");
        let desc = frontmatter_value(&f, "description");
        let stem = base.strip_suffix(".md").unwrap_or(&base);
        out.push_str(&format!("- [{stem}]({base}) — {typ}: {desc}\n"));
    }
    (out, warns)
}

/// Unique, bytewise-sorted `[[slug]]` targets in a file (non-empty slugs).
pub fn wiki_links(path: &str) -> Vec<String> {
    let b = std::fs::read(path).unwrap_or_default();
    let mut out: Vec<String> = vec![];
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'[' && b[i + 1] == b'[' {
            let mut j = i + 2;
            while j < b.len()
                && (b[j].is_ascii_lowercase() || b[j].is_ascii_digit() || b[j] == b'-')
            {
                j += 1;
            }
            if j + 1 < b.len() && b[j] == b']' && b[j + 1] == b']' {
                if j > i + 2 {
                    out.push(String::from_utf8_lossy(&b[i + 2..j]).into_owned());
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out.sort();
    out.dedup();
    out
}

pub fn count_index_entries(path: &str) -> usize {
    read_string(path)
        .unwrap_or_default()
        .split('\n')
        .filter(|l| l.starts_with("- ["))
        .count()
}
