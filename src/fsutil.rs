//! Path and filesystem helpers mirroring the shell semantics the script relies on.

use std::fs;

pub fn is_dir(p: &str) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}
pub fn is_file(p: &str) -> bool {
    fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}
pub fn exists(p: &str) -> bool {
    fs::metadata(p).is_ok()
}
pub fn is_link(p: &str) -> bool {
    fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}
pub fn exists_or_link(p: &str) -> bool {
    fs::symlink_metadata(p).is_ok()
}

pub fn dirname(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.starts_with('/') {
            "/".into()
        } else {
            ".".into()
        };
    }
    match t.rfind('/') {
        None => ".".into(),
        Some(i) => {
            let d = t[..i].trim_end_matches('/');
            if d.is_empty() {
                "/".into()
            } else {
                d.into()
            }
        }
    }
}

pub fn basename(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.starts_with('/') {
            "/".into()
        } else {
            String::new()
        };
    }
    match t.rfind('/') {
        None => t.into(),
        Some(i) => t[i + 1..].into(),
    }
}

/// `cd -P dir && pwd -P`
pub fn canon_dir(p: &str) -> Option<String> {
    if !is_dir(p) {
        return None;
    }
    fs::canonicalize(p)
        .ok()
        .map(|x| x.to_string_lossy().into_owned())
}

pub fn pwd() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| fs::canonicalize(p).ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn readlink(p: &str) -> Option<String> {
    fs::read_link(p)
        .ok()
        .map(|x| x.to_string_lossy().into_owned())
}

pub fn read_string(p: &str) -> Option<String> {
    fs::read(p)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// `$(<file)`: whole content with trailing newlines removed.
pub fn read_trim(p: &str) -> String {
    read_string(p)
        .unwrap_or_default()
        .trim_end_matches('\n')
        .to_string()
}

/// First line without its newline (`IFS= read -r x < file`).
pub fn first_line(p: &str) -> String {
    let c = read_string(p).unwrap_or_default();
    c.split('\n').next().unwrap_or("").to_string()
}

pub fn canonical_path(path: &str) -> Option<String> {
    let mut path = path.to_string();
    let mut guard = 0;
    while is_link(&path) {
        guard += 1;
        if guard > 100 {
            return None;
        }
        let dir = canon_dir(&dirname(&path))?;
        let target = readlink(&path)?;
        path = if target.starts_with('/') {
            target
        } else {
            format!("{dir}/{target}")
        };
    }
    let dir = canon_dir(&dirname(&path))?;
    Some(format!("{dir}/{}", basename(&path)))
}

/// Canonicalize a store entry even if it is missing (parent must exist).
pub fn canonical_entry(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    if exists(path) {
        return canon_dir(path);
    }
    let parent = canon_dir(&dirname(path))?;
    Some(format!("{parent}/{}", basename(path)))
}

pub fn is_in_store(p: &str) -> bool {
    let s = crate::ctx::store();
    p == s || p.starts_with(&format!("{s}/"))
}

pub fn safe_link_target(link: &str) -> Option<String> {
    if !is_link(link) {
        return None;
    }
    let target = readlink(link)?;
    if !target.starts_with('/') {
        return None;
    }
    let resolved = canonical_entry(&target)?;
    if !is_in_store(&resolved) {
        return None;
    }
    Some(resolved)
}

pub fn valid_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

pub fn require_slug(kind: &str, v: &str) {
    if !valid_slug(v) {
        crate::die!("invalid {kind} '{v}': use lowercase letters, digits, and hyphens");
    }
}

/// Entries of a directory (not dotfiles) as full paths `dir/name`, sorted bytewise.
pub fn glob_dir(dir: &str, pred: impl Fn(&str, &str) -> bool) -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect(),
        Err(_) => vec![],
    };
    names.sort();
    names
        .into_iter()
        .map(|n| format!("{dir}/{n}"))
        .filter(|p| pred(p, &basename(p)))
        .collect()
}

/// `dir/*.md`, existing entries only (like `[ -e ]` after the glob).
pub fn glob_md(dir: &str) -> Vec<String> {
    glob_dir(dir, |p, n| n.ends_with(".md") && exists(p))
}

/// `dir/*.md` including dangling symlinks.
pub fn glob_md_all(dir: &str) -> Vec<String> {
    glob_dir(dir, |p, n| n.ends_with(".md") && exists_or_link(p))
}

/// `dir/*/` subdirectories (symlinks to directories count), as `dir/name`.
pub fn glob_subdirs(dir: &str) -> Vec<String> {
    glob_dir(dir, |p, _| is_dir(p))
}

pub fn mkdir_p(p: &str) {
    if let Err(e) = fs::create_dir_all(p) {
        crate::die!("cannot create directory {p}: {e}");
    }
}

pub fn touch_empty(p: &str) {
    if let Err(e) = fs::write(p, b"") {
        crate::die!("cannot write {p}: {e}");
    }
}

pub fn write_file(p: &str, content: &str) {
    if let Err(e) = fs::write(p, content) {
        crate::die!("cannot write {p}: {e}");
    }
}

/// `ln -sfn target link`
pub fn symlink_force(target: &str, link: &str) {
    if exists_or_link(link) && (is_link(link) || !is_dir(link)) {
        let _ = fs::remove_file(link);
    }
    if let Err(e) = std::os::unix::fs::symlink(target, link) {
        crate::die!("cannot link {link}: {e}");
    }
}
