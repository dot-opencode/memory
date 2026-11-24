//! Install and inspect the per-user nightly dream job.

use crate::ctx::{cap_trim, memory_home, run};
use crate::{die, outln};
use std::env;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str =
    "usage: memory schedule install [--at HH:MM] [--model <provider/model>] | uninstall | status";
const SERVICE: &str = "agent-memory-dream.service";
const TIMER: &str = "agent-memory-dream.timer";
const LABEL: &str = "io.github.dot-opencode.agent-memory.dream";

enum Backend {
    Systemd { service: PathBuf, timer: PathBuf },
    Launchd { plist: PathBuf, log: PathBuf },
}

impl Backend {
    fn detect() -> Self {
        let kind = match env::var("AGENT_MEMORY_SCHEDULER") {
            Ok(value) => value,
            Err(env::VarError::NotPresent) if cfg!(target_os = "macos") => "launchd".into(),
            Err(env::VarError::NotPresent) => "systemd".into(),
            Err(e) => die!("invalid AGENT_MEMORY_SCHEDULER: {e} (expected systemd or launchd)"),
        };
        match kind.as_str() {
            "systemd" => {
                let dir = env::var_os("XDG_CONFIG_HOME")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home().join(".config"))
                    .join("systemd/user");
                Self::Systemd {
                    service: dir.join(SERVICE),
                    timer: dir.join(TIMER),
                }
            }
            "launchd" => {
                let library = home().join("Library");
                Self::Launchd {
                    plist: library.join(format!("LaunchAgents/{LABEL}.plist")),
                    log: library.join("Logs/agent-memory-dream.log"),
                }
            }
            _ => die!("invalid AGENT_MEMORY_SCHEDULER '{kind}' (expected systemd or launchd)"),
        }
    }
}

fn home() -> PathBuf {
    env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| die!("schedule: HOME is not set"))
}

pub fn schedule(args: &[String]) -> i32 {
    match args.split_first() {
        Some((cmd, rest)) if cmd == "install" => {
            let mut time = "05:30";
            let mut model = None;
            for pair in rest.chunks(2) {
                match pair {
                    [flag, value] if flag == "--at" => {
                        if time_parts(value).is_none() {
                            die!("invalid --at time '{value}': expected HH:MM (00:00–23:59)\n{USAGE}");
                        }
                        time = value;
                    }
                    [flag, value]
                        if flag == "--model" && !value.is_empty() && !value.starts_with('-') =>
                    {
                        model = Some(value.as_str());
                    }
                    _ => die!("{USAGE}"),
                }
            }
            install(Backend::detect(), time, model);
        }
        Some((cmd, [])) if cmd == "uninstall" => uninstall(Backend::detect()),
        Some((cmd, [])) if cmd == "status" => status(Backend::detect()),
        _ => die!("{USAGE}"),
    }
    0
}

fn time_parts(time: &str) -> Option<(u8, u8)> {
    let (hour, minute) = time.split_once(':')?;
    let parse = |s: &str| -> Option<u8> {
        (s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (hour, minute) = (parse(hour)?, parse(minute)?);
    (hour <= 23 && minute <= 59).then_some((hour, minute))
}

fn install(backend: Backend, time: &str, model: Option<&str>) {
    let cli = crate::self_path();
    if cli.is_empty() {
        die!("schedule: could not determine the memory CLI path");
    }
    let mut args = vec![cli.as_str(), "dream", "--if-changed"];
    if let Some(model) = model {
        args.extend(["--model", model]);
    }
    let vars: Vec<_> = [
        "PATH",
        "AGENT_MEMORY_HOME",
        "AGENT_MEMORY_BIN",
        "AGENT_MEMORY_SKILL_DIR",
        "AGENT_MEMORY_DREAM_MODEL",
        "AGENT_MEMORY_DREAM_PLUGINS",
        "AGENT_MEMORY_DREAM_TIMEOUT",
    ]
    .into_iter()
    .filter_map(|key| {
        let value = if key == "AGENT_MEMORY_HOME" {
            memory_home()
        } else {
            env::var(key).ok()?
        };
        (!value.is_empty()).then_some((key, value))
    })
    .collect();

    let path = match backend {
        Backend::Systemd { service, timer } => {
            let mut unit =
                String::from("[Unit]\nDescription=Agent Memory Dream\n\n[Service]\nType=oneshot\n");
            for (key, value) in &vars {
                writeln!(
                    unit,
                    "Environment={}",
                    systemd_quote(&format!("{key}={value}"), false)
                )
                .unwrap();
            }
            writeln!(
                unit,
                "ExecStart={}\nNice=10",
                args.iter()
                    .map(|arg| systemd_quote(arg, true))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .unwrap();
            write_job(&service, &unit);
            write_job(
                &timer,
                &format!("[Unit]\nDescription=Nightly Agent Memory Dream\n\n[Timer]\nOnCalendar=*-*-* {time}:00\nPersistent=true\nRandomizedDelaySec=0\n\n[Install]\nWantedBy=timers.target\n"),
            );
            checked(
                &mut systemctl(&["daemon-reload"]),
                "systemctl --user daemon-reload",
            );
            checked(
                &mut systemctl(&["enable", "--now", TIMER]),
                "systemctl --user enable --now",
            );
            if let Ok(user) = env::var("USER") {
                if !user.is_empty()
                    && cap_trim(Command::new("loginctl").args(["show-user", &user, "-p", "Linger"]))
                        .is_some_and(|s| s.trim() == "Linger=no")
                {
                    outln!("Note: this timer runs only while you are logged in. Missed runs catch up at next login (Persistent=true). `loginctl enable-linger` makes it run without a session.");
                }
            }
            outln!("Service: {}", service.display());
            timer
        }
        Backend::Launchd { plist, log } => {
            make_dir(log.parent().unwrap());
            write_job(&plist, &launchd_plist(time, &args, &vars, &log));
            let domain = launchd_domain();
            bootout(&domain);
            checked(
                Command::new("launchctl")
                    .args(["bootstrap", &domain])
                    .arg(&plist),
                "launchctl bootstrap",
            );
            plist
        }
    };
    outln!("Installed: {}", path.display());
    outln!("Time: {time}");
    outln!("CLI: {cli}");
    let note =
        "re-run `memory schedule install` if this path changes (for example after a plugin update)";
    if cli.contains("/.cache/") {
        eprintln!("Warning: {note}");
    } else {
        outln!("Note: {note}");
    }
}

/// Quotes a unit-file value. `exec` also escapes `$`: ExecStart expands
/// variables, Environment= does not.
fn systemd_quote(s: &str, exec: bool) -> String {
    let mut quoted = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '%' => quoted.push_str("%%"),
            '$' if exec => quoted.push_str("$$"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            ch => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

fn launchd_plist(time: &str, args: &[&str], vars: &[(&str, String)], log: &Path) -> String {
    let (hour, minute) = time_parts(time).unwrap();
    let mut plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         <key>Label</key><string>{LABEL}</string>\n\
         <key>ProgramArguments</key><array>\n"
    );
    for arg in args {
        writeln!(plist, "<string>{}</string>", xml_escape(arg)).unwrap();
    }
    plist.push_str("</array>\n<key>EnvironmentVariables</key><dict>\n");
    for (key, value) in vars {
        writeln!(
            plist,
            "<key>{key}</key><string>{}</string>",
            xml_escape(value)
        )
        .unwrap();
    }
    let log = xml_escape(&log.to_string_lossy());
    write!(
        plist,
        "</dict>\n<key>StartCalendarInterval</key><dict>\n\
         <key>Hour</key><integer>{hour}</integer>\n\
         <key>Minute</key><integer>{minute}</integer>\n</dict>\n\
         <key>StandardOutPath</key><string>{log}</string>\n\
         <key>StandardErrorPath</key><string>{log}</string>\n\
         <key>Nice</key><integer>10</integer>\n</dict>\n</plist>\n"
    )
    .unwrap();
    plist
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
}

fn uninstall(backend: Backend) {
    match backend {
        Backend::Systemd { service, timer } if present(&service) || present(&timer) => {
            let _ = cap_trim(&mut systemctl(&["disable", "--now", TIMER]));
            remove_job(&timer);
            remove_job(&service);
            checked(
                &mut systemctl(&["daemon-reload"]),
                "systemctl --user daemon-reload",
            );
        }
        Backend::Launchd { plist, .. } if present(&plist) => {
            bootout(&launchd_domain());
            remove_job(&plist);
        }
        _ => {
            outln!("Nightly dream job is not installed.");
            return;
        }
    }
    outln!("Uninstalled nightly dream job.");
}

fn status(backend: Backend) {
    match backend {
        Backend::Systemd { service, timer } => {
            outln!("Backend: systemd");
            outln!("Service: {}", service.display());
            outln!("Timer: {}", timer.display());
            let (unit, timer) = (read_job(&service), read_job(&timer));
            show_installed(unit.is_some() && timer.is_some());
            if let Some(command) = unit.as_deref().and_then(|s| field(s, "ExecStart=")) {
                outln!("Command: {command}");
                if let Some(cli) = systemd_cli(command) {
                    check_cli(&cli);
                }
            }
            if let Some(time) = timer.as_deref().and_then(|s| field(s, "OnCalendar=*-*-* ")) {
                outln!("Time: {}", time.strip_suffix(":00").unwrap_or(time));
            }
            let rc = run(&mut systemctl(&["list-timers", "--all", TIMER]));
            if rc != 0 {
                eprintln!("Warning: systemctl --user list-timers failed (exit {rc})");
            }
        }
        Backend::Launchd { plist, .. } => {
            outln!("Backend: launchd");
            outln!("LaunchAgent: {}", plist.display());
            let text = read_job(&plist);
            show_installed(text.is_some());
            if let Some(text) = text {
                if let Some((array, _)) = text
                    .split_once("<key>ProgramArguments</key><array>")
                    .and_then(|(_, rest)| rest.split_once("</array>"))
                {
                    let args: Vec<_> = array
                        .lines()
                        .filter_map(|line| {
                            line.strip_prefix("<string>")
                                .and_then(|s| s.strip_suffix("</string>"))
                                .map(xml_unescape)
                        })
                        .collect();
                    if let Some(cli) = args.first() {
                        let command = args
                            .iter()
                            .map(|arg| format!("{arg:?}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        outln!("Command: {command}");
                        check_cli(cli);
                    }
                }
                let part = |name| {
                    field(&text, name)?
                        .strip_suffix("</integer>")?
                        .parse::<u8>()
                        .ok()
                };
                if let (Some(hour), Some(minute)) = (
                    part("<key>Hour</key><integer>"),
                    part("<key>Minute</key><integer>"),
                ) {
                    outln!("Time: {hour:02}:{minute:02}");
                }
            }
            let loaded = cap_trim(
                Command::new("launchctl")
                    .arg("print")
                    .arg(launchd_target(&launchd_domain())),
            )
            .is_some();
            outln!("LaunchAgent is {}loaded.", if loaded { "" } else { "not " });
        }
    }
}

fn field<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find_map(|line| line.strip_prefix(prefix))
}

// Recover the executable from the first quoted argument of our generated ExecStart.
fn systemd_cli(command: &str) -> Option<String> {
    let mut chars = command.strip_prefix('"')?.chars().peekable();
    let mut cli = String::new();
    while let Some(ch) = chars.next() {
        cli.push(match ch {
            '"' => return Some(cli),
            '\\' => match chars.next()? {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                escaped => escaped,
            },
            '%' | '$' if chars.peek() == Some(&ch) => chars.next()?,
            ch => ch,
        });
    }
    None
}

fn xml_unescape(s: &str) -> String {
    s.replace("&#13;", "\r")
        .replace("&#10;", "\n")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

fn show_installed(installed: bool) {
    outln!(
        "Nightly dream job is {}installed.",
        if installed { "" } else { "not " }
    );
}

fn check_cli(cli: &str) {
    if !Path::new(cli).exists() {
        eprintln!("Warning: embedded CLI path no longer exists: {cli}");
    }
}

fn systemctl(args: &[&str]) -> Command {
    let mut cmd = Command::new("systemctl");
    cmd.env("SYSTEMD_PAGER", "cat").arg("--user").args(args);
    cmd
}

fn launchd_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

fn launchd_target(domain: &str) -> String {
    format!("{domain}/{LABEL}")
}

fn bootout(domain: &str) {
    let _ = cap_trim(
        Command::new("launchctl")
            .arg("bootout")
            .arg(launchd_target(domain)),
    );
}

fn checked(cmd: &mut Command, action: &str) {
    let rc = run(cmd);
    if rc != 0 {
        die!("schedule: {action} failed (exit {rc})");
    }
}

fn make_dir(path: &Path) {
    fs::create_dir_all(path)
        .unwrap_or_else(|e| die!("schedule: cannot create {}: {e}", path.display()));
}

fn write_job(path: &Path, text: &str) {
    make_dir(path.parent().unwrap());
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .unwrap_or_else(|e| die!("schedule: cannot create {}: {e}", temp.display()));
    if let Err(e) = file
        .write_all(text.as_bytes())
        .and_then(|()| fs::rename(&temp, path))
    {
        let _ = fs::remove_file(&temp);
        die!("schedule: cannot write {}: {e}", path.display());
    }
}

fn present(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => die!("schedule: cannot inspect {}: {e}", path.display()),
    }
}

fn read_job(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => die!("schedule: cannot read {}: {e}", path.display()),
    }
}

fn remove_job(path: &Path) {
    if let Err(e) = fs::remove_file(path) {
        if e.kind() != io::ErrorKind::NotFound {
            die!("schedule: cannot remove {}: {e}", path.display());
        }
    }
}
