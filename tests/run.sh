#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Test the script the skill actually ships; the root bin/memory is only a convenience symlink.
MEMORY="$ROOT/skills/agent-memory/bin/memory"
PASS=0; FAIL=0

setup() {
  set -e
  TMP="$(mktemp -d)"
  export AGENT_MEMORY_HOME="$TMP/store"
  export HOME="$TMP/home"                 # isolates global git config too
  unset XDG_CONFIG_HOME                   # CI runners set it; config must follow $HOME
  export AGENT_MEMORY_BACKUP_LOCK_WAIT=0
  unset AGENT_MEMORY_DREAM_MODEL          # a user's shell default must not leak into the dream tests
  export GIT_CONFIG_NOSYSTEM=1
  mkdir -p "$HOME"
  git config --global user.email "test@example.com"
  git config --global user.name "Test"
  git config --global init.defaultBranch main
  PROJ="$TMP/code/some-api"
  mkdir -p "$PROJ"
  cd "$PROJ"
}

fail() { echo "  assertion failed: $*"; exit 1; }
assert_eq() { [ "$1" = "$2" ] || fail "expected '$2', got '$1'"; }
assert_contains() { case "$1" in *"$2"*) ;; *) fail "no '$2' in: $1" ;; esac; }
assert_link() {
  [ -L "$1" ] || fail "$1 is not a symlink"
  local expected
  expected="$(cd -P "$(dirname "$2")" && pwd -P)/$(basename "$2")"
  assert_eq "$(readlink "$1")" "$expected"
}
assert_file() { [ -f "$1" ] || fail "missing file: $1"; }

extract_cursor_body() {   # extract_cursor_body <cursor-rule> <output>
  local body_line
  body_line="$(awk 'NR == 1 && $0 == "---" { skip = 1; next } skip && /^---$/ { print NR + 1; exit }' "$1")"
  [ -n "$body_line" ] || return 1
  tail -n +"$body_line" "$1" > "$2"
}

test_integration_snippets_exist_and_agree() {
  local s="$ROOT/integrations/agent-memory.snippet.md" c="$ROOT/integrations/cursor/agent-memory.mdc" body="$TMP/cursor-body.md" key
  assert_file "$s"; assert_file "$c"; assert_file "$ROOT/integrations/README.md"
  grep -qF 'OpenCode V2' "$ROOT/README.md" || fail "README does not identify the V2 plugin integration"
  grep -qF 'restart OpenCode' "$ROOT/README.md" || fail "README lacks the required restart instruction"
  # shellcheck disable=SC2016 # The backticks are literal snippet test data.
  for key in '.memory/project/INDEX.md' '.memory/groups/*/INDEX.md' '.memory/workspace/INDEX.md' \
             'memory index' 'memory grep' 'never run `memory init`' 'Never store secrets' 'description:' 'type:'; do
    grep -qF -- "$key" "$s" || fail "snippet lacks: $key"
  done
  assert_eq "$(head -1 "$c")" "---"
  grep -q '^alwaysApply: true$' "$c" || fail "cursor rule is not alwaysApply"
  extract_cursor_body "$c" "$body" || fail "could not extract cursor rule body"
  cmp -s "$body" "$s" || fail "cursor rule body differs from snippet"
}

test_integration_cursor_body_detects_final_newline_drift() {
  local s="$TMP/snippet.md" c="$TMP/cursor.mdc" body="$TMP/cursor-body.md"
  printf 'body\n' > "$s"
  printf '%s\n' '---' 'description: fixture' 'alwaysApply: true' '---' 'body' '' > "$c"
  extract_cursor_body "$c" "$body"
  if cmp -s "$body" "$s"; then fail "cursor body comparison ignored final-newline drift"; fi
}

write_fact_only() {   # write_fact_only <level-dir> <slug> <type> <description>  (no INDEX line)
  cat > "$1/$2.md" <<EOF
---
description: $4
type: $3
created: 2026-09-04
---
body of $2
EOF
}

write_fact() {   # write_fact <level-dir> <slug> <type> <description>
  cat > "$1/$2.md" <<EOF
---
description: $4
type: $3
created: 2026-09-04
---
body of $2
EOF
  printf -- '- [%s](%s.md) — %s: %s\n' "$2" "$2" "$3" "$4" >> "$1/INDEX.md"
}

fill_index() {   # fill_index <INDEX.md> <count>
  local i
  for i in $(seq 1 "$2"); do printf -- '- [f%03d](f%03d.md) — gotcha: entry %d\n' "$i" "$i" "$i" >> "$1"; done
}

run_test() {
  # Run setup and the test together, outside an `if` condition, so errexit is
  # active for both.  The subshell keeps its cwd and temporary environment.
  (
    setup
    trap 'cd /; rm -rf "$TMP"' EXIT
    "$1"
  )
  local rc=$?
  if [ "$rc" -eq 0 ]; then PASS=$((PASS+1)); echo "ok - $1"
  else FAIL=$((FAIL+1)); echo "FAIL - $1"; fi
}

# ---------------- tests ----------------

test_backup_explains_commit_failure() {
  "$MEMORY" init work >/dev/null
  write_fact .memory/project a-fact context "something learned"
  git config --global --unset user.email
  git config --global user.useConfigOnly true      # make git refuse to guess an identity
  local out rc=0
  out="$("$MEMORY" backup 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "user.email"
}

test_usage_without_args() {
  local out rc=0
  out="$("$MEMORY" 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage:"
}

test_unknown_command() {
  local out rc=0
  out="$("$MEMORY" frobnicate 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unknown command"
}

test_version_flag() {
  local out rc=0
  out="$("$MEMORY" --version)"
  assert_eq "$out" "memory 0.1.0"
  out="$("$MEMORY" -v)"
  assert_eq "$out" "memory 0.1.0"
  out="$("$MEMORY" --version extra 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory --version"
}

test_init_creates_structure() {
  local out
  out="$("$MEMORY" init work 2>&1)"
  assert_contains "$out" "initialized"
  assert_link .memory/project "$AGENT_MEMORY_HOME/work/projects/some-api"
  assert_link .memory/workspace "$AGENT_MEMORY_HOME/work/workspace"
  [ -d .memory/groups ] || fail "missing .memory/groups"
  assert_file "$AGENT_MEMORY_HOME/work/projects/some-api/INDEX.md"
  assert_file "$AGENT_MEMORY_HOME/work/workspace/INDEX.md"
  assert_eq "$(cat "$AGENT_MEMORY_HOME/work/projects/some-api/.project-path")" "$(cd -P "$PROJ" && pwd -P)"
  [ -d "$AGENT_MEMORY_HOME/.git" ] || fail "store is not a git repo"
}

test_init_respects_name_flag() {
  "$MEMORY" init work --name custom-name >/dev/null
  assert_link .memory/project "$AGENT_MEMORY_HOME/work/projects/custom-name"
}

test_rejects_path_traversal_names() {
  local out rc=0
  out="$("$MEMORY" init ../evil 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid workspace name"
  "$MEMORY" init work >/dev/null
  rc=0
  out="$("$MEMORY" link-group ../../x 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid group name"
}

test_relative_memory_home_creates_absolute_links() {
  export AGENT_MEMORY_HOME="relative-store"
  "$MEMORY" init work >/dev/null
  assert_link .memory/project "$PROJ/relative-store/work/projects/some-api"
  assert_link .memory/workspace "$PROJ/relative-store/work/workspace"
}

test_init_adds_global_gitignore() {
  "$MEMORY" init work >/dev/null
  local f
  f="$(git config --global core.excludesfile)"
  grep -qxF '.memory/' "$f" || fail "global gitignore lacks .memory/"
}

test_init_without_arg_lists_workspaces() {
  "$MEMORY" init work >/dev/null
  local out rc=0
  out="$("$MEMORY" init 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "work"
  assert_contains "$out" "usage"
}

test_init_is_idempotent_and_repairs() {
  "$MEMORY" init work >/dev/null
  rm .memory/project                       # simulate a broken/deleted symlink
  "$MEMORY" init work >/dev/null
  assert_link .memory/project "$AGENT_MEMORY_HOME/work/projects/some-api"
}

test_init_name_collision() {
  "$MEMORY" init work >/dev/null
  local other="$TMP/elsewhere/some-api"
  mkdir -p "$other"; cd "$other"
  local out rc=0
  out="$("$MEMORY" init work 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "--name"
  "$MEMORY" init work --name some-api-2 >/dev/null
  assert_link .memory/project "$AGENT_MEMORY_HOME/work/projects/some-api-2"
}

test_reinit_after_rename_keeps_binding() {
  "$MEMORY" init work >/dev/null
  cd "$TMP"; mv "$PROJ" "$TMP/code/renamed-api"; cd "$TMP/code/renamed-api"
  "$MEMORY" init work >/dev/null
  # still bound to the original store dir, no second dir created
  assert_link .memory/project "$AGENT_MEMORY_HOME/work/projects/some-api"
  [ ! -d "$AGENT_MEMORY_HOME/work/projects/renamed-api" ] \
    || fail "created a duplicate store dir"
  # backlink updated to the new location
  assert_eq "$(cat .memory/project/.project-path)" "$(pwd -P)"
}

test_init_via_symlinked_path_is_not_a_collision() {
  "$MEMORY" init work >/dev/null
  ln -s "$TMP/code" "$TMP/alias"
  cd "$TMP/alias/some-api"
  "$MEMORY" init work --name some-api >/dev/null
  assert_link .memory/project "$AGENT_MEMORY_HOME/work/projects/some-api"
}

test_init_rebinds_when_recorded_path_does_not_exist_here() {
  mkdir -p "$AGENT_MEMORY_HOME/work/projects/some-api"
  : > "$AGENT_MEMORY_HOME/work/projects/some-api/INDEX.md"
  printf '%s\n' "/home/other-machine/some-api" > "$AGENT_MEMORY_HOME/work/projects/some-api/.project-path"
  "$MEMORY" init work >/dev/null
  assert_link .memory/project "$AGENT_MEMORY_HOME/work/projects/some-api"
  assert_eq "$(cat .memory/project/.project-path)" "$(pwd -P)"
}

test_reinit_with_different_workspace_is_rejected() {
  "$MEMORY" init work >/dev/null
  local out rc=0
  out="$("$MEMORY" init home 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unlink/rebind"
}

test_link_group() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  assert_link .memory/groups/spines "$AGENT_MEMORY_HOME/work/groups/spines"
  assert_file "$AGENT_MEMORY_HOME/work/groups/spines/INDEX.md"
}

test_link_multiple_groups() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  "$MEMORY" link-group python-services >/dev/null
  assert_link .memory/groups/spines "$AGENT_MEMORY_HOME/work/groups/spines"
  assert_link .memory/groups/python-services "$AGENT_MEMORY_HOME/work/groups/python-services"
}

test_link_group_requires_init() {
  local out rc=0
  out="$("$MEMORY" link-group spines 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "memory init"
}

test_unlink_group() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  "$MEMORY" unlink-group spines >/dev/null
  [ ! -e .memory/groups/spines ] || fail "symlink still present"
  [ -d "$AGENT_MEMORY_HOME/work/groups/spines" ] || fail "group memory was deleted"
}

test_unlink_group_not_linked() {
  "$MEMORY" init work >/dev/null
  local out rc=0
  out="$("$MEMORY" unlink-group nope 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "not linked"
}

test_doctor_ok() {
  "$MEMORY" init work >/dev/null
  local out
  out="$("$MEMORY" doctor)"
  assert_contains "$out" "OK"
}

test_doctor_broken_symlink() {
  "$MEMORY" init work >/dev/null
  rm -rf "$AGENT_MEMORY_HOME/work/projects/some-api"
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "BROKEN"
}

test_doctor_flags_plain_project_or_workspace() {
  "$MEMORY" init work >/dev/null
  rm .memory/project
  mkdir .memory/project
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "NOT A SYMLINK: .memory/project"
}

test_doctor_without_memory_dir_says_so() {
  local out
  out="$("$MEMORY" doctor)"            # exit 0: not an error state
  assert_contains "$out" "no .memory/"
  case "$out" in *OK*) fail "doctor printed OK with nothing to check" ;; esac
}

test_doctor_no_valid_levels_does_not_crash() {
  "$MEMORY" init work >/dev/null
  rm .memory/project .memory/workspace
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "MISSING LINK: .memory/project"
  assert_contains "$out" "MISSING LINK: .memory/workspace"
  case "$out" in *"unbound variable"*) fail "doctor crashed: $out" ;; esac
}

test_doctor_store_missing() {
  "$MEMORY" init work >/dev/null
  rm -rf "$AGENT_MEMORY_HOME"
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "STORE MISSING: "
  case "$out" in *"UNSAFE"*) fail "missing store misreported as unsafe: $out" ;; esac
}

test_doctor_unsafe_symlink_outside_store() {
  "$MEMORY" init work >/dev/null
  mkdir -p "$TMP/outside"
  rm .memory/project
  ln -s "$TMP/outside" .memory/project
  local out rc=0 store
  store="$(cd -P "$AGENT_MEMORY_HOME" && pwd -P)"
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "UNSAFE SYMLINK: .memory/project"
  assert_contains "$out" "(outside $store)"
}

test_doctor_relative_symlink_is_unsafe() {
  "$MEMORY" init work >/dev/null
  rm .memory/project
  ln -s ../somewhere .memory/project
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "UNSAFE SYMLINK: .memory/project"
  assert_contains "$out" "relative"
}

test_doctor_file_not_in_index() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/some-fact.md <<'EOF'
---
description: a fact
type: gotcha
created: 2026-07-07
---
body
EOF
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "NOT IN INDEX"
}

test_doctor_index_entry_without_file() {
  "$MEMORY" init work >/dev/null
  echo "- [ghost](ghost.md) — gotcha: long gone" > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "MISSING FILE"
}

test_doctor_flags_malformed_index_entry() {
  "$MEMORY" init work >/dev/null
  echo "this is not an index entry" > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "MALFORMED INDEX ENTRY"
}

test_doctor_accepts_hyphen_and_en_dash_in_index() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact gotcha "hyphen line"
  write_fact_only .memory/project b-fact gotcha "en dash line"
  printf -- '- [a-fact](a-fact.md) - gotcha: hyphen line\n' > .memory/project/INDEX.md
  printf -- '- [b-fact](b-fact.md) – gotcha: en dash line\n' >> .memory/project/INDEX.md
  local out
  out="$("$MEMORY" doctor)"
  assert_contains "$out" "OK"
}

test_doctor_accepts_valid_updated_field() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/a-fact.md <<'EOF'
---
description: has an updated date
type: decision
created: 2026-07-01
updated: 2026-09-04
---
body
EOF
  printf -- '- [a-fact](a-fact.md) — decision: has an updated date\n' > .memory/project/INDEX.md
  assert_contains "$("$MEMORY" doctor)" "OK"
}

test_doctor_flags_updated_before_created() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/a-fact.md <<'EOF'
---
description: time travel
type: decision
created: 2026-09-04
updated: 2026-01-01
---
body
EOF
  printf -- '- [a-fact](a-fact.md) — decision: time travel\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "updated 2026-01-01 is before created 2026-09-04"
}

test_doctor_flags_invalid_updated_format() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/a-fact.md <<'EOF'
---
description: bad updated
type: decision
created: 2026-09-04
updated: yesterday
---
body
EOF
  printf -- '- [a-fact](a-fact.md) — decision: bad updated\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid updated date"
}

test_doctor_flags_crlf_frontmatter() {
  "$MEMORY" init work >/dev/null
  printf -- '---\r\ndescription: windows file\r\ntype: gotcha\r\ncreated: 2026-09-04\r\n---\r\nbody\r\n' > .memory/project/crlf.md
  printf -- '- [crlf](crlf.md) — gotcha: windows file\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "CRLF"
}

test_index_generates_sorted_index_from_frontmatter() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project zeta-fact gotcha "last by slug"
  write_fact_only .memory/project alpha-fact decision "first by slug"
  local out
  out="$("$MEMORY" index)"
  assert_contains "$out" "indexed .memory/project (2 facts)"
  assert_eq "$(cat .memory/project/INDEX.md)" "$(printf -- '- [alpha-fact](alpha-fact.md) — decision: first by slug\n- [zeta-fact](zeta-fact.md) — gotcha: last by slug')"
  assert_contains "$("$MEMORY" doctor)" "OK"
}

test_index_covers_groups_and_workspace() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  write_fact_only .memory/groups/spines shared convention "shared thing"
  write_fact_only .memory/workspace editor context "user prefers vim"
  "$MEMORY" index >/dev/null
  assert_contains "$(cat .memory/groups/spines/INDEX.md)" "[shared](shared.md) — convention: shared thing"
  assert_contains "$(cat .memory/workspace/INDEX.md)" "[editor](editor.md) — context: user prefers vim"
}

test_index_skips_unusable_facts_and_doctor_still_reports_them() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project good gotcha "fine"
  echo "no frontmatter at all" > .memory/project/naked.md
  local out rc=0
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 0
  assert_contains "$out" "skipping .memory/project/naked.md"
  assert_eq "$(grep -c . .memory/project/INDEX.md)" 1
  rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "NOT IN INDEX: .memory/project/naked.md"
}

test_index_requires_memory_dir() {
  local out rc=0
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "no .memory/"
}

test_index_rejects_unsafe_project_link_without_writing_external_store() {
  "$MEMORY" init work >/dev/null
  local outside="$TMP/outside" out rc=0
  mkdir -p "$outside"
  printf 'external sentinel\n' > "$outside/INDEX.md"
  rm .memory/project
  ln -s "$outside" .memory/project
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" ".memory/project"
  assert_eq "$(cat "$outside/INDEX.md")" "external sentinel"
}

test_index_does_not_follow_preexisting_temp_symlink() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project safe gotcha "safe fact"
  local outside="$TMP/outside"
  mkdir -p "$outside"
  printf 'external sentinel\n' > "$outside/sentinel"
  ln -s "$outside/sentinel" .memory/project/INDEX.md.tmp
  "$MEMORY" index >/dev/null
  assert_eq "$(cat "$outside/sentinel")" "external sentinel"
}

test_index_skips_invalid_frontmatter_type_and_created_values() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project good gotcha "good fact"
  write_fact_only .memory/project bad-type banana "bad type"
  cat > .memory/project/bad-created.md <<'EOF'
---
description: bad date
type: gotcha
created: today
---
body
EOF
  printf 'no frontmatter\n' > .memory/project/no-frontmatter.md
  local out
  out="$("$MEMORY" index 2>&1)"
  assert_contains "$out" "skipping .memory/project/bad-type.md"
  assert_contains "$out" "skipping .memory/project/bad-created.md"
  assert_contains "$out" "skipping .memory/project/no-frontmatter.md"
  assert_eq "$(cat .memory/project/INDEX.md)" "$(printf -- '- [good](good.md) — gotcha: good fact')"
}

test_index_skips_mixed_crlf_frontmatter() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project good gotcha "good fact"
  printf -- '---\ndescription: windows fact\r\ntype: gotcha\ncreated: 2026-09-04\n---\nbody\n' > .memory/project/mixed-crlf.md
  local out
  out="$("$MEMORY" index 2>&1)"
  assert_contains "$out" "skipping .memory/project/mixed-crlf.md"
  assert_eq "$(cat .memory/project/INDEX.md)" "$(printf -- '- [good](good.md) — gotcha: good fact')"
}

test_doctor_and_index_reject_duplicate_updated_fields() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/a-fact.md <<'EOF'
---
description: duplicate updated
type: decision
created: 2026-09-04
updated: yesterday
updated: 2026-09-04
---
body
EOF
  printf -- '- [a-fact](a-fact.md) — decision: duplicate updated\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid updated date"
  out="$("$MEMORY" index 2>&1)"
  assert_contains "$out" "skipping .memory/project/a-fact.md"
  assert_eq "$(grep -c . .memory/project/INDEX.md || true)" 0
}

test_doctor_checks_exact_index_membership() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact gotcha "missing from index"
  write_fact_only .memory/project other gotcha "mentions ](a-fact.md)"
  printf -- '- [other](other.md) — gotcha: mentions ](a-fact.md)\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "NOT IN INDEX: .memory/project/a-fact.md"
}

test_index_fails_when_no_usable_linked_level_exists() {
  mkdir -p "$AGENT_MEMORY_HOME" .memory/groups
  local out rc=0
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "no usable linked levels"
}

test_index_sort_failure_preserves_existing_indexes() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact gotcha "a fact"
  printf 'project sentinel\n' > .memory/project/INDEX.md
  printf 'workspace sentinel\n' > .memory/workspace/INDEX.md
  mkdir -p "$TMP/fake-bin"
  cat > "$TMP/fake-bin/sort" <<'EOF'
#!/bin/sh
exit 42
EOF
  chmod +x "$TMP/fake-bin/sort"
  local out rc=0
  out="$(PATH="$TMP/fake-bin:$PATH" "$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "failed to render index"
  assert_eq "$(cat .memory/project/INDEX.md)" "project sentinel"
  assert_eq "$(cat .memory/workspace/INDEX.md)" "workspace sentinel"
}

test_index_broken_required_and_group_links_fail_without_publishing() {
  "$MEMORY" init work >/dev/null
  printf 'project sentinel\n' > .memory/project/INDEX.md
  rm .memory/workspace
  ln -s "$AGENT_MEMORY_HOME/work/missing-workspace" .memory/workspace
  local out rc=0
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "broken required memory link"
  assert_eq "$(cat .memory/project/INDEX.md)" "project sentinel"

  ln -sfn "$AGENT_MEMORY_HOME/work/workspace" .memory/workspace
  "$MEMORY" link-group spines >/dev/null
  rm .memory/groups/spines
  ln -s "$AGENT_MEMORY_HOME/work/groups/missing-spines" .memory/groups/spines
  rc=0
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "broken group memory link"
  assert_eq "$(cat .memory/project/INDEX.md)" "project sentinel"
}

test_index_rejects_directory_destinations_before_any_publish() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact gotcha "a fact"
  printf 'project sentinel\n' > .memory/project/INDEX.md
  rm .memory/workspace/INDEX.md
  mkdir .memory/workspace/INDEX.md
  local out rc=0
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid index destination"
  assert_eq "$(cat .memory/project/INDEX.md)" "project sentinel"
  rmdir .memory/workspace/INDEX.md

  local outside="$TMP/outside-index"
  mkdir "$outside"
  ln -s "$outside" .memory/workspace/INDEX.md
  rc=0
  out="$("$MEMORY" index 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid index destination"
  assert_eq "$(cat .memory/project/INDEX.md)" "project sentinel"
  rm .memory/workspace/INDEX.md
  rmdir "$outside"
}

test_doctor_and_index_reject_duplicate_description() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/duplicate.md <<'EOF'
---
description:
description: valid description
type: gotcha
created: 2026-09-04
---
body
EOF
  printf -- '- [duplicate](duplicate.md) — gotcha: valid description\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "missing or duplicate description"
  out="$("$MEMORY" index 2>&1)"
  assert_contains "$out" "skipping .memory/project/duplicate.md"
  assert_eq "$(grep -c . .memory/project/INDEX.md || true)" 0
}

test_doctor_and_index_reject_duplicate_type() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/duplicate.md <<'EOF'
---
description: valid description
type: banana
type: gotcha
created: 2026-09-04
---
body
EOF
  printf -- '- [duplicate](duplicate.md) — gotcha: valid description\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "missing or invalid type"
  out="$("$MEMORY" index 2>&1)"
  assert_contains "$out" "skipping .memory/project/duplicate.md"
  assert_eq "$(grep -c . .memory/project/INDEX.md || true)" 0
}

test_doctor_and_index_reject_duplicate_created() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/duplicate.md <<'EOF'
---
description: valid description
type: gotcha
created: yesterday
created: 2026-09-04
---
body
EOF
  printf -- '- [duplicate](duplicate.md) — gotcha: valid description\n' > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "missing or invalid created date"
  out="$("$MEMORY" index 2>&1)"
  assert_contains "$out" "skipping .memory/project/duplicate.md"
  assert_eq "$(grep -c . .memory/project/INDEX.md || true)" 0
}

test_doctor_and_index_reject_invalid_calendar_dates() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/bad-created.md <<'EOF'
---
description: non-leap day
type: gotcha
created: 2026-02-29
---
body
EOF
  cat > .memory/project/bad-month.md <<'EOF'
---
description: bad month
type: gotcha
created: 2026-13-01
---
body
EOF
  cat > .memory/project/bad-updated.md <<'EOF'
---
description: bad updated day
type: gotcha
created: 2024-02-29
updated: 2025-02-29
---
body
EOF
  cat > .memory/project/leap-day.md <<'EOF'
---
description: valid leap day
type: gotcha
created: 2024-02-29
updated: 2024-02-29
---
body
EOF
  "$MEMORY" index >/dev/null
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid created date"
  assert_contains "$out" "invalid updated date"
  out="$("$MEMORY" index 2>&1)"
  assert_contains "$out" "skipping .memory/project/bad-created.md"
  assert_contains "$out" "skipping .memory/project/bad-month.md"
  assert_contains "$out" "skipping .memory/project/bad-updated.md"
  assert_eq "$(cat .memory/project/INDEX.md)" "$(printf -- '- [leap-day](leap-day.md) — gotcha: valid leap day')"
}

test_doctor_warns_on_long_description_but_passes() {
  "$MEMORY" init work >/dev/null
  local long
  long="$(printf 'word %.0s' $(seq 1 30))"
  write_fact_only .memory/project wordy gotcha "$long"
  "$MEMORY" index >/dev/null
  local out
  out="$("$MEMORY" doctor)"
  assert_contains "$out" "warnings (not failures):"
  assert_contains "$out" "LONG DESCRIPTION (150 chars > 120): .memory/project/wordy.md"
  assert_contains "$out" "OK"
}

test_doctor_warns_on_index_description_drift() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact gotcha "new wording"
  printf -- '- [a-fact](a-fact.md) — gotcha: old wording\n' > .memory/project/INDEX.md
  local out
  out="$("$MEMORY" doctor)"
  assert_contains "$out" "INDEX DESCRIPTION DRIFT: .memory/project/a-fact.md"
  assert_contains "$out" "memory index"
  assert_contains "$out" "OK"
}

test_doctor_warns_on_dangling_wiki_link_and_accepts_cross_level() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  write_fact_only .memory/groups/spines shared-topology reference "shared topology"
  cat > .memory/project/a-fact.md <<'EOF'
---
description: links elsewhere
type: context
created: 2026-09-04
---
See [[shared-topology]] (exists in the group) and [[does-not-exist]].
EOF
  "$MEMORY" index >/dev/null
  local out
  out="$("$MEMORY" doctor)"
  assert_contains "$out" "DANGLING WIKI LINK: .memory/project/a-fact.md -> [[does-not-exist]]"
  case "$out" in *"[[shared-topology]]"*) fail "cross-level link wrongly reported: $out" ;; esac
  assert_contains "$out" "OK"
}

test_doctor_checks_group_levels_too() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  cat > .memory/groups/spines/shared-fact.md <<'EOF'
---
description: shared
type: convention
created: 2026-07-07
---
body
EOF
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "NOT IN INDEX"
  assert_contains "$out" "spines"
}

test_doctor_bad_frontmatter() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/bad-fact.md <<'EOF'
---
description: has a bogus type
type: banana
---
body
EOF
  echo "- [bad-fact](bad-fact.md) — has a bogus type" > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid type"
}

test_doctor_missing_frontmatter() {
  "$MEMORY" init work >/dev/null
  echo "just a body, no frontmatter" > .memory/project/naked-fact.md
  echo "- [naked-fact](naked-fact.md) — naked" > .memory/project/INDEX.md
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "FRONTMATTER"
}

test_doctor_does_not_call_stale_project_path_an_orphan() {
  "$MEMORY" init work >/dev/null
  mkdir -p "$AGENT_MEMORY_HOME/work/projects/dead"
  : > "$AGENT_MEMORY_HOME/work/projects/dead/INDEX.md"
  printf '%s\n' "$TMP/gone-forever" \
    > "$AGENT_MEMORY_HOME/work/projects/dead/.project-path"
  local out
  out="$("$MEMORY" doctor)"          # informational only — still exit 0
  assert_contains "$out" "OK"
}

test_rejects_extra_arguments() {
  local out rc=0
  out="$("$MEMORY" init work extra 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory init"
  rc=0
  out="$("$MEMORY" doctor extra 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory doctor"
  "$MEMORY" init work >/dev/null
  rc=0
  out="$("$MEMORY" link-group one two 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory link-group"
}

test_status_shows_levels_counts_and_unlinked_groups() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  mkdir -p "$AGENT_MEMORY_HOME/work/groups/platform"
  : > "$AGENT_MEMORY_HOME/work/groups/platform/INDEX.md"
  write_fact_only .memory/project a-fact gotcha "a"
  write_fact_only .memory/project b-fact gotcha "b"
  write_fact_only .memory/groups/spines s-fact convention "s"
  local out
  out="$("$MEMORY" status)"
  assert_contains "$out" "store:      $(cd -P "$AGENT_MEMORY_HOME" && pwd -P)"
  assert_contains "$out" "workspace:  work  (0 facts)"
  assert_contains "$out" "project:    some-api  (2 facts)"
  assert_contains "$out" "group:      spines  (1 facts)"
  assert_contains "$out" "not linked: platform"
  assert_contains "$out" "backup:     last never"
}

test_status_reports_backup_state() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact gotcha "a"
  "$MEMORY" backup >/dev/null
  local out
  out="$("$MEMORY" status)"
  assert_contains "$out" "backup:     last $(date +%Y-%m-%d), store clean"
  write_fact_only .memory/project b-fact gotcha "b"
  out="$("$MEMORY" status)"
  assert_contains "$out" "1 uncommitted path(s) — run 'memory backup'"
}

test_status_requires_init() {
  local out rc=0
  out="$("$MEMORY" status 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "no .memory/"
}

test_grep_searches_bodies_across_levels_following_symlinks() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group spines >/dev/null
  cat > .memory/project/pool.md <<'EOF'
---
description: pool limit
type: gotcha
created: 2026-09-04
---
pgbouncer in transaction mode returns stale statements above 50 connections.
EOF
  cat > .memory/groups/spines/topology.md <<'EOF'
---
description: shared topology
type: reference
created: 2026-09-04
---
All services share one PgBouncer in front of the primary.
EOF
  "$MEMORY" index >/dev/null
  local out
  out="$("$MEMORY" grep pgbouncer)"
  assert_contains "$out" ".memory/project/pool.md:6:pgbouncer in transaction mode"
  assert_contains "$out" ".memory/groups/spines/topology.md:6:All services share one PgBouncer"
}

test_grep_no_match_exits_zero_with_message() {
  "$MEMORY" init work >/dev/null
  local out rc=0
  out="$("$MEMORY" grep nothing-here-at-all)" || rc=$?
  assert_eq "$rc" 0
  assert_eq "$out" "no matches for 'nothing-here-at-all'"
}

test_grep_excludes_project_path_file() {
  "$MEMORY" init work >/dev/null
  local out
  out="$("$MEMORY" grep some-api)"          # the path is recorded in .project-path
  case "$out" in *".project-path"*) fail "matched .project-path: $out" ;; esac
}

test_grep_requires_pattern() {
  "$MEMORY" init work >/dev/null
  local out rc=0
  out="$("$MEMORY" grep 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory grep"
}

test_status_rejects_unsafe_and_broken_required_links() {
  "$MEMORY" init work >/dev/null
  rm .memory/workspace
  ln -s "$AGENT_MEMORY_HOME/work/missing-workspace" .memory/workspace
  local out rc=0
  out="$("$MEMORY" status 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unsafe or broken symlink: .memory/workspace (run 'memory doctor')"

  ln -sfn "$AGENT_MEMORY_HOME/work/workspace" .memory/workspace
  mkdir -p "$TMP/outside"
  rm .memory/project
  ln -s "$TMP/outside" .memory/project
  rc=0
  out="$("$MEMORY" status 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unsafe or broken symlink: .memory/project (run 'memory doctor')"
}

test_status_labels_unsafe_and_broken_group_links() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group shared >/dev/null
  mkdir -p "$TMP/outside"
  write_fact_only "$TMP/outside" secret gotcha "outside secret"
  rm .memory/groups/shared
  ln -s "$TMP/outside" .memory/groups/shared
  ln -s "$AGENT_MEMORY_HOME/work/groups/missing" .memory/groups/missing
  local out
  out="$("$MEMORY" status)"
  assert_contains "$out" "group:      shared  (UNSAFE OR BROKEN LINK — run 'memory doctor')"
  assert_contains "$out" "group:      missing  (UNSAFE OR BROKEN LINK — run 'memory doctor')"
  case "$out" in *"outside secret"*|*"$TMP/outside"*|*"shared  (1 facts)"*) fail "disclosed unsafe group data: $out" ;; esac
}

test_status_reports_backup_unavailable_without_git() {
  "$MEMORY" init work >/dev/null
  rm -rf "$AGENT_MEMORY_HOME/.git"
  assert_contains "$("$MEMORY" status)" "backup:     unavailable (store is not a git repository)"
}

test_status_does_not_use_parent_git_repository() {
  local parent="$TMP/parent"
  mkdir -p "$parent"
  git -C "$parent" init -q
  printf 'parent repository\n' > "$parent/README.md"
  git -C "$parent" add README.md
  git -C "$parent" commit -q -m "parent commit"
  export AGENT_MEMORY_HOME="$parent/store"
  "$MEMORY" init work >/dev/null
  rm -rf "$AGENT_MEMORY_HOME/.git"
  assert_contains "$("$MEMORY" status)" "backup:     unavailable (store is not a git repository)"
}

test_status_counts_untracked_when_git_hides_them() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" backup >/dev/null
  git -C "$AGENT_MEMORY_HOME" config status.showUntrackedFiles no
  write_fact_only .memory/project untracked gotcha "still must count"
  assert_contains "$("$MEMORY" status)" "1 uncommitted path(s) — run 'memory backup'"
}

test_grep_rejects_unsafe_and_broken_required_links() {
  "$MEMORY" init work >/dev/null
  mkdir -p "$TMP/outside"
  rm .memory/project
  ln -s "$TMP/outside" .memory/project
  local out rc=0
  out="$("$MEMORY" grep needle 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unsafe or broken symlink: .memory/project (run 'memory doctor')"

  ln -sfn "$AGENT_MEMORY_HOME/work/projects/some-api" .memory/project
  rm .memory/workspace
  ln -s "$AGENT_MEMORY_HOME/work/missing-workspace" .memory/workspace
  rc=0
  out="$("$MEMORY" grep needle 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unsafe or broken symlink: .memory/workspace (run 'memory doctor')"
}

test_grep_rejects_unsafe_and_broken_group_links() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group shared >/dev/null
  mkdir -p "$TMP/outside"
  printf 'external needle\n' > "$TMP/outside/secret.md"
  rm .memory/groups/shared
  ln -s "$TMP/outside" .memory/groups/shared
  local out rc=0
  out="$("$MEMORY" grep needle 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unsafe or broken symlink: .memory/groups/shared (run 'memory doctor')"

  rm .memory/groups/shared
  ln -s "$AGENT_MEMORY_HOME/work/groups/missing" .memory/groups/shared
  rc=0
  out="$("$MEMORY" grep needle 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unsafe or broken symlink: .memory/groups/shared (run 'memory doctor')"
}

test_grep_uses_lowercase_r_to_avoid_nested_symlinks() {
  "$MEMORY" init work >/dev/null
  mkdir -p "$TMP/fake-bin"
  GREP_ARGS="$TMP/grep-args"
  export GREP_ARGS
  cat > "$TMP/fake-bin/grep" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$GREP_ARGS"
exit 1
EOF
  chmod +x "$TMP/fake-bin/grep"
  local out rc=0 args
  out="$(PATH="$TMP/fake-bin:$PATH" "$MEMORY" grep needle)" || rc=$?
  assert_eq "$rc" 0
  assert_eq "$out" "no matches for 'needle'"
  args="$(<"$GREP_ARGS")"
  assert_contains "$args" "-r"
  case "$args" in *"-R"*) fail "grep followed nested symlinks: $args" ;; esac
}

test_backup_commits_store() {
  "$MEMORY" init work >/dev/null
  cat > .memory/project/a-fact.md <<'EOF'
---
description: something learned
type: context
created: 2026-07-07
---
body
EOF
  "$MEMORY" backup >/dev/null
  local n
  n="$(git -C "$AGENT_MEMORY_HOME" rev-list --count HEAD)"
  assert_eq "$n" 1
  if git -C "$AGENT_MEMORY_HOME" status --porcelain | grep -q .; then
    fail "store has uncommitted changes"
  fi
}

test_backup_first_commit_in_fresh_store() {
  # A fresh store has no HEAD yet; `git diff --cached --name-status` still
  # works (diffs against the empty tree) and classifies paths correctly.
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact context "something"
  "$MEMORY" backup >/dev/null
  local n msg
  n="$(git -C "$AGENT_MEMORY_HOME" rev-list --count HEAD)"
  assert_eq "$n" 1
  msg="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)"
  # init's .project-path write lands in the same first commit as the fact,
  # under the fallback "other" scope, so this is a multi-scope subject.
  assert_contains "$msg" "add"
  assert_contains "$msg" "2 scopes"
}

test_backup_single_scope_add_update_remove_subject() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact context "a"
  write_fact_only .memory/project b-fact context "b"
  "$MEMORY" backup >/dev/null
  # Distinct content keeps git's rename detection from pairing the removed
  # and added files together.
  cat > .memory/project/c-fact.md <<'EOF'
---
description: a brand new unrelated fact about something else entirely
type: reference
created: 2026-09-04
---
this body is deliberately unlike anything else in the store so it is
never mistaken for a rename of another file
EOF
  printf 'more\n' >> .memory/project/a-fact.md                # update
  rm .memory/project/b-fact.md                                # remove
  "$MEMORY" backup >/dev/null
  local subj
  subj="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)"
  assert_eq "$subj" "memory(some-api): add c-fact; update a-fact; remove b-fact"
}

test_backup_multiple_scopes_subject_and_body() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group grp1 >/dev/null
  "$MEMORY" backup >/dev/null
  write_fact_only .memory/project a-fact context "a"
  write_fact_only .memory/groups/grp1 g-fact context "g"
  "$MEMORY" backup >/dev/null
  local subj body
  subj="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)"
  assert_eq "$subj" "memory: add 2 in 2 scopes"
  body="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%b)"
  assert_contains "$body" "group:grp1: add g-fact"
  assert_contains "$body" "some-api: add a-fact"
  assert_contains "$body" "backup 20"
}

test_backup_rename_shows_slug_arrow() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact context "a"
  "$MEMORY" backup >/dev/null
  mv .memory/project/a-fact.md .memory/project/a-fact-renamed.md
  "$MEMORY" index >/dev/null
  "$MEMORY" backup >/dev/null
  local subj
  subj="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)"
  assert_eq "$subj" "memory(some-api): rename a-fact->a-fact-renamed"
}

test_backup_index_only_reindex_subject() {
  "$MEMORY" init work >/dev/null
  write_fact_only .memory/project a-fact context "a"
  "$MEMORY" backup >/dev/null
  "$MEMORY" index >/dev/null    # only INDEX.md changes
  "$MEMORY" backup >/dev/null
  local subj body
  subj="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)"
  assert_eq "$subj" "memory(some-api): reindex"
  body="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%b)"
  assert_contains "$body" "backup 20"
}

test_backup_long_list_truncates_within_72_chars() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" backup >/dev/null
  local i
  for i in $(seq 1 20); do
    write_fact_only .memory/project "fact-$i-with-a-long-slug-name" context "d $i"
  done
  "$MEMORY" backup >/dev/null
  local subj
  subj="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)"
  [ "${#subj}" -le 72 ] || fail "subject exceeds 72 chars: $subj"
  assert_contains "$subj" "memory(some-api): add"
  assert_contains "$subj" "more"
}

test_backup_single_long_slug_falls_back_to_counts() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" backup >/dev/null
  write_fact_only .memory/project "single-leg-then-two-leg-exchange-reimport-with-extra-words" context "d"
  "$MEMORY" backup >/dev/null
  local subj
  subj="$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)"
  assert_eq "$subj" "memory(some-api): add 1"
}

test_backup_nothing_to_do() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" backup >/dev/null
  local out
  out="$("$MEMORY" backup)"
  assert_contains "$out" "nothing to back up"
}

test_backup_push_without_remote_fails() {
  "$MEMORY" init work >/dev/null
  local out rc=0
  out="$("$MEMORY" backup --push 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "remote"
}

test_install_opencode_creates_plugin_symlink() {
  "$MEMORY" install-opencode >/dev/null
  assert_link "$HOME/.config/opencode/plugins/agent-memory.js" "$ROOT/skills/agent-memory/opencode/agent-memory.js"
}

test_install_opencode_is_idempotent() {
  "$MEMORY" install-opencode >/dev/null
  local out
  out="$("$MEMORY" install-opencode)"
  assert_contains "$out" "already installed"
}

test_install_opencode_replaces_legacy_symlink_in_xdg_config() {
  export XDG_CONFIG_HOME="$TMP/xdg"
  mkdir -p "$XDG_CONFIG_HOME/opencode/plugins" "$TMP/legacy"
  : > "$TMP/legacy/agent-memory.js"
  ln -s "$TMP/legacy/agent-memory.js" "$XDG_CONFIG_HOME/opencode/plugins/agent-memory.js"
  "$MEMORY" install-opencode >/dev/null
  assert_link "$XDG_CONFIG_HOME/opencode/plugins/agent-memory.js" "$ROOT/skills/agent-memory/opencode/agent-memory.js"
  [ ! -e "$HOME/.config/opencode/plugins/agent-memory.js" ] || fail "wrote HOME config instead of XDG_CONFIG_HOME"
}

test_install_opencode_replaces_dangling_symlink() {
  mkdir -p "$HOME/.config/opencode/plugins"
  ln -s "$TMP/moved/agent-memory.js" "$HOME/.config/opencode/plugins/agent-memory.js"
  "$MEMORY" install-opencode >/dev/null
  assert_link "$HOME/.config/opencode/plugins/agent-memory.js" "$ROOT/skills/agent-memory/opencode/agent-memory.js"
}

test_install_opencode_does_not_write_config_or_presets() {
  export XDG_CONFIG_HOME="$TMP/xdg"
  mkdir -p "$XDG_CONFIG_HOME/opencode"
  printf 'config sentinel\n' > "$XDG_CONFIG_HOME/opencode/opencode.json"
  printf 'preset sentinel\n' > "$XDG_CONFIG_HOME/opencode/oh-my-opencode-slim.json"
  "$MEMORY" install-opencode >/dev/null
  assert_eq "$(<"$XDG_CONFIG_HOME/opencode/opencode.json")" "config sentinel"
  assert_eq "$(<"$XDG_CONFIG_HOME/opencode/oh-my-opencode-slim.json")" "preset sentinel"
}

test_install_opencode_refuses_regular_file() {
  mkdir -p "$HOME/.config/opencode/plugins"
  printf 'keep me\n' > "$HOME/.config/opencode/plugins/agent-memory.js"
  local out rc=0
  out="$("$MEMORY" install-opencode 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "refusing to overwrite"
  assert_eq "$(<"$HOME/.config/opencode/plugins/agent-memory.js")" "keep me"
}

# ---------------- dream ----------------

assert_not_contains() { case "$1" in *"$2"*) fail "unexpected '$2' in: $1" ;; *) ;; esac; }

dfact() {   # dfact <level-dir> <slug> <type> <description> <created> [updated]  (no INDEX line)
  {
    printf -- '---\ndescription: %s\ntype: %s\ncreated: %s\n' "$4" "$3" "$5"
    if [ -n "${6:-}" ]; then printf 'updated: %s\n' "$6"; fi
    printf -- '---\nbody of %s\n' "$2"
  } > "$1/$2.md"
}

# Workspace "work" with projects some-api ($PROJ) and other-api; indexes built.
dream_fixture() {
  export AGENT_MEMORY_TODAY=2026-09-30
  DS="$AGENT_MEMORY_HOME/work"
  DP1="$DS/projects/some-api"
  DP2="$DS/projects/other-api"
  DW="$DS/workspace"
  "$MEMORY" init work >/dev/null
  mkdir -p "$TMP/code/other-api"
  (cd "$TMP/code/other-api" && "$MEMORY" init work >/dev/null)
}

dream_reindex() {
  "$MEMORY" index >/dev/null
  (cd "$TMP/code/other-api" && "$MEMORY" index >/dev/null)
}

test_skill_memory_dream_frontmatter() {
  local f="$ROOT/skills/memory-dream/SKILL.md"
  assert_file "$f"
  assert_eq "$(head -1 "$f")" "---"
  grep -q '^name: memory-dream$' "$f" || fail "memory-dream skill has no name: memory-dream"
  grep -q '^description: .' "$f" || fail "memory-dream skill has no description"
  grep -q '^license: MIT$' "$f" || fail "memory-dream skill has no license"
}

test_dream_report_tidy_store() {
  dream_fixture
  dfact "$DP1" pool-limit gotcha "pgbouncer pool breaks above fifty connections" 2026-09-04
  dfact "$DW" editor-setup convention "vim keybindings everywhere" 2026-09-04
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_eq "$out" "dream: memory is tidy — nothing to consolidate"
}

test_dream_report_runs_without_memory_dir() {
  dream_fixture
  dfact "$DP1" pool-limit gotcha "pgbouncer pool breaks above fifty connections" 2026-09-04
  dream_reindex
  mkdir -p "$TMP/elsewhere"
  cd "$TMP/elsewhere"
  [ ! -e .memory ] || fail "unexpected .memory in $TMP/elsewhere"
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "dream:"
}

test_dream_report_similar_within_a_level() {
  dream_fixture
  dfact "$DP1" pgbouncer-pool-limit gotcha "pgbouncer connection pool breaks above 50 connections" 2026-09-04
  dfact "$DP1" pgbouncer-pool-size gotcha "pgbouncer connection pool size limit fifty" 2026-09-04
  dfact "$DP1" deploy-checklist convention "release steps for staging" 2026-09-04
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "similar facts (merge, or check for contradiction):"
  assert_contains "$out" "  work/projects/some-api/pgbouncer-pool-limit.md [code: $(cd -P "$PROJ" && pwd -P)] <-> work/projects/some-api/pgbouncer-pool-size.md [code: $(cd -P "$PROJ" && pwd -P)] (shared: connection, limit, pgbouncer, pool)"
  assert_contains "$out" "dream: 1 similar, 0 repeated across projects, 0 shadowed, 0 stale, 0 unusable"
  assert_not_contains "$out" "deploy-checklist"
}

test_dream_report_unrelated_facts_are_not_paired() {
  dream_fixture
  dfact "$DP1" pool-limit gotcha "pgbouncer pool breaks above fifty connections" 2026-09-04
  dfact "$DP1" release-steps convention "staging deploy needs manual approval" 2026-09-04
  dfact "$DW" editor-setup convention "vim keybindings everywhere" 2026-09-04
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_not_contains "$out" "similar facts"
  assert_not_contains "$out" "<->"
}

# bash 3.2 leaked a descriptor per `< <(...)` in the per-fact loop and died
# with SIGTRAP (exit 133) after ~250 facts; the real store has 500+.
test_dream_report_survives_many_facts() {
  dream_fixture
  local i=0
  while [ "$i" -lt 320 ]; do
    i=$((i + 1))
    printf -- '---\ndescription: note %s\ntype: context\ncreated: 2026-09-01\n---\nsee [[n%s]]\n' "q${i}x" "$((i + 1))" > "$DW/n$i.md"
  done
  local out rc=0
  out="$("$MEMORY" dream --report 2>&1)" || rc=$?
  assert_eq "$rc" 0
  assert_contains "$out" "dangling link: work/workspace/n320.md -> [[n321]]"
}

test_dream_report_repeated_across_projects() {
  dream_fixture
  dfact "$DP1" retry-policy convention "clients retry three times with backoff" 2026-09-04
  dfact "$DP2" retry-policy convention "always exponential delays for HTTP calls" 2026-09-04
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "repeated across projects (move shared part to a group or workspace):"
  assert_contains "$out" "  work: retry-policy in projects/other-api, projects/some-api (same slug)"
  assert_not_contains "$out" "same slug in several levels"
  assert_not_contains "$out" "similar facts"
  assert_contains "$out" "dream: 0 similar, 1 repeated across projects, 0 shadowed, 0 stale, 0 unusable"
}

test_dream_report_shadowing() {
  dream_fixture
  dfact "$DP1" editor-setup convention "vim keybindings" 2026-09-04
  dfact "$DW" editor-setup convention "docker compose ports notes" 2026-09-04
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "same slug in several levels (shadowing):"
  assert_contains "$out" "  work: editor-setup in projects/some-api, workspace"
  assert_contains "$out" "1 shadowed"
}

test_dream_report_never_compares_across_workspaces() {
  dream_fixture
  mkdir -p "$TMP/code/home-proj"
  (cd "$TMP/code/home-proj" && "$MEMORY" init home >/dev/null)
  dfact "$DP1" pool-limit gotcha "pgbouncer pool breaks above fifty connections" 2026-09-04
  dfact "$AGENT_MEMORY_HOME/home/workspace" pool-limit gotcha "pgbouncer pool breaks above fifty connections" 2026-09-04
  dream_reindex
  (cd "$TMP/code/home-proj" && "$MEMORY" index >/dev/null)
  local out
  out="$("$MEMORY" dream --report)"
  assert_not_contains "$out" "<->"
  assert_not_contains "$out" "pool-limit in"
  assert_contains "$out" "dream: memory is tidy"
}

test_dream_report_workspace_filter() {
  dream_fixture
  mkdir -p "$TMP/code/home-proj"
  (cd "$TMP/code/home-proj" && "$MEMORY" init home >/dev/null)
  dfact "$DP1" pgbouncer-pool-limit gotcha "pgbouncer connection pool breaks above 50 connections" 2026-09-04
  dfact "$DP1" pgbouncer-pool-size gotcha "pgbouncer connection pool size limit fifty" 2026-09-04
  dfact "$AGENT_MEMORY_HOME/home/workspace" old-note context "very old note about laptop" 2025-01-01
  dream_reindex
  (cd "$TMP/code/home-proj" && "$MEMORY" index >/dev/null)
  local out rc=0
  out="$("$MEMORY" dream --report --workspace home)"
  assert_contains "$out" "home/workspace/old-note.md"
  assert_not_contains "$out" "pgbouncer"
  out="$("$MEMORY" dream --report --workspace work)"
  assert_contains "$out" "pgbouncer-pool-limit"
  assert_not_contains "$out" "old-note"
  out="$("$MEMORY" dream --report --workspace nope 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "no such workspace"
  rc=0
  out="$("$MEMORY" dream --report --workspace 'Bad_Name' 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid workspace name"
  rc=0
  out="$("$MEMORY" dream --report --workspace 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory dream"
}

test_dream_report_stale_facts() {
  dream_fixture
  dfact "$DP1" old-decision decision "chose the queue library long ago" 2026-01-01
  dfact "$DP1" fresh-decision decision "picked the metrics backend" 2026-01-01 2026-09-01
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "stale facts (older than 180 days — verify, update 'updated:', or delete):"
  assert_contains "$out" "  work/projects/some-api/old-decision.md [code: $(cd -P "$PROJ" && pwd -P)] — decision, 272d since 2026-01-01"
  assert_not_contains "$out" "fresh-decision"
  assert_contains "$out" "1 stale"
  out="$("$MEMORY" dream --report --stale-days 300)"
  assert_eq "$out" "dream: memory is tidy — nothing to consolidate"
  out="$("$MEMORY" dream --report --stale-days 10)"
  assert_contains "$out" "older than 10 days"
  assert_contains "$out" "fresh-decision.md [code: $(cd -P "$PROJ" && pwd -P)] — decision, 29d since 2026-09-01"
}

test_dream_report_rejects_bad_arguments() {
  dream_fixture
  local out rc args
  for args in "--report --stale-days 0" "--report --stale-days abc" "--report --stale-days -5" \
              "--report --stale-days" "--report --stale-days 1.5" "--report --bogus" \
              "--report --model x/y" "--report --if-changed" "--stale-days 5" "--report extra"; do
    rc=0
    # shellcheck disable=SC2086 # Word splitting of the fixed argument lists is intended.
    out="$("$MEMORY" dream $args 2>&1)" || rc=$?
    assert_eq "$rc" 1
    assert_contains "$out" "usage: memory dream"
  done
  rc=0
  out="$(AGENT_MEMORY_TODAY=2026-02-30 "$MEMORY" dream --report 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid AGENT_MEMORY_TODAY"
}

test_dream_report_lists_unusable_facts_without_failing() {
  dream_fixture
  printf 'no frontmatter here\n' > "$DP1/broken.md"
  dfact "$DP1" good-fact convention "vim keybindings everywhere" 2026-09-04
  dream_reindex
  local out rc=0
  out="$("$MEMORY" dream --report)" || rc=$?
  assert_eq "$rc" 0
  assert_contains "$out" "unusable facts (fix frontmatter):"
  assert_contains "$out" "work/projects/some-api/broken.md"
  assert_not_contains "$out" "good-fact"
  assert_contains "$out" "1 unusable"
}

test_dream_report_orphan_project() {
  dream_fixture
  dfact "$DP2" some-fact convention "vim keybindings everywhere" 2026-09-04
  (cd "$TMP/code/other-api" && "$MEMORY" index >/dev/null)
  rm -rf "$TMP/code/other-api"
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "hygiene:"
  assert_contains "$out" "  orphan project: work/projects/other-api (recorded dir is gone)"
  rm -f "$DP1/.project-path"
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "  orphan project: work/projects/some-api (.project-path missing)"
}

test_dream_report_hygiene_long_description_and_dangling_link() {
  dream_fixture
  local long
  long="$(printf 'word%.0s ' $(seq 1 40))"
  dfact "$DP1" long-one gotcha "$long" 2026-09-04
  dfact "$DP1" linker convention "vim keybindings everywhere" 2026-09-04
  printf 'see [[gone-fact]] and [[linker]]\n' >> "$DP1/linker.md"
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "long description ("
  assert_contains "$out" "dangling link: work/projects/some-api/linker.md"
  assert_contains "$out" "-> [[gone-fact]]"
  assert_not_contains "$out" "[[linker]]"
}

test_dream_report_index_drift_and_large_index() {
  dream_fixture
  dfact "$DP1" pool-limit gotcha "pgbouncer pool breaks above fifty connections" 2026-09-04
  dream_reindex
  dfact "$DP1" unindexed-fact convention "vim keybindings everywhere" 2026-09-04
  rm "$DW/INDEX.md"
  fill_index "$DP2/INDEX.md" 41
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "index drift (run 'memory dream' or 'memory index'):"
  assert_contains "$out" "  work/projects/some-api/INDEX.md (differs from the facts)"
  assert_contains "$out" "  work/workspace/INDEX.md (missing)"
  assert_contains "$out" "large indexes (> 40 entries — merge or prune):"
  assert_contains "$out" "  work/projects/other-api/INDEX.md (41 entries)"
}

test_dream_report_writes_nothing() {
  dream_fixture
  dfact "$DP1" pgbouncer-pool-limit gotcha "pgbouncer connection pool breaks above 50 connections" 2026-09-04
  dfact "$DP1" pgbouncer-pool-size gotcha "pgbouncer connection pool size limit fifty" 2026-01-01
  dfact "$DP2" retry-policy convention "clients retry three times with backoff" 2026-09-04
  dfact "$DP1" retry-policy convention "always exponential delays for HTTP calls" 2026-09-04
  printf 'broken\n' > "$DP1/broken.md"
  dream_reindex
  dfact "$DP1" unindexed convention "vim keybindings everywhere" 2026-09-04
  local before after out
  before="$(find "$AGENT_MEMORY_HOME" -type f -exec cksum {} + | LC_ALL=C sort)"
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "dream:"
  after="$(find "$AGENT_MEMORY_HOME" -type f -exec cksum {} + | LC_ALL=C sort)"
  assert_eq "$after" "$before"
}

test_dream_report_output_is_deterministic() {
  dream_fixture
  dfact "$DP1" pgbouncer-pool-limit gotcha "pgbouncer connection pool breaks above 50 connections" 2026-09-04
  dfact "$DP1" pgbouncer-pool-size gotcha "pgbouncer connection pool size limit fifty" 2026-01-01
  dfact "$DP2" retry-policy convention "clients retry three times with backoff" 2026-01-02
  dfact "$DP1" retry-policy convention "always exponential delays for HTTP calls" 2026-01-03
  dream_reindex
  local a b
  a="$("$MEMORY" dream --report)"
  b="$(LC_ALL=en_US.UTF-8 "$MEMORY" dream --report)"
  assert_eq "$b" "$a"
}

test_dream_report_handles_non_ascii_descriptions() {
  dream_fixture
  dfact "$DP1" uvicorn-workers-first gotcha "воркеры uvicorn падают при перезапуске" 2026-09-04
  dfact "$DP1" uvicorn-workers-second gotcha "воркеры uvicorn падают при перезапуске сервера" 2026-09-04
  dream_reindex
  local out
  out="$("$MEMORY" dream --report)"
  assert_contains "$out" "uvicorn-workers-first.md [code: $(cd -P "$PROJ" && pwd -P)] <-> work/projects/some-api/uvicorn-workers-second.md"
  assert_contains "$out" "воркеры"
}

# ---- dream launcher (fake opencode) ----

install_opencode_stub() {
  STUB_LOG="$TMP/stub"
  export STUB_LOG
  mkdir -p "$TMP/bin" "$STUB_LOG"
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
pwd -P > "$STUB_LOG/pwd"
printf '%s\n' "$@" > "$STUB_LOG/args"
printf '%s' "${OPENCODE_CONFIG_CONTENT:-}" > "$STUB_LOG/config"
printf '%s' "${OPENCODE_DB:-}" > "$STUB_LOG/db"
if [ -e "${OPENCODE_DB:-}" ] || [ -e "${OPENCODE_DB:-}-wal" ] || [ -e "${OPENCODE_DB:-}-shm" ]; then
  echo yes > "$STUB_LOG/db-existed"
else
  echo no > "$STUB_LOG/db-existed"
fi
git status --porcelain --untracked-files=all > "$STUB_LOG/status-at-run"
git log --oneline | wc -l | tr -d ' ' > "$STUB_LOG/commits-at-run"
if [ -n "${STUB_EDIT:-}" ]; then
  printf -- '---\ndescription: merged replacement\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' \
    > work/projects/some-api/merged-fact.md
fi
exit "${STUB_EXIT:-0}"
EOF
  chmod +x "$TMP/bin/opencode"
  PATH="$TMP/bin:$PATH"
}

# Fixture store with a committed baseline that contains stale-fact.
dream_launch_fixture() {
  dream_fixture
  dfact "$DP1" stale-fact decision "an outdated decision" 2026-09-04
  dream_reindex
  "$MEMORY" backup >/dev/null
  install_opencode_stub
}

commit_count() { git -C "$AGENT_MEMORY_HOME" log --oneline | wc -l | tr -d ' '; }

test_dream_print_config_renders_without_launching() {
  dream_fixture
  local out store
  store="$(cd -P "$AGENT_MEMORY_HOME" && pwd -P)"
  # No opencode on PATH: --print-config must not need it.
  out="$(PATH="/usr/bin:/bin" "$MEMORY" dream --print-config)"
  assert_not_contains "$out" "__"
  assert_contains "$out" "$store"
  assert_contains "$out" '"policies"'
  assert_contains "$out" "memory-dream-check"
  assert_contains "$out" "$HOME/.ssh/*"
  assert_contains "$out" 'edit:*/.git/*'
  assert_contains "$out" "$ROOT/skills/agent-memory/bin/memory dream --report"
  assert_contains "$out" "---"
  assert_not_contains "$out" "Workspace scope:"
  out="$("$MEMORY" dream --print-config --workspace work)"
  assert_contains "$out" "Workspace scope: work"
}

test_dream_print_config_is_valid_json() {
  command -v node >/dev/null 2>&1 || { echo "  (skipped: node not found)"; return 0; }
  dream_fixture
  local json
  json="$("$MEMORY" dream --print-config | awk '$0 == "---" { exit } { print }')"
  [ -n "$json" ] || fail "no config part before ---"
  printf '%s' "$json" | node -e '
    const c = JSON.parse(require("fs").readFileSync(0, "utf8"));
    if (c.default_agent !== "memory-dream") throw new Error("default_agent");
    if (!Array.isArray(c.policies) || c.policies[0].effect !== "deny") throw new Error("policies");
    if (c.agents["memory-dream"].mode !== "primary") throw new Error("primary");
    if (c.agents["memory-dream-check"].mode !== "subagent") throw new Error("subagent");
    const check = c.agents["memory-dream-check"].permissions;
    if (check.some((p) => p.action === "edit" && p.effect === "allow")) throw new Error("check agent can edit");
    if (check.some((p) => p.action === "subagent" && p.effect === "allow")) throw new Error("check agent can spawn");
    if (JSON.stringify(c).includes("\"ask\"")) throw new Error("ask effect used");
  ' || fail "print-config JSON invalid or wrong shape"
}

test_dream_rejects_json_unsafe_home() {
  dream_fixture
  local out rc=0
  mkdir -p "$TMP/ho\"me"
  out="$(HOME="$TMP/ho\"me" "$MEMORY" dream --print-config 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "cannot run dream"
}

test_dream_requires_opencode_on_path() {
  dream_fixture
  local out rc=0
  if PATH="/usr/bin:/bin" command -v opencode >/dev/null 2>&1; then
    echo "  (skipped: opencode installed in a system directory)"; return 0
  fi
  out="$(PATH="/usr/bin:/bin" "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "opencode not found on PATH"
}

test_dream_requires_git_store() {
  local out rc=0
  out="$("$MEMORY" dream --print-config 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "no store"
  mkdir -p "$AGENT_MEMORY_HOME/work"
  rc=0
  out="$("$MEMORY" dream --print-config 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "not a git repository"
}

test_dream_print_config_writes_nothing() {
  dream_launch_fixture
  local before after
  before="$(find "$AGENT_MEMORY_HOME" -type f -exec cksum {} + | LC_ALL=C sort)"
  dfact "$DP1" pending convention "uncommitted fact" 2026-09-04
  before="$before
$(cksum "$DP1/pending.md")"
  before="$(printf '%s\n' "$before" | LC_ALL=C sort)"
  "$MEMORY" dream --print-config >/dev/null
  after="$(find "$AGENT_MEMORY_HOME" -type f -exec cksum {} + | LC_ALL=C sort)"
  assert_eq "$after" "$before"
  [ ! -e "$STUB_LOG/args" ] || fail "print-config launched opencode"
  [ -n "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ] || fail "print-config committed the pending change"
}

test_dream_run_commits_pending_changes_first() {
  dream_launch_fixture
  dfact "$DP1" pending convention "uncommitted fact" 2026-09-04
  local base
  base="$(commit_count)"
  "$MEMORY" dream >/dev/null
  assert_eq "$(<"$STUB_LOG/status-at-run")" ""
  assert_eq "$(<"$STUB_LOG/commits-at-run")" "$((base + 1))"
}

test_dream_run_launches_from_store_with_agent_and_config() {
  dream_launch_fixture
  "$MEMORY" dream >/dev/null
  assert_eq "$(<"$STUB_LOG/pwd")" "$(cd -P "$AGENT_MEMORY_HOME" && pwd -P)"
  assert_eq "$(head -4 "$STUB_LOG/args" | tr '\n' ' ')" "run --standalone --agent memory-dream "
  assert_contains "$(<"$STUB_LOG/args")" "You run unattended over the memory store at $(cd -P "$AGENT_MEMORY_HOME" && pwd -P)"
  assert_contains "$(<"$STUB_LOG/config")" "\"default_agent\": \"memory-dream\""
  assert_not_contains "$(<"$STUB_LOG/config")" "__STORE__"
  assert_not_contains "$(<"$STUB_LOG/args")" "--model"
  assert_eq "$(<"$STUB_LOG/db")" "$(cd -P "$AGENT_MEMORY_HOME" && pwd -P)/.git/memory-dream.db"
}

test_dream_run_starts_with_a_fresh_untracked_db() {
  dream_launch_fixture
  local db
  db="$AGENT_MEMORY_HOME/.git/memory-dream.db"
  echo stale > "$db"
  echo stale > "$db-wal"
  local out
  out="$("$MEMORY" dream)"
  assert_eq "$(<"$STUB_LOG/db-existed")" "no"
  assert_contains "$out" "dream: session transcript in "
  assert_not_contains "$(<"$STUB_LOG/status-at-run")" "memory-dream.db"
  assert_not_contains "$(git -C "$AGENT_MEMORY_HOME" status --porcelain --untracked-files=all)" "memory-dream.db"
}

test_dream_report_and_print_config_keep_the_db() {
  dream_launch_fixture
  local db
  db="$AGENT_MEMORY_HOME/.git/memory-dream.db"
  echo keep > "$db"
  "$MEMORY" dream --report >/dev/null
  assert_file "$db"
  "$MEMORY" dream --print-config >/dev/null
  assert_file "$db"
  assert_eq "$(<"$db")" "keep"
}

test_dream_run_passes_model_and_workspace() {
  dream_launch_fixture
  "$MEMORY" dream --model example/model-1 --workspace work >/dev/null
  assert_contains "$(<"$STUB_LOG/args")" "--model
example/model-1"
  assert_contains "$(<"$STUB_LOG/args")" "Workspace scope: work"
}

test_dream_run_model_from_env_and_flag_override() {
  dream_launch_fixture
  AGENT_MEMORY_DREAM_MODEL=example/from-env "$MEMORY" dream >/dev/null
  assert_contains "$(<"$STUB_LOG/args")" "--model
example/from-env"
  AGENT_MEMORY_DREAM_MODEL=example/from-env "$MEMORY" dream --model example/flag >/dev/null
  assert_contains "$(<"$STUB_LOG/args")" "--model
example/flag"
  assert_not_contains "$(<"$STUB_LOG/args")" "from-env"
  # The env default must not make --report reject its arguments.
  AGENT_MEMORY_DREAM_MODEL=example/from-env "$MEMORY" dream --report >/dev/null
}

test_dream_run_if_changed_skips_when_store_is_unchanged() {
  dream_launch_fixture
  local out
  # First run: no record of a previous run, so it runs and records HEAD.
  out="$("$MEMORY" dream --if-changed)"
  assert_not_contains "$out" "skipping"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)" "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
  rm -f "$STUB_LOG/args"
  out="$("$MEMORY" dream --if-changed)"
  assert_contains "$out" "dream: no fact changed since the last run — skipping"
  [ ! -e "$STUB_LOG/args" ] || fail "opencode ran although nothing changed"
  # Commits that touch no fact (an index rebuild, a new project) do not count.
  printf '%s\n' "- touched" >> "$DP1/INDEX.md"
  "$MEMORY" backup >/dev/null
  "$MEMORY" index >/dev/null
  "$MEMORY" backup >/dev/null
  out="$("$MEMORY" dream --if-changed)"
  assert_contains "$out" "skipping"
  [ ! -e "$STUB_LOG/args" ] || fail "opencode ran after an index-only commit"
  # A new fact (even uncommitted) makes the next run happen.
  dfact "$DP1" new-fact convention "a brand new convention" 2026-09-30
  out="$("$MEMORY" dream --if-changed)"
  assert_not_contains "$out" "skipping"
  assert_file "$STUB_LOG/args"
  # Without the flag it always runs.
  rm -f "$STUB_LOG/args"
  "$MEMORY" dream >/dev/null
  assert_file "$STUB_LOG/args"
  # A failed run does not move the marker.
  local before
  before="$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)"
  dfact "$DP1" another-fact convention "yet another convention" 2026-09-30
  STUB_EXIT=1 "$MEMORY" dream --if-changed >/dev/null 2>&1 || true
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)" "$before"
}

# A stub that, like the OpenCode plugin after an agent step, runs
# `memory backup` in the middle of the dream run.
install_backup_calling_stub() {
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf -- '---\ndescription: mid-run edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' \
  > work/projects/some-api/mid-run.md
"$MEMORY_BIN" backup > "$STUB_LOG/backup-out" 2>&1
[ -d .git/memory-dream.lock ] && echo held > "$STUB_LOG/lock"
printf '%s\n' "$@" > "$STUB_LOG/args"
exit "${STUB_EXIT:-0}"
EOF
  chmod +x "$TMP/bin/opencode"
}

test_dream_run_holds_lock_so_other_backups_wait() {
  dream_launch_fixture
  install_backup_calling_stub
  local base
  base="$(commit_count)"
  MEMORY_BIN="$MEMORY" "$MEMORY" dream >/dev/null
  assert_contains "$(<"$STUB_LOG/backup-out")" "request NOT saved"
  assert_eq "$(<"$STUB_LOG/lock")" "held"
  # One commit for the whole run, holding the mid-run edit; the lock is gone.
  assert_eq "$(commit_count)" "$((base + 1))"
  git -C "$AGENT_MEMORY_HOME" show --name-only --format= HEAD | grep -q 'mid-run.md' \
    || fail "mid-run edit not in the dream commit"
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ] || fail "lock left behind"
  dfact "$DP1" after-run convention "written after the run" 2026-09-30
  assert_contains "$("$MEMORY" backup)" "backed up"
}

test_dream_uncertain_and_stale_locks_require_manual_recovery() {
  dream_template_fixture
  local lock="$AGENT_MEMORY_HOME/.git/memory-dream.lock" out rc=0
  mkdir "$lock"
  out="$("$TMEM" backup 2>&1)" && fail "busy backup reported success"
  assert_contains "$out" "request NOT saved"
  out="$("$TMEM" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "manual recovery"
  [ -d "$lock" ] || fail "unpublished lock removed"
  printf '999999\nold start\n' > "$lock/owner"
  out="$("$TMEM" backup 2>&1)" && fail "busy backup reported success"
  assert_contains "$out" "request NOT saved"
  [ -f "$lock/owner" ] || fail "stale metadata removed automatically"
  rm "$lock/owner"; rmdir "$lock"
  "$TMEM" dream >/dev/null
}

test_dream_run_refuses_a_second_concurrent_run() {
  dream_launch_fixture
  sleep 30 &
  local live=$! out rc=0
  printf '%s\n' "$live" > "$AGENT_MEMORY_HOME/.git/memory-dream.lock"
  out="$("$MEMORY" dream 2>&1)" || rc=$?
  kill "$live" 2>/dev/null || true
  wait "$live" 2>/dev/null || true
  assert_eq "$rc" 1
  assert_contains "$out" "already running on this store (pid $live"
  [ ! -e "$STUB_LOG/args" ] || fail "second run launched opencode"
}

test_dream_run_rebuilds_indexes_and_commits() {
  dream_launch_fixture
  local base out
  base="$(commit_count)"
  out="$(STUB_EDIT=1 "$MEMORY" dream)"
  assert_eq "$(commit_count)" "$((base + 1))"
  assert_contains "$(<"$DP1/INDEX.md")" "[merged-fact](merged-fact.md) — decision: merged replacement"
  # The agent cannot delete: the obsolete fact stays for manual removal.
  assert_contains "$(<"$DP1/INDEX.md")" "stale-fact"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
  assert_contains "$out" "dream: changes since"
  assert_contains "$out" "merged-fact.md"
  "$MEMORY" doctor >/dev/null
  [ -z "$(git -C "$AGENT_MEMORY_HOME" remote)" ] || fail "store unexpectedly has a remote"
}

test_dream_run_without_changes_reports_none() {
  dream_launch_fixture
  local base out
  base="$(commit_count)"
  out="$("$MEMORY" dream)"
  assert_eq "$(commit_count)" "$base"
  assert_contains "$out" "dream: no changes"
}

test_dream_run_failure_commits_separately_and_keeps_marker() {
  dream_launch_fixture
  "$MEMORY" dream >/dev/null
  local marker head out rc=0
  marker="$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)"
  head="$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
  out="$(STUB_EDIT=1 STUB_EXIT=3 "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "opencode run failed (exit 3)"
  assert_contains "$out" "revert HEAD"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)" "memory: dream FAILED (exit 3) — review"
  [ "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)" != "$head" ] || fail "failed run was not committed"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)" "$marker"
  assert_contains "$(<"$DP1/INDEX.md")" "merged-fact"
  git -C "$AGENT_MEMORY_HOME" revert --no-edit HEAD >/dev/null
  [ -f "$DP1/stale-fact.md" ] || fail "revert lost a pre-existing fact"
  [ ! -e "$DP1/merged-fact.md" ] || fail "revert did not undo the failed run"
}

test_dream_run_times_out_and_commits_failure() {
  dream_launch_fixture
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf -- '---\ndescription: partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
printf -- '---\ndescription: interrupted edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/interrupted.md
echo $$ > "$STUB_LOG/pid"
exec sleep 60
EOF
  chmod +x "$TMP/bin/opencode"
  local out rc=0 pid
  out="$(AGENT_MEMORY_DREAM_TIMEOUT_SECS=1 AGENT_MEMORY_DREAM_KILL_GRACE=1 "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "timed out"
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)" "dream FAILED"
  pid="$(<"$STUB_LOG/pid")"
  ! kill -0 "$pid" 2>/dev/null || fail "opencode still running after timeout"
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ] || fail "lock left after timeout"
}

test_dream_timeout_flag_and_env_validation() {
  dream_launch_fixture
  local out rc=0
  out="$("$MEMORY" dream --timeout abc 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory dream"
  rc=0; out="$(AGENT_MEMORY_DREAM_TIMEOUT=x "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "invalid dream timeout"
  "$MEMORY" dream --timeout 5 >/dev/null
  assert_file "$STUB_LOG/args"
}

test_dream_signal_kills_child_and_releases_lock() {
  dream_template_fixture
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf -- '---\ndescription: interrupted edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/interrupted.md
echo $$ > "$STUB_LOG/pid"
exec sleep 60
EOF
  chmod +x "$TMP/bin/opencode"
  AGENT_MEMORY_DREAM_KILL_GRACE=1 "$TMEM" dream >"$STUB_LOG/dream-out" 2>&1 &
  local dpid=$! i=0 cpid
  while [ ! -s "$STUB_LOG/pid" ] && [ "$i" -lt 600 ]; do sleep 0.1; i=$((i + 1)); done
  if [ ! -s "$STUB_LOG/pid" ]; then
    kill -TERM "$dpid" 2>/dev/null || true
    wait "$dpid" 2>/dev/null || true
    fail "opencode did not become ready: $(<"$STUB_LOG/dream-out")"
  fi
  cpid="$(<"$STUB_LOG/pid")"
  kill -TERM "$dpid"
  wait "$dpid" 2>/dev/null || true
  ! kill -0 "$cpid" 2>/dev/null || fail "child survived SIGTERM of dream"
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ] || fail "lock left after SIGTERM"
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)" "dream FAILED"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
}

test_dream_lock_is_taken_before_pending_backup() {
  dream_launch_fixture
  sleep 30 &
  local live=$! head out rc=0
  head="$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
  dfact "$DP1" pending-fact convention "pending" 2026-09-30
  printf '%s\n%s\n%s\n' "$live" "$(ps -o lstart= -p "$live" | sed 's/^ *//; s/ *$//')" "$(date +%s)" \
    > "$AGENT_MEMORY_HOME/.git/memory-dream.lock"
  out="$("$MEMORY" dream --if-changed 2>&1)" || rc=$?
  kill "$live" 2>/dev/null || true
  wait "$live" 2>/dev/null || true
  assert_eq "$rc" 1
  assert_contains "$out" "already running"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)" "$head"
}

test_dream_live_writer_never_expires_by_age() {
  dream_template_fixture
  local lock="$AGENT_MEMORY_HOME/.git/memory-dream.lock" out rc=0
  mkdir "$lock"
  printf '%s\n%s\n1\n' "$$" "$(ps -o lstart= -p "$$" | sed 's/^ *//; s/ *$//')" > "$lock/owner"
  out="$("$TMEM" dream --timeout 480 2>&1)" || rc=$?
  assert_eq "$rc" 1
  [ -f "$lock/owner" ] || fail "live old lock removed"
  rm "$lock/owner"; rmdir "$lock"
}

test_backup_ignores_user_hooks_and_nothing_to_back_up_in_empty_store() {
  mkdir "$TMP/empty-store"
  git init -q "$TMP/empty-store"
  git config --global user.useConfigOnly true
  git config --global --unset user.email
  assert_eq "$(AGENT_MEMORY_HOME="$TMP/empty-store" "$MEMORY" backup)" "nothing to back up"
  git config --global user.email "test@example.com"
  "$MEMORY" init work >/dev/null
  mkdir -p "$AGENT_MEMORY_HOME/.git/hooks"
  printf '#!/bin/sh\nexit 1\n' > "$AGENT_MEMORY_HOME/.git/hooks/pre-commit"
  chmod +x "$AGENT_MEMORY_HOME/.git/hooks/pre-commit"
  write_fact .memory/project a-fact context "something learned"
  assert_contains "$("$MEMORY" backup)" "backed up"
}

test_backup_retries_while_index_lock_exists() {
  "$MEMORY" init work >/dev/null
  write_fact .memory/project a-fact context "something learned"
  : > "$AGENT_MEMORY_HOME/.git/index.lock"
  ( sleep 2; rm -f "$AGENT_MEMORY_HOME/.git/index.lock" ) &
  assert_contains "$("$MEMORY" backup)" "backed up"
  wait
}

test_backup_push_refuses_detached_head() {
  "$MEMORY" init work >/dev/null
  write_fact .memory/project a-fact context "something learned"
  "$MEMORY" backup >/dev/null
  git init -q --bare "$TMP/remote.git"
  git -C "$AGENT_MEMORY_HOME" remote add origin "$TMP/remote.git"
  git -C "$AGENT_MEMORY_HOME" checkout -q --detach
  local out rc=0
  out="$("$MEMORY" backup --push 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "detached HEAD"
  git -C "$AGENT_MEMORY_HOME" checkout -q main
  "$MEMORY" backup --push >/dev/null
  git -C "$TMP/remote.git" rev-parse -q --verify main >/dev/null || fail "push did not reach the remote"
}

test_dream_if_changed_workspace_scope() {
  dream_launch_fixture
  "$MEMORY" dream --workspace work >/dev/null
  mkdir -p "$AGENT_MEMORY_HOME/home/workspace"
  dfact "$AGENT_MEMORY_HOME/home/workspace" other-ws convention "another workspace" 2026-09-30
  rm -f "$STUB_LOG/args"
  local out
  out="$("$MEMORY" dream --if-changed --workspace work)"
  assert_contains "$out" "skipping"
  [ ! -e "$STUB_LOG/args" ] || fail "a change in another workspace triggered the run"
}

test_init_rejects_project_link_from_other_workspace() {
  "$MEMORY" init work >/dev/null
  rm .memory/workspace
  local out rc=0
  out="$("$MEMORY" init other 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "outside workspace 'other'"
}

test_doctor_reports_workspace_mismatch() {
  "$MEMORY" init work >/dev/null
  mkdir -p "$AGENT_MEMORY_HOME/other/workspace"
  : > "$AGENT_MEMORY_HOME/other/workspace/INDEX.md"
  ln -sfn "$(cd -P "$AGENT_MEMORY_HOME" && pwd -P)/other/workspace" .memory/workspace
  local out rc=0
  out="$("$MEMORY" doctor 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "WORKSPACE MISMATCH"
}

test_dangling_link_check_survives_invalid_utf8() {
  "$MEMORY" init work >/dev/null
  write_fact .memory/project a-fact context "something learned"
  printf 'bad \377\376 bytes [[missing-one]]\n' >> .memory/project/a-fact.md
  assert_contains "$("$MEMORY" doctor 2>&1)" "DANGLING WIKI LINK: .memory/project/a-fact.md -> [[missing-one]]"
  assert_contains "$("$MEMORY" dream --report 2>&1)" "dangling link"
}

test_symlinked_facts_are_skipped() {
  "$MEMORY" init work >/dev/null
  write_fact .memory/project real-fact context "a real one"
  printf -- '---\ndescription: outside\ntype: context\ncreated: 2026-09-04\n---\nx\n' > "$TMP/outside.md"
  ln -s "$TMP/outside.md" .memory/project/linked-fact.md
  "$MEMORY" index >/dev/null 2>&1
  assert_not_contains "$(<.memory/project/INDEX.md)" "linked-fact"
  assert_contains "$(<.memory/project/INDEX.md)" "real-fact"
  assert_contains "$("$MEMORY" dream --report 2>&1)" "symlinked fact ignored"
}

test_init_excludes_scratch_files_from_store_git() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" init work >/dev/null
  local ex="$AGENT_MEMORY_HOME/.git/info/exclude"
  grep -qxF '.memory-index.*' "$ex" || fail "index temp pattern missing"
  ! grep -qxF 'memory-dream.*' "$ex" || fail "broad dream pattern retained"
  assert_eq "$(grep -cxF '.memory-index.*' "$ex")" 1
  : > .memory/project/.memory-index.AbCdEf
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain --untracked-files=all | grep -c memory-index || true)" 0
}

test_stale_index_temp_files_are_removed_on_publish() {
  "$MEMORY" init work >/dev/null
  : > .memory/project/.memory-index.old123
  touch -t 202001010000 .memory/project/.memory-index.old123
  "$MEMORY" index >/dev/null
  [ ! -e .memory/project/.memory-index.old123 ] || fail "stale temp file kept"
}

test_home_unset_gives_clear_error() {
  local out rc=0
  out="$(env -u HOME -u AGENT_MEMORY_HOME "$MEMORY" status 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "HOME is not set"
}

dream_template_fixture() {
  dream_launch_fixture
  cp -R "$ROOT/skills/agent-memory" "$TMP/skill"
  cat > "$TMP/skill/dream/opencode.json" <<'EOF'
{
  "store": "__STORE__",
  "dirs": [
    "__PROJECT_DIR__",
  ],
  "web": "x", "_tag": "__WEB_ONLY__"
  "end": 1
}
EOF
  TMEM="$TMP/skill/bin/memory"
}

test_dream_template_project_dir_and_web_tokens() {
  dream_template_fixture
  local out
  out="$("$TMEM" dream --print-config)"
  assert_not_contains "$out" "__PROJECT_DIR__"
  assert_not_contains "$out" "__WEB_ONLY__"
  assert_contains "$out" "\"$(cd -P "$PROJ" && pwd -P)\","
  assert_contains "$out" "\"$(cd -P "$TMP/code/other-api" && pwd -P)\","
  assert_not_contains "$out" "_tag"
  out="$("$TMEM" dream --print-config --web)"
  assert_contains "$out" '"_tag": ""'
  assert_not_contains "$out" "__WEB_ONLY__"
  # Missing directories are skipped; a workspace scope limits the list.
  rm -r "$TMP/code/other-api"
  out="$("$TMEM" dream --print-config)"
  assert_not_contains "$out" "other-api"
  mkdir -p "$AGENT_MEMORY_HOME/home/projects/p"
  mkdir -p "$TMP/code/home-proj"
  printf '%s\n' "$TMP/code/home-proj" > "$AGENT_MEMORY_HOME/home/projects/p/.project-path"
  out="$("$TMEM" dream --print-config --workspace work)"
  assert_not_contains "$out" "home-proj"
  out="$("$TMEM" dream --print-config --workspace home)"
  assert_contains "$out" "home-proj"
  assert_not_contains "$out" "some-api"
}

test_dream_template_without_project_dirs_deletes_line() {
  dream_template_fixture
  rm -r "$TMP/code/other-api"
  cd "$TMP"
  rm -r "$PROJ"
  local out
  out="$("$TMEM" dream --print-config)"
  assert_not_contains "$out" "__PROJECT_DIR__"
  assert_contains "$out" '"dirs": ['
}

test_dream_run_disables_project_config() {
  dream_launch_fixture
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf '%s %s\n' "${OPENCODE_CONFIG_PROJECT_DISABLE:-}" "${OPENCODE_DISABLE_PROJECT_CONFIG:-}" > "$STUB_LOG/env"
EOF
  chmod +x "$TMP/bin/opencode"
  "$MEMORY" dream >/dev/null
  assert_eq "$(<"$STUB_LOG/env")" "1 1"
}

test_dream_prompt_explains_restricted_git_commands() {
  dream_fixture
  local out
  out="$("$MEMORY" dream --print-config)"
  assert_contains "$out" 'Git history and content-reading commands are disabled'
  assert_contains "$out" 'git -C <dir> status --short'
}

test_dream_first_run_is_full_then_incremental() {
  dream_launch_fixture
  local out
  # No previous run: the whole store is in scope.
  out="$("$MEMORY" dream --print-config)"
  assert_not_contains "$out" "Incremental run"
  "$MEMORY" dream >/dev/null
  assert_not_contains "$(<"$STUB_LOG/args")" "Incremental run"
  # Nothing changed since: only hygiene.
  out="$("$MEMORY" dream --print-config)"
  assert_contains "$out" "Incremental run"
  assert_contains "$out" "No fact changed since then"
  # Changed, new (uncommitted) and deleted facts; INDEX.md does not count.
  dfact "$DP1" fresh-one convention "a fresh convention" 2026-09-30
  dfact "$DW" fresh-two gotcha "a fresh gotcha" 2026-09-30
  git -C "$AGENT_MEMORY_HOME" rm -q work/projects/some-api/stale-fact.md
  "$MEMORY" backup >/dev/null
  dfact "$DP1" untracked-one decision "an untracked decision" 2026-09-30
  out="$("$MEMORY" dream --print-config)"
  assert_contains "$out" "Only these 3 facts were added or changed since then:"
  assert_contains "$out" "- work/projects/some-api/fresh-one.md"
  assert_contains "$out" "- work/projects/some-api/untracked-one.md"
  assert_contains "$out" "- work/workspace/fresh-two.md"
  assert_not_contains "$out" "stale-fact.md"
  assert_not_contains "$out" "INDEX.md
"
  # --full ignores the marker.
  out="$("$MEMORY" dream --print-config --full)"
  assert_not_contains "$out" "Incremental run"
  "$MEMORY" dream --full >/dev/null
  assert_not_contains "$(<"$STUB_LOG/args")" "Incremental run"
}

test_dream_incremental_is_per_workspace() {
  dream_launch_fixture
  local out
  "$MEMORY" dream >/dev/null
  # A workspace run has its own marker, so the first one is full.
  out="$("$MEMORY" dream --print-config --workspace work)"
  assert_not_contains "$out" "Incremental run"
  "$MEMORY" dream --workspace work >/dev/null
  mkdir -p "$AGENT_MEMORY_HOME/home/workspace"
  dfact "$AGENT_MEMORY_HOME/home/workspace" other-ws convention "another workspace" 2026-09-30
  out="$("$MEMORY" dream --print-config --workspace work)"
  assert_contains "$out" "No fact changed since then"
  out="$("$MEMORY" dream --print-config)"
  assert_contains "$out" "- home/workspace/other-ws.md"
}

test_description_length_counts_characters_without_locale() {
  dream_fixture
  # 76 characters, 136 bytes: under launchd (no LANG) bash used to count bytes.
  dfact "$DP1" cyrillic gotcha "Падающий ARK HAL-плагин может скрыть все аудиоустройства, включая встроенные" 2026-09-30
  dream_reindex
  local env_args=(HOME="$HOME" PATH="$PATH" AGENT_MEMORY_HOME="$AGENT_MEMORY_HOME" AGENT_MEMORY_TODAY="$AGENT_MEMORY_TODAY")
  local setting out
  for setting in "" "LANG=C" "LC_ALL=C" "LC_ALL=POSIX"; do
    out="$(env -i "${env_args[@]}" ${setting:+"$setting"} "$MEMORY" dream --report)"
    assert_not_contains "$out" "long description"
    out="$(cd "$PROJ" && env -i "${env_args[@]}" ${setting:+"$setting"} "$MEMORY" doctor)"
    assert_not_contains "$out" "LONG DESCRIPTION"
  done
  # A really long one is still reported, with its length in characters.
  dfact "$DP1" cyrillic gotcha "$(printf 'щ%.0s' $(seq 1 121))" 2026-09-30
  dream_reindex
  out="$(env -i "${env_args[@]}" "$MEMORY" dream --report)"
  assert_contains "$out" "long description (121 chars)"
}

test_dream_rejects_full_with_report() {
  dream_fixture
  local out rc=0
  out="$("$MEMORY" dream --report --full 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "usage: memory dream"
}

test_init_migrates_broad_exclude_and_keeps_legitimate_fact() {
  "$MEMORY" init work >/dev/null
  printf 'memory-dream.*\nkeep-me\n' >> "$AGENT_MEMORY_HOME/.git/info/exclude"
  "$MEMORY" init work >/dev/null
  ! grep -qxF 'memory-dream.*' "$AGENT_MEMORY_HOME/.git/info/exclude" || fail "broad exclude kept"
  grep -qxF 'keep-me' "$AGENT_MEMORY_HOME/.git/info/exclude" || fail "unrelated exclude removed"
  write_fact .memory/project memory-dream context "legitimate fact"
  "$MEMORY" backup >/dev/null
  git -C "$AGENT_MEMORY_HOME" ls-files --error-unmatch work/projects/some-api/memory-dream.md >/dev/null
}

test_init_rejects_new_symlink_destinations_before_writes() {
  "$MEMORY" init work >/dev/null
  mkdir "$TMP/outside"
  ln -s "$TMP/outside" "$AGENT_MEMORY_HOME/escape"
  mkdir "$TMP/code/new"
  local out rc=0
  out="$(cd "$TMP/code/new" && "$MEMORY" init escape 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_eq "$(ls -A "$TMP/outside")" ""
  ln -s "$TMP/outside" "$AGENT_MEMORY_HOME/work/projects/evil"
  rc=0; out="$(cd "$TMP/code/new" && "$MEMORY" init work --name evil 2>&1)" || rc=$?
  assert_eq "$rc" 1
  ln -s "$TMP/outside" "$AGENT_MEMORY_HOME/work/groups/evil"
  rc=0; out="$("$MEMORY" link-group evil 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_eq "$(ls -A "$TMP/outside")" ""
}

test_init_rejects_symlink_metadata_and_index() {
  "$MEMORY" init work >/dev/null
  printf 'unchanged\n' > "$TMP/outside"
  rm .memory/project/.project-path
  ln -s "$TMP/outside" .memory/project/.project-path
  local rc=0
  "$MEMORY" init work >/dev/null 2>&1 || rc=$?
  assert_eq "$rc" 1
  assert_eq "$(<"$TMP/outside")" unchanged
  rm .memory/project/.project-path
  rm .memory/workspace/INDEX.md
  ln -s "$TMP/outside" .memory/workspace/INDEX.md
  rc=0; "$MEMORY" init work >/dev/null 2>&1 || rc=$?
  assert_eq "$rc" 1
  assert_eq "$(<"$TMP/outside")" unchanged
}

test_init_mismatch_has_no_new_workspace_side_effect() {
  "$MEMORY" init work >/dev/null
  rm .memory/workspace
  "$MEMORY" init other >/dev/null 2>&1 || true
  [ ! -e "$AGENT_MEMORY_HOME/other" ] || fail "mismatch created workspace"
}

test_dream_template_literal_paths_and_controls() {
  dream_template_fixture
  local literal="$TMP/code/__HOME__-__STORE__" out
  mkdir "$literal"
  printf '%s\n' "$literal" > "$DP1/.project-path"
  out="$("$TMEM" dream --print-config)"
  assert_contains "$out" "$literal"
  mkdir "$TMP/code/"$'tab\tpath'
  printf '%s\n' "$TMP/code/"$'tab\tpath' > "$DP1/.project-path"
  out="$("$TMEM" dream --print-config)"
  assert_not_contains "$out" $'tab\tpath'
  mkdir "$TMP/code/"$'cr\rpath'
  ln -s "$TMP/code/"$'cr\rpath' "$TMP/code/safe-name"
  printf '%s\n' "$TMP/code/safe-name" > "$DP1/.project-path"
  out="$("$TMEM" dream --print-config)"
  assert_not_contains "$out" $'cr\rpath'
}

test_failed_dream_commit_failure_survives_next_backup() {
  dream_template_fixture
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf -- '---\ndescription: failed partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
git config user.useConfigOnly true
git config --global --unset user.email
git config --global user.useConfigOnly true
exit 7
EOF
  chmod +x "$TMP/bin/opencode"
  local out rc=0
  out="$("$TMEM" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "could not be created"
  assert_not_contains "$out" "no changes to review"
  assert_file "$AGENT_MEMORY_HOME/.git/memory-dream.failed"
  git -C "$AGENT_MEMORY_HOME" config user.email test@example.com
  out="$("$TMEM" backup)"
  assert_contains "$out" "FAILED review"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)" "memory: dream FAILED (exit 7) — review"
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.failed" ] || fail "marker retained after commit"
}

test_backups_share_atomic_writer_lock() {
  dream_template_fixture
  local realgit
  realgit="$(command -v git)"
  export REAL_GIT="$realgit"
  cat > "$TMP/bin/git" <<'EOF'
#!/usr/bin/env bash
for arg in "$@"; do
  if [ "$arg" = add ] && [ ! -f "$STUB_LOG/release" ]; then
    : > "$STUB_LOG/adding"
    while [ ! -f "$STUB_LOG/release" ]; do sleep 0.1; done
  fi
done
exec "$REAL_GIT" "$@"
EOF
  chmod +x "$TMP/bin/git"
  dfact "$DP1" pending decision "pending" 2026-09-04
  "$TMEM" backup > "$STUB_LOG/first" 2>&1 &
  local writer=$! i=0 out rc=0
  while [ ! -f "$STUB_LOG/adding" ] && [ "$i" -lt 300 ]; do sleep 0.1; i=$((i+1)); done
  assert_file "$STUB_LOG/adding"
  out="$("$TMEM" backup 2>&1)" && fail "busy backup reported success"
  assert_contains "$out" "request NOT saved"
  out="$("$TMEM" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  : > "$STUB_LOG/release"
  wait "$writer"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
}

test_dream_success_marker_updated_under_writer_lock() {
  dream_template_fixture
  REAL_GIT="$(command -v git)"
  export REAL_GIT
  cat > "$TMP/bin/git" <<'EOF'
#!/usr/bin/env bash
for arg in "$@"; do
  if [ "$arg" = update-ref ]; then
    if [ -d "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ]; then
      echo held > "$STUB_LOG/ref-lock"
    else
      echo released > "$STUB_LOG/ref-lock"
    fi
  fi
done
exec "$REAL_GIT" "$@"
EOF
  chmod +x "$TMP/bin/git"
  "$TMEM" dream >/dev/null
  assert_eq "$(<"$STUB_LOG/ref-lock")" held
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)" "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
}

test_timeout_kills_stubborn_grandchildren() {
  dream_template_fixture
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
bash -c 'trap "" TERM; bash -c '\''trap "" TERM; echo $$ > "$STUB_LOG/grandchild"; while :; do sleep 1; done'\'' & echo $$ > "$STUB_LOG/child"; wait' &
while [ ! -f "$STUB_LOG/grandchild" ]; do sleep 0.1; done
echo $$ > "$STUB_LOG/pid"
wait
EOF
  chmod +x "$TMP/bin/opencode"
  AGENT_MEMORY_DREAM_TIMEOUT_SECS=2 AGENT_MEMORY_DREAM_KILL_GRACE=1 "$TMEM" dream >/dev/null 2>&1 || true
  local f pid state
  for f in child grandchild; do
    assert_file "$STUB_LOG/$f"
    pid="$(<"$STUB_LOG/$f")"
    state="$(ps -o stat= -p "$pid" 2>/dev/null || true)"
    case "$state" in ''|*Z*) ;; *) fail "$f still live after timeout: $pid $state" ;; esac
  done
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ] || fail "timeout retained writer lock"
}

test_dream_policy_paths_reject_globs() {
  dream_template_fixture
  local name out rc=0
  for name in 'app?' 'app*' 'app[12]'; do
    mkdir "$TMP/code/$name"
    printf '%s\n' "$TMP/code/$name" > "$DP1/.project-path"
    out="$("$TMEM" dream --print-config 2>"$TMP/warning")"
    assert_not_contains "$out" "$TMP/code/$name"
    assert_contains "$(<"$TMP/warning")" "skipping unsafe"
  done
  mkdir "$TMP/home?"
  out="$(HOME="$TMP/home?" "$TMEM" dream --print-config 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "unsafe JSON/policy"
}

test_successful_dream_persistence_failure_keeps_failed_marker() {
  dream_template_fixture
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf -- '---\ndescription: successful partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
git config --global --unset user.email
git config --global user.useConfigOnly true
exit 0
EOF
  chmod +x "$TMP/bin/opencode"
  local out rc=0
  out="$("$TMEM" dream 2>&1)" || rc=$?
  [ "$rc" -ne 0 ] || fail "save failure returned success"
  assert_contains "$out" "final persistence failed"
  assert_file "$AGENT_MEMORY_HOME/.git/memory-dream.failed"
  git config --global user.email test@example.com
  out="$("$TMEM" backup)"
  assert_contains "$out" "FAILED review"
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)" "dream FAILED"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
}

test_dream_update_ref_failure_keeps_failed_marker() {
  dream_template_fixture
  REAL_GIT="$(command -v git)"; export REAL_GIT
  cat > "$TMP/bin/git" <<'EOF'
#!/usr/bin/env bash
for arg in "$@"; do if [ "$arg" = update-ref ]; then exit 9; fi; done
exec "$REAL_GIT" "$@"
EOF
  chmod +x "$TMP/bin/git"
  local out rc=0
  out="$("$TMEM" dream 2>&1)" || rc=$?
  [ "$rc" -ne 0 ] || fail "ref failure returned success"
  assert_file "$AGENT_MEMORY_HOME/.git/memory-dream.failed"
  assert_contains "$out" "final persistence failed"
  ! "$REAL_GIT" -C "$AGENT_MEMORY_HOME" rev-parse -q --verify refs/memory-dream/last >/dev/null || fail "advanced failed marker"
}

test_successful_dream_index_and_add_failure_keep_marker() {
  dream_template_fixture
  REAL_GIT="$(command -v git)"; export REAL_GIT
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf -- '---\ndescription: partial\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/partial.md
if [ "$FAIL_SAVE" = index ]; then
  printf '#!/bin/sh\nexit 8\n' > "$FAIL_BIN/mv"
  chmod +x "$FAIL_BIN/mv"
else
  : > "$STUB_LOG/fail-add"
fi
EOF
  cat > "$TMP/bin/git" <<'EOF'
#!/usr/bin/env bash
if [ -f "$STUB_LOG/fail-add" ]; then
  for arg in "$@"; do if [ "$arg" = add ]; then exit 8; fi; done
fi
exec "$REAL_GIT" "$@"
EOF
  chmod +x "$TMP/bin/opencode" "$TMP/bin/git"
  local mode out rc
  for mode in index add; do
    rc=0
    out="$(FAIL_SAVE="$mode" FAIL_BIN="$TMP/bin" "$TMEM" dream 2>&1)" || rc=$?
    [ "$rc" -ne 0 ] || fail "$mode save failure returned success"
    assert_file "$AGENT_MEMORY_HOME/.git/memory-dream.failed"
    assert_contains "$out" "final persistence failed"
    rm -f "$TMP/bin/mv" "$STUB_LOG/fail-add"
    "$TMEM" backup >/dev/null
  done
}

test_backup_after_add_contention_commits_newer_edit() {
  dream_template_fixture
  REAL_GIT="$(command -v git)"; export REAL_GIT
  cat > "$TMP/bin/git" <<'EOF'
#!/usr/bin/env bash
"$REAL_GIT" "$@" || exit $?
for arg in "$@"; do
  if [ "$arg" = add ] && [ ! -f "$STUB_LOG/release" ]; then
    : > "$STUB_LOG/after-add"
    while [ ! -f "$STUB_LOG/release" ]; do sleep 0.1; done
  fi
done
EOF
  chmod +x "$TMP/bin/git"
  dfact "$DP1" contention decision "version A" 2026-09-04
  "$TMEM" backup > "$STUB_LOG/A" 2>&1 &
  local first=$! second i=0
  while [ ! -f "$STUB_LOG/after-add" ] && [ "$i" -lt 300 ]; do sleep 0.1; i=$((i+1)); done
  assert_file "$STUB_LOG/after-add"
  dfact "$DP1" contention decision "version B" 2026-09-04
  AGENT_MEMORY_BACKUP_LOCK_WAIT=10 "$TMEM" backup > "$STUB_LOG/B" 2>&1 &
  second=$!
  : > "$STUB_LOG/release"
  wait "$first"
  wait "$second"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" show HEAD:work/projects/some-api/contention.md)" "version B"
}


# Stub opencode that writes a fake run database like the real one: assistant
# rows in session_message whose content holds tool parts. STUB_TOOLS is a
# space-separated list of  <shell-command-or-name>=<ok|denied>.
install_db_stub() {
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
rm -f "$OPENCODE_DB"
sqlite3 "$OPENCODE_DB" "create table session_message (id text primary key, session_id text, type text, seq integer, time_created integer, time_updated integer, data text);"
seq=0
for spec in $STUB_TOOLS; do
  seq=$((seq + 1))
  cmd="${spec%%=*}"; state="${spec#*=}"; cmd="${cmd//+/ }"
  if [ "$state" = ok ]; then
    part="{\"type\":\"tool\",\"name\":\"shell\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"$cmd\"}}}"
  else
    part="{\"type\":\"tool\",\"name\":\"shell\",\"state\":{\"status\":\"error\",\"input\":{\"command\":\"$cmd\"},\"error\":{\"type\":\"permission.rejected\",\"message\":\"Permission denied: shell\"}}}"
  fi
  sqlite3 "$OPENCODE_DB" "insert into session_message values ('m$seq','s','assistant',$seq,0,0,'{\"content\":[$part]}');"
done
exit 0
EOF
  chmod +x "$TMP/bin/opencode"
}

dream_db_fixture() {
  command -v sqlite3 >/dev/null 2>&1 || { echo "  (skipped: sqlite3 not found)"; return 1; }
  dream_launch_fixture
  install_db_stub
}

test_dream_all_tools_denied_is_a_failed_run() {
  dream_db_fixture || return 0
  local out rc=0
  out="$(STUB_TOOLS='x=denied y=denied' "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "sandbox denied the agent's tools; run not counted"
  assert_contains "$out" "0 tool call(s) succeeded, 2 denied"
  ! git -C "$AGENT_MEMORY_HOME" rev-parse -q --verify refs/memory-dream/last >/dev/null \
    || fail "success ref advanced after a broken sandbox"
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ] || fail "lock left behind"
}

test_dream_denied_first_report_call_is_a_failed_run() {
  dream_db_fixture || return 0
  local out rc=0
  out="$(STUB_TOOLS='/x/memory+dream+--report+now=denied other=ok' "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "run not counted"
  ! git -C "$AGENT_MEMORY_HOME" rev-parse -q --verify refs/memory-dream/last >/dev/null \
    || fail "success ref advanced although the report was denied"
}

test_dream_other_denials_are_reported_but_not_fatal() {
  dream_db_fixture || return 0
  local out
  out="$(STUB_TOOLS='/x/memory+dream+--report+now=ok probe=denied probe2=denied' "$MEMORY" dream 2>&1)"
  assert_contains "$out" "2 tool call(s) were denied by the sandbox (1 succeeded)"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)" "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
}

test_dream_unreadable_session_db_warns_and_still_succeeds() {
  dream_launch_fixture   # default stub writes no database
  local out
  out="$("$MEMORY" dream 2>&1)"
  if command -v sqlite3 >/dev/null 2>&1; then
    assert_contains "$out" "cannot inspect the session database"
  fi
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)" "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
}

test_dream_signal_prints_interrupted_notice() {
  dream_launch_fixture
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
printf -- '---\ndescription: interrupted edit\ntype: decision\ncreated: 2026-09-04\n---\nbody\n' > work/projects/some-api/interrupted.md
echo $$ > "$STUB_LOG/pid"
exec sleep 60
EOF
  chmod +x "$TMP/bin/opencode"
  AGENT_MEMORY_DREAM_KILL_GRACE=1 "$MEMORY" dream > "$STUB_LOG/dream-out" 2>&1 &
  local dpid=$! i=0
  while [ ! -s "$STUB_LOG/pid" ] && [ "$i" -lt 600 ]; do sleep 0.1; i=$((i + 1)); done
  assert_file "$STUB_LOG/pid"
  kill -TERM "$dpid"
  wait "$dpid" 2>/dev/null || true
  assert_contains "$(<"$STUB_LOG/dream-out")" "opencode run interrupted (exit 143)"
  assert_contains "$(<"$STUB_LOG/dream-out")" "committed as FAILED review"
}


# Stub opencode that runs $STUB_ACT (a shell snippet) inside the store, then exits 0.
install_acting_stub() {
  cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
eval "$STUB_ACT"
exit 0
EOF
  chmod +x "$TMP/bin/opencode"
}

FACT_BODY='---\ndescription: escaped\ntype: decision\ncreated: 2026-09-04\n---\nbody\n'

assert_escape_is_failed_review() {   # assert_escape_is_failed_review <act> <expected-path>
  local out rc=0 marker_before
  marker_before="$(git -C "$AGENT_MEMORY_HOME" rev-parse -q --verify refs/memory-dream/last || true)"
  out="$(STUB_ACT="$1" "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "run not counted"
  assert_contains "$out" "$2"
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)" "dream FAILED"
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%B)" "$2"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse -q --verify refs/memory-dream/last || true)" "$marker_before"
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ] || fail "lock left behind"
  # Nothing is deleted automatically.
  [ -e "$AGENT_MEMORY_HOME/$2" ] || fail "unexpected path was deleted: $2"
}

test_dream_new_project_directory_is_a_failed_review() {
  dream_launch_fixture
  install_acting_stub
  assert_escape_is_failed_review "mkdir work/projects/brand-new && printf -- '$FACT_BODY' > work/projects/brand-new/x.md" "work/projects/brand-new/x.md"
}

test_dream_new_group_directory_is_a_failed_review() {
  dream_launch_fixture
  install_acting_stub
  assert_escape_is_failed_review "mkdir -p work/groups/brand-new && printf -- '$FACT_BODY' > work/groups/brand-new/x.md" "work/groups/brand-new/x.md"
}

test_dream_nested_hidden_and_non_markdown_files_are_failed_reviews() {
  dream_launch_fixture
  install_acting_stub
  assert_escape_is_failed_review "mkdir work/projects/some-api/deeper && printf -- '$FACT_BODY' > work/projects/some-api/deeper/x.md" "work/projects/some-api/deeper/x.md"
  assert_escape_is_failed_review "printf x > work/projects/some-api/.hidden.md" "work/projects/some-api/.hidden.md"
  assert_escape_is_failed_review "printf x > work/projects/some-api/notes.txt" "work/projects/some-api/notes.txt"
}

test_dream_deleted_fact_is_a_failed_review_without_deleting_more() {
  dream_launch_fixture
  install_acting_stub
  local out rc=0
  out="$(STUB_ACT='rm work/projects/some-api/stale-fact.md' "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "work/projects/some-api/stale-fact.md"
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%s)" "dream FAILED"
}

test_dream_staged_rename_is_reported_as_plain_delete_and_add() {
  dream_launch_fixture
  install_acting_stub
  local out rc=0
  # A staged rename of an almost identical file: git would pair them as "R".
  out="$(STUB_ACT='git mv work/projects/some-api/stale-fact.md work/projects/some-api/renamed-fact.md' "$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "run not counted"
  assert_contains "$out" "
  work/projects/some-api/stale-fact.md
"
  assert_not_contains "$out" " -> "
  assert_contains "$(git -C "$AGENT_MEMORY_HOME" log -1 --format=%B)" "- work/projects/some-api/stale-fact.md"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" status --porcelain)" ""
  ! git -C "$AGENT_MEMORY_HOME" rev-parse -q --verify refs/memory-dream/last >/dev/null || fail "ref advanced"
}

test_dream_allowed_writes_in_existing_levels_still_succeed() {
  dream_launch_fixture
  install_acting_stub
  local out
  out="$(STUB_ACT="printf -- '$FACT_BODY' > work/projects/some-api/new-fact.md; printf -- '$FACT_BODY' > work/workspace/ws-fact.md" "$MEMORY" dream 2>&1)"
  assert_not_contains "$out" "run not counted"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse refs/memory-dream/last)" "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
  assert_contains "$(<"$DP1/INDEX.md")" "new-fact"
}

test_dream_refuses_to_run_with_symlinks_in_the_store() {
  dream_launch_fixture
  mkdir "$TMP/outside"
  ln -s "$TMP/outside" "$DP1/escape"
  local out rc=0 head
  head="$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)"
  out="$("$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "store contains symlinks"
  assert_contains "$out" "$DP1/escape"
  [ ! -e "$STUB_LOG/args" ] || fail "opencode launched with a symlink in the store"
  assert_eq "$(git -C "$AGENT_MEMORY_HOME" rev-parse HEAD)" "$head"
  [ ! -e "$AGENT_MEMORY_HOME/.git/memory-dream.lock" ] || fail "lock left behind"
  # A symlinked fact is caught too.
  rm "$DP1/escape"
  ln -s "$TMP/outside" "$DP1/linked-fact.md"
  rc=0; out="$("$MEMORY" dream 2>&1)" || rc=$?
  assert_eq "$rc" 1
  assert_contains "$out" "linked-fact.md"
}

test_legit_init_and_link_group_leave_no_symlinks_in_the_store() {
  "$MEMORY" init work >/dev/null
  "$MEMORY" link-group shared >/dev/null
  mkdir "$TMP/code/second"
  (cd "$TMP/code/second" && "$MEMORY" init work >/dev/null && "$MEMORY" link-group shared >/dev/null)
  assert_eq "$(find "$AGENT_MEMORY_HOME" -name .git -prune -o -type l -print)" ""
  [ -L .memory/project ] || fail "the .memory links must stay in the project directory"
}

# ---------------- runner ----------------

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do run_test "$t"; done
echo "passed $PASS, failed $FAIL"
[ "$FAIL" -eq 0 ]
