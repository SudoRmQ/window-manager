#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Shrink on all sides by `px` (never below zero size).
    pub fn inset(self, px: i32) -> Self {
        Self {
            x: self.x + px,
            y: self.y + px,
            w: (self.w - 2 * px).max(0),
            h: (self.h - 2 * px).max(0),
        }
    }

    /// Split horizontally (side by side). `ratio` is the left share.
    pub fn split_h(self, ratio: f32) -> (Self, Self) {
        let lw = (self.w as f32 * ratio).round() as i32;
        (
            Self::new(self.x, self.y, lw, self.h),
            Self::new(self.x + lw, self.y, self.w - lw, self.h),
        )
    }

    /// Split vertically (stacked). `ratio` is the top share.
    pub fn split_v(self, ratio: f32) -> (Self, Self) {
        let th = (self.h as f32 * ratio).round() as i32;
        (
            Self::new(self.x, self.y, self.w, th),
            Self::new(self.x, self.y + th, self.w, self.h - th),
        )
    }

    /// Split along the longer edge, so tiles stay roughly square.
    pub fn split_auto(self, ratio: f32) -> (Self, Self) {
        if self.w >= self.h {
            self.split_h(ratio)
        } else {
            self.split_v(ratio)
        }
    }

    /// Divide into `n` equal rows (stacked), absorbing rounding in the last one.
    pub fn rows(self, n: usize) -> Vec<Self> {
        if n == 0 {
            return vec![];
        }
        let step = self.h / n as i32;
        (0..n)
            .map(|i| {
                let y = self.y + step * i as i32;
                let h = if i == n - 1 {
                    self.y + self.h - y
                } else {
                    step
                };
                Self::new(self.x, y, self.w, h)
            })
            .collect()
    }
}
