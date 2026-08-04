#!/usr/bin/env bash
# Regression test for the bug fixed in commit 7a5b6c7: clicking the tray
# icon showed only a "Quit easy-term" menu instead of opening the popover,
# because attaching a native menu to the NSStatusItem makes AppKit show it
# on every click. There's no menu attached anymore — left-click must always
# toggle the popover.
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/common.sh"

flow_main() {
  log "flow: tray icon toggles the popover"

  if popover_is_visible; then
    click_tray_icon
    wait_until 5 '! popover_is_visible'
  fi

  if popover_is_visible; then
    fail "popover was already visible and didn't hide before the test started"
    return
  fi

  click_tray_icon
  if wait_until 5 'popover_is_visible'; then
    pass "left-click on the tray icon opened the popover"
  else
    fail "left-click on the tray icon did not open the popover (regression: the old bug showed only a Quit menu instead)"
    return
  fi

  click_tray_icon
  if wait_until 5 '! popover_is_visible'; then
    pass "left-click on the tray icon again closed the popover"
  else
    fail "left-click on the tray icon did not close the popover on the second click"
  fi
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  trap teardown_app EXIT
  build_app && launch_app && flow_main
  [ "$FAILURES" -eq 0 ]
fi
