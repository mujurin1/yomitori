//! 範囲指定の間だけ WH_MOUSE_LL を張り、窓の重なり順に関係なくマウスを受ける
//! （通知などが受け皿より手前に出ても、枠の追従とクリックを横取りされないため）。
//! 移動は握りつぶさない（カーソルが動かなくなるため）。ボタンとホイールは握りつぶして下へ渡さない。
//! フックは専用スレッドのメッセージループで受けて即座に戻り、処理は UI スレッドへ投げる

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW,
    PostMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN,
    WM_XBUTTONUP,
};

use crate::logic::geometry::PixelPoint;
use crate::logic::region_selection::MouseInput;

// フックの手続きは引数で状態を受け取れないので、動いている 1 つの宛先をここに置く
static NOTIFY_HWND: AtomicIsize = AtomicIsize::new(0);
static BUTTON_MESSAGE: AtomicU32 = AtomicU32::new(0);
static MOVE_MESSAGE: AtomicU32 = AtomicU32::new(0);
static LAST_X: AtomicI32 = AtomicI32::new(0);
static LAST_Y: AtomicI32 = AtomicI32::new(0);
/// 移動の知らせを投げて、まだ UI スレッドが受け取っていない。移動が速い時に知らせを積み上げないため
static MOVE_PENDING: AtomicBool = AtomicBool::new(false);

const LEFT_DOWN: usize = 1;
const LEFT_UP: usize = 2;
const RIGHT_DOWN: usize = 3;
const RIGHT_UP: usize = 4;

/// 張っている間だけ持つ。drop で外す。同時に 1 つだけ使う
pub struct MouseHook {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl MouseHook {
    /// notify に、ボタンは button_message（wParam が種類、lParam が位置）、移動は move_message を投げる
    pub fn start(notify: HWND, button_message: u32, move_message: u32) -> Result<Self, String> {
        NOTIFY_HWND.store(notify.0 as isize, Ordering::SeqCst);
        BUTTON_MESSAGE.store(button_message, Ordering::SeqCst);
        MOVE_MESSAGE.store(move_message, Ordering::SeqCst);
        MOVE_PENDING.store(false, Ordering::SeqCst);
        let (installed, result) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("yomitori mouse hook".into())
            .spawn(move || run(&installed))
            .map_err(|e| e.to_string())?;
        match result.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("マウスのフックのスレッドが止まりました".into()),
        }
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        NOTIFY_HWND.store(0, Ordering::SeqCst);
    }
}

/// 移動の知らせを受けた時に、最新の位置を取り出す（UI スレッドで呼ぶ）
pub fn take_move() -> PixelPoint {
    MOVE_PENDING.store(false, Ordering::SeqCst);
    PixelPoint::new(LAST_X.load(Ordering::SeqCst), LAST_Y.load(Ordering::SeqCst))
}

/// ボタンの知らせ（wParam・lParam）を MouseInput にする
pub fn decode_button(wparam: WPARAM, lparam: LPARAM) -> Option<MouseInput> {
    let point = PixelPoint::unpack(lparam.0 as i64);
    match wparam.0 {
        LEFT_DOWN => Some(MouseInput::LeftDown(point)),
        LEFT_UP => Some(MouseInput::LeftUp(point)),
        RIGHT_DOWN => Some(MouseInput::RightDown(point)),
        RIGHT_UP => Some(MouseInput::RightUp(point)),
        _ => None,
    }
}

fn run(installed: &mpsc::Sender<Result<u32, String>>) {
    let thread_id = unsafe { GetCurrentThreadId() };
    let hook = unsafe {
        GetModuleHandleW(None).and_then(|module| {
            SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), Some(module.into()), 0)
        })
    };
    let hook = match hook {
        Ok(h) => h,
        Err(e) => {
            let _ = installed.send(Err(e.message()));
            return;
        }
    };
    let mut msg = MSG::default();
    // 終了の WM_QUIT を PostThreadMessageW で受けられるよう、知らせる前にメッセージキューを作る
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    let _ = installed.send(Ok(thread_id));
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let _ = unsafe { UnhookWindowsHookEx(hook) };
}

fn post(message: &AtomicU32, wparam: usize, lparam: isize) {
    let target = NOTIFY_HWND.load(Ordering::SeqCst);
    if target == 0 {
        return;
    }
    let _ = unsafe {
        PostMessageW(
            Some(HWND(target as _)),
            message.load(Ordering::SeqCst),
            WPARAM(wparam),
            LPARAM(lparam),
        )
    };
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // 位置は PerMonitorV2 のプロセスでは物理ピクセル
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let (x, y) = (info.pt.x, info.pt.y);
        let packed = PixelPoint::new(x, y).pack() as isize;
        let button = |kind: usize| {
            post(&BUTTON_MESSAGE, kind, packed);
            LRESULT(1)
        };
        match wparam.0 as u32 {
            WM_MOUSEMOVE => {
                LAST_X.store(x, Ordering::SeqCst);
                LAST_Y.store(y, Ordering::SeqCst);
                if !MOVE_PENDING.swap(true, Ordering::SeqCst) {
                    post(&MOVE_MESSAGE, 0, 0);
                }
            }
            WM_LBUTTONDOWN => return button(LEFT_DOWN),
            WM_LBUTTONUP => return button(LEFT_UP),
            WM_RBUTTONDOWN => return button(RIGHT_DOWN),
            WM_RBUTTONUP => return button(RIGHT_UP),
            WM_MBUTTONDOWN | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP | WM_MOUSEWHEEL
            | WM_MOUSEHWHEEL => return LRESULT(1),
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
