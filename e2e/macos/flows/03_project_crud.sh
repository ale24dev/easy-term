#!/usr/bin/env bash
# Happy-path smoke test: create a project, start it, stop it, delete it.
# Doesn't touch pnpm/node — the scratch folder has no package.json, and the
# command is overridden to a plain `sleep`, so this only exercises easy-term's
# own process lifecycle, not a real dev server.
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/common.sh"

flow_main() {
  log "flow: create, start, stop, delete a project"

  local scratch_dir="$E2E_HOME/scratch-project"
  mkdir -p "$scratch_dir"

  if ! popover_is_visible; then
    click_tray_icon
    wait_until 5 'popover_is_visible' || {
      fail "could not open the popover to start this flow"
      return
    }
  fi

  click_by_accessible_text "+ Proyecto"
  wait_until 3 'element_with_text_exists "Elegir…"' || {
    fail "new-project form did not open"
    return
  }

  click_by_accessible_text "Elegir…"
  wait_until 3 'open_folder_picker_sheet_exists' || {
    fail "folder picker never opened"
    return
  }
  pick_folder_via_go_to_folder "$scratch_dir"
  if ! wait_until 5 '! open_folder_picker_sheet_exists'; then
    fail "folder picker did not close after selecting the scratch folder"
    return
  fi
  pass "picked the scratch folder as the project path"

  # A short, dependency-free command so start/stop is fast and doesn't
  # require pnpm/node to be on the CI runner's PATH.
  set_text_field_value "Comando" "sleep 30"

  click_by_accessible_text "Guardar"
  if wait_until 5 'element_with_text_exists "Iniciar"'; then
    pass "project was saved and appears in the list (Iniciar button present)"
  else
    fail "project row with an 'Iniciar' action never appeared after saving"
    return
  fi

  click_by_accessible_text "Iniciar"
  if wait_until 5 'element_with_text_exists "Detener"'; then
    pass "starting the project swapped Iniciar for Detener (running)"
  else
    fail "project never reached the running state after clicking Iniciar"
  fi

  click_by_accessible_text "Detener"
  if wait_until 5 'element_with_text_exists "Iniciar"'; then
    pass "stopping the project swapped Detener back for Iniciar"
  else
    fail "project never returned to the stopped state after clicking Detener"
  fi

  click_by_accessible_text "Eliminar"
  if wait_until 5 '! element_with_text_exists "Iniciar"'; then
    pass "deleting the project removed it from the list"
  else
    fail "project row was still present after clicking Eliminar"
  fi
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  trap teardown_app EXIT
  build_app && launch_app && flow_main
  [ "$FAILURES" -eq 0 ]
fi
