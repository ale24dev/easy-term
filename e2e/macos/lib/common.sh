#!/usr/bin/env bash
# Shared helpers for the macOS UI E2E flows. Sourced by run.sh and by each
# flows/NN_*.sh script (which can also be run standalone for local debugging).
set -uo pipefail

APP_NAME="easy-term"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BUNDLE_DIR="$REPO_ROOT/src-tauri/target/debug/bundle/macos"
APP_BUNDLE="$BUNDLE_DIR/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/$APP_NAME"

# Isolated HOME-relative data dirs so a CI run never touches (or is polluted
# by) a real user's projects.json / diagnostics log. easy-term resolves both
# under $HOME, so pointing HOME at a throwaway dir is enough. Deliberately
# NOT exported yet — cargo/rustup also resolve their toolchain under $HOME
# (~/.rustup, ~/.cargo) by default, so overriding it before build_app() runs
# `pnpm tauri build` makes rustup unable to find the toolchain at all
# ("no default is configured", even right after installing one). HOME gets
# swapped in launch_app(), once the build no longer needs the real one.
E2E_HOME="$(mktemp -d)"

APP_PID=""
FAILURES=0

log() { echo "[e2e] $*"; }
pass() { echo "  ✓ $*"; }
fail() {
  echo "  ✗ $*" >&2
  FAILURES=$((FAILURES + 1))
}

# Runs an inline AppleScript snippet via osascript. Usage:
#   osa 'tell application "System Events" to ...'
# Prints stdout on success; on failure prints stderr and returns osascript's
# exit code so callers can branch on it.
osa() {
  osascript -e "$1"
}

build_app() {
  log "building debug .app bundle (pnpm tauri build --debug)…"
  (cd "$REPO_ROOT" && pnpm tauri build --debug --bundles app) || {
    log "build failed"
    return 1
  }
  if [ ! -x "$APP_BINARY" ]; then
    log "expected binary not found at $APP_BINARY"
    return 1
  fi
}

launch_app() {
  export HOME="$E2E_HOME"
  log "launching $APP_BINARY (HOME=$E2E_HOME)…"
  "$APP_BINARY" &
  APP_PID=$!

  if ! wait_until 10 "osa 'tell application \"System Events\" to exists process \"$APP_NAME\"' | grep -q true"; then
    log "process never registered with System Events"
    return 1
  fi
  # A freshly-registered process may not have its menu bar item attached
  # yet — give the tray setup a beat to finish.
  sleep 1
}

# Best-effort cleanup: SIGTERM first (in case a flow left the popover open
# rather than actually quitting through the UI), then SIGKILL after a grace
# period. Safe to call even if the app already exited on its own.
teardown_app() {
  if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null
    wait_until 5 "! kill -0 $APP_PID 2>/dev/null" || kill -9 "$APP_PID" 2>/dev/null
  fi
  rm -rf "$E2E_HOME"
}

# wait_until <timeout_seconds> <shell command as a single string>
# Polls every 250ms until the command exits 0, or the timeout elapses.
wait_until() {
  local timeout="$1" cmd="$2"
  local deadline=$((SECONDS + timeout))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if eval "$cmd"; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

click_tray_icon() {
  # NSStatusItem-based tray icons show up as the *second* menu bar
  # (`menu bar 2`) in a process's accessibility tree — `menu bar 1` is
  # reserved for a real app menu bar (File/Edit/...), which an Accessory-
  # policy app like this one doesn't have. This is the standard incantation
  # for scripting third-party menu-bar-only apps.
  osa "tell application \"System Events\" to tell process \"$APP_NAME\" to click menu bar item 1 of menu bar 2"
}

popover_window_count() {
  osa "tell application \"System Events\" to tell process \"$APP_NAME\" to count windows"
}

popover_is_visible() {
  [ "$(popover_window_count)" -ge 1 ] 2>/dev/null
}

click_button() {
  osa "tell application \"System Events\" to tell process \"$APP_NAME\" to click button \"$1\" of window 1"
}

# React icon-only buttons here set an HTML `title=` attribute for the
# tooltip (e.g. `<button title="Iniciar">▶</button>`) — WebKit accessibility
# usually maps that to AXHelp rather than the button's AXTitle/name (which
# comes from the emoji text content instead). Matching on name OR help OR
# description covers plain-text buttons ("Proyecto", "Salir de easy-term")
# and title-only icon buttons ("Iniciar", "Detener", ...) with one helper,
# without needing to know in advance which attribute WebKit picked.
click_by_accessible_text() {
  osa "tell application \"System Events\" to tell process \"$APP_NAME\" to click (first UI element of window 1 whose (name is \"$1\") or (help is \"$1\") or (description is \"$1\"))"
}

element_with_text_exists() {
  osa "tell application \"System Events\" to tell process \"$APP_NAME\" to exists (first UI element of window 1 whose (name is \"$1\") or (help is \"$1\") or (description is \"$1\"))" | grep -q true
}

open_folder_picker_sheet_exists() {
  osa "tell application \"System Events\" to tell process \"$APP_NAME\" to exists sheet 1 of window 1" | grep -q true
}

# Robust NSOpenPanel path entry: Cmd+Shift+G ("Go to Folder…") + typing the
# path is the standard reliable way to script a macOS open/save panel,
# far less brittle than clicking through the sidebar or double-clicking
# rows in the file list. For a directory-choosing panel (this app's picker
# is `open({ directory: true })`), typing a directory's own path into "Go to
# Folder" and pressing Return selects and confirms *that* directory in one
# shot — it does not merely navigate inside it. Only one Return, deliberately:
# a second keystroke here would land on whatever regains focus once the
# sheet closes (the popover/form), risking an accidental early form submit.
pick_folder_via_go_to_folder() {
  local target_path="$1"
  osa "tell application \"System Events\"
    keystroke \"g\" using {command down, shift down}
    delay 0.3
    keystroke \"$target_path\"
    keystroke return
  end tell" >/dev/null
}

close_open_folder_picker() {
  osa "tell application \"System Events\" to tell process \"$APP_NAME\" to click button \"Cancel\" of sheet 1 of window 1" >/dev/null 2>&1
}

# Replaces a labeled text field's value via a real focus + select-all +
# keystroke sequence — this goes through actual DOM input events, unlike
# setting the accessibility value directly, so React's onChange sees it.
# Relies on <Label htmlFor="…"><Input id="…"/> association exposing the
# label text as the field's accessible name, which ProjectForm.tsx uses
# throughout.
set_text_field_value() {
  local label="$1" value="$2"
  osa "tell application \"System Events\" to tell process \"$APP_NAME\"
    click text field \"$label\" of window 1
    keystroke \"a\" using {command down}
    keystroke \"$value\"
  end tell" >/dev/null
}
