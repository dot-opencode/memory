# agent-memory

A three-level, file-based memory system for coding agents. No servers or
databases — markdown files, symlinks, and one small Rust binary.

It is a **personal** memory: one developer's agents, across all of that
developer's projects. The store lives in your home directory; "group" memory
is shared between *your* projects, not between people. It is not a team
knowledge base (see [Limitations](#limitations)).

- **project** memory — facts about one codebase
- **group** memory — facts shared by a group of projects (a project may join
  several groups)
- **workspace** memory — facts about a whole environment (work, home, ...)

All facts live in `~/.agent-memory/` (a git repo). Each project gets a
`.memory/` directory of symlinks into the store, so any agent that can read
files can *read* the memory. The protocol (when to read, where and how to
write) is a skill, [`skills/agent-memory/SKILL.md`](skills/agent-memory/SKILL.md).
OpenCode discovers the skill and can load it on demand; for Cursor, Copilot,
Codex and other AGENTS.md readers, install the 20-line snippet from
[`integrations/`](integrations/README.md).

## Install for OpenCode V2

Install the skill, then install its native OpenCode V2 plugin once:

```bash
npx skills add dot-opencode/memory -g -a opencode
~/.agents/skills/agent-memory/bin/memory install-opencode
```

`npx skills add` installs every skill in this repository: `agent-memory`
(protocol, CLI, OpenCode plugin) and `memory-dream` (interactive cleanup of the
current project's memory, see [Dream](#dream)). The `agent-memory` skill is
self-contained: the `memory` CLI and the OpenCode plugin ship inside it. To call the CLI as plain `memory` (as in the examples below), put it
on your `PATH`:

```bash
ln -s ~/.agents/skills/agent-memory/bin/memory ~/.local/bin/memory
```

`bin/memory` is a small launcher. On first run it downloads the prebuilt
binary pinned in `bin/release.txt` from GitHub Releases, checks its sha256 and
caches it under `~/.cache/agent-memory/` (`$XDG_CACHE_HOME` if set). Prebuilt
binaries exist for macOS (arm64, x86_64) and Linux (x86_64, aarch64). Offline
or on another platform, build it yourself and point the launcher at it:

```bash
cargo install --git https://github.com/dot-opencode/memory agent-memory
export AGENT_MEMORY_BIN="$HOME/.cargo/bin/memory"
```

The installer creates only this symlink:
`~/.config/opencode/plugins/agent-memory.js`. It does not change
`opencode.json`; restart OpenCode after installing or updating the skill.

The plugin runs in every OpenCode project with `.memory/`:

- **Prompt and compaction** — adds project, group, and workspace indexes to
  context, capped at 25,600 characters.
- **Idle session** — runs `memory backup`. The command is idempotent; a failure
  is reported through the plugin's error logger without interrupting the session.

## Quick start

```bash
cd ~/work/some-api
memory install-opencode      # one-time OpenCode setup
memory init work              # bind this project to workspace "work"
memory link-group spines      # optional, repeatable per group
memory index                  # rebuild INDEX.md files from fact frontmatter
memory status                 # levels, fact counts, groups you could link, backup state
memory grep pgbouncer         # search facts (rg/grep -r do not see .memory/)
memory doctor                 # check symlinks / indexes / frontmatter
memory dream --report         # read-only report: duplicates, stale facts, drift
memory dream                  # tidy the whole store with a sandboxed OpenCode run
memory backup                 # commit the store; --push to push
```

`memory init` has side effects outside the project, all idempotent:

- creates `~/.agent-memory` (or `$AGENT_MEMORY_HOME`) and runs `git init` there
  on first use;
- creates the workspace and project directories in the store;
- writes **absolute** symlinks into `./.memory/`;
- appends `.memory/` to your **global** gitignore (`git config --global
  core.excludesfile`, creating `~/.config/git/ignore` if none is set).

The store location defaults to `~/.agent-memory` and can be overridden with
`AGENT_MEMORY_HOME`.

## Dream

Memory rots: facts get duplicated, contradict each other or go stale. Dream
helps consolidate the store: correct facts, resolve contradictions, combine
shared details and identify facts to remove.

- `memory dream --report [--stale-days N] [--workspace <ws>]` — read-only
  report over the whole store (works from any directory, writes nothing).
- `memory dream [--workspace <ws>] [--model <provider/model>] [--web] [--timeout <minutes>]` — an unattended
  OpenCode agent works through the report and edits ordinary fact Markdown.
  It can update or create a retained fact for a merge or move, but cannot delete
  or rename the old file: it reports paths for manual deletion instead. The CLI
  rebuilds indexes afterwards.
  `--print-config` shows the exact config and prompt without running anything.
  `--if-changed` skips the run when no fact changed since
  the last successful run, for use from a nightly scheduler (launchd, cron).
  After the first successful run, later runs are incremental: the agent gets
  the list of facts added or changed since the last run and checks only those
  against the rest (more than 150 changed facts means a full run). `--full`
  forces a full pass over the store.
  Web access is denied by default; `--web` enables it for checking references.
  `--timeout` sets a positive whole number of minutes (default 120, or
  `AGENT_MEMORY_DREAM_TIMEOUT`); the CLI stops the run when the limit expires.
  Without `--model` it uses `AGENT_MEMORY_DREAM_MODEL`, or OpenCode's default
  model. The consolidation is judgment work, so a strong model is worth it: in
  testing, a small model read 35 of 500 facts and changed only descriptions.
- The local `memory-dream` skill works interactively on the current project's
  memory. Unlike the unattended CLI run, it may delete or move facts after
  asking for approval.

The unattended run uses best-effort application-level permissions, not OS
isolation. The intended scope is reading the store and known project directories,
editing ordinary fact Markdown and a read-only checker subagent. The only Git
shell commands allowed are exactly `git -C <dir> status` and
`git -C <dir> status --short` for the store or a known project directory, with no
extra arguments. Verification uses current files through the read tool; facts
requiring Git history remain unresolved for manual review. History and content
commands are disabled because they bypass file-read restrictions.
Grep and glob tools are disabled because their permission
resources are search expressions rather than paths. Git add/mv/rm are disabled
because wildcard permissions cannot safely constrain their arguments. MCP and
plugin integrations are denied by policy. These restrictions cannot guarantee
confidentiality from the model or its provider. Do not run it on data you are not
willing to send to that provider.
The rules were verified with live runs on OpenCode v2.0.31; enforcement may
differ in other versions.

The permission matcher cannot express everything, so the CLI adds its own
checks. It refuses to run if the store contains symlinks outside `.git`. After
the run, any change other than a `<slug>.md` fact in a level directory that
already existed (a new project or group directory, a nested, hidden or
non-Markdown file, a delete or rename) fails the run as a review commit. If
`sqlite3` is available, it reads the run's session database and fails the run
when no tool call succeeded or `memory dream --report` was denied, so a broken
sandbox is not counted as a quiet night; without `sqlite3` it only warns.

It runs `opencode run --standalone` from the store directory with the config
passed in `OPENCODE_CONFIG_CONTENT` and project config loading disabled, so your
OpenCode config files are not modified. The run uses its own OpenCode database at
`<store>/.git/memory-dream.db`, which holds the latest run only, so dream
sessions are kept separate from the usual OpenCode session list. The permission
config is merged with global config; this is not an isolated OpenCode installation.
`memory dream --print-config` shows the exact rules.

Pending store changes are committed before the run, separately from its result;
nothing is pushed. A successful run commits its changes and advances the local
last-successful-run ref. On failure or timeout, any changes are committed as
`memory: dream FAILED (exit N) — review`, without advancing that ref. If there
are no changes, there is no result commit.

Dream and backup share a writer lock. An external `memory backup` waits up to
10 seconds by default; if still busy, it exits 75 and explicitly reports that
the request was **not saved**. Retry after the writer finishes. Stale or uncertain
locks also require manual recovery and return nonzero; they never expire
automatically. Before removing a lock, stop **all** memory writers and inspect
the owner metadata, verifying both PID and process start time; missing metadata
does not prove a lock is stale. Follow the CLI's recovery guidance, not a blind
lock deletion.

If final indexing or saving fails even after the agent exits successfully, a
FAILED review marker is retained. Fix the git/index error and run
`memory backup` to save remaining changes as a FAILED review commit. Service Git
operations intentionally disable hooks and commit signing so unattended saves
do not invoke user hooks or signing prompts.

Review the store's git log and diff before undoing a result. If its commit is
still `HEAD`, use this (replace the path if you use `AGENT_MEMORY_HOME`):

```bash
git -C ~/.agent-memory revert HEAD
```

Otherwise revert the result's commit hash instead of `HEAD`. This leaves the
separate pre-run backup intact; reverting does not reset the last-successful-run
ref. Use `memory dream --full` if the next run should reconsider all facts.

## Other agents

Only OpenCode has a plugin. Other agents get the start-of-task memory protocol
from the instruction snippets in [`integrations/`](integrations/README.md).
The snippets do not back up the store — run `memory backup` yourself.

## Dependencies

The CLI is a single static Rust binary. At runtime it calls `git`, `grep`
(for `memory grep`), and, for `memory dream`, `opencode` and `sqlite3`; it needs
filesystem symlink support. The launcher needs `curl` or `wget`, `tar`, and
`shasum` or `sha256sum` for the first download. The OpenCode integration uses OpenCode's
built-in Bun runtime and imports only Node built-ins; it has no package
dependencies.

## Layout

```
~/.agent-memory/<ws>/
  workspace/               # workspace-level facts + INDEX.md
  groups/<group>/          # group facts + INDEX.md
  projects/<project>/      # project facts + INDEX.md

<project>/.memory/
  project/    → store      # absolute symlinks
  groups/<g>/ → store
  workspace/  → store
```

`.memory/` is ignored via the global gitignore (added by `memory init`).

The agent protocol is defined in [`skills/agent-memory/SKILL.md`](skills/agent-memory/SKILL.md).

## Limitations

- **Windows** is not supported (symlinks).
- **Containers / CI / remote dev**: `.memory/*` are absolute symlinks into the
  host store. Inside a container they resolve only if the store is mounted at
  the same absolute path. Sandboxed agents restricted to the workspace cannot
  follow them.
- **`git worktree`** checkouts have no `.memory/` and `memory init` there would
  create a *second* project memory named after the worktree directory. Until
  rebinding lands, recreate the links by hand
  (`ln -s "$(readlink ../main/.memory/project)" .memory/project`, same for
  `workspace` and `groups/*`).
- **Search**: `rg`/`grep -r` do not see `.memory/` (gitignored symlinks). Read
  `INDEX.md` by path, or use `memory grep <term>` / `rg -L --no-ignore <term> .memory/`.
- **Sharing**: `memory backup --push` pushes `HEAD` without pulling. Two people
  pushing the same store will conflict; this is a single-writer backup, not a
  sync protocol.
- **Commit messages**: `backup` builds a structured message from the staged
  diff, e.g. `memory(some-api): add pool-limits; update retry-policy` for one
  scope, or `memory: add 3, update 1 in 2 scopes` with a per-scope body when
  several projects/groups/workspace change together.
- **Secrets**: `backup` commits everything under the store. Never write
  credentials into facts.

## Tests

```bash
cargo test
bash tests/launcher.sh
npm run test:opencode
```

## Releasing

Bump `version` in `Cargo.toml`, merge, then push a tag `v<version>`. The
release workflow builds the four targets, publishes them to GitHub Releases
and commits the new hashes to `skills/agent-memory/bin/release.txt`. The
launcher only downloads what that file pins.
