# agent-memory

A three-level, file-based memory system for coding agents. No servers or
databases — markdown files, symlinks, and one Bash script.

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
consolidates the store — merge duplicates, resolve contradictions, verify and
prune stale facts, move facts shared by several projects up to a group or the
workspace.

- `memory dream --report [--stale-days N] [--workspace <ws>]` — read-only
  report over the whole store (works from any directory, writes nothing).
- `memory dream [--workspace <ws>] [--model <provider/model>]` — an unattended
  OpenCode agent works through the report and fixes the store.
  `--print-config` shows the exact config and prompt without running anything.
  `--if-changed` skips the run when nothing was committed to the store since
  the last successful run, for use from a nightly scheduler (launchd, cron).
  After the first successful run, later runs are incremental: the agent gets
  the list of facts added or changed since the last run and checks only those
  against the rest (more than 150 changed facts means a full run). `--full`
  forces a full pass over the store.
  Without `--model` it uses `AGENT_MEMORY_DREAM_MODEL`, or OpenCode's default
  model. The consolidation is judgment work, so a strong model is worth it: in
  testing, a small model read 35 of 500 facts and changed only descriptions.
- The `memory-dream` skill does the same interactively for the current project,
  asking for approval before deleting, merging or moving facts.

The unattended run is sandboxed. The agent can read anywhere except secret
paths (`~/.ssh`, `~/.aws`, `.env` files, ...), write only inside the store
(never `.git`), and run only `memory dream --report`, read-only git commands
(`log`, `show`, `diff`, `status`, `rev-list`, `ls-tree`, `blame`) and
`git add/mv/rm` in the store (the only way it can delete or move a fact). There is no
`cat`/`rg` in the shell: file reading goes through OpenCode's own read tools,
where the secret-path rules apply. Web access is allowed for checking references,
and the only subagent is a read-only checker. The boundary is enforced through
OpenCode `experimental.policies`, which a project's `opencode.json` cannot
override. It runs `opencode run --standalone` from the store directory with the
config passed in `OPENCODE_CONFIG_CONTENT`, so your OpenCode config files are
not modified. The run uses its own OpenCode database at
`<store>/.git/memory-dream.db`, which holds the latest run only, so dream
sessions never show up in OpenCode's session list. Your global config and
plugins still load, and the sandbox config is merged on top of them.
`memory dream --print-config` shows the exact rules.

The store is committed before the run (if it has pending changes) and after it;
nothing is ever pushed. To undo a run:

```bash
git -C ~/.agent-memory revert HEAD
```

## Other agents

Only OpenCode has a plugin. Other agents get the start-of-task memory protocol
from the instruction snippets in [`integrations/`](integrations/README.md).
The snippets do not back up the store — run `memory backup` yourself.

## Dependencies

The CLI uses Bash plus standard shell utilities (`grep`, `awk`, `readlink`),
`git`, and filesystem symlink support. The OpenCode integration uses OpenCode's
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

- **Windows** is not supported (symlinks, Bash).
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
bash tests/run.sh
npm run test:opencode
```
