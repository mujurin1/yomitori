//! Win+Q の範囲指定と、Win+Q / Win+W の枠に添える表示の判定

use super::geometry::{PixelPoint, PixelRect};

/// Win+Q の範囲指定の状態。
/// Win+Q を押した瞬間のカーソル位置が起点。ボタンを押さずに動かすと反対側の角がカーソルに付いてくる。
/// 右ボタンを押している間の移動は起点も同じだけ動かす（矩形の大きさを保った平行移動）。
/// 左クリックで確定。矩形がほぼ点ならキャンセル
#[derive(Clone, Debug)]
pub struct RegionSelection {
    anchor: PixelPoint,
    current: PixelPoint,
}

impl RegionSelection {
    /// これより小さい矩形（幅か高さ）で左クリックしたらキャンセル扱い（ほぼ動かさずにクリック）
    pub const MIN_SIZE: i32 = 4;

    pub fn new(start: PixelPoint) -> Self {
        Self {
            anchor: start,
            current: start,
        }
    }

    pub fn anchor(&self) -> PixelPoint {
        self.anchor
    }

    pub fn current(&self) -> PixelPoint {
        self.current
    }

    pub fn rect(&self) -> PixelRect {
        PixelRect::from_corners(self.anchor, self.current)
    }

    /// OCR のプレビューに回せる大きさか
    pub fn is_usable(&self) -> bool {
        let r = self.rect();
        r.width >= Self::MIN_SIZE && r.height >= Self::MIN_SIZE
    }

    pub fn move_to(&mut self, cursor: PixelPoint, right_button_down: bool) {
        if right_button_down {
            self.anchor += cursor - self.current;
        }
        self.current = cursor;
    }

    /// 左クリックした。確定した矩形、キャンセルなら None
    pub fn click(&mut self, cursor: PixelPoint) -> Option<PixelRect> {
        self.move_to(cursor, false);
        self.is_usable().then(|| self.rect())
    }
}

/// 範囲指定中のマウスの 1 件（位置は画面の物理ピクセル）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseInput {
    Move(PixelPoint),
    LeftDown(PixelPoint),
    LeftUp(PixelPoint),
    RightDown(PixelPoint),
    RightUp(PixelPoint),
}

/// マウスの 1 件を受けた結果
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOutcome {
    /// 枠が動いた（描き直す）
    Moved,
    /// 左の押しから離しまでで終わった。確定した矩形、ほぼ点ならキャンセルで None
    Finished(Option<PixelRect>),
    /// 何もしない
    Ignored,
}

/// 範囲指定中のマウスの受け方。左の押しから離しで確定する（離しだけでは確定しない）。
/// 右を押している間の移動は平行移動
#[derive(Clone, Debug)]
pub struct RegionInput {
    selection: RegionSelection,
    left_pressed: bool,
    right_down: bool,
}

impl RegionInput {
    pub fn new(start: PixelPoint) -> Self {
        Self {
            selection: RegionSelection::new(start),
            left_pressed: false,
            right_down: false,
        }
    }

    pub fn selection(&self) -> &RegionSelection {
        &self.selection
    }

    pub fn handle(&mut self, input: MouseInput) -> InputOutcome {
        match input {
            MouseInput::Move(p) => {
                self.selection.move_to(p, self.right_down);
                InputOutcome::Moved
            }
            MouseInput::RightDown(p) => {
                self.right_down = true;
                self.selection.move_to(p, true);
                InputOutcome::Moved
            }
            MouseInput::RightUp(p) => {
                self.right_down = false;
                self.selection.move_to(p, false);
                InputOutcome::Moved
            }
            MouseInput::LeftDown(_) => {
                self.left_pressed = true;
                InputOutcome::Ignored
            }
            MouseInput::LeftUp(p) if self.left_pressed => {
                InputOutcome::Finished(self.selection.click(p))
            }
            MouseInput::LeftUp(_) => InputOutcome::Ignored,
        }
    }
}

/// Win+Q の範囲指定中のプレビューに何を出すか
pub mod region_preview {
    /// 出す文字列。出さない（プレビューを消す）なら None。
    /// 範囲が小さすぎる（OCR に回さない大きさ）時と、OCR の結果が文字なしの時は出さない
    pub fn text_to_show(recognized: &str, selection_usable: bool) -> Option<&str> {
        (selection_usable && !recognized.trim().is_empty()).then_some(recognized)
    }
}

/// 選択の枠に添えるプレビュー（OCR 結果）の置き場所
pub mod preview_placement {
    /// 矩形の左上の外側（矩形の上）に置く。上端からはみ出すなら矩形の内側の上端に置く。
    /// 単位は呼び出し側で揃える（DIP でもピクセルでもよい）
    pub fn compute(
        rect_left: f64,
        rect_top: f64,
        preview_height: f64,
        area_top: f64,
    ) -> (f64, f64) {
        let above = rect_top - preview_height;
        (rect_left, if above >= area_top { above } else { rect_top })
    }
}

/// Win+W の読み取り範囲の枠をいつ消すか
pub mod line_marker_timing {
    use std::time::Duration;

    /// 枠とプレビューを出しておく最低の時間
    pub const MINIMUM_VISIBLE: Duration = Duration::from_millis(700);

    /// 訳が届いた時点（表示から elapsed 経過）から、あとどれだけ枠を残すか。
    /// 「訳が届くまで」と「表示から MINIMUM_VISIBLE」の遅い方で消す
    pub fn remaining_after_arrival(elapsed_since_shown: Duration) -> Duration {
        MINIMUM_VISIBLE.saturating_sub(elapsed_since_shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const fn p(x: i32, y: i32) -> PixelPoint {
        PixelPoint::new(x, y)
    }

    #[test]
    fn 始めた瞬間は起点のカーソル位置だけの空の矩形() {
        let s = RegionSelection::new(p(5, 5));

        assert_eq!(p(5, 5), s.anchor());
        assert!(s.rect().is_empty());
        assert!(!s.is_usable());
    }

    #[test]
    fn ボタンを押さずに動かすと反対側の角がカーソルに付いてくる() {
        let mut s = RegionSelection::new(p(10, 20));

        s.move_to(p(60, 40), false);
        s.move_to(p(110, 70), false);

        assert_eq!(PixelRect::new(10, 20, 100, 50), s.rect());
    }

    #[test]
    fn 起点より左上へ動かしても矩形は正規化される() {
        let mut s = RegionSelection::new(p(100, 100));

        s.move_to(p(40, 30), false);

        assert_eq!(PixelRect::new(40, 30, 60, 70), s.rect());
    }

    #[test]
    fn 右ボタンを押している間は大きさを保ったまま平行移動する() {
        let mut s = RegionSelection::new(p(10, 10));
        s.move_to(p(60, 40), false);

        s.move_to(p(80, 35), true);
        s.move_to(p(100, 50), true);

        assert_eq!(p(50, 20), s.anchor());
        assert_eq!(PixelRect::new(50, 20, 50, 30), s.rect());
    }

    #[test]
    fn 右ボタンを離すと反対側の角の追従に戻る() {
        let mut s = RegionSelection::new(p(10, 10));
        s.move_to(p(60, 40), false);
        s.move_to(p(70, 50), true);

        s.move_to(p(120, 90), false);

        assert_eq!(p(20, 20), s.anchor());
        assert_eq!(PixelRect::new(20, 20, 100, 70), s.rect());
    }

    #[test]
    fn 左クリックでクリック位置までの矩形で確定する() {
        let mut s = RegionSelection::new(p(10, 10));
        s.move_to(p(50, 50), false);

        assert_eq!(Some(PixelRect::new(10, 10, 90, 40)), s.click(p(100, 50)));
    }

    #[test]
    fn ほぼ動かさずに左クリックするとキャンセル() {
        let mut s = RegionSelection::new(p(10, 10));

        assert_eq!(None, s.click(p(12, 11)));
    }

    #[test]
    fn 幅か高さが閾値未満ならキャンセル() {
        let min = RegionSelection::MIN_SIZE;
        for (width, height) in [(min - 1, 100), (100, min - 1)] {
            let mut s = RegionSelection::new(p(0, 0));

            assert_eq!(None, s.click(p(width, height)), "{width}x{height}");
        }
    }

    #[test]
    fn 幅と高さが閾値ちょうどなら確定する() {
        let min = RegionSelection::MIN_SIZE;
        let mut s = RegionSelection::new(p(0, 0));

        assert_eq!(Some(PixelRect::new(0, 0, min, min)), s.click(p(min, min)));
    }

    #[test]
    fn 左の押しから離しで離した位置までの矩形で確定する() {
        let mut input = RegionInput::new(p(10, 10));

        assert_eq!(
            InputOutcome::Moved,
            input.handle(MouseInput::Move(p(50, 40)))
        );
        assert_eq!(
            InputOutcome::Ignored,
            input.handle(MouseInput::LeftDown(p(50, 40)))
        );
        assert_eq!(
            InputOutcome::Finished(Some(PixelRect::new(10, 10, 60, 30))),
            input.handle(MouseInput::LeftUp(p(70, 40)))
        );
    }

    #[test]
    fn 押しの無い左の離しでは確定しない() {
        let mut input = RegionInput::new(p(10, 10));
        input.handle(MouseInput::Move(p(50, 40)));

        assert_eq!(
            InputOutcome::Ignored,
            input.handle(MouseInput::LeftUp(p(50, 40)))
        );
        assert_eq!(PixelRect::new(10, 10, 40, 30), input.selection().rect());
    }

    #[test]
    fn 左の押し離しがほぼ点ならキャンセル() {
        let mut input = RegionInput::new(p(10, 10));
        input.handle(MouseInput::LeftDown(p(11, 11)));

        assert_eq!(
            InputOutcome::Finished(None),
            input.handle(MouseInput::LeftUp(p(12, 11)))
        );
    }

    #[test]
    fn 右を押している間の移動は平行移動で離すと追従に戻る() {
        let mut input = RegionInput::new(p(10, 10));
        input.handle(MouseInput::Move(p(60, 40)));

        input.handle(MouseInput::RightDown(p(60, 40)));
        input.handle(MouseInput::Move(p(80, 50)));
        assert_eq!(PixelRect::new(30, 20, 50, 30), input.selection().rect());

        input.handle(MouseInput::RightUp(p(80, 50)));
        input.handle(MouseInput::Move(p(100, 70)));
        assert_eq!(PixelRect::new(30, 20, 70, 50), input.selection().rect());
    }

    #[test]
    fn 文字があり範囲が使える大きさならその文字列を出す() {
        assert_eq!(Some("Hello"), region_preview::text_to_show("Hello", true));
    }

    #[test]
    fn 文字なしならプレビューを出さない() {
        for recognized in ["", " ", "\t\n"] {
            assert_eq!(
                None,
                region_preview::text_to_show(recognized, true),
                "{recognized:?}"
            );
        }
    }

    #[test]
    fn 範囲が小さすぎるなら文字があっても出さない() {
        assert_eq!(None, region_preview::text_to_show("Hello", false));
    }

    #[test]
    fn 最低表示時間は700ms() {
        assert_eq!(
            Duration::from_millis(700),
            line_marker_timing::MINIMUM_VISIBLE
        );
    }

    #[test]
    fn 訳が早く届いたら表示から最低表示時間までの残りだけ残す() {
        assert_eq!(
            Duration::from_millis(500),
            line_marker_timing::remaining_after_arrival(Duration::from_millis(200))
        );
    }

    #[test]
    fn 訳が最低表示時間より後に届いたらすぐ消す() {
        assert_eq!(
            Duration::ZERO,
            line_marker_timing::remaining_after_arrival(Duration::from_millis(700))
        );
        assert_eq!(
            Duration::ZERO,
            line_marker_timing::remaining_after_arrival(Duration::from_secs(3))
        );
    }

    #[test]
    fn 収まるなら矩形の上の外側に置く() {
        assert_eq!(
            (30.0, 70.0),
            preview_placement::compute(30.0, 100.0, 30.0, 0.0)
        );
    }

    #[test]
    fn 上にはみ出すなら矩形の内側の上端に置く() {
        assert_eq!(
            (30.0, 10.0),
            preview_placement::compute(30.0, 10.0, 30.0, 0.0)
        );
    }

    #[test]
    fn ちょうど上端に収まる時は外側に置く() {
        assert_eq!((0.0, 0.0), preview_placement::compute(0.0, 30.0, 30.0, 0.0));
    }
}
