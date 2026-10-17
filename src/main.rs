//! agent-memory CLI (`memory`): Rust port of the former Bash script.

mod backup;
mod cmds;
mod ctx;
mod dream;
mod facts;
mod fsutil;
mod report;

use ctx::*;

const VERSION: &str = "0.1.0";

const USAGE: &str = "usage: memory <command> [args]        (memory --version prints the version)

commands:
  init [workspace] [--name <project-name>]  set up .memory/ for this project
  link-group <group>                        link a group's shared memory
  unlink-group <group>                      remove a group link
  index                                     rebuild every INDEX.md from fact frontmatter
  status                                    levels, fact counts, unlinked groups, backup state
  grep <pattern> [grep opts]                search all linked levels (follows the symlinks)
  doctor                                    check symlinks, indexes, frontmatter
  dream --report [--stale-days N] [--workspace <ws>]  read-only whole-store report: duplicates, stale facts
  dream [--workspace <ws>] [--model <m>] [--if-changed] [--full] [--web] [--timeout <min>] [--print-config]
                                            tidy the store with a sandboxed OpenCode run
  backup [--push]                           commit (and push) the store
  install-opencode                          install the global OpenCode plugin";

/// Path of the CLI itself: `AGENT_MEMORY_SELF` (set by the launcher) or the executable.
pub fn self_path() -> String {
    match std::env::var("AGENT_MEMORY_SELF") {
        Ok(v) if !v.is_empty() => v,
        _ => std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// Skill directory (`<skill>/bin/..`) holding dream/ and opencode/ resources.
pub fn skill_dir() -> Option<String> {
    if let Ok(v) = std::env::var("AGENT_MEMORY_SKILL_DIR") {
        if !v.is_empty() {
            return fsutil::canon_dir(&v).or(Some(v));
        }
    }
    let sp = self_path();
    if sp.is_empty() {
        return None;
    }
    let script = fsutil::canonical_path(&sp)?;
    fsutil::canon_dir(&format!("{}/..", fsutil::dirname(&script)))
}

fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let home = match std::env::var("AGENT_MEMORY_HOME") {
        Ok(v) if !v.is_empty() => v,
        _ => match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => format!("{h}/.agent-memory"),
            _ => die!("HOME: HOME is not set"),
        },
    };
    let home = if home.starts_with('/') {
        home
    } else {
        format!("{}/{home}", fsutil::pwd())
    };
    st(|s| s.memory_home = home);

    let cmd = args.first().cloned().unwrap_or_default();
    let rest: &[String] = if args.is_empty() { &[] } else { &args[1..] };
    let rc = match cmd.as_str() {
        "init" => cmds::init(rest),
        "link-group" => cmds::link_group(rest),
        "unlink-group" => cmds::unlink_group(rest),
        "index" => cmds::index(rest),
        "status" => cmds::status(rest),
        "grep" => cmds::grep(rest),
        "doctor" => cmds::doctor(rest),
        "dream" => dream::dream(rest),
        "backup" => backup::backup(rest),
        "install-opencode" => cmds::install_opencode(rest),
        "-v" | "--version" => {
            if !rest.is_empty() {
                die!("usage: memory --version");
            }
            outln!("memory {VERSION}");
            0
        }
        "-h" | "--help" => {
            if !rest.is_empty() {
                die!("usage: memory --help");
            }
            outln!("{USAGE}");
            0
        }
        "" => {
            eprintln!("{USAGE}");
            exit_with(1)
        }
        _ => die!("unknown command: {cmd} (see 'memory --help')"),
    };
    exit_with(rc)
}
