//! OCR 文字列（編集可）と訳文。1 つを使い回し、閉じても隠すだけ（位置・大きさを保つ）。
//! 出す時もフォーカスを取らない（user が使っているウィンドウから勝手にフォーカスを奪わない）。
//! Topmost が既定でオンなので、フォーカスが無くても見える

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DeleteObject, FONT_QUALITY, FW_NORMAL, HBRUSH,
    HDC, HFONT, SetBkColor,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, CheckDlgButton, IsDlgButtonChecked};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CW_USEDEFAULT, CreateWindowExW,
    DefWindowProcW, DestroyWindow, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, ES_WANTRETURN,
    GetClientRect, GetDlgItem, GetWindowTextLengthW, GetWindowTextW, HMENU, HWND_NOTOPMOST,
    HWND_TOPMOST, ICON_BIG, ICON_SMALL, IDCANCEL, IDOK, IsDialogMessageW, IsIconic,
    IsWindowVisible, MINMAXINFO, MSG, MoveWindow, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowPos,
    SetWindowTextW, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_GETMINMAXINFO, WM_SETFONT, WM_SETICON, WM_SIZE, WM_USER,
    WM_WINDOWPOSCHANGED, WNDCLASSW, WS_CHILD, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_TOPMOST,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::{UiPoster, repost, with_app};
use crate::logic::frame;
use crate::logic::geometry::PixelRect;
use crate::logic::result_layout;
use crate::logic::settings::ResultWindowPlacement;
use crate::logic::translation::{
    LatestTranslation, PendingTranslation, TranslationOutcome, Translator,
};
use crate::logic::window_placement::{self, PlacementTracker};
use crate::platform::icon::{self, AppIcon};
use crate::platform::system;

const CLASS: PCWSTR = w!("YomitoriResult");
const ID_RETRANSLATE: i32 = 1001;
const ID_TOPMOST: i32 = 1002;
const ID_SOURCE: i32 = 1003;
const ID_TRANSLATION: i32 = 1004;
const ID_OK: i32 = IDOK.0;

const EDIT_FONT_DIP: f64 = 14.0;
const BUTTON_FONT_DIP: f64 = 12.0;
/// 下段（読み取り専用）の背景 #F6F6F6
const TRANSLATION_BACKGROUND: COLORREF = COLORREF(0x00F6_F6F6);
const DM_GETDEFID: u32 = WM_USER;
const DC_HASDEFID: usize = 0x534B;

const SOURCE_LANGUAGE: &str = "en";
const TARGET_LANGUAGE: &str = "ja";

static RESULT_HWND: AtomicIsize = AtomicIsize::new(0);

struct Resources {
    dpi: u32,
    edit_font: HFONT,
    button_font: HFONT,
    translation_brush: HBRUSH,
    _small_icon: Option<AppIcon>,
    _large_icon: Option<AppIcon>,
}

thread_local! {
    /// 最後に見えていた時の位置と大きさ。終了時にはウィンドウを読まず、これを保存する。
    /// App を借りている最中の SetWindowPos からも届くので、App の外に置く
    static TRACKER: RefCell<PlacementTracker> = RefCell::new(PlacementTracker::new());
    static RESOURCES: RefCell<Option<Resources>> = const { RefCell::new(None) };
}

/// 結果ウィンドウ宛てのメッセージなら Tab の移動と既定のボタン（OK）を処理して true
pub fn is_dialog_message(msg: &MSG) -> bool {
    let hwnd = RESULT_HWND.load(Ordering::Relaxed);
    hwnd != 0 && unsafe { IsDialogMessageW(HWND(hwnd as _), msg) }.as_bool()
}

fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

pub struct ResultWindow {
    hwnd: HWND,
    poster: UiPoster,
    translation: LatestTranslation,
    /// 次に出す時に当てる位置と大きさ（起動時・設定の再読み込みで入る）
    pending_placement: Option<ResultWindowPlacement>,
}

impl ResultWindow {
    pub fn create(
        translator: Arc<dyn Translator>,
        poster: UiPoster,
    ) -> windows::core::Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                hbrBackground: HBRUSH((windows::Win32::Graphics::Gdi::COLOR_BTNFACE.0 + 1) as _),
                hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                    None,
                    windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
                )
                .unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_CONTROLPARENT,
                CLASS,
                w!("yomitori"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                600,
                200,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let child = |ex: WINDOW_EX_STYLE, class: PCWSTR, text: PCWSTR, style: u32, id: i32| {
                CreateWindowExW(
                    ex,
                    class,
                    text,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(style),
                    0,
                    0,
                    10,
                    10,
                    Some(hwnd),
                    Some(HMENU(id as _)),
                    Some(instance.into()),
                    None,
                )
            };
            let edit_style = WS_VSCROLL.0 | (ES_MULTILINE | ES_AUTOVSCROLL) as u32;
            child(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                w!(""),
                edit_style | ES_WANTRETURN as u32,
                ID_SOURCE,
            )?;
            child(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                w!(""),
                edit_style | ES_READONLY as u32,
                ID_TRANSLATION,
            )?;
            child(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                w!("Topmost"),
                BS_AUTOCHECKBOX as u32,
                ID_TOPMOST,
            )?;
            child(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                w!("Re-translate"),
                BS_PUSHBUTTON as u32,
                ID_RETRANSLATE,
            )?;
            child(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                w!("OK"),
                BS_DEFPUSHBUTTON as u32,
                ID_OK,
            )?;
            let _ = CheckDlgButton(hwnd, ID_TOPMOST, BST_CHECKED);
            apply_resources(hwnd);
            layout(hwnd);
            RESULT_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
            Ok(Self {
                hwnd,
                poster,
                translation: LatestTranslation::new(translator, SOURCE_LANGUAGE, TARGET_LANGUAGE),
                pending_placement: None,
            })
        }
    }

    fn item(&self, id: i32) -> HWND {
        unsafe { GetDlgItem(Some(self.hwnd), id) }.unwrap_or_default()
    }

    fn set_item_text(&self, id: i32, text: &str) {
        let _ = unsafe { SetWindowTextW(self.item(id), &HSTRING::from(to_crlf(text))) };
    }

    fn item_text(&self, id: i32) -> String {
        let item = self.item(id);
        let len = unsafe { GetWindowTextLengthW(item) }.max(0) as usize;
        let mut buffer = vec![0u16; len + 1];
        let copied = unsafe { GetWindowTextW(item, &mut buffer) }.max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied])
    }

    /// OCR 結果を出してすぐ翻訳する（Win+Q 用）
    pub fn show_text(&mut self, text: &str) {
        self.set_item_text(ID_SOURCE, text);
        self.present();
        if text.is_empty() {
            self.show_message("文字が見つかりません", "");
            return;
        }
        self.translate(text);
    }

    /// 先に翻訳を始める（Win+W 用）。訳（または失敗のエラー）が届いたら deliver_then_show で出す
    pub fn start_translate_then_show(&mut self, text: &str) -> PendingTranslation {
        self.translation.start(text)
    }

    /// 届く前に別の翻訳に置き換えられていたら出さない。出したら true
    pub fn deliver_then_show(
        &mut self,
        pending: &PendingTranslation,
        text: &str,
        outcome: TranslationOutcome,
    ) -> bool {
        if pending.is_superseded() {
            return false;
        }
        let translation = match outcome {
            TranslationOutcome::Success(t) => t,
            TranslationOutcome::Failure(m) => format!("翻訳に失敗しました: {m}"),
            TranslationOutcome::Superseded => return false,
        };
        self.set_item_text(ID_SOURCE, text);
        self.set_item_text(ID_TRANSLATION, &translation);
        self.present();
        true
    }

    /// 翻訳せずに下段へ知らせを出す（OCR が使えない等）。source が空でなければ上段も差し替える
    pub fn show_message(&mut self, message: &str, source: &str) {
        if !source.is_empty() {
            self.set_item_text(ID_SOURCE, source);
        }
        self.translation.cancel();
        self.set_item_text(ID_TRANSLATION, message);
        self.present();
    }

    fn translate(&mut self, text: &str) {
        self.set_item_text(ID_TRANSLATION, "翻訳中…");
        let pending = self.translation.start(text);
        let poster = self.poster;
        let spawned = thread::Builder::new()
            .name("yomitori translation".into())
            .spawn(move || {
                let outcome = pending.run();
                poster.post(move |app| app.result.deliver_translation(&pending, outcome));
            });
        if let Err(e) = spawned {
            self.set_item_text(ID_TRANSLATION, &format!("翻訳に失敗しました: {e}"));
        }
    }

    fn deliver_translation(&mut self, pending: &PendingTranslation, outcome: TranslationOutcome) {
        if pending.is_superseded() {
            return;
        }
        match outcome {
            TranslationOutcome::Success(t) => self.set_item_text(ID_TRANSLATION, &t),
            TranslationOutcome::Failure(m) => {
                self.set_item_text(ID_TRANSLATION, &format!("翻訳に失敗しました: {m}"))
            }
            TranslationOutcome::Superseded => {}
        }
    }

    /// 次に出す時から使う位置と大きさ（物理ピクセル）
    pub fn set_placement(&mut self, placement: ResultWindowPlacement) {
        self.pending_placement = Some(placement);
    }

    /// まだ保存していない「最後に見えていた時の位置と大きさ」。一度も見えていないか、保存済みから変わっていなければ None
    pub fn take_unsaved_placement(&self) -> Option<ResultWindowPlacement> {
        TRACKER
            .with(|t| t.borrow_mut().take_unsaved())
            .map(|r| ResultWindowPlacement {
                left: Some(f64::from(r.x)),
                top: Some(f64::from(r.y)),
                width: f64::from(r.width),
                height: f64::from(r.height),
            })
    }

    /// OK・× で隠す。保存すべき位置と大きさがあれば返す
    pub fn hide_by_user(&mut self) -> Option<ResultWindowPlacement> {
        // 隠す前の見えている状態を取ってから隠す
        observe(self.hwnd);
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        self.take_unsaved_placement()
    }

    /// ボタンの押し。OK なら保存すべき位置と大きさを返す
    pub fn on_command(&mut self, id: i32) -> Option<ResultWindowPlacement> {
        match id {
            ID_OK => return self.hide_by_user(),
            ID_RETRANSLATE => {
                let text = self.item_text(ID_SOURCE).trim().to_string();
                if text.is_empty() {
                    self.show_message("文字が見つかりません", "");
                } else {
                    self.translate(&text);
                }
            }
            ID_TOPMOST => {
                let topmost = unsafe { IsDlgButtonChecked(self.hwnd, ID_TOPMOST) } == BST_CHECKED.0;
                let after = if topmost {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                };
                let _ = unsafe {
                    SetWindowPos(
                        self.hwnd,
                        Some(after),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    )
                };
            }
            _ => {}
        }
        None
    }

    fn present(&mut self) {
        if let Some(placement) = self.pending_placement.take() {
            self.apply_placement(placement);
        }
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() || IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// 物理ピクセルで置く。保存した位置がどのモニターにも収まらない（モニター構成が変わった等）か
    /// 位置が None なら、カーソルのあるモニターの作業領域の中央に置く
    fn apply_placement(&self, placement: ResultWindowPlacement) {
        let width = placement.width.round() as i32;
        let height = placement.height.round() as i32;
        let saved = match (placement.left, placement.top) {
            (Some(left), Some(top)) => Some(PixelRect::new(
                left.round() as i32,
                top.round() as i32,
                width,
                height,
            )),
            _ => None,
        };
        let rect = match saved {
            Some(r) if window_placement::fits_on_monitors(r, &system::monitors()) => r,
            _ => window_placement::center_in(
                system::work_area_at(system::cursor_position()),
                width,
                height,
            ),
        };
        let _ = unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }

    /// アプリ終了時だけ本当に閉じる
    pub fn destroy(&mut self) {
        self.translation.cancel();
        RESULT_HWND.store(0, Ordering::Relaxed);
        let _ = unsafe { DestroyWindow(self.hwnd) };
        RESOURCES.with(|r| {
            if let Some(res) = r.borrow_mut().take() {
                free_resources(res);
            }
        });
    }
}

fn observe(hwnd: HWND) {
    let (visible, minimized) =
        unsafe { (IsWindowVisible(hwnd).as_bool(), IsIconic(hwnd).as_bool()) };
    let rect = system::window_rect(hwnd);
    TRACKER.with(|t| t.borrow_mut().observe(rect, visible, minimized));
}

fn create_font(dip: f64, dpi: u32) -> HFONT {
    unsafe {
        CreateFontW(
            -frame::to_pixels(dip, dpi),
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

fn free_resources(res: Resources) {
    unsafe {
        let _ = DeleteObject(res.edit_font.into());
        let _ = DeleteObject(res.button_font.into());
        let _ = DeleteObject(res.translation_brush.into());
    }
}

/// ウィンドウの DPI に合わせてフォントを作り直して部品に当てる
fn apply_resources(hwnd: HWND) {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let current = RESOURCES.with(|r| r.borrow().as_ref().map(|res| res.dpi));
    if current == Some(dpi) {
        return;
    }
    let res = Resources {
        dpi,
        edit_font: create_font(EDIT_FONT_DIP, dpi),
        button_font: create_font(BUTTON_FONT_DIP, dpi),
        translation_brush: unsafe { CreateSolidBrush(TRANSLATION_BACKGROUND) },
        _small_icon: icon::small(dpi),
        _large_icon: icon::large(dpi),
    };
    for (kind, icon) in [(ICON_SMALL, &res._small_icon), (ICON_BIG, &res._large_icon)] {
        if let Some(icon) = icon {
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(icon.0.0 as isize)),
                )
            };
        }
    }
    for (id, font) in [
        (ID_SOURCE, res.edit_font),
        (ID_TRANSLATION, res.edit_font),
        (ID_TOPMOST, res.button_font),
        (ID_RETRANSLATE, res.button_font),
        (ID_OK, res.button_font),
    ] {
        if let Ok(item) = unsafe { GetDlgItem(Some(hwnd), id) } {
            unsafe {
                SendMessageW(
                    item,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                )
            };
        }
    }
    RESOURCES.with(|r| {
        if let Some(old) = r.borrow_mut().replace(res) {
            free_resources(old);
        }
    });
}

fn layout(hwnd: HWND) {
    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return;
    }
    let scale = f64::from(unsafe { GetDpiForWindow(hwnd) }.max(96)) / 96.0;
    let l = result_layout::compute(
        client.right - client.left,
        client.bottom - client.top,
        scale,
    );
    for (id, r) in [
        (ID_SOURCE, l.source),
        (ID_TRANSLATION, l.translation),
        (ID_TOPMOST, l.topmost),
        (ID_RETRANSLATE, l.retranslate),
        (ID_OK, l.ok),
    ] {
        if let Ok(item) = unsafe { GetDlgItem(Some(hwnd), id) } {
            let _ = unsafe { MoveWindow(item, r.x, r.y, r.width, r.height, true) };
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            layout(hwnd);
            LRESULT(0)
        }
        WM_WINDOWPOSCHANGED => {
            // 既定の処理が WM_SIZE / WM_MOVE を出すので先に呼ぶ
            let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            observe(hwnd);
            result
        }
        WM_GETMINMAXINFO => {
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            info.ptMinTrackSize.x = frame::to_pixels(ResultWindowPlacement::MIN_WIDTH, dpi);
            info.ptMinTrackSize.y = frame::to_pixels(ResultWindowPlacement::MIN_HEIGHT, dpi);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            apply_resources(hwnd);
            let r = unsafe { &*(lparam.0 as *const RECT) };
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let translation = unsafe { GetDlgItem(Some(hwnd), ID_TRANSLATION) }.unwrap_or_default();
            if lparam.0 == translation.0 as isize
                && let Some(brush) =
                    RESOURCES.with(|r| r.borrow().as_ref().map(|res| res.translation_brush))
            {
                unsafe { SetBkColor(HDC(wparam.0 as _), TRANSLATION_BACKGROUND) };
                return LRESULT(brush.0 as isize);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        DM_GETDEFID => LRESULT(((DC_HASDEFID << 16) | ID_OK as usize) as isize),
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let notification = (wparam.0 >> 16) & 0xFFFF;
            const BN_CLICKED: usize = 0;
            if id != IDCANCEL.0
                && notification == BN_CLICKED
                && [ID_OK, ID_RETRANSLATE, ID_TOPMOST].contains(&id)
                && !with_app(|app| app.on_result_command(id))
            {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            if !with_app(|app| app.on_result_command(ID_OK)) {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
