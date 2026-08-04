#!/usr/bin/env bash
# Regression test for the bug fixed in commit fabbed6: opening the folder
# picker ("Elegir…" in the new-project form) would immediately close itself,
# because it's presented as a sheet attached to the popover window, and the
# popover's hide-on-blur handler fired the moment the sheet took focus,
# hiding the window (and the sheet riding on it) right back closed.
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/common.sh"

flow_main() {
  log "flow: folder picker stays open instead of closing itself"

  if ! popover_is_visible; then
    click_tray_icon
    wait_until 5 'popover_is_visible' || {
      fail "could not open the popover to start this flow"
      return
    }
  fi

  click_by_accessible_text "+ Proyecto"
  if ! wait_until 3 'element_with_text_exists "Elegir…"'; then
    fail "new-project form did not open (missing the 'Elegir…' button)"
    return
  fi
  pass "new-project form opened"

  click_by_accessible_text "Elegir…"
  if ! wait_until 3 'open_folder_picker_sheet_exists'; then
    fail "folder picker sheet never appeared after clicking 'Elegir…'"
    return
  fi
  pass "folder picker sheet opened"

  # This delay is the whole point of the test: the old bug closed the sheet
  # almost immediately (it rode out with the hidden parent window). Long
  # enough to make the flakiness margin comfortable, short enough to keep
  # the suite fast.
  sleep 2

  if open_folder_picker_sheet_exists; then
    pass "folder picker sheet is still open after 2s (did not close itself)"
  else
    fail "folder picker sheet closed itself (regression: the popover hid out from under it)"
    close_open_folder_picker
    return
  fi

  if popover_is_visible; then
    pass "popover window is still visible while the sheet is open"
  else
    fail "popover window hid itself while the folder picker sheet was open"
  fi

  close_open_folder_picker
  wait_until 3 '! open_folder_picker_sheet_exists'
  click_by_accessible_text "Cancelar"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  trap teardown_app EXIT
  build_app && launch_app && flow_main
  [ "$FAILURES" -eq 0 ]
fi
