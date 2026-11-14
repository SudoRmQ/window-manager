use crate::geometry::Rect;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutKind {
    /// Binary space partitioning (dwindle): each new window splits the previous tile.
    Bsp,
    /// One or more master windows on the left, the rest stacked on the right.
    MasterStack,
    /// Every window fills the work area; only the focused one is visible on top.
    Monocle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutParams {
    pub kind: LayoutKind,
    /// Share of the area given to the first split / master column (0.1..=0.9).
    pub ratio: f32,
    pub master_count: usize,
    /// Gap between windows and around the screen edge, in pixels.
    pub gap: i32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            kind: LayoutKind::Bsp,
            ratio: 0.5,
            master_count: 1,
            gap: 8,
        }
    }
}

impl LayoutParams {
    pub fn adjust_ratio(&mut self, delta: f32) {
        self.ratio = (self.ratio + delta).clamp(0.1, 0.9);
    }
}

/// Compute one rectangle per window, in window order.
pub fn compute(p: &LayoutParams, area: Rect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return vec![];
    }
    let half = p.gap / 2;
    // Outer gap = half from the area + half from each tile's inset = full gap at edges.
    let area = area.inset(half);
    let tiles = match p.kind {
        LayoutKind::Monocle => vec![area; n],
        LayoutKind::Bsp => bsp(area, n, p.ratio),
        LayoutKind::MasterStack => master_stack(area, n, p.ratio, p.master_count),
    };
    tiles.into_iter().map(|r| r.inset(half)).collect()
}

fn bsp(area: Rect, n: usize, ratio: f32) -> Vec<Rect> {
    let mut out = Vec::with_capacity(n);
    let mut rest = area;
    for i in 0..n {
        if i == n - 1 {
            out.push(rest);
        } else {
            // Only the first split honours the configured ratio; deeper splits are even.
            let r = if i == 0 { ratio } else { 0.5 };
            let (a, b) = rest.split_auto(r);
            out.push(a);
            rest = b;
        }
    }
    out
}

fn master_stack(area: Rect, n: usize, ratio: f32, master_count: usize) -> Vec<Rect> {
    let m = master_count.clamp(1, n);
    if n <= m {
        return area.rows(n);
    }
    let (left, right) = area.split_h(ratio);
    let mut out = left.rows(m);
    out.extend(right.rows(n - m));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

    fn no_gap(kind: LayoutKind) -> LayoutParams {
        LayoutParams {
            kind,
            gap: 0,
            ..Default::default()
        }
    }

    fn total_area(rs: &[Rect]) -> i64 {
        rs.iter().map(|r| r.w as i64 * r.h as i64).sum()
    }

    #[test]
    fn single_window_fills_screen() {
        for k in [
            LayoutKind::Bsp,
            LayoutKind::MasterStack,
            LayoutKind::Monocle,
        ] {
            assert_eq!(compute(&no_gap(k), SCREEN, 1), vec![SCREEN]);
        }
    }

    #[test]
    fn bsp_covers_screen_without_overlap() {
        for n in 1..=8 {
            let rs = compute(&no_gap(LayoutKind::Bsp), SCREEN, n);
            assert_eq!(rs.len(), n);
            assert_eq!(total_area(&rs), 1920 * 1080, "n={n}");
        }
    }

    #[test]
    fn bsp_alternates_split_direction() {
        let rs = compute(&no_gap(LayoutKind::Bsp), SCREEN, 3);
        assert_eq!(rs[0], Rect::new(0, 0, 960, 1080));
        assert_eq!(rs[1], Rect::new(960, 0, 960, 540));
        assert_eq!(rs[2], Rect::new(960, 540, 960, 540));
    }

    #[test]
    fn master_stack_layout() {
        let mut p = no_gap(LayoutKind::MasterStack);
        p.ratio = 0.6;
        let rs = compute(&p, SCREEN, 3);
        assert_eq!(rs[0], Rect::new(0, 0, 1152, 1080));
        assert_eq!(rs[1], Rect::new(1152, 0, 768, 540));
        assert_eq!(rs[2], Rect::new(1152, 540, 768, 540));
        assert_eq!(total_area(&rs), 1920 * 1080);
    }

    #[test]
    fn gaps_are_uniform() {
        let p = LayoutParams {
            kind: LayoutKind::MasterStack,
            gap: 10,
            ..Default::default()
        };
        let rs = compute(&p, SCREEN, 2);
        assert_eq!(rs[0], Rect::new(10, 10, 945, 1060));
        assert_eq!(rs[1], Rect::new(965, 10, 945, 1060));
    }

    #[test]
    fn ratio_is_clamped() {
        let mut p = LayoutParams::default();
        p.adjust_ratio(5.0);
        assert_eq!(p.ratio, 0.9);
        p.adjust_ratio(-5.0);
        assert_eq!(p.ratio, 0.1);
    }
}
