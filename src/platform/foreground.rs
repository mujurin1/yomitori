//! 前面のウィンドウが変わったら知らせる（範囲指定中の割り込みを拾うため）

use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, PostMessageW, WINEVENT_OUTOFCONTEXT,
};

// フックの手続きは引数で状態を受け取れないので、見張っている 1 つの宛先をここに置く
static NOTIFY_HWND: AtomicIsize = AtomicIsize::new(0);
static NOTIFY_MESSAGE: AtomicU32 = AtomicU32::new(0);
static NOTIFY_WPARAM: AtomicUsize = AtomicUsize::new(0);
static START_FOREGROUND: AtomicIsize = AtomicIsize::new(0);
/// 今の見張りのフック。外した後に遅れて届いた古いフックのイベントを捨てるため
static CURRENT_HOOK: AtomicIsize = AtomicIsize::new(0);

/// 見張っている間だけ持つ。drop で見張りをやめる。同時に 1 つだけ使う
pub struct ForegroundWatch(HWINEVENTHOOK);

impl ForegroundWatch {
    /// 前面が始めた時のウィンドウから変わったら、notify に message（wParam = wparam）を投げる。
    /// 呼んだスレッドがメッセージを回している間だけ届く（WINEVENT_OUTOFCONTEXT）
    pub fn start(notify: HWND, message: u32, wparam: usize) -> Option<Self> {
        NOTIFY_HWND.store(notify.0 as isize, Ordering::SeqCst);
        NOTIFY_MESSAGE.store(message, Ordering::SeqCst);
        NOTIFY_WPARAM.store(wparam, Ordering::SeqCst);
        START_FOREGROUND.store(
            unsafe { GetForegroundWindow() }.0 as isize,
            Ordering::SeqCst,
        );
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(on_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        CURRENT_HOOK.store(hook.0 as isize, Ordering::SeqCst);
        (!hook.is_invalid()).then_some(Self(hook))
    }
}

impl Drop for ForegroundWatch {
    fn drop(&mut self) {
        let _ = unsafe { UnhookWinEvent(self.0) };
        NOTIFY_HWND.store(0, Ordering::SeqCst);
        CURRENT_HOOK.store(0, Ordering::SeqCst);
    }
}

unsafe extern "system" fn on_event(
    hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let target = NOTIFY_HWND.load(Ordering::SeqCst);
    // イベントの hwnd ではなく今の前面を見る。外から前面を移された時、イベントの hwnd が実際の前面と
    // 食い違ったり遅れて届いたりした（実測）。同じウィンドウが前面になり直しただけなら割り込みではない
    let now = unsafe { GetForegroundWindow() }.0 as isize;
    if target == 0
        || hook.0 as isize != CURRENT_HOOK.load(Ordering::SeqCst)
        || now == START_FOREGROUND.load(Ordering::SeqCst)
    {
        return;
    }
    let _ = unsafe {
        PostMessageW(
            Some(HWND(target as _)),
            NOTIFY_MESSAGE.load(Ordering::SeqCst),
            WPARAM(NOTIFY_WPARAM.load(Ordering::SeqCst)),
            LPARAM(0),
        )
    };
}
