//! 結果ウィンドウの部品の置き場所。上段 OCR 文・下段 訳文を同じ高さで並べ、下の行に
//! 左 Topmost、右 Re-translate・OK を置く。長さは DIP で決め、表示倍率で物理ピクセルにする

use super::geometry::PixelRect;

const MARGIN: f64 = 8.0;
const SPLIT_GAP: f64 = 6.0;
const ROW_GAP: f64 = 8.0;
const BUTTON_HEIGHT: f64 = 26.0;
const OK_WIDTH: f64 = 80.0;
const RETRANSLATE_WIDTH: f64 = 100.0;
const BUTTON_GAP: f64 = 6.0;
const CHECK_WIDTH: f64 = 90.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultLayout {
    pub source: PixelRect,
    pub translation: PixelRect,
    pub topmost: PixelRect,
    pub retranslate: PixelRect,
    pub ok: PixelRect,
}

/// クライアント領域の大きさ（物理ピクセル）と表示倍率（dpi / 96）から置き場所を決める。
/// 小さすぎても大きさは負にしない
pub fn compute(client_width: i32, client_height: i32, scale: f64) -> ResultLayout {
    let px = |dip: f64| (dip * scale).round() as i32;
    let margin = px(MARGIN);
    let button_height = px(BUTTON_HEIGHT);
    let inner_width = (client_width - margin * 2).max(0);

    let buttons_top = (client_height - margin - button_height).max(margin);
    let ok_width = px(OK_WIDTH);
    let ok = PixelRect::new(
        client_width - margin - ok_width,
        buttons_top,
        ok_width,
        button_height,
    );
    let retranslate_width = px(RETRANSLATE_WIDTH);
    let retranslate = PixelRect::new(
        ok.x - px(BUTTON_GAP) - retranslate_width,
        buttons_top,
        retranslate_width,
        button_height,
    );
    let topmost = PixelRect::new(margin, buttons_top, px(CHECK_WIDTH), button_height);

    let boxes_height = (buttons_top - px(ROW_GAP) - margin - px(SPLIT_GAP)).max(0);
    let source_height = boxes_height / 2;
    let source = PixelRect::new(margin, margin, inner_width, source_height);
    let translation = PixelRect::new(
        margin,
        margin + source_height + px(SPLIT_GAP),
        inner_width,
        boxes_height - source_height,
    );

    ResultLayout {
        source,
        translation,
        topmost,
        retranslate,
        ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 上段と下段は同じ高さで余白の内側に並ぶ() {
        let l = compute(600, 200, 1.0);

        assert_eq!(PixelRect::new(8, 8, 584, l.source.height), l.source);
        assert!((l.source.height - l.translation.height).abs() <= 1);
        assert_eq!(l.source.bottom() + 6, l.translation.top());
        assert_eq!(l.translation.bottom() + 8, l.ok.top());
    }

    #[test]
    fn okは右下の余白に付きre_translateはその左に並ぶ() {
        let l = compute(600, 200, 1.0);

        assert_eq!(PixelRect::new(512, 166, 80, 26), l.ok);
        assert_eq!(l.ok.left() - 6, l.retranslate.right());
        assert_eq!(l.ok.top(), l.retranslate.top());
        assert_eq!(8, l.topmost.left());
    }

    #[test]
    fn 長さは表示倍率に合わせる() {
        let l = compute(900, 300, 1.5);

        assert_eq!(PixelRect::new(12, 12, 876, l.source.height), l.source);
        assert_eq!(PixelRect::new(900 - 12 - 120, 300 - 12 - 39, 120, 39), l.ok);
    }

    #[test]
    fn 小さすぎても大きさは負にならない() {
        let l = compute(10, 10, 1.0);

        for r in [l.source, l.translation, l.topmost, l.retranslate, l.ok] {
            assert!(r.width >= 0 && r.height >= 0, "{r:?}");
        }
    }
}
