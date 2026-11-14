//! Fallback for unsupported OSes (and CI): no real windows, just prints actions.
//! Lets the binary and the core be exercised on Linux.

use super::*;

#[derive(Default)]
pub struct Headless;

impl Backend for Headless {
    fn work_area(&self) -> Rect {
        Rect::new(0, 0, 1920, 1080)
    }
    fn existing_windows(&self) -> Vec<WindowId> {
        vec![WindowId(1), WindowId(2), WindowId(3)]
    }
    fn matches_rule(&self, _: WindowId, _: &[String]) -> bool {
        false
    }
    fn apply(&mut self, action: &Action) {
        println!("{action:?}");
    }
    fn run(&mut self, _: &[(Hotkey, Command)], handler: &mut Handler<'_>) -> Result<(), String> {
        println!("headless backend: no window system on this OS, running a short demo");
        for c in [Command::CycleLayout, Command::FocusNext, Command::Promote, Command::Quit] {
            println!("> {c:?}");
            for a in handler(Event::Command(c)) {
                self.apply(&a);
            }
        }
        Ok(())
    }
}
