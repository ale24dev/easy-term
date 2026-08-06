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
- **Launch at login**, with running projects restored automatically.
- **Global shortcut** (`Option+Space`) to toggle the popover from anywhere.
- **Light/dark/system theme**, and an internal diagnostics log (Settings → Diagnostics) for troubleshooting the app itself.

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

Projects are persisted as JSON at `~/Library/Application Support/easy-term/projects.json`. Runtime state (PIDs, live status, output buffers) lives only in memory — nothing about a running process survives an app restart except the "was it running" flag used to restore it. Killing a project kills its whole process group (`setsid` + `killpg`), so tools like Vite/Next that spawn child processes don't leave orphans behind. Internal app errors (not your projects' output) are logged separately to `~/Library/Logs/easy-term/` for troubleshooting — see Settings → Diagnostics.

For the full design history — every decision, the bugs found along the way, and why — see [`PLAN.md`](PLAN.md).

## Roadmap

Not yet built, in rough priority order:

- Interactive terminal input (respond to a dev server's own prompts)
- Per-repo config file (`easyterm.json`) for team-shared project setups
- Git branch shown next to the project name
- Signed, notarized, auto-updating releases (currently source-only)

## License

[MIT](LICENSE)
