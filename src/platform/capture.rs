use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
};

use crate::logic::geometry::PixelRect;
pub use crate::logic::image::BgraImage;

/// 画面の矩形（物理ピクセル）を写す。アルファは 255（不透明）にする
pub fn capture_screen(rect: PixelRect) -> Result<BgraImage, String> {
    if rect.is_empty() {
        return Err("写す範囲が空です".into());
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("画面の DC を取れませんでした".into());
        }
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rect.width,
                // 負の高さで上から順の並びにする
                biHeight: -rect.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let result = match CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        {
            Ok(dib) => {
                let previous = SelectObject(memory, dib.into());
                let copied = BitBlt(
                    memory,
                    0,
                    0,
                    rect.width,
                    rect.height,
                    Some(screen),
                    rect.x,
                    rect.y,
                    SRCCOPY,
                );
                let image = copied
                    .map_err(|e| format!("画面を写せませんでした: {}", e.message()))
                    .map(|()| {
                        let len = rect.width as usize * rect.height as usize * 4;
                        let mut pixels =
                            std::slice::from_raw_parts(bits as *const u8, len).to_vec();
                        for pixel in pixels.as_chunks_mut::<4>().0 {
                            pixel[3] = 255;
                        }
                        BgraImage {
                            width: rect.width,
                            height: rect.height,
                            pixels,
                        }
                    });
                SelectObject(memory, previous);
                let _ = DeleteObject(dib.into());
                image
            }
            Err(e) => Err(format!("画像の領域を作れませんでした: {}", e.message())),
        };
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        result
    }
}
