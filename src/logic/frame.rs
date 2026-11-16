//! Win+Q / Win+W の枠の線の置き場所

use super::geometry::PixelRect;

/// 線の太さ（DIP）。表示倍率を掛けて物理ピクセルにする
pub const THICKNESS_DIP: f64 = 2.0;

/// 枠の線の矩形（上・下・左・右）。線は矩形の内側に引く。
/// 大きさ 0 でも起点が分かるよう、幅と高さは最小でも線の太さ分とる
pub fn edges(rect: PixelRect, thickness: i32) -> [PixelRect; 4] {
    let t = thickness.max(1);
    let w = rect.width.max(t);
    let h = rect.height.max(t);
    let (x, y) = (rect.x, rect.y);
    [
        PixelRect::new(x, y, w, t),
        PixelRect::new(x, y + h - t, w, t),
        PixelRect::new(x, y, t, h),
        PixelRect::new(x + w - t, y, t, h),
    ]
}

/// DIP の長さを表示倍率（dpi / 96）で物理ピクセルにする。最小 1
pub fn to_pixels(dip: f64, dpi: u32) -> i32 {
    ((dip * f64::from(dpi) / 96.0).round() as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::geometry::PixelPoint;

    #[test]
    fn 線は矩形の内側に引き四辺で矩形を囲む() {
        let rect = PixelRect::new(10, 20, 100, 50);

        let [top, bottom, left, right] = edges(rect, 2);

        assert_eq!(PixelRect::new(10, 20, 100, 2), top);
        assert_eq!(PixelRect::new(10, 68, 100, 2), bottom);
        assert_eq!(PixelRect::new(10, 20, 2, 50), left);
        assert_eq!(PixelRect::new(108, 20, 2, 50), right);
        for edge in [top, bottom, left, right] {
            assert_eq!(edge, edge.intersect(rect), "{edge:?} が矩形の外に出ている");
        }
    }

    #[test]
    fn 大きさ0の矩形でも起点に線の太さ分の点を描く() {
        let rect = PixelRect::from_corners(PixelPoint::new(5, 5), PixelPoint::new(5, 5));

        for edge in edges(rect, 3) {
            assert_eq!(PixelRect::new(5, 5, 3, 3), edge);
        }
    }

    #[test]
    fn dipは表示倍率で物理ピクセルにする() {
        for (dpi, expected) in [(96, 2), (120, 3), (144, 3), (192, 4)] {
            assert_eq!(expected, to_pixels(THICKNESS_DIP, dpi), "dpi {dpi}");
        }
        assert_eq!(1, to_pixels(0.1, 96));
    }
}
