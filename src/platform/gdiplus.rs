//! GDI+ の開始と終了、拡大。トレイのアイコンと OCR 前の拡大を C# 版（System.Drawing = GDI+）と同じ描き方にするため

use windows::Win32::Graphics::GdiPlus::{
    BitmapData, GdipBitmapLockBits, GdipBitmapUnlockBits, GdipCreateBitmapFromScan0,
    GdipDeleteGraphics, GdipDisposeImage, GdipDrawImageRectI, GdipGetImageGraphicsContext,
    GdipSetInterpolationMode, GdipSetPixelOffsetMode, GdiplusShutdown, GdiplusStartup,
    GdiplusStartupInput, GpBitmap, GpGraphics, GpImage, ImageLockModeRead,
    InterpolationModeHighQualityBicubic, InterpolationModeNearestNeighbor,
    PixelOffsetModeHighQuality, Rect, Status,
};

use super::capture::BgraImage;

// GDI+ の PixelFormat。windows クレートの生成物に無いので Gdiplus のヘッダーの値を書く
pub const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;
pub const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;

/// GDI+ を使う間だけ持つ。複数あってもよい（GDI+ 側で数える）
pub struct GdiPlus(usize);

impl GdiPlus {
    pub fn start() -> Result<Self, String> {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        check(
            unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) },
            "GdiplusStartup",
        )?;
        Ok(Self(token))
    }
}

impl Drop for GdiPlus {
    fn drop(&mut self) {
        unsafe { GdiplusShutdown(self.0) };
    }
}

pub fn check(status: Status, what: &str) -> Result<(), String> {
    if status.0 == 0 {
        Ok(())
    } else {
        Err(format!("{what} が失敗しました（GDI+ {}）", status.0))
    }
}

/// 確保した GDI+ の画像。drop で捨てる
pub struct Bitmap(pub *mut GpBitmap);

impl Bitmap {
    pub fn new(width: i32, height: i32, format: i32) -> Result<Self, String> {
        let mut bitmap = std::ptr::null_mut();
        check(
            unsafe { GdipCreateBitmapFromScan0(width, height, 0, format, None, &mut bitmap) },
            "GdipCreateBitmapFromScan0",
        )?;
        Ok(Self(bitmap))
    }

    pub fn image(&self) -> *mut GpImage {
        self.0.cast()
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { GdipDisposeImage(self.image()) };
    }
}

/// 画像に描く Graphics。drop で捨てる
pub struct Graphics(pub *mut GpGraphics);

impl Graphics {
    pub fn of(bitmap: &Bitmap) -> Result<Self, String> {
        let mut graphics = std::ptr::null_mut();
        check(
            unsafe { GdipGetImageGraphicsContext(bitmap.image(), &mut graphics) },
            "GdipGetImageGraphicsContext",
        )?;
        Ok(Self(graphics))
    }
}

impl Drop for Graphics {
    fn drop(&mut self) {
        unsafe { GdipDeleteGraphics(self.0) };
    }
}

/// 整数倍に拡大した premultiplied BGRA。scale が 1 なら補間せずに写す（C# 版と同じ）
pub fn scale_image(image: &BgraImage, scale: i32) -> Result<BgraImage, String> {
    let width = image.width * scale;
    let height = image.height * scale;
    let mut source = std::ptr::null_mut();
    // scan0 を渡した画像は image.pixels を参照し続けるので、この関数の中で捨てる
    check(
        unsafe {
            GdipCreateBitmapFromScan0(
                image.width,
                image.height,
                image.width * 4,
                PIXEL_FORMAT_32BPP_ARGB,
                Some(image.pixels.as_ptr()),
                &mut source,
            )
        },
        "GdipCreateBitmapFromScan0",
    )?;
    let source = Bitmap(source);
    let target = Bitmap::new(width, height, PIXEL_FORMAT_32BPP_PARGB)?;
    {
        let g = Graphics::of(&target)?;
        let mode = if scale > 1 {
            InterpolationModeHighQualityBicubic
        } else {
            InterpolationModeNearestNeighbor
        };
        unsafe {
            check(
                GdipSetInterpolationMode(g.0, mode),
                "GdipSetInterpolationMode",
            )?;
            check(
                GdipSetPixelOffsetMode(g.0, PixelOffsetModeHighQuality),
                "GdipSetPixelOffsetMode",
            )?;
            check(
                GdipDrawImageRectI(g.0, source.image(), 0, 0, width, height),
                "GdipDrawImageRectI",
            )?;
        }
    }
    drop(source);
    read_pixels(&target, width, height, PIXEL_FORMAT_32BPP_PARGB)
}

/// 画像の画素を上から順の BGRA（stride = 幅 × 4）で読む
fn read_pixels(bitmap: &Bitmap, width: i32, height: i32, format: i32) -> Result<BgraImage, String> {
    let rect = Rect {
        X: 0,
        Y: 0,
        Width: width,
        Height: height,
    };
    let mut data = BitmapData::default();
    check(
        unsafe {
            GdipBitmapLockBits(
                bitmap.0,
                &rect,
                ImageLockModeRead.0 as u32,
                format,
                &mut data,
            )
        },
        "GdipBitmapLockBits",
    )?;
    let row = width as usize * 4;
    let mut pixels = vec![0u8; row * height as usize];
    for (y, dest) in pixels.chunks_exact_mut(row).enumerate() {
        let src = unsafe { (data.Scan0 as *const u8).offset(data.Stride as isize * y as isize) };
        unsafe { std::ptr::copy_nonoverlapping(src, dest.as_mut_ptr(), row) };
    }
    unsafe { GdipBitmapUnlockBits(bitmap.0, &mut data) };
    Ok(BgraImage {
        width,
        height,
        pixels,
    })
}
