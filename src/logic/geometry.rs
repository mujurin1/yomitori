use std::ops::{Add, AddAssign, Sub};

/// 物理ピクセル座標の点
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelPoint {
    pub x: i32,
    pub y: i32,
}

impl PixelPoint {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl Add for PixelPoint {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl AddAssign for PixelPoint {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl Sub for PixelPoint {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl PixelPoint {
    /// 1 つの整数に詰める（ウィンドウのメッセージの lParam に載せるため）。負の座標もそのまま戻る
    pub const fn pack(self) -> i64 {
        ((self.x as i64) << 32) | (self.y as u32 as i64)
    }

    pub const fn unpack(packed: i64) -> Self {
        Self::new((packed >> 32) as i32, packed as i32)
    }
}

/// 物理ピクセル座標の矩形。right / bottom は含まない（半開区間）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self::new(left, top, right - left, bottom - top)
    }

    /// 2 点を対角とする矩形。どちらの点が左上でもよい
    pub fn from_corners(a: PixelPoint, b: PixelPoint) -> Self {
        Self::from_edges(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    }

    pub const fn left(&self) -> i32 {
        self.x
    }

    pub const fn top(&self) -> i32 {
        self.y
    }

    pub const fn right(&self) -> i32 {
        self.x + self.width
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub const fn contains(&self, p: PixelPoint) -> bool {
        p.x >= self.left() && p.x < self.right() && p.y >= self.top() && p.y < self.bottom()
    }

    /// 共通部分。重ならなければ幅か高さが 0 の矩形
    pub fn intersect(&self, other: PixelRect) -> PixelRect {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            return PixelRect::new(left, top, 0, 0);
        }
        PixelRect::from_edges(left, top, right, bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 点は詰めて戻すと同じになる() {
        for (x, y) in [
            (0, 0),
            (1920, 1080),
            (-2560, -200),
            (-1, 7),
            (i32::MAX, i32::MIN),
        ] {
            let p = PixelPoint::new(x, y);
            assert_eq!(p, PixelPoint::unpack(p.pack()), "({x}, {y})");
        }
    }

    #[test]
    fn 重ならない矩形との共通部分は空() {
        let a = PixelRect::new(0, 0, 10, 10);

        assert!(a.intersect(PixelRect::new(10, 0, 10, 10)).is_empty());
    }

    #[test]
    fn 共通部分は両方に含まれる範囲() {
        let a = PixelRect::new(0, 0, 10, 10);

        assert_eq!(
            PixelRect::new(5, 3, 5, 7),
            a.intersect(PixelRect::new(5, 3, 20, 20))
        );
    }
}
