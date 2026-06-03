#!/usr/bin/env bash
# Offline smoke test: for every preview format, boot `als preview` on a
# temp port, curl `/`, expect HTTP 200 plus a known marker. First
# failure short-circuits. Driven by `just preview-sweep`.

set -euo pipefail

# Resolve repo root from the script location so this works regardless
# of cwd or how `just` invoked it.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURES="$SCRIPT_DIR"
TOSS="${ALS_BIN:-$REPO_ROOT/target/release/als}"
ZIP="$REPO_ROOT/target/preview-fixtures/html-dir.zip"

if [[ ! -x "$TOSS" ]]; then
  echo "sweep: $TOSS not built; run \`just build\` first" >&2
  exit 2
fi
if [[ ! -f "$ZIP" ]]; then
  echo "sweep: $ZIP missing; run \`just preview-zip-build\` first" >&2
  exit 2
fi

PORT_BASE="${PREVIEW_SWEEP_PORT_BASE:-28200}"
PASS=0
FAIL=0

run_one() {
  local label="$1" path="$2" marker="$3" offset="$4"
  shift 4
  local extra_args=("$@")
  local port=$((PORT_BASE + offset))
  local out logf code
  out="$(mktemp)"
  logf="$(mktemp)"

  "$TOSS" preview "$path" --bind 127.0.0.1 --port "$port" --quiet "${extra_args[@]}" \
    >"$logf" 2>&1 &
  local pid=$!

  # Poll briefly for readiness.
  local i
  for i in $(seq 1 30); do
    if curl -fsS "http://127.0.0.1:$port/" -o "$out" 2>/dev/null; then
      break
    fi
    sleep 0.2
  done

  code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/")
  if [[ "$code" == "200" ]] && grep -qF "$marker" "$out"; then
    printf '  %-18s PASS  (port %s, marker=%q)\n' "$label" "$port" "$marker"
    PASS=$((PASS + 1))
  else
    printf '  %-18s FAIL  (port %s, http=%s, marker=%q)\n' "$label" "$port" "$code" "$marker"
    echo "  --- preview log:"; sed 's/^/    /' "$logf" | head -8
    echo "  --- body head:"; sed 's/^/    /' "$out" | head -4
    FAIL=$((FAIL + 1))
  fi

  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  rm -f "$out" "$logf"
}

echo "==> als preview format sweep"
run_one html-dir      "$FIXTURES/html-dir"    'probe-html-dir'                  0
run_one single-html   "$FIXTURES/single.html" 'probe-single-html'               1
run_one single-md     "$FIXTURES/single.md"   'als preview single-md fixture'   2
run_one md-dir        "$FIXTURES/md-dir"      'als preview md-dir fixture'      3
run_one mdbook        "$FIXTURES/mdbook"      'als preview mdbook fixture'      4
run_one zip           "$ZIP"                  'probe-html-dir'                  5
run_one code-dir      "$FIXTURES/code-dir"    'als preview code-dir fixture'    6  --kind code
run_one single-code   "$FIXTURES/single.ts"   'single.ts'                       7

echo
echo "==> $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
