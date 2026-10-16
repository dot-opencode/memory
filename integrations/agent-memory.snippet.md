## Agent memory (`.memory/`)

If `./.memory/` exists, this project has long-term agent memory (the
agent-memory skill). Rules:

- At the start of a task read `./.memory/project/INDEX.md`,
  `./.memory/groups/*/INDEX.md` and `./.memory/workspace/INDEX.md` **by path**
  (one line per fact). Search tools do not see `.memory/` (gitignored
  symlinks): use `memory grep <term>` or `rg -L --no-ignore <term> .memory/`.
- Open a fact file only when its description is relevant to the task.
- When you learn something durable that is not derivable from the code or git
  history, write one markdown file per fact into the matching level —
  `project/` (this codebase), `groups/<g>/` (shared by the group's projects),
  `workspace/` (the user or the environment). Update an existing fact instead
  of adding a duplicate. Never store secrets.

  ```
  ---
  description: <one line, under 120 characters>
  type: gotcha | decision | convention | context | reference
  created: YYYY-MM-DD
  ---
  body
  ```

  Then run `memory index` (or add `- [slug](slug.md) — type: description`
  to that level's `INDEX.md`).
- To check memory (duplicates, stale facts), run `memory dream --report`.
  Suggest `memory dream` to the user for unattended whole-store cleanup; do not
  launch it autonomously, since it can take time and incur model costs.
  It edits facts but recommends manual deletion rather than deleting or renaming
  files itself.
- You never run `memory init` or `memory link-group` yourself — suggest them
  to the user. Full protocol: `skills/agent-memory/SKILL.md` in the
  agent-memory repository.
