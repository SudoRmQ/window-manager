# tiler — a cross-platform tiling window manager (Windows + macOS)

Status: **scaffold**. The platform-independent core is implemented and tested; the
Windows and macOS backends are typed stubs with an implementation plan in their doc comments.

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

1. Windows backend: discovery, event hook, placement, hotkeys (single monitor).
2. macOS backend: Accessibility permission, AX observers, placement, hotkeys.
3. Float rules, minimised-window handling, DPI / invisible-border fixes.
4. Multiple monitors (one workspace set per monitor).
5. IPC: a CLI (`tiler msg focus-next`) over a named pipe / Unix socket, reusing `Command`.
6. Optional status bar.

## Caveats

- macOS needs Accessibility permission, and has no public API for Spaces, so workspaces are
  emulated by moving windows off-screen.
- Some Windows apps (UWP, elevated processes) can't be moved by a non-elevated manager.
