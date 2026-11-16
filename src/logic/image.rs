use super::geometry::PixelRect;

/// 上から順の BGRA（stride = 幅 × 4）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BgraImage {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

/// 画面の一部を写した画像（bounds は写した範囲の画面座標）から、画面座標の矩形を切り出す。
/// 範囲外は切り捨て、何も残らなければ None
pub fn crop(image: &BgraImage, bounds: PixelRect, screen_rect: PixelRect) -> Option<BgraImage> {
    let r = screen_rect.intersect(bounds);
    if r.is_empty() {
        return None;
    }
    let left = (r.x - bounds.x) as usize;
    let top = (r.y - bounds.y) as usize;
    let row = r.width as usize * 4;
    let stride = image.width as usize * 4;
    let mut pixels = Vec::with_capacity(row * r.height as usize);
    for y in top..top + r.height as usize {
        let start = y * stride + left * 4;
        pixels.extend_from_slice(&image.pixels[start..start + row]);
    }
    Some(BgraImage {
        width: r.width,
        height: r.height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 各画素の B に x、G に y を入れた画像
    fn numbered(width: i32, height: i32) -> BgraImage {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        BgraImage {
            width,
            height,
            pixels,
        }
    }

    fn at(image: &BgraImage, x: i32, y: i32) -> (u8, u8) {
        let i = ((y * image.width + x) * 4) as usize;
        (image.pixels[i], image.pixels[i + 1])
    }

    #[test]
    fn 画面座標の矩形を写した範囲の原点に合わせて切り出す() {
        let image = numbered(10, 8);
        let bounds = PixelRect::new(-100, 50, 10, 8);

        let cropped = crop(&image, bounds, PixelRect::new(-97, 52, 4, 3)).unwrap();

        assert_eq!((4, 3), (cropped.width, cropped.height));
        assert_eq!((3, 2), at(&cropped, 0, 0));
        assert_eq!((6, 4), at(&cropped, 3, 2));
        assert_eq!(4 * 3 * 4, cropped.pixels.len());
    }

    #[test]
    fn 写した範囲からはみ出した部分は切り捨てる() {
        let image = numbered(10, 8);
        let bounds = PixelRect::new(0, 0, 10, 8);

        let cropped = crop(&image, bounds, PixelRect::new(7, -5, 10, 7)).unwrap();

        assert_eq!((3, 2), (cropped.width, cropped.height));
        assert_eq!((7, 0), at(&cropped, 0, 0));
        assert_eq!((9, 1), at(&cropped, 2, 1));
    }

    #[test]
    fn 重ならなければnone() {
        let image = numbered(10, 8);

        assert_eq!(
            None,
            crop(
                &image,
                PixelRect::new(0, 0, 10, 8),
                PixelRect::new(10, 0, 5, 5)
            )
        );
        assert_eq!(
            None,
            crop(
                &image,
                PixelRect::new(0, 0, 10, 8),
                PixelRect::new(2, 2, 0, 5)
            )
        );
    }
}
