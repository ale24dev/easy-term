<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="easy-term icon" />
</p>

<h1 align="center">easy-term</h1>

<p align="center">
  A macOS menu bar app for running your local dev servers — no more N terminal tabs full of <code>pnpm run dev</code>.
</p>

<p align="center">
  <a href="https://github.com/ale24dev/easy-term/actions/workflows/ci.yml"><img src="https://github.com/ale24dev/easy-term/workflows/CI/badge.svg" alt="CI status" /></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey" alt="macOS only" />
  <img src="https://img.shields.io/badge/built%20with-Tauri%20v2-24C8DB" alt="Built with Tauri v2" />
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT License" /></a>
</p>

---

Instead of juggling a terminal window per project, you define each project once — folder, start command, port — and start, stop, and watch its logs from a popover anchored to your menu bar icon. Real PTY under the hood, so dev servers behave exactly like they do in a real terminal: ANSI colors, spinners, the works.

## Screenshots

<!--
  TODO(ale24dev): this repo doesn't have real screenshots yet — I (Claude) can't
  produce them myself, there's no macOS environment available in this sandbox to
  run and capture the actual app. Drop PNGs at the paths below (same filenames)
  and this section will render as-is; no other edits needed.

  Suggested shots, ~800px wide, either theme:
    docs/screenshots/project-list.png   — main popover: a couple of groups expanded,
                                           some projects running (green dot) and one
                                           stopped, port/error badges visible
    docs/screenshots/logs.png           — LogView open on a running project: colored
                                           terminal output, search bar, "open in
                                           browser" button
    docs/screenshots/project-form.png   — the new/edit project form with a detected
                                           package.json script picker
    docs/screenshots/port-conflict.png  — the "port already in use" dialog showing
                                           the owning process + free-and-start
    docs/screenshots/settings.png       — Settings → Diagnostics panel
-->

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/project-list.png" alt="Project list popover with running and stopped projects" /><br/><sub align="center">Project list — grouped, with live status</sub></td>
    <td width="50%"><img src="docs/screenshots/logs.png" alt="Live terminal output for a running project" /><br/><sub align="center">Live logs via a real PTY (xterm.js)</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/screenshots/project-form.png" alt="Project form with auto-detected scripts" /><br/><sub align="center">Add a project — scripts auto-detected from package.json</sub></td>
    <td width="50%"><img src="docs/screenshots/port-conflict.png" alt="Port conflict dialog" /><br/><sub align="center">Port already taken? Free it in one click</sub></td>
  </tr>
</table>

## Features

- **Real PTY, not a piped subprocess.** Dev servers see a real TTY, so you get actual ANSI colors and live spinners instead of buffered, mangled output.
- **One popover, many projects.** Grouped into workspaces you can start/stop together, in dependency order, with readiness checks between them.
- **Auto-detected scripts.** Point it at a folder and it reads `package.json` + your lockfile to pre-fill the command and package manager (npm/pnpm/yarn/bun).
- **Port conflicts resolved in a click.** If the port's taken, it shows you who owns it and offers to free it before starting.
- **Auto-restart with backoff.** A project that crashes retries with exponential backoff (1s → 2s → 4s → 8s → 16s, capped, 5 attempts), cancelable anytime.
- **Search, error highlighting, native crash notifications.** Cmd/Ctrl+F in any log, a live error/warning counter per project, and a system notification that jumps straight to the right log on click.
- **Reachable over full-screen apps.** Built on a non-activating `NSPanel` (the same mechanism Spotlight uses), so the popover shows up even while another app owns a full-screen Space — a regular Tauri window can't do this.
- **Quick actions.** Open a project in your editor, reveal it in Finder, copy its detected URL, open it in the browser.
- **Your dev servers survive the app.** Quit easy-term and they keep running; reopen it and you're reconnected to the same processes, with the logs from while you were away. See [why there's a daemon](#why-theres-a-daemon).
- **Launch at login.**
- **Light/dark/system theme**, and an internal diagnostics log (Settings → Diagnostics) for troubleshooting the app itself.

## Install

```bash
brew tap ale24dev/easy-term
brew install --cask easy-term
```

Each release is a Developer ID–signed, notarized `.dmg` — Gatekeeper accepts it without any "unidentified developer" warning. See [Publishing a release](#publishing-a-release) below for how that gets built.

## Install / run from source

Requires macOS, [pnpm](https://pnpm.io), and the [Rust toolchain](https://rustup.rs).

```bash
pnpm install
pnpm tauri dev      # run in development
pnpm tauri build    # produce a release .app
```

## Testing

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # Rust: unit + integration tests
pnpm test                                          # frontend: Vitest
./e2e/macos/run.sh                                 # macOS UI E2E (see e2e/macos/README.md)
```

CI runs all of the above (plus `tsc --noEmit` and `cargo fmt --check`) on every push and pull request — see [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## Publishing a release

Distribution is a Developer ID–signed, notarized `.dmg` via a [personal Homebrew tap](https://github.com/ale24dev/homebrew-easy-term) — the official `homebrew/homebrew-cask` tap has notability requirements a new app doesn't meet yet, but a personal tap needs no one's approval.

### Automated (recommended)

[`.github/workflows/release-homebrew.yml`](.github/workflows/release-homebrew.yml) runs the whole cycle on a GitHub-hosted macOS runner: builds the universal binary, signs it with a Developer ID certificate, notarizes it with Apple, verifies the result, creates the GitHub Release with the `.dmg`, and updates the Cask in the tap. The release flow is two steps:

```bash
# 1. Bump the version in src-tauri/tauri.conf.json, commit, push to master
git add src-tauri/tauri.conf.json
git commit -m "chore: version 0.2.0"
git push

# 2. Tag it — this triggers the whole pipeline
git tag v0.2.0
git push origin v0.2.0
```

One-time setup — 5 secrets in **Settings → Secrets and variables → Actions → Repository secrets**:

| Secret | How to get it |
| --- | --- |
| `APPLE_CERTIFICATE` | In Keychain Access, right-click your **"Developer ID Application"** certificate → Export → `.p12` format, with a password. Then `base64 -i Certificate.p12 \| pbcopy`. |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set when exporting that `.p12`. |
| `APPLE_ID` | Your Apple ID email. |
| `APPLE_PASSWORD` | A new, dedicated app-specific password for this workflow (don't reuse a local one) — [appleid.apple.com/account/manage](https://appleid.apple.com/account/manage) → Sign-In and Security → App-Specific Passwords. |
| `HOMEBREW_TAP_TOKEN` | A GitHub [fine-grained PAT](https://github.com/settings/personal-access-tokens/new) scoped to only `ale24dev/homebrew-easy-term`, with `Contents: Read and write`. The workflow's default token can't push to a different repo. |

### Manual

```bash
export APPLE_ID="you@icloud.com"
export APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx"
./scripts/build-homebrew.sh
```

Builds, signs, notarizes, staples the ticket, and prints the `.dmg`'s sha256. From there: create a GitHub Release (tag `v<version>`) with the `.dmg` as an asset, fill in [`homebrew/easy-term.rb.in`](homebrew/easy-term.rb.in) with that version and sha256, and publish it as `Casks/easy-term.rb` in the tap.

## How it works

```
┌─────────────────────────── Frontend (WebView) ───────────────────────────┐
│  React + Zustand                                                         │
│  ├─ ProjectList   (status, quick actions)                                │
│  ├─ ProjectForm   (create/edit, script auto-detection)                   │
│  └─ LogView       (xterm.js, one persistent instance per project)        │
└───────────────┬──────────────────────────────▲───────────────────────────┘
                │  invoke (commands)            │  emit (events)
┌───────────────▼──────────────────────────────┴───────────────────────────┐
│  Backend (Rust / Tauri)                                                  │
│  ├─ process_manager  spawn/kill/restart over a real PTY (portable-pty)   │
│  ├─ project_store    CRUD + atomic JSON persistence                      │
│  ├─ port_checker     is a port free? who owns it if not?                 │
│  ├─ script_detector  reads package.json + lockfiles                     │
│  ├─ env_resolver     real PATH via the user's login shell                │
│  └─ tray / macos_window   menu bar icon, NSPanel popover                 │
└──────────────────────────────────────────────────────────────────────────┘
```

Projects are persisted as JSON at `~/Library/Application Support/easy-term/projects.json`. Killing a project kills its whole process group (`setsid` + `killpg`), so tools like Vite/Next that spawn child processes don't leave orphans behind. Internal app errors (not your projects' output) are logged separately to `~/Library/Logs/easy-term/` for troubleshooting — see Settings → Diagnostics.

### Why there's a daemon

Your dev servers keep running when you quit the app, and reopening it reconnects to them — same processes, same scrollback.

That needs a second process, and not for the reason you'd guess. Project processes run inside a PTY, and closing the **master** end of a PTY makes the kernel hang up the terminal and SIGHUP everything in its foreground process group. So whoever holds that end can never quit without taking the processes down with it — ignoring SIGHUP doesn't help either, since writes to a hung-up terminal then fail with `EIO`. Keeping the processes alive isn't a matter of not killing them; it requires somebody to keep holding that file descriptor.

That somebody is `easy-term --daemon`: the same binary, running headless, owning every PTY. The GUI is a client that connects to it over a Unix socket at `~/Library/Application Support/easy-term/daemon.sock`, sends commands, and streams events back — the model tmux uses, for the same reason. It's spawned automatically on first launch if it isn't already running. Because the daemon also holds the output ring buffers, reconnecting restores the logs from before you closed the app, not just the process list.

A reboot still clears everything: the daemon isn't a launch agent, so nothing survives it. Quitting the app leaves both the daemon and your projects running.

For the full design history — every decision, the bugs found along the way, and why — see [`PLAN.md`](PLAN.md).

## Roadmap

Not yet built, in rough priority order:

- Interactive terminal input (respond to a dev server's own prompts)
- Per-repo config file (`easyterm.json`) for team-shared project setups
- Git branch shown next to the project name
- Signed, notarized, auto-updating releases (currently source-only)

## License

[MIT](LICENSE)
