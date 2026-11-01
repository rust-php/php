#!/bin/sh
# Goal check for the rust-php rewrite. Prints GOAL_MET only when every project phase target is reached.
# Oracle: php-src tests that the real php 8.1.2 passes (tests/oracle_*.txt).
# Targets (from the Project): Phase 3 Zend/tests >= 90%, Phase 4 ext/standard >= 85%, Phase 8 JIT present.
set -u
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUNNER="$ROOT/phpt-runner/target/release/phpt-runner"
PHP="$ROOT/php/target/release/php"
SRC="$ROOT/php-src"
ORA="$ROOT/php/tests"
fail=0

cargo build --release -q --manifest-path "$ROOT/php/Cargo.toml" || exit 1

pct() { # name oracle-file test-dir target
  r=$(timeout 1200 "$RUNNER" "$PHP" "$SRC/$3" --only "$ORA/$2")
  p=$(echo "$r" | sed -E 's/.*"pass":([0-9]+).*"total":([0-9]+).*/\1/')
  t=$(echo "$r" | sed -E 's/.*"pass":([0-9]+).*"total":([0-9]+).*/\2/')
  v=$(( p * 100 / t ))
  echo "$1: $p/$t = $v% (target $4%)"
  [ "$v" -ge "$4" ] || fail=1
}

pct "Phase 3 Zend/tests" oracle_zend_pass.txt Zend/tests 90
pct "Phase 4 ext/standard" oracle_ext_standard.txt ext/standard/tests 85

state=$(gh issue view 4 -R rust-php/php --json state --jq .state 2>/dev/null)
echo "Phase 3 issue #4 state: ${state:-unknown}"
[ "$state" = "CLOSED" ] || fail=1

jit=$(grep -rl "cranelift" "$ROOT/php/Cargo.toml" 2>/dev/null)
echo "Phase 8 JIT dependency: ${jit:-missing}"
[ -n "$jit" ] || fail=1

[ "$fail" -eq 0 ] && echo GOAL_MET || echo GOAL_NOT_MET
