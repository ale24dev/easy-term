#!/usr/bin/env bash
# Runs every flows/NN_*.sh against a single build + launch of the app.
# Exits non-zero if any flow reported a failure — the exit code is what
# CI gates on.
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/common.sh"

if [ "$(uname)" != "Darwin" ]; then
  echo "[e2e] this suite only runs on macOS (uses AppleScript/System Events); skipping." >&2
  exit 0
fi

trap teardown_app EXIT

build_app || { echo "[e2e] build failed, aborting."; exit 1; }
launch_app || { echo "[e2e] app failed to launch, aborting."; exit 1; }

TOTAL_FAILURES=0
for flow in "$SCRIPT_DIR"/flows/*.sh; do
  FAILURES=0
  # shellcheck disable=SC1090
  source "$flow"
  flow_main
  if [ "$FAILURES" -gt 0 ]; then
    echo "[e2e] $(basename "$flow"): $FAILURES failure(s)"
    TOTAL_FAILURES=$((TOTAL_FAILURES + FAILURES))
  else
    echo "[e2e] $(basename "$flow"): ok"
  fi
done

echo "---"
if [ "$TOTAL_FAILURES" -eq 0 ]; then
  echo "[e2e] all flows passed"
  exit 0
else
  echo "[e2e] $TOTAL_FAILURES failure(s) total"
  exit 1
fi
