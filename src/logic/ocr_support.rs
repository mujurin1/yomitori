//! OCR まわりの定数と、画面に依存しない補助

/// 高さがこれ未満の画像は拡大してから OCR に渡す（小さい文字の精度のため）
pub const UPSCALE_BELOW_HEIGHT: i32 = 400;

/// 拡大する時の倍率の上限
pub const MAX_UPSCALE_FACTOR: i32 = 3;

/// OCR に渡す時の整数倍率。高さが閾値未満なら閾値に届く倍率（上限 MAX_UPSCALE_FACTOR）。
/// 拡大後の長辺が max_dimension（OcrEngine.MaxImageDimension）を超えない範囲に抑える
pub fn compute_scale(width: i32, height: i32, max_dimension: i32) -> i32 {
    if width <= 0 || height <= 0 || height >= UPSCALE_BELOW_HEIGHT {
        return 1;
    }
    let wanted = MAX_UPSCALE_FACTOR.min((UPSCALE_BELOW_HEIGHT + height - 1) / height);
    let longest = width.max(height);
    let allowed = (max_dimension / longest).max(1);
    wanted.min(allowed).max(1)
}

/// 複数行を空白 1 つで連結して 1 段落にする（行内の連続空白も 1 つに詰める）
pub fn join_lines<I, S>(lines: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut joined = String::new();
    for line in lines {
        for word in line.as_ref().split_whitespace() {
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(word);
        }
    }
    joined
}

/// OCR の行を連結する。keep_line_breaks なら行ごとに改行（LF）で区切り、そうでなければ空白 1 つで 1 段落にする。
/// どちらも行内の連続空白は 1 つに詰め、空の行は捨てる
pub fn join_ocr_lines<I, S>(lines: I, keep_line_breaks: bool) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if !keep_line_breaks {
        return join_lines(lines);
    }
    let mut kept: Vec<String> = Vec::new();
    for line in lines {
        let joined = join_lines([line]);
        if !joined.is_empty() {
            kept.push(joined);
        }
    }
    kept.join("\n")
}

/// 利用可能な OCR 言語タグから英語を選ぶ。en-US があればそれ、無ければ最初の en 系。無ければ None
pub fn pick_english<S: AsRef<str>>(available_tags: &[S]) -> Option<&S> {
    let is = |t: &S, s: &str| t.as_ref().eq_ignore_ascii_case(s);
    available_tags.iter().find(|t| is(t, "en-US")).or_else(|| {
        available_tags.iter().find(|t| {
            let b = t.as_ref().as_bytes();
            is(t, "en") || (b.len() >= 3 && b[..3].eq_ignore_ascii_case(b"en-"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::cursor_line::{BAND_HEIGHT, BAND_WIDTH};

    #[test]
    fn 閾値以上の高さなら拡大しない() {
        assert_eq!(1, compute_scale(1000, UPSCALE_BELOW_HEIGHT, 10000));
    }

    #[test]
    fn 低い画像は閾値に届く倍率で拡大する() {
        assert_eq!(2, compute_scale(500, UPSCALE_BELOW_HEIGHT / 2, 10000));
    }

    #[test]
    fn 倍率は上限を超えない() {
        assert_eq!(MAX_UPSCALE_FACTOR, compute_scale(100, 10, 10000));
    }

    #[test]
    fn 拡大後の長辺が最大寸法を超えない倍率に抑える() {
        assert_eq!(2, compute_scale(4000, 50, 10000));
        assert_eq!(1, compute_scale(6000, 50, 10000));
    }

    #[test]
    fn 帯の大きさは拡大されて最大寸法に収まる() {
        let scale = compute_scale(BAND_WIDTH, BAND_HEIGHT, 10000);

        assert!(scale > 1);
        assert!(BAND_WIDTH * scale <= 10000);
    }

    #[test]
    fn 複数行は空白1つで連結する() {
        assert_eq!("This is a pen.", join_lines(["This is", "  a pen. "]));
    }

    #[test]
    fn 改行を残すなら行ごとに改行で区切る() {
        assert_eq!(
            "This is\na pen.",
            join_ocr_lines(["This  is ", "", "  a pen. "], true)
        );
    }

    #[test]
    fn 改行を残さないなら空白1つで1段落にする() {
        assert_eq!(
            "This is a pen.",
            join_ocr_lines(["This  is ", "", "  a pen. "], false)
        );
    }

    #[test]
    fn 行が無ければどちらでも空文字列() {
        for keep in [true, false] {
            assert_eq!("", join_ocr_lines(Vec::<String>::new(), keep), "{keep}");
        }
    }

    #[test]
    fn 行が無ければ空文字列() {
        assert_eq!("", join_lines(Vec::<String>::new()));
    }

    #[test]
    fn en_usがあれば選ぶ() {
        assert_eq!(Some(&"en-US"), pick_english(&["ja", "en-GB", "en-US"]));
    }

    #[test]
    fn en_usが無ければ最初のen系を選ぶ() {
        assert_eq!(Some(&"en-GB"), pick_english(&["ja", "en-GB", "en-AU"]));
    }

    #[test]
    fn en系が無ければnone() {
        assert_eq!(None, pick_english(&["ja", "fr-FR", "eno"]));
    }
}
