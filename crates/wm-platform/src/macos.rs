//! macOS backend (Accessibility API). Not yet implemented.
//!
//! Plan, using `objc2` / `core-foundation` / `accessibility-sys`:
//! - Permission: `AXIsProcessTrustedWithOptions` with the prompt option; the user must allow
//!   the binary under System Settings > Privacy & Security > Accessibility.
//! - Discovery: `NSWorkspace.runningApplications` (regular activation policy), then for each
//!   pid `AXUIElementCreateApplication` and read `kAXWindowsAttribute`, keeping windows whose
//!   subrole is `kAXStandardWindowSubrole`. WindowId = `_AXUIElementGetWindow` (CGWindowID).
//! - Events: one `AXObserver` per app for kAXWindowCreated / kAXUIElementDestroyed /
//!   kAXFocusedWindowChanged, plus NSWorkspace launch/terminate notifications, all on the
//!   main CFRunLoop.
//! - Hotkeys: Carbon `RegisterEventHotKey` (no extra permission) or a CGEventTap.
//! - Placement: set `kAXPositionAttribute` then `kAXSizeAttribute` (AXValue CGPoint/CGSize).
//!   Coordinates have a top-left origin on the primary display.
//! - Work area: `NSScreen.mainScreen.visibleFrame`, flipped to top-left origin.
//! - Hide/Show: there is no per-window hide API; move hidden windows off-screen (the approach
//!   AeroSpace uses) rather than relying on macOS Spaces, which have no public API.
//! - Focus: `AXRaise` on the window plus `NSRunningApplication.activate`.

use super::*;

pub struct AxBackend;

impl AxBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Backend for AxBackend {
    fn work_area(&self) -> Rect {
        todo!("NSScreen.visibleFrame")
    }
    fn existing_windows(&self) -> Vec<WindowId> {
        todo!("AX window enumeration")
    }
    fn matches_rule(&self, _: WindowId, _: &[String]) -> bool {
        todo!("bundle id / kAXTitleAttribute")
    }
    fn apply(&mut self, _: &Action) {
        todo!("AXUIElementSetAttributeValue position/size")
    }
    fn run(&mut self, _: &[(Hotkey, Command)], _: &mut Handler<'_>) -> Result<(), String> {
        todo!("AXObserver + RegisterEventHotKey + CFRunLoopRun")
    }
}
