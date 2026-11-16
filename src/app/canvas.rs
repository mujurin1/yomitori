//! 選択の枠と左上のプレビュー（Win+Q / Win+W 共通の見た目）。
//! 枠は 4 本の細い窓、プレビューは小さな窓で描く。どれもクリック透過・非アクティブ・Alt+Tab に出ない
//! （仮想スクリーン全体を覆う 1 枚の窓に描くと、マウスを動かすたびに画面全体を描き直すことになるため）

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DT_CALCRECT,
    DT_NOPREFIX, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint, FONT_QUALITY,
    FW_NORMAL, FillRect, HBRUSH, HFONT, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW, HWND_TOPMOST,
    LWA_ALPHA, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, WM_PAINT, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::logic::frame;
use crate::logic::geometry::{PixelPoint, PixelRect};
use crate::logic::region_selection::preview_placement;
use crate::platform::system;

/// 枠の色 #00B4FF（COLORREF は 0x00BBGGRR）
const FRAME_COLOR: COLORREF = COLORREF(0x00FF_B400);
/// プレビューの背景 #202020。窓全体の不透明度を 0xE0 にする（C# 版の背景 #E0202020 に合わせる）
const PREVIEW_BACKGROUND: COLORREF = COLORREF(0x0020_2020);
const PREVIEW_ALPHA: u8 = 0xE0;
const PREVIEW_FONT_DIP: f64 = 14.0;
const PREVIEW_MAX_WIDTH_DIP: f64 = 600.0;
const PREVIEW_PADDING_X_DIP: f64 = 6.0;
const PREVIEW_PADDING_Y_DIP: f64 = 3.0;
const HINT_OFFSET_DIP: f64 = 16.0;

const EDGE_CLASS: PCWSTR = w!("YomitoriFrameEdge");
const PREVIEW_CLASS: PCWSTR = w!("YomitoriPreview");

/// 2 つの窓のクラスを登録する（起動時に 1 回）
pub fn register_classes() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let edge = WNDCLASSW {
            lpfnWndProc: Some(edge_proc),
            hInstance: instance.into(),
            lpszClassName: EDGE_CLASS,
            hbrBackground: CreateSolidBrush(FRAME_COLOR),
            ..Default::default()
        };
        RegisterClassW(&edge);
        let preview = WNDCLASSW {
            lpfnWndProc: Some(preview_proc),
            hInstance: instance.into(),
            lpszClassName: PREVIEW_CLASS,
            ..Default::default()
        };
        RegisterClassW(&preview);
    }
}

fn create_part(class: PCWSTR, alpha: u8) -> Option<HWND> {
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
        // 層付きの窓は不透明度を決めるまで描かれない
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
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

pub struct Canvas {
    edges: Vec<HWND>,
    preview: Option<HWND>,
    font: Option<(u32, HFONT)>,
    frame: Option<PixelRect>,
    preview_text: Option<String>,
}

impl Canvas {
    /// 作った時点では何も出さない
    pub fn new() -> Self {
        Self {
            edges: (0..4)
                .filter_map(|_| create_part(EDGE_CLASS, 255))
                .collect(),
            preview: create_part(PREVIEW_CLASS, PREVIEW_ALPHA),
            font: None,
            frame: None,
            preview_text: None,
        }
    }

    /// 窓を持たない（捨てた後の置き換え用）
    pub fn empty() -> Self {
        Self {
            edges: Vec::new(),
            preview: None,
            font: None,
            frame: None,
            preview_text: None,
        }
    }

    /// 枠を画面座標の矩形に出す
    pub fn set_frame(&mut self, rect: PixelRect) {
        self.frame = Some(rect);
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

    fn font_for(&mut self, dpi: u32) -> HFONT {
        if let Some((font_dpi, font)) = self.font
            && font_dpi == dpi
        {
            return font;
        }
        if let Some((_, old)) = self.font.take() {
            let _ = unsafe { DeleteObject(old.into()) };
        }
        let font = create_font(frame::to_pixels(PREVIEW_FONT_DIP, dpi));
        self.font = Some((dpi, font));
        font
    }

    fn layout_preview(&mut self) {
        let (Some(rect), Some(text), Some(preview)) =
            (self.frame, self.preview_text.clone(), self.preview)
        else {
            return;
        };
        let dpi = system::dpi_at(rect);
        let (width, height) = self.prepare_preview(preview, &text, dpi);
        let area_top = system::virtual_screen().top();
        let (x, y) = preview_placement::compute(
            f64::from(rect.left()),
            f64::from(rect.top()),
            f64::from(height),
            f64::from(area_top),
        );
        place(preview, PixelRect::new(x as i32, y as i32, width, height));
        let _ = unsafe { InvalidateRect(Some(preview), None, true) };
    }

    /// プレビューの窓のフォントを dpi に合わせ、text を描く大きさ（余白込み）を返す
    fn prepare_preview(&mut self, preview: HWND, text: &str, dpi: u32) -> (i32, i32) {
        let font = self.font_for(dpi);
        unsafe { SetWindowLongPtrW(preview, GWLP_USERDATA, font.0 as isize) };
        let pad_x = frame::to_pixels(PREVIEW_PADDING_X_DIP, dpi);
        let pad_y = frame::to_pixels(PREVIEW_PADDING_Y_DIP, dpi);
        let (text_width, text_height) =
            measure(font, text, frame::to_pixels(PREVIEW_MAX_WIDTH_DIP, dpi));
        (text_width + pad_x * 2, text_height + pad_y * 2)
    }

    /// 枠を出さずに、点の右下にプレビューと同じ見た目で text を出す（「文字なし」の知らせ）
    pub fn show_hint(&mut self, at: PixelPoint, text: &str) {
        let Some(preview) = self.preview else {
            return;
        };
        self.preview_text = Some(text.into());
        let _ = unsafe { SetWindowTextW(preview, &HSTRING::from(text)) };
        let dpi = system::dpi_at(PixelRect::new(at.x, at.y, 1, 1));
        let (width, height) = self.prepare_preview(preview, text, dpi);
        // カーソルの絵と重ならないよう右下へずらす
        let offset = frame::to_pixels(HINT_OFFSET_DIP, dpi);
        place(
            preview,
            PixelRect::new(at.x + offset, at.y + offset, width, height),
        );
        let _ = unsafe { InvalidateRect(Some(preview), None, true) };
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            for hwnd in self.edges.iter().chain(self.preview.iter()) {
                let _ = DestroyWindow(*hwnd);
            }
            if let Some((_, font)) = self.font.take() {
                let _ = DeleteObject(font.into());
            }
        }
    }
}

fn create_font(height: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            Default::default(),
            Default::default(),
            FONT_QUALITY(0),
            0,
            w!("Yu Gothic UI"),
        )
    }
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

unsafe extern "system" fn edge_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe extern "system" fn preview_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg != WM_PAINT {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
        let brush: HBRUSH = CreateSolidBrush(PREVIEW_BACKGROUND);
        FillRect(dc, &client, brush);
        let _ = DeleteObject(brush.into());

        let font = HFONT(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as _);
        let old = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96);
        let pad_x = frame::to_pixels(PREVIEW_PADDING_X_DIP, dpi);
        let pad_y = frame::to_pixels(PREVIEW_PADDING_Y_DIP, dpi);
        let mut text_rect = RECT {
            left: pad_x,
            top: pad_y,
            right: client.right - pad_x,
            bottom: client.bottom - pad_y,
        };
        let mut buffer = [0u16; 4096];
        let len =
            windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut buffer) as usize;
        DrawTextW(
            dc,
            &mut buffer[..len],
            &mut text_rect,
            DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        let _ = EndPaint(hwnd, &ps);
    }
    LRESULT(0)
}
