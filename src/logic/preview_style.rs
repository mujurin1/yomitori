//! 範囲の塗りとプレビューの見た目（色・不透明度・文字の大きさ・フォントの選び方・画素の合成）

/// 範囲の内側の塗り #0078D4（COLORREF は 0x00BBGGRR）
pub const FILL_COLORREF: u32 = 0x00D4_7800;
/// 塗りの不透明度 15%。下の文字が透けて読める濃さ
pub const FILL_ALPHA: u8 = 38;

/// プレビューの背景の明るさ（#141414）
pub const BACKGROUND_LEVEL: u8 = 0x14;
/// プレビューの背景の不透明度 92%。文字は不透明
pub const BACKGROUND_ALPHA: u8 = 235;
/// プレビューの文字の明るさ（#DCDCDC。白だと眩しいので落とす）
pub const TEXT_LEVEL: u8 = 0xDC;
/// プレビューの文字の大きさ（DIP）
pub const FONT_DIP: f64 = 15.0;
/// プレビューの箱の外周の縁の太さ（DIP）
pub const BORDER_DIP: f64 = 1.0;
/// 縁の画素（#787878・不透明。premultiplied BGRA）
pub const BORDER_PIXEL: [u8; 4] = [0x78, 0x78, 0x78, 255];

/// 幅 width・高さ height の箱で、(x, y) が外周の太さ border の縁に入るか
pub fn is_border(x: i32, y: i32, width: i32, height: i32, border: i32) -> bool {
    x < border || y < border || x >= width - border || y >= height - border
}

/// 箱の (x, y) の画素。縁なら白・不透明、内側は compose（背景は半透明・文字は不透明）
pub fn pixel(x: i32, y: i32, width: i32, height: i32, border: i32, level: u8) -> [u8; 4] {
    if is_border(x, y, width, height, border) {
        BORDER_PIXEL
    } else {
        compose(level)
    }
}

/// 日本語などを含むなら和文のフォント（Yu Gothic UI）、英字だけなら Segoe UI を使う
pub fn needs_japanese_font(text: &str) -> bool {
    text.chars().any(|c| u32::from(c) > 0x024F)
}

/// プレビューの書体（Semibold の書体名、それが無い時に落とす Regular の書体名）
pub fn font_faces(text: &str) -> (&'static str, &'static str) {
    if needs_japanese_font(text) {
        ("Yu Gothic UI Semibold", "Yu Gothic UI")
    } else {
        ("Segoe UI Semibold", "Segoe UI")
    }
}

/// 背景 #141414 の上に白で描いた画素の明るさ（0〜255）から、premultiplied BGRA を作る。
/// 背景は BACKGROUND_ALPHA で半透明、文字の画素ほど不透明に近づき、文字そのものは不透明の TEXT_LEVEL になる。
/// 白で描くのは文字の濃さ（アンチエイリアス）を明るさから読むため
pub fn compose(level: u8) -> [u8; 4] {
    let bg = f64::from(BACKGROUND_LEVEL);
    let coverage = ((f64::from(level) - bg) / (255.0 - bg)).clamp(0.0, 1.0);
    let alpha = f64::from(BACKGROUND_ALPHA) + coverage * (255.0 - f64::from(BACKGROUND_ALPHA));
    let color = bg + coverage * (f64::from(TEXT_LEVEL) - bg);
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
    fn 文字の画素はdcdcdcで透けない() {
        assert_eq!([0xDC, 0xDC, 0xDC, 255], compose(255));
    }

    #[test]
    fn 文字の縁は背景と白の間で不透明度も間() {
        let [b, _, _, a] = compose(0x80);

        assert!(a > BACKGROUND_ALPHA && a < 255, "{a}");
        assert!(b > 18 && b < 0xDC, "{b}");
        assert!(b <= a, "premultiplied は色が不透明度を超えない");
    }

    #[test]
    fn 箱の外周の縁は787878で不透明_内側は今の背景() {
        let (w, h, border) = (10, 6, 1);
        for (x, y) in [
            (0, 0),
            (9, 0),
            (0, 5),
            (9, 5),
            (4, 0),
            (4, 5),
            (0, 3),
            (9, 3),
        ] {
            assert_eq!(
                [0x78, 0x78, 0x78, 255],
                pixel(x, y, w, h, border, BACKGROUND_LEVEL),
                "({x}, {y})"
            );
        }
        for (x, y) in [(1, 1), (8, 4), (4, 3)] {
            assert_eq!(
                compose(BACKGROUND_LEVEL),
                pixel(x, y, w, h, border, BACKGROUND_LEVEL),
                "({x}, {y})"
            );
        }
    }

    #[test]
    fn 縁の太さは表示倍率に合わせる() {
        use crate::logic::frame::to_pixels;
        assert_eq!(1, to_pixels(BORDER_DIP, 96));
        assert_eq!(2, to_pixels(BORDER_DIP, 192));
        let (w, h, border) = (10, 6, to_pixels(BORDER_DIP, 192));
        assert!(is_border(1, 3, w, h, border));
        assert!(!is_border(2, 3, w, h, border));
        assert!(is_border(8, 3, w, h, border));
        assert!(!is_border(7, 3, w, h, border));
    }

    #[test]
    fn 書体はsemiboldで無ければ同じ書体のregular() {
        assert_eq!(
            ("Segoe UI Semibold", "Segoe UI"),
            font_faces("The quick brown fox.")
        );
        assert_eq!(
            ("Yu Gothic UI Semibold", "Yu Gothic UI"),
            font_faces("文字なし")
        );
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
