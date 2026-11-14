//! The state machine in the middle: OS events and commands in, window placements out.
//! It never calls the OS itself, which keeps it fully testable.

use crate::{
    command::Command, config::Config, geometry::Rect, layout::LayoutKind, workspace::Workspace,
    WindowId,
};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    WindowCreated(WindowId),
    WindowDestroyed(WindowId),
    WindowFocused(WindowId),
    Command(Command),
}

/// What the backend must do after an event, applied in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Place(WindowId, Rect),
    Hide(WindowId),
    Show(WindowId),
    Focus(WindowId),
    Quit,
}

pub struct Manager {
    workspaces: Vec<Workspace>,
    active: usize,
    floating: HashSet<WindowId>,
    work_area: Rect,
}

impl Manager {
    pub fn new(config: &Config, work_area: Rect) -> Self {
        Self {
            workspaces: (0..config.workspaces).map(|_| Workspace::new(config.layout)).collect(),
            active: 0,
            floating: HashSet::new(),
            work_area,
        }
    }

    pub fn active(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    fn ws(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    fn owner(&self, id: WindowId) -> Option<usize> {
        self.workspaces.iter().position(|w| w.contains(id))
    }

    fn retile(&self, out: &mut Vec<Action>) {
        out.extend(self.active().arrange(self.work_area).into_iter().map(|(w, r)| Action::Place(w, r)));
    }

    fn focus_action(&self, out: &mut Vec<Action>) {
        if let Some(f) = self.active().focused() {
            out.push(Action::Focus(f));
        }
    }

    pub fn set_work_area(&mut self, area: Rect) -> Vec<Action> {
        self.work_area = area;
        let mut out = vec![];
        self.retile(&mut out);
        out
    }

    pub fn handle(&mut self, ev: Event) -> Vec<Action> {
        let mut out = vec![];
        match ev {
            Event::WindowCreated(id) => {
                if self.owner(id).is_none() && !self.floating.contains(&id) {
                    self.ws().insert(id);
                    self.retile(&mut out);
                }
            }
            Event::WindowDestroyed(id) => {
                self.floating.remove(&id);
                if let Some(i) = self.owner(id) {
                    self.workspaces[i].remove(id);
                    if i == self.active {
                        self.retile(&mut out);
                    }
                }
            }
            Event::WindowFocused(id) => self.ws().set_focus(id),
            Event::Command(c) => self.command(c, &mut out),
        }
        out
    }

    fn command(&mut self, c: Command, out: &mut Vec<Action>) {
        use Command::*;
        match c {
            FocusNext | FocusPrev => {
                self.ws().focus_step(if c == FocusNext { 1 } else { -1 });
                self.focus_action(out);
                if self.active().layout.kind == LayoutKind::Monocle {
                    self.retile(out);
                }
            }
            SwapNext | SwapPrev => {
                self.ws().swap_step(if c == SwapNext { 1 } else { -1 });
                self.retile(out);
            }
            Promote => {
                self.ws().promote();
                self.retile(out);
            }
            GrowMaster | ShrinkMaster => {
                self.ws().layout.adjust_ratio(if c == GrowMaster { 0.05 } else { -0.05 });
                self.retile(out);
            }
            IncMasterCount => {
                self.ws().layout.master_count += 1;
                self.retile(out);
            }
            DecMasterCount => {
                let m = &mut self.ws().layout.master_count;
                *m = (*m).saturating_sub(1).max(1);
                self.retile(out);
            }
            SetLayout(k) => {
                self.ws().layout.kind = k;
                self.retile(out);
            }
            CycleLayout => {
                let l = &mut self.ws().layout.kind;
                *l = match l {
                    LayoutKind::Bsp => LayoutKind::MasterStack,
                    LayoutKind::MasterStack => LayoutKind::Monocle,
                    LayoutKind::Monocle => LayoutKind::Bsp,
                };
                self.retile(out);
            }
            ToggleFloat => {
                if let Some(f) = self.active().focused() {
                    self.ws().remove(f);
                    self.floating.insert(f);
                } else {
                    return;
                }
                self.retile(out);
            }
            Workspace(n) if n < self.workspaces.len() && n != self.active => {
                out.extend(self.active().windows().iter().map(|w| Action::Hide(*w)));
                self.active = n;
                out.extend(self.active().windows().iter().map(|w| Action::Show(*w)));
                self.retile(out);
                self.focus_action(out);
            }
            MoveToWorkspace(n) if n < self.workspaces.len() && n != self.active => {
                if let Some(f) = self.active().focused() {
                    self.ws().remove(f);
                    self.workspaces[n].insert(f);
                    out.push(Action::Hide(f));
                    self.retile(out);
                    self.focus_action(out);
                }
            }
            Workspace(_) | MoveToWorkspace(_) => {}
            Retile => self.retile(out),
            Quit => out.push(Action::Quit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> Manager {
        Manager::new(&Config::default(), Rect::new(0, 0, 1000, 1000))
    }

    fn placed(a: &[Action]) -> Vec<WindowId> {
        a.iter().filter_map(|x| if let Action::Place(w, _) = x { Some(*w) } else { None }).collect()
    }

    #[test]
    fn new_windows_are_tiled() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        let a = m.handle(Event::WindowCreated(WindowId(2)));
        assert_eq!(placed(&a), vec![WindowId(1), WindowId(2)]);
    }

    #[test]
    fn workspace_switch_hides_and_shows() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        m.handle(Event::Command(Command::MoveToWorkspace(1)));
        assert!(m.active().windows().is_empty());
        let a = m.handle(Event::Command(Command::Workspace(1)));
        assert!(a.contains(&Action::Show(WindowId(1))));
        assert!(a.contains(&Action::Focus(WindowId(1))));
    }

    #[test]
    fn floating_window_is_not_retiled() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        m.handle(Event::WindowCreated(WindowId(2)));
        let a = m.handle(Event::Command(Command::ToggleFloat));
        assert_eq!(placed(&a), vec![WindowId(1)]);
        // Re-announcing a floating window must not tile it again.
        assert!(m.handle(Event::WindowCreated(WindowId(2))).is_empty());
    }

    #[test]
    fn destroy_on_other_workspace_does_not_retile_active() {
        let mut m = mgr();
        m.handle(Event::WindowCreated(WindowId(1)));
        m.handle(Event::Command(Command::MoveToWorkspace(2)));
        m.handle(Event::WindowCreated(WindowId(2)));
        assert!(m.handle(Event::WindowDestroyed(WindowId(1))).is_empty());
    }
}
