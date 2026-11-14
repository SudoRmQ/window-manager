//! The container tree, after i3's `Con` (see i3's `include/data.h`, `src/con.c`, `src/tree.c`,
//! `src/move.c`, `src/render.c`). Written from scratch in Rust; behaviour follows i3.
//!
//! ```text
//! Root
//! └── Output "primary"
//!     ├── Workspace "1"   (layout splith)
//!     │   ├── Window A
//!     │   └── Split       (layout splitv)
//!     │       ├── Window B
//!     │       └── Window C
//!     └── Workspace "2"
//! ```
//!
//! Every node keeps an ordered `children` list (spatial order), a `focus` stack (most recently
//! focused child first, floating children included for workspaces) and a `percent` share of its
//! parent. The globally focused node is `Tree::focused`; the visible workspace of an output is
//! the head of that output's focus stack.

use crate::{geometry::Rect, WindowId};
use serde::{Deserialize, Serialize};

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Splith,
    Splitv,
    #[serde(alias = "stacking")]
    Stacked,
    Tabbed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

impl Layout {
    /// Tabbed behaves horizontally and stacked vertically for focus and move, as in i3.
    pub fn orientation(self) -> Orientation {
        match self {
            Layout::Splith | Layout::Tabbed => Orientation::Horizontal,
            Layout::Splitv | Layout::Stacked => Orientation::Vertical,
        }
    }

    pub fn is_split(self) -> bool {
        matches!(self, Layout::Splith | Layout::Splitv)
    }

    pub fn name(self) -> &'static str {
        match self {
            Layout::Splith => "splith",
            Layout::Splitv => "splitv",
            Layout::Stacked => "stacked",
            Layout::Tabbed => "tabbed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn orientation(self) -> Orientation {
        match self {
            Direction::Left | Direction::Right => Orientation::Horizontal,
            Direction::Up | Direction::Down => Orientation::Vertical,
        }
    }

    /// Right and down move towards the end of a child list.
    pub fn forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Root,
    Output { name: String },
    Workspace { name: String },
    Split,
    Window(WindowId),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// Floating windows; only workspaces have them.
    pub floating: Vec<NodeId>,
    /// Focus stack: most recently focused child (tiling or floating) first.
    pub focus: Vec<NodeId>,
    pub layout: Layout,
    /// The split layout to return to from stacked/tabbed (`layout toggle split`).
    pub last_split: Layout,
    /// Share of the parent along the parent's orientation; siblings sum to 1.
    pub percent: f64,
    /// Last rendered rectangle (outputs: the usable screen area).
    pub rect: Rect,
    pub floating_rect: Option<Rect>,
    pub fullscreen: bool,
}

impl Node {
    fn new(kind: Kind, layout: Layout) -> Self {
        Self {
            kind,
            parent: None,
            children: vec![],
            floating: vec![],
            focus: vec![],
            layout,
            last_split: if layout.is_split() {
                layout
            } else {
                Layout::Splith
            },
            percent: 1.0,
            rect: Rect::default(),
            floating_rect: None,
            fullscreen: false,
        }
    }
}

/// What a workspace target in a command refers to.
#[derive(Debug, Clone, PartialEq)]
pub enum WsTarget {
    Name(String),
    Next,
    Prev,
    BackAndForth,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeOptions {
    /// Layout for new workspaces (i3's `workspace_layout` + `default_orientation`).
    pub default_layout: Layout,
    /// Wrap focus around at the edge of the workspace (i3's `focus_wrapping`).
    pub focus_wrapping: bool,
    /// `workspace N` on the current workspace goes back (i3's `workspace_auto_back_and_forth`).
    pub auto_back_and_forth: bool,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            default_layout: Layout::Splith,
            focus_wrapping: true,
            auto_back_and_forth: false,
        }
    }
}

pub struct Tree {
    nodes: Vec<Option<Node>>,
    free: Vec<NodeId>,
    pub root: NodeId,
    pub focused: NodeId,
    pub options: TreeOptions,
    prev_workspace: Option<String>,
}

/// Sort key for workspace names: numbered workspaces first, by number, then by name (as i3).
fn ws_sort_key(name: &str) -> (u64, String) {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    (digits.parse().unwrap_or(u64::MAX), name.to_string())
}

impl Tree {
    /// One output per `(name, usable area)`; output `i` starts with workspace `i + 1`.
    pub fn new(outputs: &[(String, Rect)], options: TreeOptions) -> Self {
        let mut t = Self {
            nodes: vec![],
            free: vec![],
            root: 0,
            focused: 0,
            options,
            prev_workspace: None,
        };
        t.root = t.alloc(Node::new(Kind::Root, Layout::Splith));
        for (i, (name, rect)) in outputs.iter().enumerate() {
            let o = t.alloc(Node::new(
                Kind::Output { name: name.clone() },
                Layout::Splith,
            ));
            t.node_mut(o).rect = *rect;
            t.attach(o, t.root, None);
            let ws = t.new_workspace(&(i + 1).to_string(), o);
            if i == 0 {
                t.set_focus(ws);
            }
        }
        t
    }

    // -----------------------------------------------------------------------------------------
    // Arena
    // -----------------------------------------------------------------------------------------

    fn alloc(&mut self, n: Node) -> NodeId {
        match self.free.pop() {
            Some(id) => {
                self.nodes[id] = Some(n);
                id
            }
            None => {
                self.nodes.push(Some(n));
                self.nodes.len() - 1
            }
        }
    }

    fn release(&mut self, id: NodeId) {
        self.nodes[id] = None;
        self.free.push(id);
    }

    pub fn node(&self, id: NodeId) -> &Node {
        self.nodes[id].as_ref().expect("live node")
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.nodes[id].as_mut().expect("live node")
    }

    pub fn exists(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(Option::is_some)
    }

    fn parent(&self, id: NodeId) -> NodeId {
        self.node(id).parent.expect("node has a parent")
    }

    // -----------------------------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------------------------

    pub fn window(&self, id: NodeId) -> Option<WindowId> {
        match self.node(id).kind {
            Kind::Window(w) => Some(w),
            _ => None,
        }
    }

    pub fn is_workspace(&self, id: NodeId) -> bool {
        matches!(self.node(id).kind, Kind::Workspace { .. })
    }

    pub fn is_floating(&self, id: NodeId) -> bool {
        self.node(id)
            .parent
            .is_some_and(|p| self.node(p).floating.contains(&id))
    }

    pub fn workspace_name(&self, ws: NodeId) -> &str {
        match &self.node(ws).kind {
            Kind::Workspace { name } | Kind::Output { name } => name,
            _ => "",
        }
    }

    pub fn find_window(&self, w: WindowId) -> Option<NodeId> {
        self.nodes
            .iter()
            .position(|n| n.as_ref().is_some_and(|n| n.kind == Kind::Window(w)))
    }

    pub fn workspace_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            if self.is_workspace(id) {
                return Some(id);
            }
            id = self.node(id).parent?;
        }
    }

    pub fn output_of(&self, id: NodeId) -> Option<NodeId> {
        let ws = self.workspace_of(id)?;
        self.node(ws).parent
    }

    pub fn outputs(&self) -> Vec<NodeId> {
        self.node(self.root).children.clone()
    }

    /// All workspaces, ordered by output then name.
    pub fn workspaces(&self) -> Vec<NodeId> {
        self.outputs()
            .into_iter()
            .flat_map(|o| self.node(o).children.clone())
            .collect()
    }

    pub fn find_workspace(&self, name: &str) -> Option<NodeId> {
        self.workspaces()
            .into_iter()
            .find(|&w| self.workspace_name(w) == name)
    }

    pub fn current_workspace(&self) -> NodeId {
        self.workspace_of(self.focused)
            .expect("focus is inside a workspace")
    }

    /// The workspace shown on `output` (head of its focus stack).
    pub fn visible_workspace(&self, output: NodeId) -> Option<NodeId> {
        self.node(output).focus.first().copied()
    }

    pub fn is_visible_workspace(&self, ws: NodeId) -> bool {
        self.node(ws).parent.and_then(|o| self.visible_workspace(o)) == Some(ws)
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        self.window(self.focused)
    }

    /// All windows below `id` (tiling and floating).
    pub fn windows_in(&self, id: NodeId) -> Vec<WindowId> {
        let mut out = vec![];
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(w) = self.window(n) {
                out.push(w);
            }
            let node = self.node(n);
            stack.extend(node.children.iter().chain(&node.floating));
        }
        out
    }

    /// Follow focus stacks down to a leaf.
    pub fn descend_focused(&self, mut id: NodeId) -> NodeId {
        while let Some(&c) = self.node(id).focus.first() {
            id = c;
        }
        id
    }

    /// The most recently focused tiling child, if any.
    fn focused_tiling_child(&self, id: NodeId) -> Option<NodeId> {
        let n = self.node(id);
        n.focus.iter().copied().find(|c| n.children.contains(c))
    }

    /// Like `descend_focused`, but never enters floating windows.
    fn descend_tiling_focused(&self, mut id: NodeId) -> NodeId {
        while let Some(c) = self.focused_tiling_child(id) {
            id = c;
        }
        id
    }

    fn index_in_parent(&self, id: NodeId) -> usize {
        let p = self.parent(id);
        self.node(p)
            .children
            .iter()
            .position(|&c| c == id)
            .expect("child of parent")
    }

    fn neighbour(&self, id: NodeId, forward: bool) -> Option<NodeId> {
        let i = self.index_in_parent(id);
        let sib = &self.node(self.parent(id)).children;
        if forward {
            sib.get(i + 1).copied()
        } else {
            i.checked_sub(1).map(|j| sib[j])
        }
    }

    fn is_container_with_children(&self, id: NodeId) -> bool {
        self.window(id).is_none() && !self.node(id).children.is_empty()
    }

    // -----------------------------------------------------------------------------------------
    // Structure
    // -----------------------------------------------------------------------------------------

    /// Attach as a tiling child at `index` (default: end). The new child gets a `1/n` share and
    /// its siblings shrink proportionally, as i3 does. It is appended to the focus stack
    /// (not focused).
    fn attach(&mut self, child: NodeId, parent: NodeId, index: Option<usize>) {
        self.node_mut(child).parent = Some(parent);
        let p = self.node_mut(parent);
        let at = index.unwrap_or(p.children.len()).min(p.children.len());
        p.children.insert(at, child);
        p.focus.push(child);
        let n = p.children.len() as f64;
        let siblings: Vec<NodeId> = p.children.iter().copied().filter(|&c| c != child).collect();
        let total: f64 = siblings.iter().map(|&s| self.node(s).percent).sum();
        for s in siblings {
            let share = if total > 0.0 {
                self.node(s).percent / total
            } else {
                1.0 / (n - 1.0)
            };
            self.node_mut(s).percent = share * (1.0 - 1.0 / n);
        }
        self.node_mut(child).percent = 1.0 / n;
    }

    fn attach_floating(&mut self, child: NodeId, ws: NodeId) {
        self.node_mut(child).parent = Some(ws);
        let w = self.node_mut(ws);
        w.floating.push(child);
        w.focus.push(child);
    }

    /// Remove from the parent; remaining siblings are scaled back up to sum to 1.
    fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.node(child).parent else {
            return;
        };
        let p = self.node_mut(parent);
        p.children.retain(|&c| c != child);
        p.floating.retain(|&c| c != child);
        p.focus.retain(|&c| c != child);
        self.normalize(parent);
        self.node_mut(child).parent = None;
    }

    fn normalize(&mut self, id: NodeId) {
        let kids = self.node(id).children.clone();
        let total: f64 = kids.iter().map(|&c| self.node(c).percent).sum();
        let n = kids.len() as f64;
        for c in kids {
            let p = self.node(c).percent;
            self.node_mut(c).percent = if total > 0.0 { p / total } else { 1.0 / n };
        }
    }

    /// Put `new` where `old` is (same index, share and focus position); `old` is detached.
    fn replace(&mut self, old: NodeId, new: NodeId) {
        let parent = self.parent(old);
        let percent = self.node(old).percent;
        let p = self.node_mut(parent);
        for list in [&mut p.children, &mut p.focus] {
            if let Some(slot) = list.iter_mut().find(|c| **c == old) {
                *slot = new;
            }
        }
        let n = self.node_mut(new);
        n.parent = Some(parent);
        n.percent = percent;
        self.node_mut(old).parent = None;
    }

    /// Remove split containers left empty, and collapse a split whose only child is another
    /// split (i3's `tree_flatten`). Returns the closest surviving ancestor-or-self.
    fn flatten(&mut self, mut id: NodeId) -> NodeId {
        loop {
            if self.node(id).kind != Kind::Split {
                return id;
            }
            let n = self.node(id);
            if n.children.is_empty() {
                let p = self.parent(id);
                self.detach(id);
                self.release(id);
                if self.focused == id {
                    self.set_focus(self.descend_focused(p));
                }
                id = p;
                continue;
            }
            if n.children.len() == 1 && self.node(n.children[0]).kind == Kind::Split {
                let only = n.children[0];
                self.detach(only);
                self.replace(id, only);
                self.release(id);
                if self.focused == id {
                    self.focused = only;
                }
                return only;
            }
            return id;
        }
    }

    fn new_workspace(&mut self, name: &str, output: NodeId) -> NodeId {
        let ws = self.alloc(Node::new(
            Kind::Workspace { name: name.into() },
            self.options.default_layout,
        ));
        self.node_mut(ws).rect = self.node(output).rect;
        let key = ws_sort_key(name);
        let at = self
            .node(output)
            .children
            .iter()
            .position(|&w| ws_sort_key(self.workspace_name(w)) > key);
        self.attach(ws, output, at);
        ws
    }

    /// Focus `id`: move it to the front of every ancestor's focus stack.
    pub fn set_focus(&mut self, id: NodeId) {
        let mut c = id;
        while let Some(p) = self.node(c).parent {
            let f = &mut self.node_mut(p).focus;
            f.retain(|&x| x != c);
            f.insert(0, c);
            c = p;
        }
        self.focused = id;
    }

    /// Make `id` the focused node *within* `upto` without moving global focus.
    fn set_focus_within(&mut self, id: NodeId, upto: NodeId) {
        let mut c = id;
        while c != upto {
            let p = self.parent(c);
            let f = &mut self.node_mut(p).focus;
            f.retain(|&x| x != c);
            f.insert(0, c);
            c = p;
        }
    }

    /// Insert `con` next to `target` (in target's parent), then clean up where it came from.
    fn insert_beside(&mut self, con: NodeId, target: NodeId, after: bool) {
        let old = self.parent(con);
        self.detach(con);
        let p = self.parent(target);
        let i = self.index_in_parent(target) + after as usize;
        self.attach(con, p, Some(i));
        self.flatten(old);
    }

    /// Where a new window (or a window entering a workspace) goes: right after the focused
    /// tiling window in its container, or at the end of a focused container.
    fn attach_near_focus(&mut self, con: NodeId, ws: NodeId) {
        let t = self.descend_tiling_focused(ws);
        if self.window(t).is_some() {
            let p = self.parent(t);
            let i = self.index_in_parent(t) + 1;
            self.attach(con, p, Some(i));
        } else {
            self.attach(con, t, None);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Window lifecycle
    // -----------------------------------------------------------------------------------------

    /// Start managing a window on the current workspace and focus it (i3's `manage_window`).
    pub fn manage(&mut self, w: WindowId) -> NodeId {
        if let Some(id) = self.find_window(w) {
            return id;
        }
        let con = self.alloc(Node::new(Kind::Window(w), Layout::Splith));
        let f = self.focused;
        if self.window(f).is_some() && !self.is_floating(f) {
            let p = self.parent(f);
            let i = self.index_in_parent(f) + 1;
            self.attach(con, p, Some(i));
        } else if self.window(f).is_none() {
            // A focused container (after `focus parent`) or an empty workspace.
            self.attach(con, f, None);
        } else {
            let ws = self.current_workspace();
            self.attach_near_focus(con, ws);
        }
        self.set_focus(con);
        con
    }

    /// Stop managing a window (closed, minimised, hidden by its app).
    pub fn unmanage(&mut self, w: WindowId) -> bool {
        let Some(con) = self.find_window(w) else {
            return false;
        };
        let was_focused = self.focused == con;
        let parent = self.parent(con);
        self.detach(con);
        self.release(con);
        let survivor = self.flatten(parent);
        if was_focused {
            let ws = self.workspace_of(survivor).expect("in a workspace");
            // i3 focuses the next con in the parent's focus stack.
            let next = if self.exists(survivor) { survivor } else { ws };
            self.set_focus(self.descend_focused(next));
        }
        true
    }

    /// Focus a window because the OS says the user focused it.
    pub fn focus_window(&mut self, w: WindowId) {
        if let Some(con) = self.find_window(w) {
            let ws = self.current_workspace();
            if self.workspace_of(con) != Some(ws) {
                self.prev_workspace = Some(self.workspace_name(ws).to_string());
            }
            self.set_focus(con);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Commands
    // -----------------------------------------------------------------------------------------

    /// `focus left|right|up|down` (i3's `tree_next`).
    pub fn focus_direction(&mut self, dir: Direction) {
        let cur = self.focused;
        if self.is_workspace(cur) || self.is_floating(cur) {
            return;
        }
        let ws = self.current_workspace();
        let o = dir.orientation();
        let mut c = cur;
        while c != ws {
            let p = self.parent(c);
            if self.node(p).layout.orientation() == o {
                if let Some(n) = self.neighbour(c, dir.forward()) {
                    let target = self.descend_focused(n);
                    self.set_focus(target);
                    return;
                }
            }
            c = p;
        }
        if !self.options.focus_wrapping {
            return;
        }
        // At the edge: wrap within the outermost container of the right orientation.
        let mut c = cur;
        let mut wrap_in = None;
        while c != ws {
            let p = self.parent(c);
            if self.node(p).layout.orientation() == o && self.node(p).children.len() > 1 {
                wrap_in = Some(p);
            }
            c = p;
        }
        if let Some(p) = wrap_in {
            let kids = &self.node(p).children;
            let end = if dir.forward() {
                kids[0]
            } else {
                kids[kids.len() - 1]
            };
            let target = self.descend_focused(end);
            self.set_focus(target);
        }
    }

    pub fn focus_parent(&mut self) {
        let f = self.focused;
        if self.is_workspace(f) {
            return;
        }
        let p = self.parent(f);
        if !self.is_floating(f) {
            self.set_focus(p);
        }
    }

    pub fn focus_child(&mut self) {
        if let Some(&c) = self.node(self.focused).focus.first() {
            self.set_focus(c);
        }
    }

    /// First ancestor (up to and including the workspace) whose orientation is `o`.
    fn parent_with_orientation(&self, con: NodeId, o: Orientation, ws: NodeId) -> Option<NodeId> {
        let mut c = self.parent(con);
        loop {
            if self.node(c).layout.orientation() == o {
                return Some(c);
            }
            if c == ws {
                return None;
            }
            c = self.parent(c);
        }
    }

    /// Wrap all of a workspace's tiling children in one container and give the workspace
    /// orientation `o` (i3's `ws_force_orientation`).
    fn force_orientation(&mut self, ws: NodeId, o: Orientation) {
        let old_layout = self.node(ws).layout;
        let wrapper = self.alloc(Node::new(Kind::Split, old_layout));
        let kids = self.node(ws).children.clone();
        let focus_order: Vec<NodeId> = self
            .node(ws)
            .focus
            .iter()
            .copied()
            .filter(|c| kids.contains(c))
            .collect();
        for &k in &kids {
            self.node_mut(k).parent = Some(wrapper);
        }
        {
            let w = self.node_mut(wrapper);
            w.children = kids.clone();
            w.focus = focus_order;
            w.parent = Some(ws);
            w.percent = 1.0;
        }
        let n = self.node_mut(ws);
        let first_tiling = n.focus.iter().position(|c| kids.contains(c)).unwrap_or(0);
        n.focus.retain(|c| !kids.contains(c));
        n.focus.insert(first_tiling.min(n.focus.len()), wrapper);
        n.children = vec![wrapper];
        n.layout = match o {
            Orientation::Horizontal => Layout::Splith,
            Orientation::Vertical => Layout::Splitv,
        };
        n.last_split = n.layout;
    }

    /// Descend into `con` towards the side we enter from (i3's `con_descend_direction`).
    fn descend_direction(&self, mut con: NodeId, dir: Direction) -> NodeId {
        loop {
            let n = self.node(con);
            if self.window(con).is_some() || n.children.is_empty() {
                return con;
            }
            con = if n.layout.orientation() == dir.orientation() {
                if dir.forward() {
                    n.children[0]
                } else {
                    n.children[n.children.len() - 1]
                }
            } else {
                self.focused_tiling_child(con).unwrap_or(n.children[0])
            };
        }
    }

    /// `move left|right|up|down` (i3's `tree_move`).
    pub fn move_direction(&mut self, dir: Direction) {
        let con = self.focused;
        if self.is_workspace(con) {
            return;
        }
        if self.is_floating(con) {
            let r = self.node(con).floating_rect.unwrap_or_default();
            let (dx, dy) = match dir {
                Direction::Left => (-20, 0),
                Direction::Right => (20, 0),
                Direction::Up => (0, -20),
                Direction::Down => (0, 20),
            };
            self.node_mut(con).floating_rect = Some(Rect::new(r.x + dx, r.y + dy, r.w, r.h));
            return;
        }
        let ws = self.current_workspace();
        if self.parent(con) == ws && self.node(ws).children.len() == 1 {
            return; // alone on the workspace (moving to another output comes later)
        }
        let o = dir.orientation();
        let fwd = dir.forward();
        let after_for =
            |t: &Tree, target: NodeId| t.node(t.parent(target)).layout.orientation() != o || !fwd;

        let mut same = self.parent_with_orientation(con, o, ws);
        loop {
            let s = match same {
                Some(s) => s,
                None => {
                    self.force_orientation(ws, o);
                    ws
                }
            };
            if s != self.parent(con) {
                same = Some(s);
                break;
            }
            // The easy case: move within this container.
            if let Some(sib) = self.neighbour(con, fwd) {
                if self.is_container_with_children(sib) {
                    let t = self.descend_direction(sib, dir);
                    let after = after_for(self, t);
                    self.insert_beside(con, t, after);
                } else {
                    let p = self.parent(con);
                    let (i, j) = (self.index_in_parent(con), self.index_in_parent(sib));
                    self.node_mut(p).children.swap(i, j);
                }
                self.set_focus(con);
                return;
            }
            if s == ws {
                return; // already at the edge of the workspace
            }
            same = self.parent_with_orientation(s, o, ws);
            if let Some(s2) = same {
                same = Some(s2);
                break;
            }
        }

        // Move out of the current container into an ancestor with the right orientation.
        let s = same.expect("found a container");
        let mut above = con;
        while self.parent(above) != s {
            above = self.parent(above);
        }
        match self.neighbour(above, fwd) {
            Some(next) if self.is_container_with_children(next) => {
                let t = self.descend_direction(next, dir);
                let after = after_for(self, t);
                self.insert_beside(con, t, after);
            }
            _ => {
                let old = self.parent(con);
                self.detach(con);
                let i = self.index_in_parent(above) + fwd as usize;
                self.attach(con, s, Some(i));
                self.flatten(old);
            }
        }
        self.set_focus(con);
    }

    /// `split h|v` (i3's `tree_split`).
    pub fn split(&mut self, layout: Layout) {
        let con = self.focused;
        if self.is_workspace(con) {
            let n = self.node_mut(con);
            n.layout = layout;
            n.last_split = layout;
            return;
        }
        if self.is_floating(con) {
            return;
        }
        let p = self.parent(con);
        if self.node(p).children.len() == 1 && self.node(p).layout.is_split() {
            let n = self.node_mut(p);
            n.layout = layout;
            n.last_split = layout;
            return;
        }
        let s = self.alloc(Node::new(Kind::Split, layout));
        self.replace(con, s);
        self.attach(con, s, None);
        self.set_focus(con);
    }

    /// The container whose layout `layout ...` changes: the focused con's parent.
    fn layout_target(&self) -> Option<NodeId> {
        let f = self.focused;
        if self.is_workspace(f) {
            Some(f)
        } else if self.is_floating(f) {
            None
        } else {
            Some(self.parent(f))
        }
    }

    pub fn set_layout(&mut self, layout: Layout) {
        if let Some(t) = self.layout_target() {
            let n = self.node_mut(t);
            n.layout = layout;
            if layout.is_split() {
                n.last_split = layout;
            }
        }
    }

    pub fn toggle_split(&mut self) {
        if let Some(t) = self.layout_target() {
            let l = match self.node(t).layout {
                Layout::Splith => Layout::Splitv,
                Layout::Splitv => Layout::Splith,
                _ => self.node(t).last_split,
            };
            self.set_layout(l);
        }
    }

    pub fn toggle_all_layouts(&mut self) {
        if let Some(t) = self.layout_target() {
            let l = match self.node(t).layout {
                Layout::Splith => Layout::Splitv,
                Layout::Splitv => Layout::Stacked,
                Layout::Stacked => Layout::Tabbed,
                Layout::Tabbed => Layout::Splith,
            };
            self.set_layout(l);
        }
    }

    pub fn layout_default(&mut self) {
        if let Some(t) = self.layout_target() {
            let l = self.node(t).last_split;
            self.set_layout(l);
        }
    }

    /// `resize grow|shrink width|height N ppt`.
    pub fn resize(&mut self, o: Orientation, delta: f64) {
        let ws = self.current_workspace();
        let mut c = self.focused;
        if self.is_floating(c) {
            let r = self.node(c).floating_rect.unwrap_or_default();
            let px = (delta * 1000.0) as i32;
            let r = match o {
                Orientation::Horizontal => Rect::new(r.x, r.y, (r.w + px).max(50), r.h),
                Orientation::Vertical => Rect::new(r.x, r.y, r.w, (r.h + px).max(50)),
            };
            self.node_mut(c).floating_rect = Some(r);
            return;
        }
        while c != ws {
            let p = self.parent(c);
            let pn = self.node(p);
            if pn.layout.is_split() && pn.layout.orientation() == o && pn.children.len() > 1 {
                let other = self
                    .neighbour(c, true)
                    .or_else(|| self.neighbour(c, false))
                    .expect("sibling");
                let (a, b) = (self.node(c).percent, self.node(other).percent);
                let d = delta.clamp((0.05 - a).min(0.0), (b - 0.05).max(0.0));
                self.node_mut(c).percent = a + d;
                self.node_mut(other).percent = b - d;
                return;
            }
            c = p;
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        let f = self.focused;
        if self.is_workspace(f) {
            return;
        }
        let on = !self.node(f).fullscreen;
        if on {
            let ws = self.current_workspace();
            let mut stack = vec![ws];
            while let Some(n) = stack.pop() {
                self.node_mut(n).fullscreen = false;
                let node = self.node(n);
                stack.extend(node.children.iter().chain(&node.floating));
            }
        }
        self.node_mut(f).fullscreen = on;
    }

    pub fn toggle_floating(&mut self) {
        let con = self.focused;
        if self.window(con).is_none() {
            return;
        }
        let ws = self.current_workspace();
        if self.is_floating(con) {
            self.detach(con);
            self.attach_near_focus(con, ws);
        } else {
            let old = self.parent(con);
            self.detach(con);
            self.flatten(old);
            if self.node(con).floating_rect.is_none() {
                let r = self.node(ws).rect;
                let (w, h) = (r.w * 3 / 5, r.h * 3 / 5);
                self.node_mut(con).floating_rect =
                    Some(Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - h) / 2, w, h));
            }
            self.attach_floating(con, ws);
        }
        self.set_focus(con);
    }

    fn resolve_workspace(&self, target: &WsTarget) -> Option<String> {
        let cur = self.current_workspace();
        let all = self.workspaces();
        let i = all.iter().position(|&w| w == cur)?;
        match target {
            WsTarget::Name(n) => Some(n.clone()),
            WsTarget::BackAndForth => self.prev_workspace.clone(),
            WsTarget::Next => Some(self.workspace_name(all[(i + 1) % all.len()]).to_string()),
            WsTarget::Prev => Some(
                self.workspace_name(all[(i + all.len() - 1) % all.len()])
                    .to_string(),
            ),
        }
    }

    fn get_or_create_workspace(&mut self, name: &str) -> NodeId {
        match self.find_workspace(name) {
            Some(ws) => ws,
            None => {
                let out = self.output_of(self.focused).expect("focus is on an output");
                self.new_workspace(name, out)
            }
        }
    }

    /// i3 deletes a workspace once it is empty and no longer shown.
    fn cleanup_workspace(&mut self, ws: NodeId) {
        if !self.exists(ws) || !self.is_workspace(ws) {
            return;
        }
        let n = self.node(ws);
        if n.children.is_empty() && n.floating.is_empty() && !self.is_visible_workspace(ws) {
            self.detach(ws);
            self.release(ws);
        }
    }

    /// `workspace <name>|next|prev|back_and_forth`.
    pub fn switch_workspace(&mut self, target: &WsTarget) {
        let Some(mut name) = self.resolve_workspace(target) else {
            return;
        };
        let cur = self.current_workspace();
        let cur_name = self.workspace_name(cur).to_string();
        if name == cur_name {
            if !(self.options.auto_back_and_forth && matches!(target, WsTarget::Name(_))) {
                return;
            }
            match self.prev_workspace.clone() {
                Some(p) => name = p,
                None => return,
            }
        }
        let ws = self.get_or_create_workspace(&name);
        self.prev_workspace = Some(cur_name);
        let leaf = self.descend_focused(ws);
        self.set_focus(leaf);
        self.cleanup_workspace(cur);
    }

    /// `move container to workspace <name>`: the focused con moves, focus stays here.
    pub fn move_to_workspace(&mut self, target: &WsTarget) {
        let con = self.focused;
        if self.is_workspace(con) {
            return;
        }
        let Some(name) = self.resolve_workspace(target) else {
            return;
        };
        let cur = self.current_workspace();
        if name == self.workspace_name(cur) {
            return;
        }
        let ws = self.get_or_create_workspace(&name);
        let floating = self.is_floating(con);
        let old = self.parent(con);
        self.detach(con);
        let survivor = self.flatten(old);
        self.node_mut(con).fullscreen = false;
        if floating {
            self.attach_floating(con, ws);
        } else {
            self.attach_near_focus(con, ws);
        }
        self.set_focus_within(con, ws);
        let next = if self.exists(survivor) { survivor } else { cur };
        self.set_focus(self.descend_focused(next));
    }

    // -----------------------------------------------------------------------------------------
    // Rendering (i3's render.c, minus decorations)
    // -----------------------------------------------------------------------------------------

    /// Compute rectangles for every visible window. `gap` is the space between windows and
    /// around the edge. Stacked/tabbed containers show only their focused child (title bars
    /// are not drawn yet). A fullscreen con covers the whole output and hides the rest.
    pub fn render(&mut self, gap: i32) -> Vec<(WindowId, Rect)> {
        let mut out = vec![];
        for o in self.outputs() {
            let Some(ws) = self.visible_workspace(o) else {
                continue;
            };
            let area = self.node(o).rect;
            self.node_mut(ws).rect = area;
            if let Some(fs) = self.find_fullscreen(ws) {
                self.render_con(fs, area, 0, &mut out);
                continue;
            }
            self.render_con(ws, area.inset(gap / 2), gap / 2, &mut out);
            for f in self.node(ws).floating.clone() {
                if let (Some(w), Some(r)) = (self.window(f), self.node(f).floating_rect) {
                    out.push((w, r));
                }
            }
        }
        out
    }

    fn find_fullscreen(&self, ws: NodeId) -> Option<NodeId> {
        let mut stack = vec![ws];
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            if node.fullscreen {
                return Some(n);
            }
            stack.extend(node.children.iter().chain(&node.floating));
        }
        None
    }

    fn render_con(
        &mut self,
        id: NodeId,
        rect: Rect,
        half_gap: i32,
        out: &mut Vec<(WindowId, Rect)>,
    ) {
        self.node_mut(id).rect = rect;
        if let Some(w) = self.window(id) {
            out.push((w, rect.inset(half_gap)));
            return;
        }
        let n = self.node(id);
        let kids = n.children.clone();
        if kids.is_empty() {
            return;
        }
        match n.layout {
            Layout::Stacked | Layout::Tabbed => {
                let shown = self.focused_tiling_child(id).unwrap_or(kids[0]);
                self.render_con(shown, rect, half_gap, out);
            }
            Layout::Splith | Layout::Splitv => {
                let horizontal = n.layout == Layout::Splith;
                let total = if horizontal { rect.w } else { rect.h };
                let mut offset = 0;
                for (i, &k) in kids.iter().enumerate() {
                    let size = if i == kids.len() - 1 {
                        total - offset
                    } else {
                        (self.node(k).percent * total as f64).round() as i32
                    };
                    let r = if horizontal {
                        Rect::new(rect.x + offset, rect.y, size, rect.h)
                    } else {
                        Rect::new(rect.x, rect.y + offset, rect.w, size)
                    };
                    self.render_con(k, r, half_gap, out);
                    offset += size;
                }
            }
        }
    }

    /// Debug helper: the tree as a compact string, e.g. `H[1 V[2 3]]` (focused marked `*`).
    pub fn sketch(&self, id: NodeId) -> String {
        let n = self.node(id);
        let star = if id == self.focused { "*" } else { "" };
        if let Some(w) = self.window(id) {
            return format!("{}{star}", w.0);
        }
        let tag = match n.layout {
            Layout::Splith => "H",
            Layout::Splitv => "V",
            Layout::Stacked => "S",
            Layout::Tabbed => "T",
        };
        let kids: Vec<String> = n.children.iter().map(|&c| self.sketch(c)).collect();
        let fl: Vec<String> = n
            .floating
            .iter()
            .map(|&c| format!("~{}", self.sketch(c)))
            .collect();
        let all: Vec<String> = kids.into_iter().chain(fl).collect();
        format!("{tag}{star}[{}]", all.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::*;

    fn tree() -> Tree {
        Tree::new(
            &[("primary".into(), Rect::new(0, 0, 1200, 800))],
            TreeOptions::default(),
        )
    }

    fn with(n: u64) -> Tree {
        let mut t = tree();
        for i in 1..=n {
            t.manage(WindowId(i));
        }
        t
    }

    fn ws(t: &Tree) -> String {
        t.sketch(t.current_workspace())
    }

    fn focus(t: &mut Tree, w: u64) {
        t.focus_window(WindowId(w));
    }

    #[test]
    fn new_windows_open_after_focused() {
        let mut t = with(3);
        assert_eq!(ws(&t), "H[1 2 3*]");
        focus(&mut t, 1);
        t.manage(WindowId(4));
        assert_eq!(ws(&t), "H[1 4* 2 3]");
    }

    #[test]
    fn equal_shares_after_open_and_close() {
        let mut t = with(3);
        let r = t.render(0);
        assert_eq!(r[0].1, Rect::new(0, 0, 400, 800));
        assert_eq!(r[2].1, Rect::new(800, 0, 400, 800));
        t.unmanage(WindowId(2));
        let r = t.render(0);
        assert_eq!(
            r,
            vec![
                (WindowId(1), Rect::new(0, 0, 600, 800)),
                (WindowId(3), Rect::new(600, 0, 600, 800))
            ]
        );
    }

    #[test]
    fn split_wraps_focused_window() {
        let mut t = with(2);
        t.split(Layout::Splitv);
        assert_eq!(ws(&t), "H[1 V[2*]]");
        t.manage(WindowId(3));
        assert_eq!(ws(&t), "H[1 V[2 3*]]");
        let r = t.render(0);
        assert_eq!(r[1], (WindowId(2), Rect::new(600, 0, 600, 400)));
        assert_eq!(r[2], (WindowId(3), Rect::new(600, 400, 600, 400)));
    }

    #[test]
    fn split_on_lone_child_changes_orientation() {
        let mut t = with(1);
        t.split(Layout::Splitv);
        assert_eq!(ws(&t), "V[1*]");
    }

    #[test]
    fn closing_last_window_in_split_removes_container() {
        let mut t = with(2);
        t.split(Layout::Splitv);
        t.manage(WindowId(3));
        t.unmanage(WindowId(3));
        assert_eq!(ws(&t), "H[1 V[2*]]");
        t.unmanage(WindowId(2));
        assert_eq!(ws(&t), "H[1*]");
    }

    #[test]
    fn focus_by_direction_uses_focus_stack() {
        let mut t = with(2);
        t.split(Layout::Splitv);
        t.manage(WindowId(3));
        focus(&mut t, 2);
        t.focus_direction(Left);
        assert_eq!(t.focused_window(), Some(WindowId(1)));
        // Coming back right lands on the last focused window in that column (2, not 3).
        t.focus_direction(Right);
        assert_eq!(t.focused_window(), Some(WindowId(2)));
        t.focus_direction(Down);
        assert_eq!(t.focused_window(), Some(WindowId(3)));
    }

    #[test]
    fn focus_wraps_at_edge() {
        let mut t = with(3);
        t.focus_direction(Right);
        assert_eq!(t.focused_window(), Some(WindowId(1)));
        t.focus_direction(Left);
        assert_eq!(t.focused_window(), Some(WindowId(3)));
    }

    #[test]
    fn move_swaps_within_container() {
        let mut t = with(3);
        t.move_direction(Left);
        assert_eq!(ws(&t), "H[1 3* 2]");
    }

    #[test]
    fn move_into_neighbouring_container() {
        let mut t = with(2);
        t.split(Layout::Splitv);
        t.manage(WindowId(3));
        focus(&mut t, 1);
        t.move_direction(Right);
        // Entering a vertical column from the left: placed next to its focused window (3).
        assert_eq!(ws(&t), "H[V[2 3 1*]]");
    }

    #[test]
    fn move_out_of_container() {
        let mut t = with(2);
        t.split(Layout::Splitv);
        t.manage(WindowId(3));
        t.move_direction(Right);
        assert_eq!(ws(&t), "H[1 V[2] 3*]");
    }

    #[test]
    fn move_perpendicular_forces_workspace_orientation() {
        let mut t = with(3);
        t.move_direction(Down);
        assert_eq!(ws(&t), "V[H[1 2] 3*]");
    }

    #[test]
    fn tabbed_shows_only_focused_child() {
        let mut t = with(3);
        t.set_layout(Layout::Tabbed);
        assert_eq!(t.render(0), vec![(WindowId(3), Rect::new(0, 0, 1200, 800))]);
        t.focus_direction(Left);
        assert_eq!(t.render(0), vec![(WindowId(2), Rect::new(0, 0, 1200, 800))]);
        t.toggle_split();
        assert_eq!(t.render(0).len(), 3);
    }

    #[test]
    fn layout_applies_to_parent_of_focused() {
        let mut t = with(2);
        t.split(Layout::Splitv);
        t.manage(WindowId(3));
        t.set_layout(Layout::Stacked);
        assert_eq!(ws(&t), "H[1 S[2 3*]]");
    }

    #[test]
    fn workspaces_switch_and_clean_up() {
        let mut t = with(1);
        t.switch_workspace(&WsTarget::Name("2".into()));
        assert_eq!(t.workspace_name(t.current_workspace()), "2");
        assert!(t.render(0).is_empty());
        t.switch_workspace(&WsTarget::BackAndForth);
        assert_eq!(t.focused_window(), Some(WindowId(1)));
        // Workspace 2 was empty and hidden, so it is gone.
        assert!(t.find_workspace("2").is_none());
    }

    #[test]
    fn move_container_to_workspace() {
        let mut t = with(2);
        t.move_to_workspace(&WsTarget::Name("3".into()));
        assert_eq!(ws(&t), "H[1*]");
        t.switch_workspace(&WsTarget::Name("3".into()));
        assert_eq!(ws(&t), "H[2*]");
        t.switch_workspace(&WsTarget::Prev);
        assert_eq!(t.focused_window(), Some(WindowId(1)));
    }

    #[test]
    fn floating_toggle_round_trip() {
        let mut t = with(2);
        t.toggle_floating();
        assert_eq!(ws(&t), "H[1 ~2*]");
        assert_eq!(t.render(0)[1], (WindowId(2), Rect::new(240, 160, 720, 480)));
        t.toggle_floating();
        assert_eq!(ws(&t), "H[1 2*]");
    }

    #[test]
    fn fullscreen_covers_output() {
        let mut t = with(2);
        t.toggle_fullscreen();
        assert_eq!(
            t.render(10),
            vec![(WindowId(2), Rect::new(0, 0, 1200, 800))]
        );
    }

    #[test]
    fn resize_moves_share_between_neighbours() {
        let mut t = with(2);
        t.resize(Orientation::Horizontal, 0.1);
        let r = t.render(0);
        assert_eq!(r[1].1.w, 720);
        assert_eq!(r[0].1.w, 480);
    }

    #[test]
    fn focus_parent_then_new_window_goes_inside() {
        let mut t = with(2);
        t.split(Layout::Splitv);
        t.manage(WindowId(3));
        t.focus_parent();
        t.move_direction(Left);
        assert_eq!(ws(&t), "H[V*[2 3] 1]");
    }

    /// Check structural invariants after every operation.
    fn check(t: &Tree) {
        assert!(t.exists(t.focused), "focused node exists");
        assert!(
            t.is_visible_workspace(t.current_workspace()),
            "focus is on a visible workspace"
        );
        let mut stack = vec![t.root];
        while let Some(id) = stack.pop() {
            let n = t.node(id);
            for &c in n.children.iter().chain(&n.floating) {
                assert_eq!(t.node(c).parent, Some(id), "parent link of {c}");
                assert!(n.focus.contains(&c), "{c} in focus stack of {id}");
                stack.push(c);
            }
            assert_eq!(
                n.focus.len(),
                n.children.len() + n.floating.len(),
                "focus stack size of {id}"
            );
            if n.kind == Kind::Split {
                assert!(!n.children.is_empty(), "empty split {id} survived");
            }
            if !n.children.is_empty() && !matches!(n.kind, Kind::Root | Kind::Output { .. }) {
                let sum: f64 = n.children.iter().map(|&c| t.node(c).percent).sum();
                assert!((sum - 1.0).abs() < 1e-6, "percents of {id} sum to {sum}");
            }
        }
    }

    #[test]
    fn random_operations_keep_tree_consistent() {
        for start in [0x2545_F491_4F6C_DD1Du64, 1, 42, 0xDEAD_BEEF, 7_777_777] {
            random_run(start);
        }
    }

    fn random_run(mut seed: u64) {
        let mut t = tree();
        let mut next_window = 1;
        for _ in 0..8_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let dir = [Left, Right, Up, Down][(seed >> 8) as usize % 4];
            let ws = WsTarget::Name(((seed >> 16) % 4 + 1).to_string());
            match seed % 16 {
                0 | 1 => {
                    t.manage(WindowId(next_window));
                    next_window += 1;
                }
                2 => {
                    let all = t.windows_in(t.root);
                    if !all.is_empty() {
                        t.unmanage(all[(seed >> 20) as usize % all.len()]);
                    }
                }
                3 => t.focus_direction(dir),
                4 | 5 => t.move_direction(dir),
                6 => t.split(if seed & 0x100 == 0 {
                    Layout::Splith
                } else {
                    Layout::Splitv
                }),
                7 => t.set_layout(
                    [
                        Layout::Splith,
                        Layout::Splitv,
                        Layout::Stacked,
                        Layout::Tabbed,
                    ][(seed >> 9) as usize % 4],
                ),
                8 => t.focus_parent(),
                9 => t.focus_child(),
                10 => t.switch_workspace(&ws),
                11 => t.move_to_workspace(&ws),
                12 => t.toggle_floating(),
                13 => t.toggle_fullscreen(),
                14 => t.resize(
                    dir.orientation(),
                    if seed & 0x200 == 0 { 0.05 } else { -0.05 },
                ),
                _ => {
                    let all = t.windows_in(t.root);
                    if !all.is_empty() {
                        t.focus_window(all[(seed >> 20) as usize % all.len()]);
                    }
                }
            }
            check(&t);
            let rendered = t.render(4);
            for (_, r) in &rendered {
                assert!(r.w >= 0 && r.h >= 0);
            }
        }
    }

    #[test]
    fn gaps_are_uniform() {
        let mut t = with(2);
        let r = t.render(10);
        assert_eq!(r[0].1, Rect::new(10, 10, 585, 780));
        assert_eq!(r[1].1, Rect::new(605, 10, 585, 780));
    }
}
