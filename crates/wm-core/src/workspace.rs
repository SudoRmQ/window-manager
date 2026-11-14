use crate::{
    geometry::Rect,
    layout::{self, LayoutParams},
    WindowId,
};

/// An ordered list of tiled windows plus the layout that arranges them.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    windows: Vec<WindowId>,
    focused: Option<usize>,
    pub layout: LayoutParams,
}

impl Workspace {
    pub fn new(layout: LayoutParams) -> Self {
        Self {
            layout,
            ..Default::default()
        }
    }

    pub fn windows(&self) -> &[WindowId] {
        &self.windows
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.windows.contains(&id)
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focused.map(|i| self.windows[i])
    }

    /// New windows go right after the focused one and take focus.
    pub fn insert(&mut self, id: WindowId) {
        if self.contains(id) {
            return;
        }
        let at = self.focused.map_or(self.windows.len(), |i| i + 1);
        self.windows.insert(at, id);
        self.focused = Some(at);
    }

    pub fn remove(&mut self, id: WindowId) -> bool {
        let Some(i) = self.windows.iter().position(|w| *w == id) else {
            return false;
        };
        self.windows.remove(i);
        self.focused = match self.focused {
            _ if self.windows.is_empty() => None,
            Some(f) if f > i || f == self.windows.len() => Some(f - 1),
            other => other,
        };
        true
    }

    pub fn set_focus(&mut self, id: WindowId) {
        if let Some(i) = self.windows.iter().position(|w| *w == id) {
            self.focused = Some(i);
        }
    }

    fn step(&self, delta: isize) -> Option<(usize, usize)> {
        let n = self.windows.len() as isize;
        let f = self.focused? as isize;
        (n > 1).then(|| (f as usize, (f + delta).rem_euclid(n) as usize))
    }

    pub fn focus_step(&mut self, delta: isize) -> Option<WindowId> {
        if let Some((_, to)) = self.step(delta) {
            self.focused = Some(to);
        }
        self.focused()
    }

    pub fn swap_step(&mut self, delta: isize) {
        if let Some((from, to)) = self.step(delta) {
            self.windows.swap(from, to);
            self.focused = Some(to);
        }
    }

    /// Move the focused window to the master (first) slot.
    pub fn promote(&mut self) {
        if let Some(f) = self.focused {
            let w = self.windows.remove(f);
            self.windows.insert(0, w);
            self.focused = Some(0);
        }
    }

    pub fn arrange(&self, area: Rect) -> Vec<(WindowId, Rect)> {
        let rects = layout::compute(&self.layout, area, self.windows.len());
        self.windows.iter().copied().zip(rects).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(ids: &[u64]) -> Workspace {
        let mut w = Workspace::default();
        for &i in ids {
            w.insert(WindowId(i));
        }
        w
    }

    #[test]
    fn insert_after_focused() {
        let mut w = ws(&[1, 2]);
        w.set_focus(WindowId(1));
        w.insert(WindowId(3));
        assert_eq!(w.windows(), &[WindowId(1), WindowId(3), WindowId(2)]);
        assert_eq!(w.focused(), Some(WindowId(3)));
    }

    #[test]
    fn remove_keeps_focus_valid() {
        let mut w = ws(&[1, 2, 3]);
        w.remove(WindowId(3));
        assert_eq!(w.focused(), Some(WindowId(2)));
        w.set_focus(WindowId(2));
        w.remove(WindowId(1));
        assert_eq!(w.focused(), Some(WindowId(2)));
        w.remove(WindowId(2));
        assert_eq!(w.focused(), None);
    }

    #[test]
    fn focus_and_swap_wrap() {
        let mut w = ws(&[1, 2, 3]);
        assert_eq!(w.focus_step(1), Some(WindowId(1)));
        w.swap_step(-1);
        assert_eq!(w.windows(), &[WindowId(3), WindowId(2), WindowId(1)]);
        assert_eq!(w.focused(), Some(WindowId(1)));
    }

    #[test]
    fn promote_to_master() {
        let mut w = ws(&[1, 2, 3]);
        w.promote();
        assert_eq!(w.windows()[0], WindowId(3));
    }
}
