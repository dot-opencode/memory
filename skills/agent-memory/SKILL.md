---
name: agent-memory
description: Read and write three-level agent memory (project / groups / workspace) stored as markdown under ./.memory/. Use at the start of any task in a project that has a .memory/ directory, and whenever you learn something worth keeping for future sessions.
license: MIT
---

# Agent Memory

Long-term memory lives in `./.memory/` as plain markdown. Three levels:

- `./.memory/project/` — this project only
- `./.memory/groups/<group>/` — shared by a group of projects (zero or more groups)
- `./.memory/workspace/` — the whole working environment (user preferences, tools, setup)

No `.memory/` directory → this project has no memory set up. Work without it;
you may suggest `memory init <workspace>` to the user, but never run it
yourself. Likewise suggest, never run, `memory link-group <group>`: joining a
group is the user's decision.

## The `memory` CLI

The CLI ships with this skill: `bin/memory` next to this file. Use `memory`
from PATH if installed; otherwise call the bundled script by its path
(`<this skill's directory>/bin/memory`). It handles mechanics only — setting
up `.memory/` (`init`), linking groups (`link-group`/`unlink-group`),
consistency checks (`doctor`), memory consolidation (`dream --report` is a
read-only whole-store report of duplicates, stale facts and drift; `dream`
runs the cleanup unattended), OpenCode plugin installation
(`install-opencode`), and store backup (`backup`). `backup` commits with a
structured message derived from the staged diff (e.g.
`memory(<project>): add <slug>, ...` or `memory: add N in S scopes` with a
per-scope body) — nothing to configure, it just describes what changed.
Reading and writing facts is always plain file I/O, no CLI needed.

Suggest unattended `memory dream` to the user rather than launching it
autonomously: it can take time and incur model costs. It edits ordinary fact
Markdown and recommends manual deletions; it cannot delete or rename facts.
For merges or moves it can update or create the retained fact, leaving the old
path for manual deletion. The CLI rebuilds indexes. Grep/glob and Git add/mv/rm
are disabled, and MCP/plugin integrations are denied by policy.
The only Git shell commands allowed are exactly `git -C <dir> status` and
`git -C <dir> status --short` for the store or a known project directory, without
extra arguments. Verification reads current files only; facts requiring Git
history remain unresolved for manual review because history/content commands
bypass file-read restrictions and are disabled.
Web access is denied by default (`--web` opts in). `--timeout <minutes>` sets a
positive whole number of minutes, defaulting to 120 or
`AGENT_MEMORY_DREAM_TIMEOUT`. Project config loading
is disabled for the run. Its permissions are application-level best effort, not
OS isolation, and cannot guarantee confidentiality from the model/provider.
The CLI also refuses a store with symlinks and fails the run (as a review
commit) on any change other than a fact in an existing level directory; see the
README for details.

Pending changes are backed up separately before the run. Failed or timed-out
runs commit any changes as `memory: dream FAILED (exit N) — review` without
advancing the last-successful-run ref. Review the result commit before reverting
it; use `revert HEAD` only if that result is still HEAD. See the README for
review and undo instructions.

The CLI requires Bash, `git`, `grep`, `awk`, `readlink`, and filesystem
symlink support; it has no server, database, or package-runtime dependency.

## Reading

At the start of work, read every `INDEX.md` that exists:
`./.memory/project/INDEX.md`, `./.memory/groups/*/INDEX.md`,
`./.memory/workspace/INDEX.md`. They are one line per fact — cheap.

If an `<agent-memory>` block with these indexes is already in your context
(the OpenCode plugin injects it into the prompt), do not read the
`INDEX.md` files again — go straight to the relevant fact files.

Open a fact's file only when its description is relevant to the task at hand.
As work touches a topic listed in an index, read that fact before acting.

Search tools do not see memory. `.memory/*` are symlinks and `.memory/` is in
the global gitignore, so `rg`, `grep -r`, and agent Grep/Glob tools return
nothing from it even when the text is there. Always read `INDEX.md` files by
path. For a full-text search use `memory grep <term>` (bundled CLI), or
`rg -L --no-ignore <term> .memory/` when the CLI is not available. `memory
status` shows which levels and groups this project has.

## Writing

Record a fact when you learn something that is (a) not derivable from the code
or git history and (b) useful in future sessions. Do not store what the repo
already documents.

Pick the level by who will benefit:

- only this codebase → `project/`
- several projects of a group (shared infra, shared APIs, team agreements) →
  `groups/<group>/`; with several groups linked, pick the one whose scope
  matches the fact
- the user or the whole environment (preferences, tools, machine setup) →
  `workspace/`

When a fact spans several linked projects, put the shared part in the group
and keep only the project-specific details (app names, endpoints, IDs) in the
project fact — do not repeat the shared block in every project. If two levels
disagree, the more specific level wins: project > group > workspace.

Never store secrets: tokens, passwords, connection strings with credentials,
or URLs that embed them. The store is a git repository that may be pushed.

Keep `description` under ~120 characters. Every description is loaded into
context at the start of every session, so long descriptions are paid for on
each run.

Before writing, scan that level's INDEX.md for an existing fact on the same
topic — update its file instead of creating a duplicate. Delete or fix facts
you discover to be stale.

### Fact format

One fact per file. File name: kebab-case slug, `[a-z0-9-]` only, `.md`.

```markdown
---
description: pgbouncer connection pool breaks above 50 connections
type: gotcha
created: 2026-07-07
updated: 2026-09-04
---

Above 50 connections, pgbouncer in transaction mode starts returning
stale prepared statements. Workaround: ...
```

`type` is one of: `gotcha` (footgun), `decision` (choice made and why),
`convention` (we do it this way), `context` (goals, constraints),
`reference` (links to dashboards, tickets, docs).

`updated` is optional: set it (today's date) whenever you change the body, so
a reader can tell how fresh the fact is. Keep `description` under ~120
characters — it is what every session loads.

Link related facts with `[[file-name]]` wiki links (cross-level allowed).

### After every write

Run `memory index` (the bundled CLI) — it rebuilds every `INDEX.md` from the
facts' frontmatter. If you cannot run commands, add or update the fact's line
in that level's `INDEX.md` by hand:

```markdown
- [pgbouncer-connection-limit](pgbouncer-connection-limit.md) — gotcha: pool breaks above 50 connections
```

(A plain `-` instead of `—` is accepted.) `memory doctor` reports drift
between indexes and files, and warns about over-long descriptions and
`[[links]]` that point nowhere.

### Housekeeping

When an index grows past ~40 lines, or you notice near-duplicate or stale
entries, use the `memory-dream` skill for this project, or suggest
`memory dream` (whole store, unattended) to the user. Task status ("still needs
staging validation") does not belong in memory — record the decision or gotcha
it produced, or nothing.
