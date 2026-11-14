# tiler — a cross-platform tiling window manager (Windows + macOS)

Status: **early**. The platform-independent core is implemented and tested, the Windows
backend is implemented (single monitor), and the macOS backend is a typed stub with an
implementation plan in its doc comments.

## Trying it on Windows

Download `wm-windows` from the latest CI run (or `cargo build --release`), then run
`wm.exe` from a terminal. It tiles your open windows straight away. `Alt+Shift+Q` quits and
brings back any windows hidden on other workspaces. Config is read from
`%APPDATA%\tiler\config.toml` or a path passed as the first argument.

Known gaps: one monitor only; hidden workspace windows disappear from the taskbar; apps
running as administrator can't be moved unless `wm.exe` is also elevated; after changing
screen resolution press `Alt+R` to retile.

## Architecture

```
            config.toml ──► Config (layout params, float rules, hotkeys → commands)
                                     │
 OS events ─► Backend ──Event──► Manager (workspaces, focus, floating) ──Action──► Backend ─► OS
 (hooks,      (per OS)               │                                             (move/resize,
  hotkeys)                           └─► layout::compute(area, n) → Vec<Rect>        hide, focus)
```

| Crate | Role | OS APIs |
|---|---|---|
| `wm-core` | Geometry, layouts (BSP / master-stack / monocle), workspaces, commands, TOML config, the `Manager` state machine. Pure Rust, no OS calls, fully unit-tested. | none |
| `wm-platform` | `Backend` trait + one backend per OS, selected at compile time. | Windows: `SetWinEventHook`, `EnumWindows`, `SetWindowPos`, `RegisterHotKey`. macOS: `AXUIElement`, `AXObserver`, `RegisterEventHotKey`. |
| `wm` | The binary: loads config, adopts existing windows, runs the backend's event loop. | — |

The key design rule: the `Manager` never calls the OS. It turns `Event`s into `Action`s, so
all tiling logic is testable on any machine, and each backend stays a thin translator.

## Build

```sh
cargo test          # core tests run on any OS
cargo run           # on Linux this uses a headless demo backend
cargo check --target x86_64-pc-windows-gnu    # type-check the Windows build
cargo check --target aarch64-apple-darwin     # type-check the macOS build
```

## Roadmap

1. ~~Windows backend: discovery, event hook, placement, hotkeys (single monitor).~~ Done.
2. macOS backend: Accessibility permission, AX observers, placement, hotkeys.
3. Float rules, minimised-window handling, DPI / invisible-border fixes.
4. Multiple monitors (one workspace set per monitor).
5. IPC: a CLI (`tiler msg focus-next`) over a named pipe / Unix socket, reusing `Command`.
6. Optional status bar.

## Caveats

- macOS needs Accessibility permission, and has no public API for Spaces, so workspaces are
  emulated by moving windows off-screen.
- Some Windows apps (UWP, elevated processes) can't be moved by a non-elevated manager.
