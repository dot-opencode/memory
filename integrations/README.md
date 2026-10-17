# Integrations

`agent-memory.snippet.md` is the protocol in ~20 lines for agents that do not
load `SKILL.md`. Install it **globally** (this is a personal tool; the store is
per user) or per project.

| Agent | Global (recommended) | Per project |
|---|---|---|
| OpenCode | append to `~/.config/opencode/AGENTS.md` | append to `AGENTS.md` |
| Codex CLI | append to `~/.codex/AGENTS.md` | append to `AGENTS.md` |
| Cursor | paste into *Settings → Rules → User Rules* | copy `cursor/agent-memory.mdc` to `.cursor/rules/` |
| GitHub Copilot | — | append to `.github/copilot-instructions.md` |

OpenCode also discovers the skill itself from `~/.agents/skills/agent-memory`;
the snippet makes the read-at-start rule
unconditional instead of depending on the agent choosing to load the skill.

`cursor/agent-memory.mdc` must stay identical to the snippet below its
frontmatter — `cargo test` checks this. Regenerate it after editing the
snippet:

```bash
{ printf '%s\n' '---' 'description: Long-term agent memory protocol for projects that have a .memory/ directory' 'alwaysApply: true' '---'; cat integrations/agent-memory.snippet.md; } > integrations/cursor/agent-memory.mdc
```
