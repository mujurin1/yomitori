//! 範囲の塗りとプレビューの見た目（色・不透明度・文字の大きさ・フォントの選び方・画素の合成）

/// 範囲の内側の塗り #0078D4（COLORREF は 0x00BBGGRR）
pub const FILL_COLORREF: u32 = 0x00D4_7800;
/// 塗りの不透明度 15%。下の文字が透けて読める濃さ
pub const FILL_ALPHA: u8 = 38;

/// プレビューの背景の明るさ（#141414）
pub const BACKGROUND_LEVEL: u8 = 0x14;
/// プレビューの背景の不透明度 92%。文字は不透明
pub const BACKGROUND_ALPHA: u8 = 235;
/// プレビューの文字の大きさ（DIP）
pub const FONT_DIP: f64 = 15.0;

/// 日本語などを含むなら和文のフォント（Yu Gothic UI）、英字だけなら Segoe UI を使う
pub fn needs_japanese_font(text: &str) -> bool {
    text.chars().any(|c| u32::from(c) > 0x024F)
}

/// 背景 #141414 の上に白で描いた画素の明るさ（0〜255）から、premultiplied BGRA を作る。
/// 背景は BACKGROUND_ALPHA で半透明、文字の画素ほど不透明に近づき、文字そのものは不透明になる
pub fn compose(level: u8) -> [u8; 4] {
    let bg = f64::from(BACKGROUND_LEVEL);
    let coverage = ((f64::from(level) - bg) / (255.0 - bg)).clamp(0.0, 1.0);
    let alpha = f64::from(BACKGROUND_ALPHA) + coverage * (255.0 - f64::from(BACKGROUND_ALPHA));
    let color = bg + coverage * (255.0 - bg);
    let premultiplied = (color * alpha / 255.0).round() as u8;
    [
        premultiplied,
        premultiplied,
        premultiplied,
        alpha.round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 塗りは0078d4の不透明度15パーセント() {
        assert_eq!(0x00D4_7800, FILL_COLORREF);
        assert_eq!(38, FILL_ALPHA);
        assert!((f64::from(FILL_ALPHA) / 255.0 - 0.15).abs() < 0.005);
    }

    #[test]
    fn 背景の画素は141414の不透明度92パーセント() {
        let [b, g, r, a] = compose(BACKGROUND_LEVEL);

        assert_eq!(235, a);
        assert!((f64::from(a) / 255.0 - 0.92).abs() < 0.005);
        // premultiplied: 0x14 × 235 / 255
        assert_eq!([18, 18, 18], [b, g, r]);
    }

    #[test]
    fn 文字の画素は白で透けない() {
        assert_eq!([255, 255, 255, 255], compose(255));
    }

    #[test]
    fn 文字の縁は背景と白の間で不透明度も間() {
        let [b, _, _, a] = compose(0x80);

        assert!(a > BACKGROUND_ALPHA && a < 255, "{a}");
        assert!(b > 18 && b < 255, "{b}");
        assert!(b <= a, "premultiplied は色が不透明度を超えない");
    }

    #[test]
    fn 日本語を含むなら和文のフォントにする() {
        for (text, japanese) in [
            ("The quick brown fox.", false),
            ("Café naïve", false),
            ("文字なし", true),
            ("OCR の結果", true),
        ] {
            assert_eq!(japanese, needs_japanese_font(text), "{text}");
        }
    }
}
