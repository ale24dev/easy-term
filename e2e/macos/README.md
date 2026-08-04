# macOS UI E2E suite

Drives the actual compiled `.app` through the real macOS menu bar / window
system, using AppleScript's `System Events` accessibility bridge — the only
option here, because `tauri-driver` (the WebDriver-based tool Tauri
recommends for E2E) does not support macOS; only Linux and Windows. See
`PLAN.md` for the fuller writeup of why this layer exists at all.

## Why this exists

The unit/integration tests under `src-tauri/src/**/tests` and
`src/**/*.test.ts` cover business logic fast and reliably, but they run
fine on Linux — and both real bugs found in this app so far (the tray icon
showing only "Quit", the folder picker closing itself) were AppKit-specific
behavior that's invisible outside real macOS. This suite exists specifically
to catch that class of regression, by driving the literal `.app` on the
`macos-latest` CI runner.

## Layout

- `lib/common.sh` — shared helpers: launch/quit the app, run inline
  AppleScript, poll-until-true, process/window introspection.
- `flows/NN_*.sh` — one flow per script, each independently runnable
  (`./flows/01_tray_toggle.sh`). `run.sh` runs all of them in order and
  reports a pass/fail summary.

## Running locally (on a real Mac)

```sh
./e2e/macos/run.sh
```

Builds a debug `.app` bundle via `pnpm tauri build --debug`, then runs every
flow against it. Requires the terminal running this script to have
Accessibility permission (System Settings → Privacy & Security →
Accessibility) — without it, every `osascript`/System Events call fails with
"not allowed assistive access" (error -1719 / -25211).

## If it fails in CI with a permission error

The `macos-e2e` CI job doesn't attempt to pre-grant System Events
Accessibility access (an earlier draft tried writing directly into
`TCC.db`, but that's an undocumented, version-fragile hack with no way to
verify the schema without a real Mac — deleted rather than shipped as a
silent no-op). If the CI log shows osascript error -1743 or -25211
("not allowed to send keystrokes" / "assistive access"), that's the
runner's Accessibility permission, not a real app regression: look up
GitHub's current guidance for macOS runners (or a `tccutil`/profile-based
grant in a setup step) and wire it into the `macos-e2e` job.

## A note on confidence

This suite was written without access to a real Mac to run it against — the
selectors (menu bar item indices, accessibility roles, button titles) follow
documented AppleScript/System Events conventions for status-bar apps and
WKWebView content, but the exact first run on real hardware or macOS CI is
the actual proof. Treat a first red run here as "go tighten the selector,"
not "the feature is broken" — cross-check by hand on a Mac before assuming
the app regressed.
