//! Fallback for OSes without a backend (Linux, CI). There is no window system: it pretends
//! three windows exist and just prints actions, but it runs a real event loop, so the IPC
//! server and `wm msg` can be exercised end to end.

use super::*;
use std::sync::{mpsc, Arc, Mutex};

pub struct Headless {
    tx: mpsc::Sender<()>,
    rx: mpsc::Receiver<()>,
}

impl Default for Headless {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }
}

impl Backend for Headless {
    fn work_area(&self) -> Rect {
        Rect::new(0, 0, 1920, 1080)
    }
    fn existing_windows(&self) -> Vec<WindowId> {
        vec![WindowId(1), WindowId(2), WindowId(3)]
    }
    fn set_float_rules(&mut self, _: &[String]) {}
    fn apply(&mut self, action: &Action) {
        println!("{action:?}");
    }
    fn waker(&self) -> Waker {
        let tx = Mutex::new(self.tx.clone());
        Arc::new(move || {
            let _ = tx.lock().map(|t| t.send(()));
        })
    }
    fn run(
        &mut self,
        _: &[(Hotkey, Vec<Command>)],
        handler: &mut Handler<'_>,
    ) -> Result<(), String> {
        println!("headless backend: no window system on this OS; drive it with `wm msg`");
        while self.rx.recv().is_ok() {
            for a in handler(Event::Wake) {
                self.apply(&a);
                if a == Action::Quit {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}
