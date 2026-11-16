//! Win+W（カーソル行キャプチャ）のロジック

use super::geometry::{PixelPoint, PixelRect};
use super::ocr_support::join_lines;

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

/// まとめる行どうしの横の隙間の上限（行の高さの何倍か）。これを超えたら別の行として切る
/// （離れた段組みや遠くの文字をつながない）
pub const MERGE_GAP_HEIGHTS: i32 = 3;

/// Windows OCR は同じ高さの文字を隙間で別の行に分けて返すことがある（等幅のターミナル等）。
/// 縦の範囲が重なる行（低い方の高さの半分以上）を左から順にまとめて 1 行にする。
/// 横の隙間が行の高さ × MERGE_GAP_HEIGHTS を超えたら、そこで切る。文字列は空白 1 つでつなぎ、矩形は外接矩形
pub fn merge_lines(lines: &[OcrLineInfo]) -> Vec<OcrLineInfo> {
    // 同じ高さの行の集まり（縦に重なるもの）
    let mut rows: Vec<Vec<&OcrLineInfo>> = Vec::new();
    let mut sorted: Vec<&OcrLineInfo> = lines.iter().collect();
    sorted.sort_by_key(|l| (l.bounds.top(), l.bounds.left()));
    for line in sorted {
        let row = rows.iter_mut().find(|row| {
            row.iter()
                .any(|other| same_height(&other.bounds, &line.bounds))
        });
        match row {
            Some(row) => row.push(line),
            None => rows.push(vec![line]),
        }
    }

    let mut merged = Vec::new();
    for mut row in rows {
        row.sort_by_key(|l| l.bounds.left());
        let mut current: Option<(Vec<&str>, PixelRect)> = None;
        for line in row {
            let b = line.bounds;
            current = Some(match current.take() {
                Some((mut texts, bounds))
                    if b.left() - bounds.right()
                        <= MERGE_GAP_HEIGHTS * bounds.height.max(b.height) =>
                {
                    texts.push(&line.text);
                    (texts, union(bounds, b))
                }
                previous => {
                    if let Some((texts, bounds)) = previous {
                        merged.push(OcrLineInfo {
                            text: join_lines(texts),
                            bounds,
                        });
                    }
                    (vec![line.text.as_str()], b)
                }
            });
        }
        if let Some((texts, bounds)) = current {
            merged.push(OcrLineInfo {
                text: join_lines(texts),
                bounds,
            });
        }
    }
    merged
}

/// 縦の範囲が、低い方の高さの半分以上重なる
fn same_height(a: &PixelRect, b: &PixelRect) -> bool {
    let overlap = a.bottom().min(b.bottom()) - a.top().max(b.top());
    overlap > 0 && overlap * 2 >= a.height.min(b.height)
}

fn union(a: PixelRect, b: PixelRect) -> PixelRect {
    PixelRect::from_edges(
        a.left().min(b.left()),
        a.top().min(b.top()),
        a.right().max(b.right()),
        a.bottom().max(b.bottom()),
    )
}

/// Win+W で読む行の範囲を、行の矩形から横に行の高さ × NEAR_X_HEIGHTS 広げる
pub const NEAR_X_HEIGHTS: f64 = 1.0;
/// Win+W で読む行の範囲を、行の矩形から縦に行の高さ × NEAR_Y_HEIGHTS 広げる
pub const NEAR_Y_HEIGHTS: f64 = 0.5;

/// カーソルが行のすぐ近く（行の矩形を NEAR_X_HEIGHTS・NEAR_Y_HEIGHTS だけ広げた範囲）にある行だけを候補にし、
/// その中から select_line で選ぶ。候補が無ければ None（離れた文字には反応しない）
pub fn select_near_line(lines: &[OcrLineInfo], cursor: PixelPoint) -> Option<&OcrLineInfo> {
    let near: Vec<OcrLineInfo> = lines
        .iter()
        .filter(|l| is_near(&l.bounds, cursor))
        .cloned()
        .collect();
    let chosen = select_line(&near, cursor)?;
    lines.iter().find(|l| *l == chosen)
}

fn is_near(b: &PixelRect, cursor: PixelPoint) -> bool {
    let h = f64::from(b.height);
    let (x, y) = (f64::from(cursor.x), f64::from(cursor.y));
    x >= f64::from(b.left()) - h * NEAR_X_HEIGHTS
        && x < f64::from(b.right()) + h * NEAR_X_HEIGHTS
        && y >= f64::from(b.top()) - h * NEAR_Y_HEIGHTS
        && y < f64::from(b.bottom()) + h * NEAR_Y_HEIGHTS
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

    fn at(text: &str, x: i32, y: i32, width: i32, height: i32) -> OcrLineInfo {
        info(text, PixelRect::new(x, y, width, height))
    }

    #[test]
    fn 同じ高さで隙間の小さい行は左から順に1行にまとめる() {
        let lines = [
            at("--project", 200, 50, 90, 18),
            at("dotnet", 100, 51, 60, 15),
            at("D:/path/to/project", 300, 49, 180, 19),
        ];

        let merged = merge_lines(&lines);

        assert_eq!(1, merged.len(), "{merged:?}");
        assert_eq!("dotnet --project D:/path/to/project", merged[0].text);
        assert_eq!(PixelRect::from_edges(100, 49, 480, 68), merged[0].bounds);
    }

    #[test]
    fn 横の隙間が行の高さの3倍を超えたら別の行にする() {
        let height = 20;
        for (gap, count) in [(height * 3, 1), (height * 3 + 1, 2), (338, 2)] {
            let lines = [
                at("left", 100, 50, 200, height),
                at("far", 300 + gap, 50, 80, height),
            ];

            assert_eq!(count, merge_lines(&lines).len(), "gap {gap}");
        }
    }

    #[test]
    fn 高さの違う行はまとめない() {
        let lines = [
            at("upper", 100, 50, 200, 20),
            at("lower", 100, 80, 200, 20),
            at("half", 320, 61, 50, 20),
        ];

        let merged = merge_lines(&lines);

        // half は upper と 9 px（半分未満）、lower とは重ならないので、どれとも同じ高さではない
        assert_eq!(3, merged.len(), "{merged:?}");
    }

    #[test]
    fn 半分以上重なれば少しずれていても同じ高さとみなす() {
        let lines = [at("a", 100, 50, 50, 20), at("b", 160, 58, 50, 20)];

        assert_eq!(
            vec!["a b"],
            merge_lines(&lines)
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn カーソルを含むまとまりが選ばれる() {
        let lines = [
            at("dotnet", 361, 49, 63, 15),
            at("run --project D: / path/ to/ project", 436, 49, 337, 19),
            at("Separate", 1111, 50, 84, 18),
            at("next line", 361, 80, 100, 18),
        ];
        let merged = merge_lines(&lines);

        let chosen = select_line(&merged, PixelPoint::new(470, 58)).expect("選ばれる");

        assert_eq!("dotnet run --project D: / path/ to/ project", chosen.text);
        assert_eq!(PixelRect::from_edges(361, 49, 773, 68), chosen.bounds);
    }

    #[test]
    fn 行が無ければまとめても無い() {
        assert!(merge_lines(&[]).is_empty());
    }

    #[test]
    fn 行の端から高さ1つ分の内なら選び外なら選ばない() {
        // 高さ 10 の行（x 100〜300、y 50〜60）
        let lines = [at("line", 100, 50, 200, 10)];
        let cases = [
            (90, 55, true),   // 左の端から 10（高さ 1 つ分ちょうど）
            (89, 55, false),  // 11
            (309, 55, true),  // 右の端（300）から 9
            (310, 55, false), // 10（右の端は含まないので外）
            (200, 45, true),  // 上の端から 5（高さの半分ちょうど）
            (200, 44, false), // 6
            (200, 64, true),  // 下の端（60）から 4
            (200, 65, false), // 5
        ];
        for (x, y, near) in cases {
            assert_eq!(
                near,
                select_near_line(&lines, PixelPoint::new(x, y)).is_some(),
                "({x}, {y})"
            );
        }
    }

    #[test]
    fn 遠くの行は選ばれない() {
        let lines = [at("far", 700, 50, 200, 10), at("below", 100, 120, 200, 10)];

        assert_eq!(None, select_near_line(&lines, PixelPoint::new(300, 55)));
    }

    #[test]
    fn 近くの候補が複数なら今の選び方で選ぶ() {
        let lines = [at("upper", 100, 40, 200, 10), at("lower", 100, 56, 200, 10)];

        assert_eq!(
            "upper",
            select_near_line(&lines, PixelPoint::new(150, 45))
                .unwrap()
                .text
        );
        assert_eq!(
            "lower",
            select_near_line(&lines, PixelPoint::new(150, 60))
                .unwrap()
                .text
        );
        // どちらにも含まれない間（y 50〜55）は縦に近い方
        assert_eq!(
            "lower",
            select_near_line(&lines, PixelPoint::new(150, 55))
                .unwrap()
                .text
        );
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
