//! 保存したウィンドウの位置を、今のモニター構成で使えるかと、結果ウィンドウの位置の追跡

use super::geometry::{PixelPoint, PixelRect};

/// ウィンドウの四隅がどれもいずれかのモニターの中にあるか（モニターをまたいでもよい）。
/// モニター構成が変わって画面外になった位置を使わないため
pub fn fits_on_monitors(window: PixelRect, monitors: &[PixelRect]) -> bool {
    let corners = [
        PixelPoint::new(window.left(), window.top()),
        PixelPoint::new(window.right() - 1, window.top()),
        PixelPoint::new(window.left(), window.bottom() - 1),
        PixelPoint::new(window.right() - 1, window.bottom() - 1),
    ];
    corners
        .iter()
        .all(|&c| monitors.iter().any(|m| m.contains(c)))
}

/// 領域（作業領域）の中央に置いた矩形
pub fn center_in(area: PixelRect, width: i32, height: i32) -> PixelRect {
    PixelRect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// 結果ウィンドウの「最後に見えていた時の位置と大きさ」を持つ。
/// 隠れている・最小化・閉じた後のウィンドウから読んだ矩形は意味が無い（0 等になる）ので覚えない。
/// 保存は、まだ保存していない値がある時だけ
#[derive(Clone, Debug, Default)]
pub struct PlacementTracker {
    last: Option<PixelRect>,
    saved: Option<PixelRect>,
}

impl PlacementTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 最後に見えていた時の矩形。一度も見えていなければ None
    pub fn last(&self) -> Option<PixelRect> {
        self.last
    }

    /// ウィンドウの今の状態。rect は読めなければ None
    pub fn observe(&mut self, rect: Option<PixelRect>, visible: bool, minimized: bool) {
        if !visible || minimized {
            return;
        }
        if let Some(r) = rect.filter(|r| !r.is_empty()) {
            self.last = Some(r);
        }
    }

    /// 保存すべき値。最後に見えていた値がまだ保存されていなければそれを返し、保存済みにする。無ければ None
    pub fn take_unsaved(&mut self) -> Option<PixelRect> {
        let last = self.last?;
        if self.saved == Some(last) {
            return None;
        }
        self.saved = Some(last);
        Some(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: PixelRect = PixelRect::new(-1920, 0, 1920, 1080);
    const MAIN: PixelRect = PixelRect::new(0, 0, 1920, 1080);

    #[test]
    fn モニターの中に収まれば使える() {
        assert!(fits_on_monitors(
            PixelRect::new(100, 100, 600, 200),
            &[MAIN]
        ));
    }

    #[test]
    fn モニターをまたいでも四隅がどれかのモニターにあれば使える() {
        assert!(fits_on_monitors(
            PixelRect::new(-300, 100, 600, 200),
            &[LEFT, MAIN]
        ));
    }

    #[test]
    fn 外したモニターの上にあった位置は使わない() {
        assert!(!fits_on_monitors(
            PixelRect::new(-1000, 100, 600, 200),
            &[MAIN]
        ));
    }

    #[test]
    fn 一部でもはみ出していたら使わない() {
        assert!(!fits_on_monitors(
            PixelRect::new(1500, 100, 600, 200),
            &[MAIN]
        ));
    }

    #[test]
    fn 中央に置く() {
        assert_eq!(
            PixelRect::new(660, 440, 600, 200),
            center_in(MAIN, 600, 200)
        );
    }

    const SHOWN: PixelRect = PixelRect::new(210, 160, 720, 330);

    #[test]
    fn 一度も見えていなければ保存しない() {
        let mut t = PlacementTracker::new();

        assert_eq!(None, t.last());
        assert_eq!(None, t.take_unsaved());
    }

    #[test]
    fn 見えている時の位置と大きさを覚えて保存する() {
        let mut t = PlacementTracker::new();

        t.observe(Some(SHOWN), true, false);

        assert_eq!(Some(SHOWN), t.take_unsaved());
    }

    #[test]
    fn 隠れた後や閉じた後に読んだ矩形では上書きしない() {
        let mut t = PlacementTracker::new();
        t.observe(Some(SHOWN), true, false);

        t.observe(Some(PixelRect::new(0, 0, 0, 0)), false, false);
        t.observe(None, false, false);
        t.observe(Some(PixelRect::new(5, 5, 100, 100)), false, false);

        assert_eq!(Some(SHOWN), t.last());
    }

    #[test]
    fn 最小化中の矩形では上書きしない() {
        let mut t = PlacementTracker::new();
        t.observe(Some(SHOWN), true, false);

        t.observe(Some(PixelRect::new(-32000, -32000, 160, 28)), true, true);

        assert_eq!(Some(SHOWN), t.last());
    }

    #[test]
    fn 見えていても大きさが0の矩形では上書きしない() {
        let mut t = PlacementTracker::new();
        t.observe(Some(SHOWN), true, false);

        t.observe(Some(PixelRect::new(0, 0, 0, 0)), true, false);
        t.observe(None, true, false);

        assert_eq!(Some(SHOWN), t.last());
    }

    #[test]
    fn 保存した値は変わるまでもう一度は保存しない() {
        let mut t = PlacementTracker::new();
        t.observe(Some(SHOWN), true, false);
        t.take_unsaved();

        assert_eq!(None, t.take_unsaved());

        let moved = PixelRect { x: 300, ..SHOWN };
        t.observe(Some(moved), true, false);
        assert_eq!(Some(moved), t.take_unsaved());
    }
}
