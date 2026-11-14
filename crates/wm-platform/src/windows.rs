//! Windows backend (Win32). Not yet implemented.
//!
//! Plan, using the `windows` crate:
//! - Discovery: `EnumWindows`, keeping windows that are visible, not cloaked
//!   (`DwmGetWindowAttribute(DWMWA_CLOAKED)`), have no owner, and lack `WS_EX_TOOLWINDOW`.
//! - Events: `SetWinEventHook` for EVENT_OBJECT_SHOW / DESTROY / EVENT_SYSTEM_FOREGROUND /
//!   EVENT_SYSTEM_MINIMIZESTART|END, delivered on the thread running `GetMessageW`.
//! - Hotkeys: `RegisterHotKey` per binding; WM_HOTKEY carries the binding index as id.
//! - Placement: `SetWindowPos` (or a batched `DeferWindowPos`), compensating for invisible
//!   borders via `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`. Call
//!   `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` first so coordinates are physical.
//! - Work area: `SystemParametersInfoW(SPI_GETWORKAREA)` / `GetMonitorInfoW`.
//! - Hide/Show: `ShowWindow(SW_HIDE / SW_SHOWNA)` for workspace switches.
//! - Focus: `SetForegroundWindow` (needs the Alt-key-press trick to bypass focus-steal rules).

use super::*;

pub struct Win32Backend;

impl Win32Backend {
    pub fn new() -> Self {
        Self
    }
}

impl Backend for Win32Backend {
    fn work_area(&self) -> Rect {
        todo!("SystemParametersInfoW(SPI_GETWORKAREA)")
    }
    fn existing_windows(&self) -> Vec<WindowId> {
        todo!("EnumWindows + filtering")
    }
    fn matches_rule(&self, _: WindowId, _: &[String]) -> bool {
        todo!("GetClassNameW / GetWindowTextW")
    }
    fn apply(&mut self, _: &Action) {
        todo!("SetWindowPos / ShowWindow / SetForegroundWindow")
    }
    fn run(&mut self, _: &[(Hotkey, Command)], _: &mut Handler<'_>) -> Result<(), String> {
        todo!("RegisterHotKey + SetWinEventHook + GetMessageW loop")
    }
}
