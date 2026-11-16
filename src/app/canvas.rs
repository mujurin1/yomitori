//! 選択の枠・内側の塗りと、左上のプレビュー（Win+Q / Win+W 共通の見た目）。
//! 枠は 4 本の細い窓、塗りは範囲の大きさの窓 1 枚（窓全体の不透明度）、プレビューは小さな窓で描く。
//! どれもクリック透過・非アクティブ・Alt+Tab に出ない
//! （仮想スクリーン全体を覆う 1 枚の窓に描くと、マウスを動かすたびに画面全体を描き直すことになるため）

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
    BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CALCRECT, DT_NOPREFIX, DT_WORDBREAK, DeleteDC,
    DeleteObject, DrawTextW, FONT_WEIGHT, FW_NORMAL, FW_SEMIBOLD, GdiFlush, GetDC, GetTextFaceW,
    HDC, HFONT, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, LWA_ALPHA, RegisterClassW,
    SW_HIDE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos,
    SetWindowTextW, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::logic::frame;
use crate::logic::geometry::{PixelPoint, PixelRect};
use crate::logic::preview_style;
use crate::logic::region_selection::preview_placement;
use crate::platform::system;

/// 枠の色 #00B4FF（COLORREF は 0x00BBGGRR）
const FRAME_COLOR: COLORREF = COLORREF(0x00FF_B400);
const PREVIEW_MAX_WIDTH_DIP: f64 = 600.0;
const PREVIEW_PADDING_X_DIP: f64 = 6.0;
const PREVIEW_PADDING_Y_DIP: f64 = 3.0;
const HINT_OFFSET_DIP: f64 = 16.0;

const EDGE_CLASS: PCWSTR = w!("YomitoriFrameEdge");
const FILL_CLASS: PCWSTR = w!("YomitoriFrameFill");
const PREVIEW_CLASS: PCWSTR = w!("YomitoriPreview");

/// 窓のクラスを登録する（起動時に 1 回）
pub fn register_classes() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        for (class, brush) in [
            (EDGE_CLASS, Some(FRAME_COLOR)),
            (FILL_CLASS, Some(COLORREF(preview_style::FILL_COLORREF))),
            (PREVIEW_CLASS, None),
        ] {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(default_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                hbrBackground: brush.map(|c| CreateSolidBrush(c)).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }
    }
}

/// alpha が Some なら窓全体の不透明度で描く窓、None なら UpdateLayeredWindow で画素ごとの不透明度で描く窓
fn create_part(class: PCWSTR, alpha: Option<u8>) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            class,
            None,
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        system::disable_transitions(hwnd);
        // 窓全体の不透明度を決めた層付きの窓には UpdateLayeredWindow が効かないので、どちらか一方だけにする
        if let Some(alpha) = alpha {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        }
        Some(hwnd)
    }
}

fn place(hwnd: HWND, r: PixelRect) {
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            r.x,
            r.y,
            r.width,
            r.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
}

/// プレビューに描いてある中身。同じ文字・DPI なら描き直さずに動かすだけにする（Win+Q の追従を軽くする）
struct Rendered {
    text: String,
    dpi: u32,
    width: i32,
    height: i32,
}

pub struct Canvas {
    edges: Vec<HWND>,
    fill: Option<HWND>,
    preview: Option<HWND>,
    frame: Option<PixelRect>,
    preview_text: Option<String>,
    rendered: Option<Rendered>,
}

impl Canvas {
    /// 作った時点では何も出さない
    pub fn new() -> Self {
        Self {
            // 塗りを先に作る（後から出した枠が塗りの上に重なる）
            fill: create_part(FILL_CLASS, Some(preview_style::FILL_ALPHA)),
            edges: (0..4)
                .filter_map(|_| create_part(EDGE_CLASS, Some(255)))
                .collect(),
            preview: create_part(PREVIEW_CLASS, None),
            frame: None,
            preview_text: None,
            rendered: None,
        }
    }

    /// 窓を持たない（捨てた後の置き換え用）
    pub fn empty() -> Self {
        Self {
            edges: Vec::new(),
            fill: None,
            preview: None,
            frame: None,
            preview_text: None,
            rendered: None,
        }
    }

    /// 枠と内側の塗りを画面座標の矩形に出す
    pub fn set_frame(&mut self, rect: PixelRect) {
        self.frame = Some(rect);
        // 塗り → 枠 → プレビューの順に最前面へ置き直す（後に置いた方が上）
        if let Some(fill) = self.fill {
            place(fill, rect);
        }
        let thickness = frame::to_pixels(frame::THICKNESS_DIP, system::dpi_at(rect));
        for (hwnd, edge) in self.edges.iter().zip(frame::edges(rect, thickness)) {
            place(*hwnd, edge);
        }
        self.layout_preview();
    }

    /// プレビューの文字を出す（枠の左上に置く）
    pub fn set_preview(&mut self, text: &str) {
        self.preview_text = Some(text.into());
        if let Some(preview) = self.preview {
            let _ = unsafe { SetWindowTextW(preview, &HSTRING::from(text)) };
        }
        self.layout_preview();
    }

    pub fn hide_preview(&mut self) {
        self.preview_text = None;
        if let Some(preview) = self.preview {
            let _ = unsafe { ShowWindow(preview, SW_HIDE) };
        }
    }

    fn layout_preview(&mut self) {
        let (Some(rect), Some(text)) = (self.frame, self.preview_text.clone()) else {
            return;
        };
        let dpi = system::dpi_at(rect);
        let area_top = system::virtual_screen().top();
        self.show_preview(&text, dpi, |_, height| {
            let (x, y) = preview_placement::compute(
                f64::from(rect.left()),
                f64::from(rect.top()),
                f64::from(height),
                f64::from(area_top),
            );
            (x as i32, y as i32)
        });
    }

    /// 枠を出さずに、点の右下にプレビューと同じ見た目で text を出す（「文字なし」の知らせ）
    pub fn show_hint(&mut self, at: PixelPoint, text: &str) {
        self.preview_text = Some(text.into());
        if let Some(preview) = self.preview {
            let _ = unsafe { SetWindowTextW(preview, &HSTRING::from(text)) };
        }
        let dpi = system::dpi_at(PixelRect::new(at.x, at.y, 1, 1));
        // カーソルの絵と重ならないよう右下へずらす
        let offset = frame::to_pixels(HINT_OFFSET_DIP, dpi);
        self.show_preview(text, dpi, |_, _| (at.x + offset, at.y + offset));
    }

    /// プレビューを (幅, 高さ) から決めた位置に出す。中身が前と同じなら動かすだけ
    fn show_preview(
        &mut self,
        text: &str,
        dpi: u32,
        position: impl FnOnce(i32, i32) -> (i32, i32),
    ) {
        let Some(preview) = self.preview else {
            return;
        };
        if let Some(r) = self
            .rendered
            .as_ref()
            .filter(|r| r.text == text && r.dpi == dpi)
        {
            let (x, y) = position(r.width, r.height);
            let _ = unsafe {
                SetWindowPos(
                    preview,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
            return;
        }
        let font = create_font(text, dpi);
        let pad_x = frame::to_pixels(PREVIEW_PADDING_X_DIP, dpi);
        let pad_y = frame::to_pixels(PREVIEW_PADDING_Y_DIP, dpi);
        let (text_width, text_height) =
            measure(font, text, frame::to_pixels(PREVIEW_MAX_WIDTH_DIP, dpi));
        let (width, height) = (text_width + pad_x * 2, text_height + pad_y * 2);
        let (x, y) = position(width, height);
        draw_layered(
            preview,
            font,
            text,
            PixelRect::new(x, y, width, height),
            pad_x,
            pad_y,
            frame::to_pixels(preview_style::BORDER_DIP, dpi),
        );
        let _ = unsafe { DeleteObject(font.into()) };
        // UpdateLayeredWindow は Z 順を変えないので、枠より上へ置き直して見せる
        let _ = unsafe {
            SetWindowPos(
                preview,
                Some(HWND_TOPMOST),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        self.rendered = Some(Rendered {
            text: text.into(),
            dpi,
            width,
            height,
        });
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            for hwnd in self
                .edges
                .iter()
                .chain(self.fill.iter())
                .chain(self.preview.iter())
            {
                let _ = DestroyWindow(*hwnd);
            }
        }
    }
}

/// Semibold（無ければ同じ書体の Regular）。日本語を含むなら Yu Gothic UI、英字だけなら Segoe UI
fn create_font(text: &str, dpi: u32) -> HFONT {
    let (semibold, regular) = preview_style::font_faces(text);
    let font = font_with_face(semibold, FW_SEMIBOLD, dpi);
    if face_of(font).eq_ignore_ascii_case(semibold) {
        return font;
    }
    // 書体が無いと GDI は別の書体に置き換えるので、選ばれた書体名で見分けて Regular に落とす
    let _ = unsafe { DeleteObject(font.into()) };
    font_with_face(regular, FW_NORMAL, dpi)
}

fn font_with_face(face: &str, weight: FONT_WEIGHT, dpi: u32) -> HFONT {
    unsafe {
        CreateFontW(
            -frame::to_pixels(preview_style::FONT_DIP, dpi),
            0,
            0,
            0,
            weight.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            Default::default(),
            Default::default(),
            // ClearType は透明な背景の上で色がにじむので、グレースケールのアンチエイリアスにする
            ANTIALIASED_QUALITY,
            0,
            &HSTRING::from(face),
        )
    }
}

/// フォントを DC に選んだ時に実際に使われる書体名
fn face_of(font: HFONT) -> String {
    let mut name = [0u16; 64];
    unsafe {
        let dc = CreateCompatibleDC(None);
        let old = SelectObject(dc, font.into());
        GetTextFaceW(dc, Some(&mut name));
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    String::from_utf16_lossy(&name[..len])
}

/// 折り返しの幅 max_width で text を描いた時の大きさ
fn measure(font: HFONT, text: &str, max_width: i32) -> (i32, i32) {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    let mut r = RECT {
        left: 0,
        top: 0,
        right: max_width,
        bottom: 0,
    };
    unsafe {
        let dc = CreateCompatibleDC(None);
        let old = SelectObject(dc, font.into());
        DrawTextW(
            dc,
            &mut units,
            &mut r,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
    }
    (r.right - r.left, r.bottom - r.top)
}

/// 背景 #141414 に白で text を描いて文字の濃さを読み、#DCDCDC の文字・半透明の背景・縁の画素にして窓に当てる
/// 外周の border px は縁（#787878）にする
fn draw_layered(
    hwnd: HWND,
    font: HFONT,
    text: &str,
    r: PixelRect,
    pad_x: i32,
    pad_y: i32,
    border: i32,
) {
    if r.is_empty() {
        return;
    }
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r.width,
                biHeight: -r.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        if let Ok(dib) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            let previous = SelectObject(memory, dib.into());
            let len = r.width as usize * r.height as usize * 4;
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, len);
            let bg = preview_style::BACKGROUND_LEVEL;
            for pixel in pixels.as_chunks_mut::<4>().0 {
                *pixel = [bg, bg, bg, 0];
            }
            let old_font = SelectObject(memory, font.into());
            SetBkMode(memory, TRANSPARENT);
            SetTextColor(memory, COLORREF(0x00FF_FFFF));
            let mut units: Vec<u16> = text.encode_utf16().collect();
            let mut text_rect = RECT {
                left: pad_x,
                top: pad_y,
                right: r.width - pad_x,
                bottom: r.height - pad_y,
            };
            DrawTextW(
                memory,
                &mut units,
                &mut text_rect,
                DT_WORDBREAK | DT_NOPREFIX,
            );
            let _ = GdiFlush();
            SelectObject(memory, old_font);
            for (i, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                // 白い文字のアンチエイリアスは灰色になるので、一番明るい成分を文字の濃さとみなす
                let level = pixel[0].max(pixel[1]).max(pixel[2]);
                let (x, y) = (i as i32 % r.width, i as i32 / r.width);
                *pixel = preview_style::pixel(x, y, r.width, r.height, border, level);
            }
            update_layered(hwnd, screen, memory, r);
            SelectObject(memory, previous);
            let _ = DeleteObject(dib.into());
        }
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
    }
}

fn update_layered(hwnd: HWND, screen: HDC, memory: HDC, r: PixelRect) {
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = unsafe {
        UpdateLayeredWindow(
            hwnd,
            Some(screen),
            Some(&POINT { x: r.x, y: r.y }),
            Some(&SIZE {
                cx: r.width,
                cy: r.height,
            }),
            Some(memory),
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    };
}

unsafe extern "system" fn default_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
