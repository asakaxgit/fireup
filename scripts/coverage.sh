#!/usr/bin/env bash
#
# Line coverage for the Fireup library, as a number and as a report you can read.
#
#   scripts/coverage.sh                 # summary on stdout, lcov + HTML in target/coverage/
#   scripts/coverage.sh --open          # ... and open the HTML report
#   FIREUP_COVERAGE_MIN=80 scripts/coverage.sh   # fail under a different floor
#
# Scope: `--lib`, the unit tests inside `src/`. That is deliberate and it is the
# same scope CI enforces. The integration suites need a PostgreSQL server, a
# Node toolchain, and in places a running `fireup local`; folding them in would
# make the number depend on which services happened to be up, which is worse
# than a smaller number that always means the same thing. `cargo llvm-cov
# --tests` is there when you want the wider picture locally.
#
# Requires: cargo-llvm-cov and the llvm-tools component.
#   cargo install cargo-llvm-cov --locked
#   rustup component add llvm-tools-preview
set -euo pipefail

cd "$(dirname "$0")/.."

# Keep this in step with .github/workflows/coverage.yml and the README badge;
# `cargo test --test fidelity_matrix_tests` checks that they agree.
MIN="${FIREUP_COVERAGE_MIN:-75}"
OUT_DIR="${FIREUP_COVERAGE_DIR:-target/coverage}"
open_report=false
[[ "${1:-}" == "--open" ]] && open_report=true

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "coverage.sh: cargo-llvm-cov is not installed." >&2
  echo "  cargo install cargo-llvm-cov --locked && rustup component add llvm-tools-preview" >&2
  exit 127
fi

mkdir -p "$OUT_DIR"

# One instrumented test run, then every report format off the same profile data.
echo "==> running the unit tests under instrumentation"
cargo llvm-cov --lib --no-report

echo "==> writing reports to $OUT_DIR"
cargo llvm-cov report --lcov --output-path "$OUT_DIR/lcov.info"
cargo llvm-cov report --html --output-dir "$OUT_DIR/html" >/dev/null
cargo llvm-cov report --json --summary-only --output-path "$OUT_DIR/summary.json"
cargo llvm-cov report --summary-only | tee "$OUT_DIR/summary.txt"

lines_pct="$(python3 -c '
import json, sys
totals = json.load(open(sys.argv[1]))["data"][0]["totals"]
print("%.2f" % totals["lines"]["percent"])
' "$OUT_DIR/summary.json")"

echo
echo "line coverage: ${lines_pct}%  (floor ${MIN}%)"
echo "  lcov:   $OUT_DIR/lcov.info"
echo "  html:   $OUT_DIR/html/index.html"

$open_report && command -v xdg-open >/dev/null && xdg-open "$OUT_DIR/html/index.html" || true

# Compared as numbers, not strings: "9.5" > "80" is true for a shell test.
if python3 -c "import sys; sys.exit(0 if float(sys.argv[1]) >= float(sys.argv[2]) else 1)" \
    "$lines_pct" "$MIN"; then
  exit 0
fi

echo "coverage.sh: ${lines_pct}% line coverage is below the ${MIN}% floor." >&2
exit 1
