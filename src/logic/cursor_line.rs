//! Win+W（カーソル行キャプチャ）のロジック

use super::geometry::{PixelPoint, PixelRect};

/// OCR の 1 行。bounds はキャプチャ画像内の物理ピクセル座標
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrLineInfo {
    pub text: String,
    pub bounds: PixelRect,
}

pub const BAND_WIDTH: i32 = 1500;
pub const BAND_HEIGHT: i32 = 120;

/// カーソルを中心にした帯の矩形。仮想スクリーンの外は切り捨てる
pub fn compute_band(cursor: PixelPoint, virtual_screen: PixelRect) -> PixelRect {
    let band = PixelRect::new(
        cursor.x - BAND_WIDTH / 2,
        cursor.y - BAND_HEIGHT / 2,
        BAND_WIDTH,
        BAND_HEIGHT,
    );
    band.intersect(virtual_screen)
}

/// カーソルの Y を縦方向に含む行を選ぶ。含む行が無ければ縦の距離が最も近い行。行が無ければ None。
/// 縦の距離が同じ行が複数あれば（段組みで同じ高さに別の列がある等）、横の距離が近い行、
/// それも同じなら行の縦の中心が近い行。全部同じなら先の行
pub fn select_line(lines: &[OcrLineInfo], cursor: PixelPoint) -> Option<&OcrLineInfo> {
    let mut best: Option<(&OcrLineInfo, (i32, i32, f64))> = None;
    for line in lines {
        let b = line.bounds;
        let key = (
            distance_outside(cursor.y, b.top(), b.bottom()),
            distance_outside(cursor.x, b.left(), b.right()),
            (f64::from(b.top()) + f64::from(b.height) / 2.0 - f64::from(cursor.y)).abs(),
        );
        if best.as_ref().is_none_or(|(_, best_key)| key < *best_key) {
            best = Some((line, key));
        }
    }
    best.map(|(line, _)| line)
}

/// 半開区間 [start, end) の外への距離。中にあれば 0
fn distance_outside(value: i32, start: i32, end: i32) -> i32 {
    if value < start {
        start - value
    } else if value >= end {
        value - (end - 1)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, top: i32, height: i32) -> OcrLineInfo {
        OcrLineInfo {
            text: text.into(),
            bounds: PixelRect::new(0, top, 100, height),
        }
    }

    fn info(text: &str, bounds: PixelRect) -> OcrLineInfo {
        OcrLineInfo {
            text: text.into(),
            bounds,
        }
    }

    fn selected(lines: &[OcrLineInfo], x: i32, y: i32) -> &str {
        &select_line(lines, PixelPoint::new(x, y))
            .expect("行が選ばれる")
            .text
    }

    #[test]
    fn カーソルのyを含む行を選ぶ() {
        let lines = [
            line("first", 0, 20),
            line("second", 30, 20),
            line("third", 60, 20),
        ];

        assert_eq!("second", selected(&lines, 50, 40));
    }

    #[test]
    fn 行の上端は含み下端は含まない() {
        let lines = [line("first", 0, 20), line("second", 20, 20)];

        assert_eq!("second", selected(&lines, 50, 20));
        assert_eq!("first", selected(&lines, 50, 19));
    }

    #[test]
    fn 含む行が無ければ縦の距離が最も近い行を選ぶ() {
        let lines = [line("upper", 0, 20), line("lower", 50, 20)];

        assert_eq!("upper", selected(&lines, 50, 25));
        assert_eq!("lower", selected(&lines, 50, 45));
        assert_eq!("lower", selected(&lines, 50, 110));
    }

    #[test]
    fn 複数の行が含むなら中心が近い行を選ぶ() {
        let lines = [line("tall", 0, 100), line("short", 50, 20)];

        assert_eq!("short", selected(&lines, 50, 58));
        assert_eq!("tall", selected(&lines, 50, 20));
    }

    #[test]
    fn 同じ高さに別の列があればカーソルの横の位置に近い行を選ぶ() {
        let left = info("left column", PixelRect::new(100, 50, 400, 20));
        let right = info("right column", PixelRect::new(1000, 52, 300, 20));

        assert_eq!(
            "left column",
            selected(&[right.clone(), left.clone()], 300, 60)
        );
        assert_eq!("right column", selected(&[left, right], 900, 60));
    }

    #[test]
    fn 横の位置より縦の距離を優先する() {
        let under = info("under cursor row", PixelRect::new(1000, 50, 300, 20));
        let near_x = info("next row", PixelRect::new(100, 80, 400, 20));

        assert_eq!("under cursor row", selected(&[near_x, under], 300, 60));
    }

    #[test]
    fn 行が無ければnone() {
        assert_eq!(None, select_line(&[], PixelPoint::new(50, 10)));
    }

    const SCREEN: PixelRect = PixelRect::new(0, 0, 1920, 1080);

    #[test]
    fn カーソルを中心に横1500縦120の帯をとる() {
        assert_eq!(
            PixelRect::new(210, 480, 1500, 120),
            compute_band(PixelPoint::new(960, 540), SCREEN)
        );
    }

    #[test]
    fn 左上の画面端で切る() {
        assert_eq!(
            PixelRect::from_edges(0, 0, 850, 70),
            compute_band(PixelPoint::new(100, 10), SCREEN)
        );
    }

    #[test]
    fn 右下の画面端で切る() {
        assert_eq!(
            PixelRect::from_edges(1150, 1010, 1920, 1080),
            compute_band(PixelPoint::new(1900, 1070), SCREEN)
        );
    }

    #[test]
    fn 原点が負の仮想スクリーンでも端で切る() {
        let virtual_screen = PixelRect::from_edges(-2560, -200, 1920, 1080);

        assert_eq!(
            PixelRect::from_edges(-2560, -200, -1750, -130),
            compute_band(PixelPoint::new(-2500, -190), virtual_screen)
        );
    }
}
