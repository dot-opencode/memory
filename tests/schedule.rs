mod common;

use common::*;
use std::path::PathBuf;

const SERVICE: &str = "agent-memory-dream.service";
const TIMER: &str = "agent-memory-dream.timer";
const LABEL: &str = "io.github.dot-opencode.agent-memory.dream";

fn fixture(backend: &str) -> (T, PathBuf) {
    let mut t = T::new();
    let bin = t.tmp.join("scheduler-bin");
    let log = t.tmp.join("scheduler.log");
    t.set_env("AGENT_MEMORY_SCHEDULER", backend);
    t.set_env("SCHEDULER_LOG", &log);
    t.set_env("SYSTEMCTL_FAIL", "");
    t.set_env("LAUNCHCTL_PRINT_EXIT", "0");
    t.set_env("USER", "schedule-test");
    t.unset_env("AGENT_MEMORY_BIN");
    t.unset_env("AGENT_MEMORY_DREAM_TIMEOUT");
    t.prepend_path(&bin);
    t.write_exec(
        bin.join("systemctl"),
        r#"#!/bin/sh
printf 'systemctl %s\n' "$*" >> "$SCHEDULER_LOG"
[ "$2" != "$SYSTEMCTL_FAIL" ] || exit 42
[ "$2" != list-timers ] || printf 'stub timer listing\n'
"#,
    );
    t.write_exec(
        bin.join("launchctl"),
        r#"#!/bin/sh
printf 'launchctl %s\n' "$*" >> "$SCHEDULER_LOG"
[ "$1" != print ] || exit "$LAUNCHCTL_PRINT_EXIT"
"#,
    );
    t.write_exec(
        bin.join("loginctl"),
        r#"#!/bin/sh
printf 'loginctl %s\n' "$*" >> "$SCHEDULER_LOG"
printf 'Linger=no\n'
"#,
    );
    (t, log)
}

#[test]
fn systemd_install_and_status() {
    let (t, log) = fixture("systemd");
    let cli = t.tmp.join(".cache/launcher 100%/memory");
    t.write_exec(&cli, "#!/bin/sh\nexit 0\n");
    let units = t.home.join(".config/systemd/user");
    t.memory(&[
        "schedule",
        "install",
        "--at",
        "23:59",
        "--model",
        "test/model",
    ])
    .env("AGENT_MEMORY_SELF", &cli)
    .env("AGENT_MEMORY_HOME", "relative store%")
    .env("AGENT_MEMORY_BIN", "binary with \"quotes\" and \\slashes\\")
    .env("AGENT_MEMORY_DREAM_MODEL", "env/model")
    .env("AGENT_MEMORY_DREAM_TIMEOUT", "12")
    .ok()
    .assert_contains("Time: 23:59")
    .assert_contains(&units.join(TIMER).display().to_string())
    .assert_contains(&cli.display().to_string())
    .assert_contains("Warning: re-run `memory schedule install`")
    .assert_contains("loginctl enable-linger");
    let timer = cat(units.join(TIMER));
    assert_contains(&timer, "OnCalendar=*-*-* 23:59:00");
    assert_contains(&timer, "Persistent=true");
    assert_contains(&timer, "RandomizedDelaySec=0");
    assert_contains(&timer, "WantedBy=timers.target");
    let service = cat(units.join(SERVICE));
    assert_contains(&service, "Type=oneshot");
    assert_contains(&service, "Nice=10");
    assert_not_contains(&service, "TimeoutStartSec");
    assert_contains(&service, "Environment=\"PATH=");
    assert_contains(
        &service,
        format!(
            "Environment=\"AGENT_MEMORY_HOME={}/relative store%%\"",
            t.proj.display()
        ),
    );
    assert_contains(
        &service,
        r#"Environment="AGENT_MEMORY_BIN=binary with \"quotes\" and \\slashes\\""#,
    );
    assert_contains(&service, "Environment=\"AGENT_MEMORY_SKILL_DIR=");
    assert_contains(
        &service,
        "Environment=\"AGENT_MEMORY_DREAM_MODEL=env/model\"",
    );
    assert_contains(&service, "Environment=\"AGENT_MEMORY_DREAM_TIMEOUT=12\"");
    assert_contains(
        &service,
        r#"/.cache/launcher 100%%/memory" "dream" "--if-changed" "--model" "test/model""#,
    );
    assert_eq_str(cat(&log), "systemctl --user daemon-reload\nsystemctl --user enable --now agent-memory-dream.timer\nloginctl show-user schedule-test -p Linger");

    t.memory(&["schedule", "status"])
        .ok()
        .assert_contains("Backend: systemd")
        .assert_contains("job is installed")
        .assert_contains("Command: ")
        .assert_contains("Time: 23:59")
        .assert_contains("stub timer listing")
        .assert_not_contains("no longer exists");
    std::fs::remove_file(&cli).unwrap();
    t.memory(&["schedule", "status"])
        .ok()
        .assert_contains(&format!(
            "embedded CLI path no longer exists: {}",
            cli.display()
        ));

    t.memory(&["schedule", "install"])
        .ok()
        .assert_contains("Time: 05:30");
    assert_contains(cat(units.join(TIMER)), "OnCalendar=*-*-* 05:30:00");
    let service = cat(units.join(SERVICE));
    assert_not_contains(&service, "--model");
    assert_not_contains(&service, "AGENT_MEMORY_BIN=");
}

#[test]
fn rejects_invalid_times() {
    let (t, log) = fixture("systemd");
    for time in ["24:00", "09:60", "5:30", "05:3", "05:30:00", "aa:00"] {
        t.memory(&["schedule", "install", "--at", time])
            .code(1)
            .assert_contains("invalid --at time")
            .assert_contains("usage: memory schedule");
    }
    t.memory(&["schedule", "install", "--at"])
        .code(1)
        .assert_contains("usage: memory schedule");
    assert_absent(log);
    assert_absent(t.home.join(".config/systemd/user"));
}

#[test]
fn systemd_uninstall_and_absent_status() {
    let (t, log) = fixture("systemd");
    let xdg = t.tmp.join("xdg");
    let units = xdg.join("systemd/user");
    let schedule = |action| t.memory(&["schedule", action]).env("XDG_CONFIG_HOME", &xdg);
    schedule("uninstall").ok().assert_contains("not installed");
    assert_absent(&log);
    schedule("status")
        .ok()
        .assert_contains("Backend: systemd")
        .assert_contains(&units.join(TIMER).display().to_string())
        .assert_contains("not installed")
        .assert_contains("stub timer listing");
    assert_eq_str(
        cat(&log),
        "systemctl --user list-timers --all agent-memory-dream.timer",
    );
    schedule("install").ok();
    schedule("uninstall").env("SYSTEMCTL_FAIL", "disable").ok();
    assert_absent(units.join(TIMER));
    assert_absent(units.join(SERVICE));
    assert!(cat(&log).ends_with(
        "systemctl --user disable --now agent-memory-dream.timer\nsystemctl --user daemon-reload"
    ));
}

#[test]
fn launchd_install_status_and_uninstall() {
    let (t, log) = fixture("launchd");
    let cli = t.tmp.join("launcher <&> \"' %/memory");
    t.write_exec(&cli, "#!/bin/sh\nexit 0\n");
    let plist = t.home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
    let job_log = t.home.join("Library/Logs/agent-memory-dream.log");
    t.memory(&["schedule", "install", "--at", "08:07"])
        .env("AGENT_MEMORY_SELF", &cli)
        .ok()
        .assert_contains(&plist.display().to_string())
        .assert_contains("Time: 08:07");
    let text = cat(&plist);
    assert_contains(&text, "<key>Hour</key><integer>8</integer>");
    assert_contains(&text, "<key>Minute</key><integer>7</integer>");
    assert_contains(
        &text,
        "launcher &lt;&amp;&gt; &quot;&apos; %/memory</string>",
    );
    assert_contains(
        &text,
        "<string>dream</string>\n<string>--if-changed</string>",
    );
    assert_contains(&text, "<key>EnvironmentVariables</key><dict>");
    assert_contains(
        &text,
        format!(
            "<key>StandardOutPath</key><string>{}</string>",
            job_log.display()
        ),
    );
    assert_contains(
        &text,
        format!(
            "<key>StandardErrorPath</key><string>{}</string>",
            job_log.display()
        ),
    );
    assert_dir(job_log.parent().unwrap());
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    assert_eq_str(
        cat(&log),
        format!(
            "launchctl bootout {domain}/{LABEL}\nlaunchctl bootstrap {domain} {}",
            plist.display()
        ),
    );
    t.memory(&["schedule", "status"])
        .ok()
        .assert_contains("Backend: launchd")
        .assert_contains("Command: ")
        .assert_contains("Time: 08:07")
        .assert_contains("LaunchAgent is loaded")
        .assert_not_contains("no longer exists");
    t.memory(&["schedule", "status"])
        .env("LAUNCHCTL_PRINT_EXIT", "3")
        .ok()
        .assert_contains("LaunchAgent is not loaded");
    assert!(cat(&log).ends_with(&format!("launchctl print {domain}/{LABEL}")));
    t.memory(&["schedule", "uninstall"]).ok();
    assert_absent(plist);
    assert!(cat(log).ends_with(&format!("launchctl bootout {domain}/{LABEL}")));
}

#[test]
fn systemctl_failure_is_reported() {
    let (t, log) = fixture("systemd");
    t.memory(&["schedule", "install"])
        .env("SYSTEMCTL_FAIL", "daemon-reload")
        .code(1)
        .assert_contains("systemctl --user daemon-reload failed (exit 42)");
    assert_eq_str(cat(log), "systemctl --user daemon-reload");
}
