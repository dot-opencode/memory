// Offline regression model, not an OS sandbox or a substitute for live probes.
// V2 whole-value wildcards include '/' and are case-sensitive. Permissions and
// policies are independent, ordered layers. external_directory is an additional
// check for outside-cwd paths, including searches whose resources aren't paths.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const STORE = "/Users/u/.agent-memory";
const HOME = "/Users/u";
const BIN = "/opt/agent-memory/bin/memory";
const PROJECTS = ["/Users/u/work/app", "/Users/u/work/other"];
const template = readFileSync(new URL("../skills/agent-memory/dream/opencode.json", import.meta.url), "utf8");
const schema = JSON.parse(readFileSync(new URL("./fixtures/opencode-config.schema.json", import.meta.url), "utf8"));

function expand(web = false, projects = PROJECTS) {
  return JSON.parse(template.split("\n").flatMap(line => {
    if (line.includes("__WEB_ONLY__")) return web ? [line.replaceAll("__WEB_ONLY__", "")] : [];
    if (line.includes("__PROJECT_DIR__")) return projects.map(p => line.replaceAll("__PROJECT_DIR__", p));
    return [line];
  }).join("\n").replaceAll("__STORE__", STORE).replaceAll("__HOME__", HOME).replaceAll("__MEMORY_BIN__", BIN));
}

function glob(pattern, value) {
  const re = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".");
  return new RegExp(`^${re}$`, "s").test(value)
    || (pattern.endsWith(" *") && new RegExp(`^${re.slice(0, -3)}$`, "s").test(value));
}
function decide(rules, action, resource) {
  let effect = "deny";
  for (const r of rules) if (glob(r.action, action) && glob(r.resource, resource)) effect = r.effect;
  return effect === "allow";
}
function allowed(config, agent, action, resource) {
  return decide(config.policies, "tool.use", `${action}:${resource}`)
    && decide(config.agents[agent].permissions, action, resource);
}
const config = expand();
const dream = (action, resource, c = config) => allowed(c, "memory-dream", action, resource);
const check = (action, resource) => allowed(config, "memory-dream-check", action, resource);

// Dependency-free, deliberately minimal schema validation: enforce the official
// policy statement schema and permission rule shape/enums, not all JSON Schema.
function validate(c) {
  const statement = schema.properties.policies.items;
  assert.equal(statement.additionalProperties, false);
  for (const r of c.policies) {
    assert.deepEqual(Object.keys(r).sort(), statement.required.toSorted());
    assert.ok(statement.properties.action.enum.includes(r.action), r.action);
    assert.ok(statement.properties.effect.enum.includes(r.effect), r.effect);
    assert.equal(typeof r.resource, "string");
  }
  for (const agent of Object.values(c.agents)) for (const r of agent.permissions) {
    assert.deepEqual(Object.keys(r).sort(), ["action", "effect", "resource"]);
    assert.equal(typeof r.action, "string");
    assert.equal(typeof r.resource, "string");
    assert.ok(["allow", "deny"].includes(r.effect));
  }
}

test("official policy schema, both optional-line contracts and JSON validity", () => {
  validate(JSON.parse(template));
  for (const web of [false, true]) for (const dirs of [[], PROJECTS]) {
    const c = expand(web, dirs);
    validate(c);
    assert.ok(!JSON.stringify(c).includes("__"));
    assert.equal(dream("webfetch", "https://example.com", c), web);
    assert.equal(dream("websearch", "reference", c), web);
    for (const p of PROJECTS) assert.equal(dream("read", `${p}/main.py`, c), dirs.includes(p));
  }
  for (const line of template.split("\n").filter(l => /__WEB_ONLY__|__PROJECT_DIR__/.test(l))) {
    assert.ok(line.trim().endsWith(","), "optional statements must not be last in arrays");
    assert.equal((line.match(/"action"/g) || []).length, 1);
  }
  const broken = structuredClone(config);
  broken.policies[0].action = "permission";
  assert.throws(() => validate(broken));
});

test("case-sensitive matcher; no legacy rtk or unrestricted git rules", () => {
  assert.equal(glob("read:*", "READ:x"), false);
  assert.equal(glob("*", "a/b"), true);
  assert.equal(glob("git status *", "git status"), true);
  assert.ok(!template.includes("rtk "));
  assert.ok(!dream("shell", `git -C /some/other/repo show HEAD:secret`));
  assert.ok(!dream("shell", `rtk git -C ${STORE} status`));
  assert.ok(!dream("shell", "readlink /etc"));
});

test("directory allowlist, external_directory gate and secrets", () => {
  for (const agent of [dream, check]) {
    for (const p of [`${STORE}/work/projects/a/x.md`, `${PROJECTS[0]}/main.py`, `${PROJECTS[1]}/README.md`]) {
      assert.ok(agent("read", p), p);
      assert.ok(agent("external_directory", p), p);
    }
    for (const p of ["/etc/hosts", `${HOME}/.ssh/id_ed25519`, `${HOME}/.aws/credentials`, "/some/other/repo/.env", `${PROJECTS[0]}-evil/x`, `${PROJECTS[0]}/.env`, `${PROJECTS[0]}/.env.production`, `${STORE}/../.ssh/id_ed25519`])
      assert.ok(!agent("read", p), p);
    assert.ok(!agent("external_directory", "/etc/*"));
    // Search expression resources cannot constrain the path. Fail closed.
    for (const resource of [".*", `${STORE}/*`, `${HOME}/.ssh/*`, `${PROJECTS[0]}/*`]) {
      assert.ok(!agent("grep", resource));
      assert.ok(!agent("glob", resource));
    }
  }
});

// OpenCode evaluates read/edit resources as the path the agent passed: relative to the
// cwd (the store) when relative, and it also rewrites in-store absolute paths to relative
// ones (observed live in v2.0.31). Every rule must therefore be safe in both forms.
const both = rel => [rel, `${STORE}/${rel}`];

test("fact markdown read/edit works for relative and absolute store paths", () => {
  for (const f of ["work/projects/a/x.md", "work/groups/g/x.md", "work/workspace/x.md"])
    for (const p of both(f)) {
      assert.ok(dream("edit", p), `edit ${p}`);
      assert.ok(dream("read", p), `read ${p}`);
      assert.ok(!check("edit", p), `checker edit ${p}`);
      assert.ok(check("read", p), `checker read ${p}`);
    }
  for (const p of [".", "work", "work/projects/a", STORE, `${STORE}/work/projects`]) {
    assert.ok(dream("read", p), `read dir ${p}`);
    assert.ok(dream("external_directory", p) || !p.startsWith("/") , p);
  }
});

test("only fact markdown edits; nested, hidden, reserved and escaping paths denied in both forms", () => {
  for (const f of [
    "work/projects/a/.hidden.md", "work/projects/.a/x.md", "work/projects/a/sub/nested.md", "work/groups/g/sub/nested.md",
    "work/workspace/sub/deep.md", "work/workspace/.dot.md", "work/projects/a/x.js", "work/projects/a/UPPER.MD",
    "work/projects/a/x.md/y", "work/projects/a/x.md/AGENTS.md",
    "opencode.json", ".opencode/plugins/x.js", "AGENTS.md", ".git/config", ".git/hooks/pre-commit", ".project-path",
    "work/AGENTS.md", "work/opencode.json", "work/.opencode/plugins/x.md", "work/.git/x.md", "work/.project-path",
    "work/projects/a/AGENTS.md", "work/projects/a/INDEX.md", "work/projects/a/INDEX.md/x", "work/projects/a/.project-path",
    "work/projects/a/.opencode/plugins/x.md", "work/projects/a/.git/hooks/x.md", "work/workspace/INDEX.md",
    "work/groups/g/AGENTS.md", "INDEX.md", "work/INDEX.md", "work/projects/a/../../AGENTS.md", "work/workspace/../../escape.md",
    "../escape.md",
  ]) for (const p of both(f)) assert.ok(!dream("edit", p), `must deny edit ${p}`);
  for (const p of ["/tmp/work/projects/a/x.md", `${HOME}/work/projects/a/x.md`, `${PROJECTS[0]}/projects/a/x.md`, `${PROJECTS[0]}/fact.md`, `${STORE}-evil/work/projects/a/x.md`])
    assert.ok(!dream("edit", p), `must deny edit ${p}`);
});

test("residual: the matcher cannot stop a new project or group directory (documented)", () => {
  // `*` crosses `/`, so a fresh <project>/<slug>.md is indistinguishable from an existing one.
  for (const p of both("work/projects/newproj/fresh.md")) assert.ok(dream("edit", p));
});

test("shell status and report are not blocked by any deny rule", () => {
  for (const agent of ["memory-dream", "memory-dream-check"]) {
    const rules = config.agents[agent].permissions;
    const cmds = [`git -C ${STORE} status`, `git -C ${STORE} status --short`, `git -C ${PROJECTS[0]} status`, `git -C ${PROJECTS[1]} status --short`];
    if (agent === "memory-dream") cmds.push(`${BIN} dream --report`, `${BIN} dream --report --workspace work`);
    for (const cmd of cmds) {
      assert.ok(decide(rules, "shell", cmd), `${agent}: ${cmd}`);
      const lastAllow = rules.map((r, i) => [r, i]).filter(([r]) => r.effect === "allow" && glob(r.action, "shell") && glob(r.resource, cmd)).pop()[1];
      const laterDeny = rules.slice(lastAllow + 1).filter(r => r.effect === "deny" && glob(r.action, "shell") && glob(r.resource, cmd));
      assert.deepEqual(laterDeny, [], cmd);
    }
  }
  // A lone-backslash deny matched every command in OpenCode 2.0.31; it must not come back.
  for (const rules of [config.policies, ...Object.values(config.agents).map(a => a.permissions)])
    assert.ok(!rules.some(r => /(^|:)\*\\\\\*$/.test(r.resource) || r.resource === "shell:*\\*"), "no *\\* shell deny");
  assert.ok(!dream("shell", `${BIN} dream --report a;b`));
  assert.ok(!dream("shell", `${BIN} dream --report $HOME`));
});

test("report and exact scoped git status only; unsafe wildcard mutations fail closed", () => {
  for (const cmd of [`${BIN} dream --report`, `${BIN} dream --report --workspace work`, `git -C ${STORE} status`, `git -C ${STORE} status --short`])
    assert.ok(dream("shell", cmd), cmd);
  for (const cmd of [`git -C ${STORE} rm -r work`, `git -C ${STORE} rm work/projects/a/x.md`, `git -C ${STORE} mv work/projects/a/x.md work/projects/a/y.md`, `git -C ${STORE} add .`, `git -C ${STORE} log --output=/etc/x`, `git -C ${STORE} diff --no-index /etc/hosts /etc/passwd`, `git -C ${STORE} log --ext-diff`, `git -C ${STORE} log --textconv`, `git -C ${STORE} status; echo x`, `git -C ${STORE} status\necho x`, `git -C ${STORE} status | sh`, `git -C ${STORE} status > x`, `git -C ${STORE} log $(echo x)`, `git -C ${STORE} log -c x`, `git -C ${STORE} log --git-dir=/elsewhere`, `git -C ${STORE} -C /elsewhere show`, `${BIN} init work`, "curl https://example.com"]) assert.ok(!dream("shell", cmd), cmd);
});

test("Git cannot bypass read denials for either agent or any registered root", () => {
  for (const agent of [dream, check]) for (const root of [STORE, ...PROJECTS]) {
    assert.ok(agent("shell", `git -C ${root} status`));
    assert.ok(agent("shell", `git -C ${root} status --short`));
    for (const args of [
      `blame --contents=${root}/private.env -- README.md`,
      "show HEAD:private.env",
      "log -p -- private.env",
      "diff -- private.env",
      "diff --no-index /etc/hosts /etc/passwd",
      "blame README.md", "show", "log --oneline -5", "diff",
      "rev-list --all", "ls-tree HEAD",
      "status --short -- private.env", "status --porcelain", "status --ignored",
      "status --short --untracked-files=all", "status -c x=y",
      "status --short; echo x", "status --short\necho x",
    ]) assert.ok(!agent("shell", `git -C ${root} ${args}`), args);
  }
  for (const rules of [config.policies, ...Object.values(config.agents).map(a => a.permissions)]) {
    for (const r of rules.filter(r => r.effect === "allow" && r.resource.includes("git -C "))) {
      assert.ok(/ status(?: --short)?$/.test(r.resource), r.resource);
      assert.ok(!r.resource.includes("*"), r.resource);
    }
  }
});

test("both layers agree and checker cannot mutate or delegate", () => {
  for (const r of config.agents["memory-dream"].permissions) {
    const resource = r.action === "*" ? "*" : `${r.action}:${r.resource}`;
    assert.ok(config.policies.some(p => p.action === "tool.use" && p.resource === resource && p.effect === r.effect));
  }
  assert.ok(dream("subagent", "memory-dream-check"));
  assert.ok(!check("subagent", "memory-dream-check"));
  assert.ok(!check("edit", `${STORE}/work/projects/a/x.md`));
  assert.ok(!check("shell", `${BIN} dream --report`));
  assert.ok(check("shell", `git -C ${PROJECTS[0]} status --short`));
  for (const resource of ["mcp:fake", "plugin:rtk"]) assert.ok(!decide(config.policies, "integration.use", resource));
  for (const action of ["question", "skill", "execute", "some_mcp_tool"])
    assert.ok(!dream(action, "*"));
});
