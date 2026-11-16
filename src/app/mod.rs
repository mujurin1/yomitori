//! 常駐アプリの組み立て。UI スレッドの隠しウィンドウが、トレイ・ホットキー・別スレッドの結果を受ける

mod canvas;
mod overlay;
mod result_window;

use std::any::Any;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetTimer,
    TranslateMessage, WINDOW_STYLE, WM_APP, WM_CAPTURECHANGED, WM_COMMAND, WM_DESTROY,
    WM_DPICHANGED, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
};
use windows::core::w;

use crate::logic::cursor_line::{self, OcrLineInfo};
use crate::logic::geometry::{PixelPoint, PixelRect};
use crate::logic::hotkey_filter::{HotkeyAction, HotkeyBinding};
use crate::logic::ocr_support;
use crate::logic::region_selection::line_marker_timing;
use crate::logic::settings::AppSettings;
use crate::logic::settings_store::{RealSettingsFiles, SettingsStore};
use crate::logic::translation::{GoogleGtxTranslator, PendingTranslation, TranslationOutcome};
use crate::platform::capture::capture_screen;
use crate::platform::foreground::ForegroundWatch;
use crate::platform::http::UreqHttp;
use crate::platform::icon::{self, AppIcon};
use crate::platform::keyboard_hook::KeyboardHook;
use crate::platform::ocr::{OcrResult, OcrService};
use crate::platform::single_instance::SingleInstance;
use crate::platform::system;
use crate::platform::tray::{self, BalloonIcon, MenuItem, Tray};
use canvas::Canvas;
use overlay::{RegionOverlay, Snapshot};
use result_window::ResultWindow;

const MUTEX_NAME: &str = r"Local\yomitori-rs-SingleInstance-7C41D2E9";

const WM_APP_HOTKEY: u32 = WM_APP + 1;
const WM_APP_TRAY: u32 = WM_APP + 2;
/// lParam に `Box<UiCall>` を Box::into_raw した値を載せる
const WM_APP_CALL: u32 = WM_APP + 3;
/// 範囲指定中に前面のウィンドウが変わった。wParam は受け皿の番号
const WM_APP_FOREGROUND_CHANGED: u32 = WM_APP + 4;

const ID_OPEN_SETTINGS: u32 = 1;
const ID_RELOAD_SETTINGS: u32 = 2;
const ID_EXIT: u32 = 3;

const MENU: [MenuItem; 4] = [
    Some((ID_OPEN_SETTINGS, "設定ファイルを開く")),
    Some((ID_RELOAD_SETTINGS, "設定を再読み込み")),
    None,
    Some((ID_EXIT, "終了")),
];

/// Win+W の枠を消すタイマーの id はこれに枠の番号を足す（枠ごとに別のタイマーにして、古い訳が遅れて届いても新しい枠の消す予定を潰さない）
const MARKER_TIMER_BASE: usize = 0x1000;
/// 「文字なし」を消すタイマーの id（同時に 1 つだけなので固定）。MARKER_TIMER_BASE より小さくする
const HINT_TIMER: usize = 0x0FFF;
/// OCR の結果が文字なしの時に、カーソルの近くに「文字なし」を出しておく時間
const NO_TEXT_HINT_VISIBLE: Duration = Duration::from_millis(700);

/// エクスプローラーが再起動した時にトレイのアイコンを置き直すための登録メッセージ
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

type UiCall = Box<dyn FnOnce(&mut App) + Send>;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// 別スレッドから UI スレッドへ処理を投げる口
#[derive(Clone, Copy)]
struct UiPoster(isize);

impl UiPoster {
    fn hwnd(self) -> HWND {
        HWND(self.0 as _)
    }

    fn post(self, call: impl FnOnce(&mut App) + Send + 'static) {
        let boxed: Box<UiCall> = Box::new(Box::new(call));
        let raw = Box::into_raw(boxed);
        if unsafe {
            PostMessageW(
                Some(self.hwnd()),
                WM_APP_CALL,
                WPARAM(0),
                LPARAM(raw as isize),
            )
        }
        .is_err()
        {
            // 届かなかった（終了の後など）。取り戻して捨てる
            drop(unsafe { Box::from_raw(raw) });
        }
    }

    fn post_hotkey(self, action: HotkeyAction) {
        let code = match action {
            HotkeyAction::RegionCapture => 1,
            HotkeyAction::CursorLine => 2,
            HotkeyAction::CancelCapture => 3,
        };
        let _ = unsafe { PostMessageW(Some(self.hwnd()), WM_APP_HOTKEY, WPARAM(code), LPARAM(0)) };
    }
}

fn action_from_code(code: usize) -> Option<HotkeyAction> {
    match code {
        1 => Some(HotkeyAction::RegionCapture),
        2 => Some(HotkeyAction::CursorLine),
        3 => Some(HotkeyAction::CancelCapture),
        _ => None,
    }
}

struct App {
    hwnd: HWND,
    poster: UiPoster,
    hook: Option<KeyboardHook>,
    overlay: Option<RegionOverlay>,
    /// 範囲指定中だけ前面のウィンドウの変化を見張る
    foreground_watch: Option<ForegroundWatch>,
    /// Win+W の枠。番号は消す予定のタイマーと突き合わせる
    marker: Option<(u64, Canvas)>,
    /// 「文字なし」の知らせ
    hint: Option<Canvas>,
    next_id: u64,
    result: ResultWindow,
    ocr: Arc<OcrService>,
    store: SettingsStore<RealSettingsFiles>,
    settings: AppSettings,
    capturing_line: bool,
    tray: Option<Tray>,
    /// トレイのアイコン。Tray より後に捨てる
    tray_icon: Option<AppIcon>,
}

/// プロセスの終了コードを返す
pub fn run() -> i32 {
    // 2 つ目の起動は何もせずに終わる
    let single = match SingleInstance::acquire(MUTEX_NAME) {
        Ok(Some(single)) => single,
        Ok(None) => return 0,
        Err(_) => return 1,
    };
    // ShellExecute のため UI スレッドは STA にする
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let hwnd = match create_main_window() {
        Ok(hwnd) => hwnd,
        Err(_) => return 1,
    };
    canvas::register_classes();
    overlay::register_class(catcher_proc);
    let app = match App::start(hwnd) {
        Ok(app) => app,
        Err(_) => return 1,
    };
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if result_window::is_dialog_message(&msg) {
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    if let Some(app) = APP.with(|cell| cell.borrow_mut().take()) {
        app.shutdown();
    }
    drop(single);
    0
}

fn create_main_window() -> windows::core::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("YomitoriMain");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        // 表示しない。メッセージ専用ウィンドウにしないのは、TaskbarCreated の通知が届かないため
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("yomitori"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

impl App {
    fn start(hwnd: HWND) -> windows::core::Result<Self> {
        let poster = UiPoster(hwnd.0 as isize);
        let translator = Arc::new(GoogleGtxTranslator::new(UreqHttp::new()));
        let tray_icon = icon::small(unsafe { GetDpiForWindow(hwnd) }.max(96));
        let result = ResultWindow::create(translator, poster)?;
        let ocr = Arc::new(OcrService::start());
        let tray = tray_icon
            .as_ref()
            .and_then(|icon| Tray::add(hwnd, WM_APP_TRAY, icon.0, "yomitori").ok());
        let settings_path = system::exe_dir()
            .unwrap_or_default()
            .join(SettingsStore::<RealSettingsFiles>::FILE_NAME);
        let mut app = Self {
            hwnd,
            poster,
            hook: None,
            overlay: None,
            foreground_watch: None,
            marker: None,
            hint: None,
            next_id: 1,
            result,
            ocr,
            store: SettingsStore::new(settings_path, RealSettingsFiles, system::local_now),
            settings: AppSettings::default(),
            capturing_line: false,
            tray,
            tray_icon,
        };

        let settings_notified = app.load_settings();
        if let Some(error) = app.ocr.error().map(str::to_string) {
            app.notify("OCR が使えません", &error, BalloonIcon::Error);
        }

        // フックのスレッドから呼ばれるので、すぐ UI スレッドへ投げて戻る
        match KeyboardHook::start(app.bindings(), move |action| poster.post_hotkey(action)) {
            Ok(hook) => {
                app.hook = Some(hook);
                // バルーンは後に出した方が前を上書きするので、設定や OCR の問題を知らせた時は案内を出さない
                if app.ocr.error().is_none()
                    && !settings_notified
                    && app.settings.show_startup_notification
                {
                    let text = app.hotkey_summary();
                    app.notify("yomitori", &text, BalloonIcon::Info);
                }
            }
            Err(e) => app.notify(
                "ホットキーを使えません",
                &format!("キーボードフックを設置できませんでした: {e}"),
                BalloonIcon::Warning,
            ),
        }
        Ok(app)
    }

    fn shutdown(mut self) {
        self.hook = None;
        self.overlay = None;
        self.marker = None;
        // ここではウィンドウを読まず、最後に見えていた時の値を保存する（未保存の時だけ）
        if let Some(placement) = self.result.take_unsaved_placement() {
            self.store.save_result_window(placement);
        }
        self.result.destroy();
        self.tray = None;
        self.tray_icon = None;
    }

    /// DPI が変わった時に、その DPI の大きさのアイコンをトレイに当て直す
    fn reload_tray_icon(&mut self) {
        let (Some(tray), Some(new)) = (
            self.tray.as_mut(),
            icon::small(unsafe { GetDpiForWindow(self.hwnd) }.max(96)),
        ) else {
            return;
        };
        tray.set_icon(new.0);
        self.tray_icon = Some(new);
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn hotkey_summary(&self) -> String {
        format!(
            "{}: 範囲指定 / {}: カーソル行",
            self.settings.region_hotkey, self.settings.line_hotkey
        )
    }

    fn bindings(&self) -> Vec<HotkeyBinding> {
        // 読み込んだ設定はホットキーを解釈済みなので失敗しない。念のため既定値に戻す
        self.settings
            .bindings()
            .or_else(|_| AppSettings::default().bindings())
            .expect("既定のホットキーは解釈できる")
    }

    fn notify(&self, title: &str, text: &str, icon: BalloonIcon) {
        if let Some(tray) = &self.tray {
            tray.notify(title, text, icon);
        }
    }

    /// 起動時と「設定を再読み込み」。ホットキーと結果ウィンドウの位置・大きさを反映する。知らせを出したら true
    fn load_settings(&mut self) -> bool {
        let loaded = self.store.load();
        self.settings = loaded.settings;
        if let Some(hook) = &self.hook {
            hook.set_bindings(self.bindings());
        }
        self.result.set_placement(self.settings.result_window);
        let Some(message) = loaded.notification else {
            return false;
        };
        self.notify("yomitori の設定", &message, BalloonIcon::Warning);
        true
    }

    fn reload_settings(&mut self) {
        if !self.load_settings() {
            let text = format!("再読み込みました（{}）", self.hotkey_summary());
            self.notify("yomitori の設定", &text, BalloonIcon::Info);
        }
    }

    fn open_settings_file(&self) {
        let path = self.store.path();
        if !path.is_file() {
            let text = format!("設定ファイルがありません: {}", path.display());
            self.notify("yomitori の設定", &text, BalloonIcon::Warning);
            return;
        }
        if let Err(e) = system::open_with_shell(path) {
            self.notify(
                "yomitori の設定",
                &format!("設定ファイルを開けませんでした: {e}"),
                BalloonIcon::Warning,
            );
        }
    }

    fn on_command(&mut self, id: u32) {
        match id {
            ID_OPEN_SETTINGS => self.open_settings_file(),
            ID_RELOAD_SETTINGS => self.reload_settings(),
            ID_EXIT => {
                let _ = unsafe { DestroyWindow(self.hwnd) };
            }
            _ => {}
        }
    }

    fn on_result_command(&mut self, id: i32) {
        if let Some(placement) = self.result.on_command(id) {
            self.store.save_result_window(placement);
        }
    }

    fn on_hotkey(&mut self, action: HotkeyAction) {
        match action {
            HotkeyAction::RegionCapture => self.start_region_capture(),
            HotkeyAction::CursorLine => self.capture_cursor_line(),
            HotkeyAction::CancelCapture => self.cancel_overlay(None),
        }
    }

    /// 範囲指定を中止する（Esc・他のキーの押下・前面の変化・マウスのキャプチャを外された）。
    /// id が Some なら、その番号の受け皿の時だけ（古い知らせで新しい範囲指定を中止しないため）
    fn cancel_overlay(&mut self, id: Option<u64>) {
        let Some(overlay) = self.overlay.as_mut() else {
            return;
        };
        if id.is_some_and(|id| id != overlay.id) {
            return;
        }
        overlay.cancel();
        self.on_region_finished(None);
    }

    /// 受け皿がマウスのキャプチャを失った。自分以外に移ったなら割り込まれたとみなして中止する
    fn on_catcher_capture_lost(&mut self, catcher: HWND, new_owner: HWND) {
        let ours = self.overlay.as_ref().and_then(RegionOverlay::catcher) == Some(catcher);
        if ours && new_owner != catcher {
            self.cancel_overlay(None);
        }
    }

    fn close_line_marker(&mut self) {
        self.marker = None;
        // 次のキャプチャに写らないよう「文字なし」も消す
        self.hint = None;
    }

    /// OCR の結果が文字なし。結果ウィンドウは出さず（出ていれば中身を残す）、翻訳もせず、
    /// カーソルの近くに「文字なし」を少しの間だけ出す
    fn show_no_text_hint(&mut self) {
        let mut hint = Canvas::new();
        hint.show_hint(system::cursor_position(), "文字なし");
        self.hint = Some(hint);
        let millis = NO_TEXT_HINT_VISIBLE.as_millis() as u32;
        unsafe { SetTimer(Some(self.hwnd), HINT_TIMER, millis, None) };
    }

    /// 範囲指定中に region・line のホットキーが押された。受け皿・枠が次のキャプチャに写らないよう、
    /// 中止して、消えた画面が合成されるのを待ってから撮る
    fn cancel_overlay_before_capture(&mut self) {
        if self.overlay.is_none() {
            return;
        }
        self.cancel_overlay(None);
        let _ = unsafe { DwmFlush() };
    }

    fn start_region_capture(&mut self) {
        // 範囲指定中なら、今のカーソル位置を起点にやり直す
        self.cancel_overlay_before_capture();
        if let Some(error) = self.ocr.error().map(str::to_string) {
            self.result.show_message(&error, "");
            return;
        }
        self.close_line_marker();
        let screen = system::virtual_screen();
        let image = match capture_screen(screen) {
            Ok(image) => image,
            Err(e) => {
                self.result
                    .show_message(&format!("OCR に失敗しました: {e}"), "");
                return;
            }
        };
        let id = self.take_id();
        let poster = self.poster;
        let snapshot = Arc::new(Snapshot {
            image,
            bounds: screen,
        });
        let keep_line_breaks = self.settings.keep_line_breaks;
        self.overlay = RegionOverlay::open(
            id,
            snapshot,
            Arc::clone(&self.ocr),
            keep_line_breaks,
            move |text| {
                poster.post(move |app| app.on_overlay_preview(id, &text));
            },
        );
        if self.overlay.is_none() {
            return;
        }
        if let Some(hook) = &self.hook {
            hook.set_capture_active(true);
        }
        self.foreground_watch =
            ForegroundWatch::start(self.hwnd, WM_APP_FOREGROUND_CHANGED, id as usize);
    }

    fn on_overlay_preview(&mut self, id: u64, text: &str) {
        if let Some(overlay) = self.overlay.as_mut().filter(|o| o.id == id) {
            overlay.on_preview(text);
        }
    }

    fn on_overlay_mouse(&mut self, msg: u32, wparam: WPARAM) {
        let finished = self.overlay.as_mut().and_then(|o| o.on_mouse(msg, wparam));
        if let Some(result) = finished {
            self.on_region_finished(result);
        }
    }

    fn on_region_finished(&mut self, rect: Option<PixelRect>) {
        let Some(overlay) = self.overlay.take() else {
            return;
        };
        self.foreground_watch = None;
        if let Some(hook) = &self.hook {
            hook.set_capture_active(false);
        }
        let Some(image) = rect.and_then(|r| overlay.snapshot().crop(r)) else {
            return;
        };
        let poster = self.poster;
        self.ocr.recognize(image, move |result| {
            poster.post(move |app| app.on_region_recognized(result));
        });
    }

    fn on_region_recognized(&mut self, result: OcrResult) {
        match result {
            Ok(lines) => {
                let text = ocr_support::join_ocr_lines(
                    lines.iter().map(|l| &l.text),
                    self.settings.keep_line_breaks,
                );
                if text.is_empty() {
                    self.show_no_text_hint();
                } else {
                    self.result.show_text(&text);
                }
            }
            Err(e) => self
                .result
                .show_message(&format!("OCR に失敗しました: {e}"), ""),
        }
    }

    fn capture_cursor_line(&mut self) {
        if self.capturing_line {
            return;
        }
        // 範囲指定中なら中止してからカーソル行を読む
        self.cancel_overlay_before_capture();
        if let Some(error) = self.ocr.error().map(str::to_string) {
            self.result.show_message(&error, "");
            return;
        }

        // 前の枠が写らないよう、キャプチャの前に消す
        self.close_line_marker();
        let cursor = system::cursor_position();
        let band = cursor_line::compute_band(cursor, system::virtual_screen());
        if band.is_empty() {
            return;
        }
        let image = match capture_screen(band) {
            Ok(image) => image,
            Err(e) => {
                self.result
                    .show_message(&format!("OCR に失敗しました: {e}"), "");
                return;
            }
        };
        self.capturing_line = true;
        let poster = self.poster;
        self.ocr.recognize(image, move |result| {
            poster.post(move |app| app.on_line_recognized(result, cursor, band));
        });
    }

    fn on_line_recognized(&mut self, result: OcrResult, cursor: PixelPoint, band: PixelRect) {
        self.capturing_line = false;
        let lines = match result {
            Ok(lines) => lines,
            Err(e) => {
                self.result
                    .show_message(&format!("OCR に失敗しました: {e}"), "");
                return;
            }
        };
        let line: Option<&OcrLineInfo> =
            cursor_line::select_line(&lines, cursor - PixelPoint::new(band.x, band.y));
        let Some(line) = line else {
            self.show_no_text_hint();
            return;
        };
        let text = ocr_support::join_lines([&line.text]);
        if text.is_empty() {
            self.show_no_text_hint();
            return;
        }

        // 読み取った行に枠とプレビューを出し、訳が届いてから結果ウィンドウを出す。
        // 枠は「訳が届くまで」と「表示から最低表示時間」の遅い方まで残す
        let b = line.bounds;
        let line_rect = PixelRect::new(band.x + b.x, band.y + b.y, b.width, b.height);
        let id = self.take_id();
        let mut marker = Canvas::new();
        marker.set_frame(line_rect);
        marker.set_preview(&text);
        self.marker = Some((id, marker));
        let shown = Instant::now();

        let pending = self.result.start_translate_then_show(&text);
        let poster = self.poster;
        let spawned = thread::Builder::new()
            .name("yomitori translation".into())
            .spawn(move || {
                let outcome = pending.run();
                poster.post(move |app| app.on_line_translated(&pending, &text, outcome, id, shown));
            });
        if let Err(e) = spawned {
            self.result
                .show_message(&format!("翻訳に失敗しました: {e}"), "");
            self.close_marker_if(id);
        }
    }

    fn on_line_translated(
        &mut self,
        pending: &PendingTranslation,
        text: &str,
        outcome: TranslationOutcome,
        marker_id: u64,
        shown: Instant,
    ) {
        self.result.deliver_then_show(pending, text, outcome);
        let remaining = line_marker_timing::remaining_after_arrival(shown.elapsed());
        if remaining.is_zero() {
            self.close_marker_if(marker_id);
            return;
        }
        let timer = MARKER_TIMER_BASE + marker_id as usize;
        let millis = u32::try_from(remaining.as_millis())
            .unwrap_or(u32::MAX)
            .max(1);
        unsafe { SetTimer(Some(self.hwnd), timer, millis, None) };
    }

    fn close_marker_if(&mut self, id: u64) {
        if self
            .marker
            .as_ref()
            .is_some_and(|(marker_id, _)| *marker_id == id)
        {
            self.marker = None;
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "不明なエラー".into())
}

/// App を借りて f を呼ぶ。借りられない（App の処理の中から入れ子で来た）時は false。
/// panic はここで止めて知らせる（wndproc の外へ出すとプロセスが落ちる）
fn with_app(f: impl FnOnce(&mut App)) -> bool {
    APP.with(|cell| {
        let Ok(mut borrowed) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(app) = borrowed.as_mut() else {
            return true;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| f(app))) {
            app.notify(
                "yomitori でエラーが起きました",
                &panic_message(payload.as_ref()),
                BalloonIcon::Error,
            );
        }
        true
    })
}

/// 入れ子で借りられなかったメッセージは、今の処理が終わった後に回す
fn repost(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    let _ = unsafe { PostMessageW(Some(hwnd), msg, wparam, lparam) };
}

unsafe extern "system" fn catcher_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(result) = overlay::catcher_default(hwnd, msg, wparam, lparam) {
        return result;
    }
    if msg == WM_CAPTURECHANGED {
        // 自分で外した時（確定・中止の後始末）は App を借りている最中に届くので、借りられなければ何もしない
        let new_owner = HWND(lparam.0 as _);
        with_app(|app| app.on_catcher_capture_lost(hwnd, new_owner));
        return LRESULT(0);
    }
    if !with_app(|app| app.on_overlay_mouse(msg, wparam)) {
        repost(hwnd, msg, wparam, lparam);
    }
    LRESULT(0)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_HOTKEY => {
            if let Some(action) = action_from_code(wparam.0)
                && !with_app(|app| app.on_hotkey(action))
            {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_APP_FOREGROUND_CHANGED => {
            let id = wparam.0 as u64;
            if !with_app(|app| app.cancel_overlay(Some(id))) {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_APP_CALL => {
            let raw = lparam.0 as *mut UiCall;
            let mut call = Some(unsafe { Box::from_raw(raw) });
            if !with_app(|app| (call.take().expect("1 回だけ呼ぶ"))(app)) {
                // 借りられなかったので、取り出していない Box をもう一度送る
                let raw = Box::into_raw(call.take().expect("呼んでいない"));
                repost(hwnd, msg, wparam, LPARAM(raw as isize));
            }
            LRESULT(0)
        }
        WM_APP_TRAY => {
            if (lparam.0 & 0xFFFF) as u32 == WM_RBUTTONUP {
                tray::show_menu(hwnd, &MENU);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u32;
            if !with_app(|app| app.on_command(id)) {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == HINT_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), HINT_TIMER) };
            if !with_app(|app| app.hint = None) {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 > MARKER_TIMER_BASE => {
            let _ = unsafe { KillTimer(Some(hwnd), wparam.0) };
            let id = (wparam.0 - MARKER_TIMER_BASE) as u64;
            if !with_app(|app| app.close_marker_if(id)) {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        // 隠しウィンドウなので、勧められた位置と大きさには合わせない
        WM_DPICHANGED => {
            if !with_app(|app| app.reload_tray_icon()) {
                repost(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with_app(|app| {
                if let Some(tray) = &app.tray {
                    tray.restore();
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
