//! OS backends. Each one turns native window events into `Event`s and carries out
//! the `Action`s the core returns. Pick the backend for the current OS with `native()`.

use wm_core::{
    command::Command,
    config::Hotkey,
    geometry::Rect,
    manager::{Action, Event},
    WindowId,
};

mod headless;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub type Handler<'a> = dyn FnMut(Event) -> Vec<Action> + 'a;

pub trait Backend {
    /// Usable area of the primary monitor, excluding taskbar / menu bar / dock.
    fn work_area(&self) -> Rect;

    /// Normal, visible, top-level windows that should be managed at startup.
    fn existing_windows(&self) -> Vec<WindowId>;

    /// Windows matching these rules (class name / bundle id, or a substring of the title)
    /// are never reported as created, so they stay floating. Call before anything else.
    fn set_float_rules(&mut self, rules: &[String]);

    /// Carry out one action from the core.
    fn apply(&mut self, action: &Action);

    /// Register global hotkeys, then block in the OS event loop, feeding every event
    /// to `handler` and applying what it returns. Returns on `Action::Quit`.
    fn run(
        &mut self,
        bindings: &[(Hotkey, Command)],
        handler: &mut Handler<'_>,
    ) -> Result<(), String>;
}

pub fn native() -> Box<dyn Backend> {
    #[cfg(target_os = "windows")]
    return Box::new(windows::Win32Backend::new());
    #[cfg(target_os = "macos")]
    return Box::new(macos::AxBackend::new());
    #[allow(unreachable_code)]
    Box::new(headless::Headless)
}
