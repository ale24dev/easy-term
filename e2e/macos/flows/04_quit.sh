#!/usr/bin/env bash
# "Salir de easy-term" (Settings) must actually terminate the process — this
# is the other half of the fix in commit 7a5b6c7 (moving Quit out of the
# now-removed native tray menu and into the popover).
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/common.sh"

flow_main() {
  log "flow: Salir de easy-term actually exits"

  if [ -z "$APP_PID" ]; then
    fail "no app PID to check against — was the app launched?"
    return
  fi

  if ! popover_is_visible; then
    click_tray_icon
    wait_until 5 'popover_is_visible' || {
      fail "could not open the popover to start this flow"
      return
    }
  fi

  click_by_accessible_text "Diagnóstico"
  if ! wait_until 3 'element_with_text_exists "Salir de easy-term"'; then
    fail "Settings view never opened (missing the 'Salir de easy-term' button)"
    return
  fi
  pass "Settings view opened"

  click_by_accessible_text "Salir de easy-term"
  if wait_until 5 '! kill -0 "$APP_PID" 2>/dev/null'; then
    pass "process exited after clicking Salir de easy-term"
  else
    fail "process was still alive 5s after clicking Salir de easy-term"
  fi
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  trap teardown_app EXIT
  build_app && launch_app && flow_main
  [ "$FAILURES" -eq 0 ]
fi
