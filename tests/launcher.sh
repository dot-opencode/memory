#!/usr/bin/env bash
# Standalone, offline tests for skills/agent-memory/bin/memory.
# Uses AGENT_MEMORY_RELEASE_BASE_URL (test-only override) with file:// URLs.
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LAUNCHER_SRC=$ROOT/skills/agent-memory/bin/memory

TMP=$(mktemp -d "${TMPDIR:-/tmp}/launcher-test.XXXXXX") && TMP=$(cd -P "$TMP" && pwd)
trap 'rm -rf "$TMP"' EXIT

pass=0
fail=0
ok() { pass=$((pass + 1)); printf 'ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf 'FAIL %s\n' "$1"; }
check() { # name, command...  (passes when the command succeeds)
  n=$1; shift
  if "$@"; then ok "$n"; else bad "$n"; fi
}
eq() { [ "$1" = "$2" ]; }

sha() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}

# Fake release: tarball containing a `memory` shell script.
REL=$TMP/releases
VERSION=9.9.9
TARGETS="aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
mkdir -p "$REL/v$VERSION" "$TMP/pkg"
cat >"$TMP/pkg/memory" <<'EOF'
#!/bin/sh
echo "fake-memory args=$* self=${AGENT_MEMORY_SELF:-}"
EOF
chmod +x "$TMP/pkg/memory"
for t in $TARGETS; do
  tar -czf "$REL/v$VERSION/memory-$t.tar.gz" -C "$TMP/pkg" memory
done
GOOD_SHA=$(sha "$REL/v$VERSION/memory-aarch64-apple-darwin.tar.gz")
# All tarballs are identical bytes content-wise but gzip headers may differ; hash each.
write_release() { # dir, good|bad
  {
    echo "version $VERSION"
    for t in $TARGETS; do
      if [ "$2" = good ]; then
        echo "sha256 $t $(sha "$REL/v$VERSION/memory-$t.tar.gz")"
      else
        echo "sha256 $t 0000000000000000000000000000000000000000000000000000000000000000"
      fi
    done
  } >"$1/release.txt"
}
: "$GOOD_SHA"

new_install() { # name -> dir with launcher
  d=$TMP/$1
  mkdir -p "$d/bin"
  cp "$LAUNCHER_SRC" "$d/bin/memory"
  chmod +x "$d/bin/memory"
  echo "$d/bin"
}

export AGENT_MEMORY_RELEASE_BASE_URL="file://$REL"
unset AGENT_MEMORY_BIN

# --- 1. download + verify, then cache hit -----------------------------------
BIN1=$(new_install t1)
write_release "$BIN1" good
export XDG_CACHE_HOME=$TMP/cache1
out=$("$BIN1/memory" hello world 2>"$TMP/err1"); rc=$?
if [ $rc -eq 0 ] && [ "$out" = "fake-memory args=hello world self=$BIN1/memory" ]; then ok "download+verify runs binary"; else bad "download+verify runs binary"; fi
cached=$(find "$XDG_CACHE_HOME/agent-memory/$VERSION" -name memory -type f | head -n1)
if [ -x "$cached" ]; then ok "binary cached"; else bad "binary cached"; fi
leftover=$(find "$XDG_CACHE_HOME" -name '.tmp.*' | wc -l | tr -d ' ')
if [ "$leftover" = 0 ]; then ok "no temp leftovers"; else bad "no temp leftovers"; fi

# Cache hit: break the base URL; must still work.
out=$(AGENT_MEMORY_RELEASE_BASE_URL="file:///nonexistent" "$BIN1/memory" again 2>"$TMP/err1b"); rc=$?
if [ $rc -eq 0 ] && [ "$out" = "fake-memory args=again self=$BIN1/memory" ]; then ok "cache hit needs no network"; else bad "cache hit needs no network"; fi

# --- 2. checksum mismatch refuses to run ------------------------------------
BIN2=$(new_install t2)
write_release "$BIN2" bad
export XDG_CACHE_HOME=$TMP/cache2
out=$("$BIN2/memory" x 2>"$TMP/err2"); rc=$?
if [ $rc -eq 1 ]; then ok "mismatch exits 1"; else bad "mismatch exits 1"; fi
if [ -z "$out" ]; then ok "mismatch prints nothing from binary"; else bad "mismatch prints nothing from binary"; fi
if grep -q '^memory: checksum mismatch' "$TMP/err2"; then ok "mismatch message on stderr"; else bad "mismatch message on stderr"; fi
nbin=$(find "$XDG_CACHE_HOME" -name memory -type f 2>/dev/null | wc -l | tr -d ' ')
if [ "$nbin" = 0 ]; then ok "mismatch leaves no binary"; else bad "mismatch leaves no binary"; fi

# --- 3. AGENT_MEMORY_BIN override -------------------------------------------
BIN3=$(new_install t3)
echo "garbage" >"$BIN3/release.txt"
export XDG_CACHE_HOME=$TMP/cache3
out=$(AGENT_MEMORY_BIN="$TMP/pkg/memory" "$BIN3/memory" ov 2>&1); rc=$?
if [ $rc -eq 0 ] && [ "$out" = "fake-memory args=ov self=$BIN3/memory" ]; then ok "AGENT_MEMORY_BIN override"; else bad "AGENT_MEMORY_BIN override"; fi
if [ ! -e "$XDG_CACHE_HOME" ]; then ok "override does not touch cache"; else bad "override does not touch cache"; fi

# --- 4. symlinked invocation ------------------------------------------------
BIN4=$(new_install t4)
write_release "$BIN4" good
mkdir -p "$TMP/t4/link/deeper"
ln -s "$BIN4/memory" "$TMP/t4/link/memory"
ln -s ../memory "$TMP/t4/link/deeper/memory"
export XDG_CACHE_HOME=$TMP/cache4
out=$("$TMP/t4/link/memory" viasym 2>/dev/null); rc=$?
if [ $rc -eq 0 ] && [ "$out" = "fake-memory args=viasym self=$BIN4/memory" ]; then ok "symlink finds release.txt, SELF is real path"; else bad "symlink finds release.txt, SELF is real path"; fi
out=$("$TMP/t4/link/deeper/memory" relsym 2>/dev/null); rc=$?
if [ $rc -eq 0 ] && [ "$out" = "fake-memory args=relsym self=$BIN4/memory" ]; then ok "relative symlink chain"; else bad "relative symlink chain"; fi

# --- 5. missing release.txt ---------------------------------------------------
BIN5=$(new_install t5)
out=$("$BIN5/memory" 2>"$TMP/err5"); rc=$?
if [ $rc -eq 1 ] && grep -q '^memory: ' "$TMP/err5"; then ok "missing release.txt errors"; else bad "missing release.txt errors"; fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
