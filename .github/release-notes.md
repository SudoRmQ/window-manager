**Early alpha.** These builds compile and pass CI but have not yet been tested on a real
Windows or macOS desktop. Expect rough edges, and please report what you find.

## Downloads

| Platform | File |
|---|---|
| Windows 10/11 (Intel/AMD) | `tiler-<version>-windows-x64.zip` |
| Windows 11 on ARM | `tiler-<version>-windows-arm64.zip` |
| macOS, Apple Silicon (M1 and later) | `tiler-<version>-macos-apple-silicon.tar.gz` |
| macOS, Intel | `tiler-<version>-macos-intel.tar.gz` |

`SHA256SUMS.txt` lists checksums.

## Running

**Windows:** unzip and run `wm.exe` from a terminal. It tiles open windows right away.
`Alt+Shift+E` exits and brings back windows hidden on other workspaces. SmartScreen may warn
because the binary is unsigned ("More info" → "Run anyway").

**macOS:** extract, then in Terminal:

```sh
xattr -d com.apple.quarantine wm   # the binary is not signed or notarized
./wm
```

macOS will ask for Accessibility permission (System Settings → Privacy & Security →
Accessibility). Grant it to your terminal app, then run `./wm` again.

**Control it** with the default Alt-based keys in `config.example.toml`
(`Alt+H/J/K/L` focus, `Alt+Shift+H/J/K/L` move, `Alt+V`/`Alt+B` split, `Alt+W` tabbed,
`Alt+1..5` workspaces), or from another terminal with `wm msg focus left`,
`wm msg -t get_tree`, etc.
