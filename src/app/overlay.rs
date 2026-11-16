//! Win+Q の範囲指定。画面は暗くせず、見た目は透明な受け皿で仮想スクリーン全体の入力を受ける
//! （クリックを下のアプリに渡さないため）。起点は Win+Q を押した瞬間のカーソル位置

use std::sync::Arc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, IDC_CROSS, LWA_ALPHA,
    LoadCursorW, MA_NOACTIVATE, RegisterClassW, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetWindowPos, WM_CONTEXTMENU, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::canvas::Canvas;
use crate::logic::geometry::PixelRect;
use crate::logic::image::{self, BgraImage};
use crate::logic::latest_only::LatestOnlyRunner;
use crate::logic::ocr_support;
use crate::logic::region_selection::{RegionSelection, region_preview};
use crate::platform::ocr::OcrService;
use crate::platform::system;

const CATCHER_CLASS: PCWSTR = w!("YomitoriCatcher");

/// 受け皿の不透明度。完全な透明（0）だとマウスが下のウィンドウへ抜けるので、見えない程度の 1 にする
const CATCHER_ALPHA: u8 = 1;

/// 開いた瞬間の画面。プレビューと確定の OCR はこれを切り出して使う（枠やプレビューが OCR に写らないように）
pub struct Snapshot {
    pub image: BgraImage,
    pub bounds: PixelRect,
}

impl Snapshot {
    pub fn crop(&self, rect: PixelRect) -> Option<BgraImage> {
        image::crop(&self.image, self.bounds, rect)
    }
}

/// 受け皿のクラスを登録する（起動時に 1 回）。wndproc は受け皿のマウスを App へ渡す
pub fn register_class(wndproc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT) {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CATCHER_CLASS,
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

/// 受け皿の wndproc の既定の部分。マウスのメッセージ以外はここで済ませる。App に渡すべきなら None
pub fn catcher_default(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        // アクティブにならない（前面のウィンドウのフォーカスを残す）
        WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
        // 右ボタンは平行移動にだけ使い、メニューを出さない
        WM_CONTEXTMENU => Some(LRESULT(0)),
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP => None,
        _ => Some(unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }),
    }
}

pub struct RegionOverlay {
    pub id: u64,
    catcher: Option<HWND>,
    canvas: Canvas,
    selection: RegionSelection,
    snapshot: Arc<Snapshot>,
    runner: LatestOnlyRunner<PixelRect, String, String>,
    last_submitted: Option<PixelRect>,
    left_pressed: bool,
    finished: bool,
}

impl RegionOverlay {
    /// 受け皿を出す。プレビューの OCR の結果は on_preview_text（OCR のスレッドから呼ばれる）で届く
    pub fn open(
        id: u64,
        snapshot: Arc<Snapshot>,
        ocr: Arc<OcrService>,
        keep_line_breaks: bool,
        on_preview_text: impl Fn(String) + Send + Sync + 'static,
    ) -> Option<Self> {
        let screen = snapshot.bounds;
        let start = system::cursor_position();
        let catcher = unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                CATCHER_CLASS,
                None,
                WS_POPUP,
                screen.x,
                screen.y,
                screen.width,
                screen.height,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ok()?;
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), CATCHER_ALPHA, LWA_ALPHA);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                screen.x,
                screen.y,
                screen.width,
                screen.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            hwnd
        };

        let work_snapshot = Arc::clone(&snapshot);
        let on_preview_text = Arc::new(on_preview_text);
        let on_failed = Arc::clone(&on_preview_text);
        let runner = LatestOnlyRunner::new(
            move |rect: &PixelRect| -> Result<String, String> {
                let Some(image) = work_snapshot.crop(*rect) else {
                    return Ok(String::new());
                };
                let lines = ocr.recognize_blocking(image)?;
                Ok(ocr_support::join_ocr_lines(
                    lines.iter().map(|l| &l.text),
                    keep_line_breaks,
                ))
            },
            move |_, text| on_preview_text(text),
            move |_, error| on_failed(error),
        );

        let mut overlay = Self {
            id,
            catcher: Some(catcher),
            canvas: Canvas::new(),
            selection: RegionSelection::new(start),
            snapshot,
            runner,
            last_submitted: None,
            left_pressed: false,
            finished: false,
        };
        overlay.canvas.set_frame(overlay.selection.rect());
        Some(overlay)
    }

    pub fn snapshot(&self) -> &Arc<Snapshot> {
        &self.snapshot
    }

    /// 受け皿のマウス。確定か中止で終わったら Some（確定した矩形、中止なら内側が None）
    pub fn on_mouse(&mut self, msg: u32, wparam: WPARAM) -> Option<Option<PixelRect>> {
        if self.finished {
            return None;
        }
        const MK_RBUTTON: usize = 0x0002;
        match msg {
            WM_MOUSEMOVE => self.on_pointer_changed(wparam.0 & MK_RBUTTON != 0),
            WM_RBUTTONDOWN => self.on_pointer_changed(true),
            WM_RBUTTONUP => self.on_pointer_changed(false),
            // 押しは受け皿で受け、離しで確定する（離しが下のアプリへ漏れないように）
            WM_LBUTTONDOWN => {
                self.left_pressed = true;
                if let Some(catcher) = self.catcher {
                    unsafe { SetCapture(catcher) };
                }
            }
            WM_LBUTTONUP if self.left_pressed => {
                let result = self.selection.click(system::cursor_position());
                self.finish();
                return Some(result);
            }
            _ => {}
        }
        None
    }

    fn on_pointer_changed(&mut self, right_button_down: bool) {
        self.selection
            .move_to(system::cursor_position(), right_button_down);
        self.canvas.set_frame(self.selection.rect());

        if !self.selection.is_usable() {
            // 範囲が小さすぎる間は前のプレビューも残さない
            self.canvas.hide_preview();
            // 同じ範囲に戻した時に OCR し直してプレビューを出すため
            self.last_submitted = None;
            return;
        }
        let rect = self.selection.rect();
        if self.last_submitted != Some(rect) {
            self.last_submitted = Some(rect);
            self.runner.submit(rect);
        }
    }

    /// プレビューの OCR の結果（UI スレッドで呼ぶ）
    pub fn on_preview(&mut self, text: &str) {
        if self.finished {
            return;
        }
        // 文字なし・結果が届いた時には範囲が小さくなっていた、なら出さない
        match region_preview::text_to_show(text, self.selection.is_usable()) {
            Some(shown) => self.canvas.set_preview(shown),
            None => self.canvas.hide_preview(),
        }
    }

    /// 中止する（フックで拾った Esc から呼ぶ）
    pub fn cancel(&mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.runner.stop();
        unsafe {
            let _ = ReleaseCapture();
            if let Some(catcher) = self.catcher.take() {
                let _ = DestroyWindow(catcher);
            }
        }
        self.canvas = Canvas::empty();
    }
}

impl Drop for RegionOverlay {
    fn drop(&mut self) {
        self.finish();
    }
}
