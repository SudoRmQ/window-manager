# tiler: an i3-style tiling window manager for Windows and macOS

tiler works like [i3](https://i3wm.org) inside: windows live in a container tree, you split
containers horizontally or vertically, switch containers between split, stacked and tabbed
layouts, move and focus by direction, and drive everything with i3's command language,
from hotkeys or over i3's IPC protocol (`wm msg`, the `i3-msg` equivalent).

Status: **early**. The core is implemented and heavily tested. The Windows and macOS
backends compile and pass CI but still need hands-on testing on real desktops.

## Architecture

```
 config.toml ─► Config (gaps, options, hotkey → i3 commands)
                                  │
 OS events ─► Backend ──Event──► Manager ──Action──► Backend ─► OS
 (hooks,      (per OS)            │  ▲                         (move/resize, hide/show,
  hotkeys)                        ▼  │                          focus, close)
                                 Tree  (i3's container tree + commands + render)
                                  ▲
 wm msg ──► IPC server (i3 protocol over Unix socket / named pipe)
```

| Crate | Role |
|---|---|
| `wm-core` | Pure Rust, no OS calls, fully unit-tested. `tree` (containers, layouts, focus stacks, i3's split/move/focus/flatten/workspace algorithms, rendering), `command` (i3 command parser), `manager` (events → minimal set of actions), `ipc` (i3 wire protocol and JSON replies), `config`. |
| `wm-platform` | `Backend` trait + one backend per OS. Windows: `SetWinEventHook`, `EnumWindows`, `SetWindowPos`, `RegisterHotKey`. macOS: `AXUIElement`, `AXObserver`, `CGEventTap`. A headless backend is used on other OSes (and CI). |
| `wm` | The binary: loads config, adopts existing windows, runs the IPC server and the backend's event loop; also the `wm msg` client. |

### How it maps to i3

The tree is i3's: **root → outputs → workspaces → containers → windows**. Each container
has a layout (`splith`, `splitv`, `stacked`, `tabbed`), children with a `percent` share, and a
focus stack (most recently focused child first). The behaviour follows i3's sources:

| i3 | tiler |
|---|---|
| `con_attach` / `manage_window`: new window goes after the focused one | `Tree::manage` |
| `tree_split`: wrap focused con in a new split con (or re-orient a lone parent) | `Tree::split` |
| `tree_next`: focus by direction through the focus stacks, with wrapping | `Tree::focus_direction` |
| `tree_move`: swap, enter neighbouring containers, move up, `ws_force_orientation` | `Tree::move_direction` |
| `tree_flatten`: drop empty / redundant split cons | `Tree::flatten` |
| `con_set_layout`, `layout toggle split/all` | `Tree::set_layout`, `toggle_split`, ... |
| workspaces created on demand, deleted when empty and hidden | `Tree::switch_workspace` |
| `render_con` | `Tree::render` |

Not yet: title bars for stacked/tabbed containers (only the focused tab is shown), marks,
scratchpad, modes, criteria (`[class="..."]`), IPC event subscriptions, multiple monitors.

## Commands

Bindings in `config.example.toml` use i3's commands (alt is `$mod`):

```
focus left|right|up|down|parent|child         move left|right|up|down
split h|v|toggle                               layout stacking|tabbed|splith|splitv|toggle split|toggle all
fullscreen toggle                              floating toggle
workspace number 3 | next | prev | back_and_forth
move container to workspace number 3
resize grow|shrink width|height 10 px or 5 ppt
kill                                           exit
```

Chain with `;`: `split v; layout tabbed`.

## IPC

```sh
wm msg focus left
wm msg 'split v; layout stacking'
wm msg -t get_tree          # also get_workspaces, get_outputs, get_version
```

The wire format is i3's (`"i3-ipc"` + length + type + JSON), so i3 IPC libraries can talk to
it. The endpoint is `$XDG_RUNTIME_DIR` / `$TMPDIR` `/tiler-$USER.sock` on macOS and
`\\.\pipe\tiler-%USERNAME%` on Windows (override with `TILER_SOCKET`).

## Build and run

```sh
cargo test                                     # core tests, including a randomized tree fuzz
cargo run                                      # Windows/macOS: manage windows; Linux: headless
cargo check --target x86_64-pc-windows-gnu
cargo check --target aarch64-apple-darwin
```

CI builds Windows, macOS and Linux and uploads `wm.exe`. Config is read from
`%APPDATA%\tiler\config.toml` (Windows), `~/.config/tiler/config.toml` (macOS) or a path
given as the first argument.

## Platform notes

- **Windows:** hidden workspaces use `ShowWindow(SW_HIDE)`, so those windows leave the taskbar
  until shown; they are restored when tiler exits. Elevated windows can't be moved unless tiler
  is elevated too.
- **macOS:** needs Accessibility permission. There is no public API for Spaces or hiding single
  windows, so hidden windows are parked in a screen corner (as AeroSpace does).
