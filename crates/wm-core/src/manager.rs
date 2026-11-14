//! The state machine in the middle: OS events and commands in, window actions out.
//! It never calls the OS itself, which keeps it fully testable.

use crate::{
    command::{self, Command, Toggle},
    config::Config,
    geometry::Rect,
    ipc,
    tree::{Tree, WsTarget},
    WindowId,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    WindowCreated(WindowId),
    WindowDestroyed(WindowId),
    WindowFocused(WindowId),
    Commands(Vec<Command>),
    /// The backend was woken from another thread (e.g. an IPC request is waiting).
    Wake,
}

/// What the backend must do after an event, applied in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Place(WindowId, Rect),
    Hide(WindowId),
    Show(WindowId),
    Focus(WindowId),
    /// Ask the window to close politely (i3's `kill`).
    Close(WindowId),
    Quit,
}

pub struct Manager {
    pub tree: Tree,
    gap: i32,
    placed: HashMap<WindowId, Rect>,
    visible: HashSet<WindowId>,
    last_focus: Option<WindowId>,
}

impl Manager {
    pub fn new(config: &Config, work_area: Rect) -> Self {
        Self {
            tree: Tree::new(&[("primary".into(), work_area)], config.tree),
            gap: config.gap,
            placed: HashMap::new(),
            visible: HashSet::new(),
            last_focus: None,
        }
    }

    pub fn handle(&mut self, ev: Event) -> Vec<Action> {
        let mut out = vec![];
        let mut external_focus = false;
        match ev {
            Event::WindowCreated(w) => {
                self.tree.manage(w);
            }
            Event::WindowDestroyed(w) => {
                self.tree.unmanage(w);
                self.placed.remove(&w);
                self.visible.remove(&w);
                if self.last_focus == Some(w) {
                    self.last_focus = None;
                }
            }
            Event::WindowFocused(w) => {
                if self.tree.find_window(w).is_none() {
                    return out;
                }
                self.tree.focus_window(w);
                external_focus = true;
            }
            Event::Commands(cmds) => {
                for c in cmds {
                    self.run(c, &mut out);
                }
            }
            Event::Wake => return out,
        }
        self.sync(external_focus, &mut out);
        out
    }

    /// Parse and run an i3 command line (as IPC `RUN_COMMAND` does).
    pub fn run_command(&mut self, line: &str) -> Result<Vec<Action>, String> {
        let cmds = command::parse(line)?;
        Ok(self.handle(Event::Commands(cmds)))
    }

    /// Handle one IPC request; returns the JSON reply and any actions to apply.
    pub fn ipc(&mut self, ty: u32, payload: &[u8]) -> (String, Vec<Action>) {
        match ty {
            ipc::RUN_COMMAND => {
                let line = String::from_utf8_lossy(payload);
                match self.run_command(&line) {
                    Ok(actions) => (r#"[{"success":true}]"#.into(), actions),
                    Err(e) => (
                        serde_json::json!([{"success": false, "parse_error": true, "error": e}])
                            .to_string(),
                        vec![],
                    ),
                }
            }
            _ => (ipc::query(&self.tree, ty), vec![]),
        }
    }

    fn run(&mut self, c: Command, out: &mut Vec<Action>) {
        let t = &mut self.tree;
        match c {
            Command::Focus(d) => t.focus_direction(d),
            Command::FocusParent => t.focus_parent(),
            Command::FocusChild => t.focus_child(),
            Command::Move(d) => t.move_direction(d),
            Command::MoveToWorkspace(w) => t.move_to_workspace(&w),
            Command::Split(l) => t.split(l),
            Command::SplitToggle => {
                let p = t.node(t.focused).parent.unwrap_or(t.focused);
                let l = match t.node(p).layout.orientation() {
                    crate::tree::Orientation::Horizontal => crate::tree::Layout::Splitv,
                    crate::tree::Orientation::Vertical => crate::tree::Layout::Splith,
                };
                t.split(l);
            }
            Command::Layout(l) => t.set_layout(l),
            Command::LayoutToggleSplit => t.toggle_split(),
            Command::LayoutToggleAll => t.toggle_all_layouts(),
            Command::LayoutDefault => t.layout_default(),
            Command::Fullscreen(tg) => {
                let on = t.node(t.focused).fullscreen;
                if tg == Toggle::Toggle || (tg == Toggle::Enable) != on {
                    t.toggle_fullscreen();
                }
            }
            Command::Floating(tg) => {
                let on = t.is_floating(t.focused);
                if tg == Toggle::Toggle || (tg == Toggle::Enable) != on {
                    t.toggle_floating();
                }
            }
            Command::Workspace(w) => t.switch_workspace(&w),
            Command::Resize { orientation, delta } => t.resize(orientation, delta),
            Command::Kill => out.extend(t.windows_in(t.focused).into_iter().map(Action::Close)),
            Command::Exit => out.push(Action::Quit),
            Command::Nop => {}
        }
    }

    /// Render the tree and emit only what changed since last time.
    fn sync(&mut self, external_focus: bool, out: &mut Vec<Action>) {
        let placements = self.tree.render(self.gap);
        let now: HashSet<WindowId> = placements.iter().map(|(w, _)| *w).collect();
        for (w, r) in &placements {
            let newly_shown = !self.visible.contains(w);
            if newly_shown {
                out.push(Action::Show(*w));
            }
            if newly_shown || self.placed.get(w) != Some(r) {
                out.push(Action::Place(*w, *r));
                self.placed.insert(*w, *r);
            }
        }
        let mut hidden: Vec<WindowId> = self.visible.difference(&now).copied().collect();
        hidden.sort();
        for w in hidden {
            if self.tree.find_window(w).is_some() {
                out.push(Action::Hide(w));
            }
        }
        self.visible = now;
        let focus = self.tree.focused_window();
        if focus != self.last_focus {
            if let (Some(w), false) = (focus, external_focus) {
                out.push(Action::Focus(w));
            }
            self.last_focus = focus;
        }
    }

    /// Switch workspace by name; convenience for tests and IPC clients.
    pub fn workspace(&mut self, name: &str) -> Vec<Action> {
        self.handle(Event::Commands(vec![Command::Workspace(WsTarget::Name(
            name.into(),
        ))]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> Manager {
        let c = Config {
            gap: 0,
            ..Default::default()
        };
        Manager::new(&c, Rect::new(0, 0, 1000, 1000))
    }

    fn placed(a: &[Action]) -> Vec<WindowId> {
        a.iter()
            .filter_map(|x| {
                if let Action::Place(w, _) = x {
                    Some(*w)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn new_windows_are_tiled_and_focused() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        let a = m.handle(Event::WindowCreated(WindowId(2)));
        assert_eq!(placed(&a), vec![WindowId(1), WindowId(2)]);
        assert!(a.contains(&Action::Focus(WindowId(2))));
    }

    #[test]
    fn only_changes_are_emitted() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        m.handle(Event::WindowCreated(WindowId(2)));
        // Focusing does not move anything.
        let a = m.run_command("focus left").unwrap();
        assert_eq!(a, vec![Action::Focus(WindowId(1))]);
    }

    #[test]
    fn workspace_switch_hides_and_shows() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        let a = m.workspace("2");
        assert_eq!(a, vec![Action::Hide(WindowId(1))]);
        let a = m.workspace("1");
        assert!(a.contains(&Action::Show(WindowId(1))));
        assert!(a.contains(&Action::Focus(WindowId(1))));
    }

    #[test]
    fn tabbed_hides_background_tabs() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        m.handle(Event::WindowCreated(WindowId(2)));
        let a = m.run_command("layout tabbed").unwrap();
        assert!(a.contains(&Action::Hide(WindowId(1))));
        let a = m.run_command("focus left").unwrap();
        assert!(a.contains(&Action::Show(WindowId(1))));
        assert!(a.contains(&Action::Hide(WindowId(2))));
    }

    #[test]
    fn external_focus_is_not_echoed() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        m.handle(Event::WindowCreated(WindowId(2)));
        assert!(m.handle(Event::WindowFocused(WindowId(1))).is_empty());
    }

    #[test]
    fn kill_closes_focused() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        assert_eq!(
            m.run_command("kill").unwrap(),
            vec![Action::Close(WindowId(1))]
        );
    }

    #[test]
    fn ipc_run_command_reports_errors() {
        let mut m = mgr();
        let (r, _) = m.ipc(ipc::RUN_COMMAND, b"split v");
        assert_eq!(r, r#"[{"success":true}]"#);
        let (r, _) = m.ipc(ipc::RUN_COMMAND, b"dance");
        assert!(r.contains("\"success\":false"));
    }
}
