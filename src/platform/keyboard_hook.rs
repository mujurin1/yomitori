//! WH_KEYBOARD_LL で設定のホットキー（既定 Win+Q / Win+W）を拾う。
//! フックは専用スレッドのメッセージループで受け、即座に戻る（UI スレッドが詰まっても
//! LowLevelHooksTimeout で外されないように）。実際の処理は on_fire が UI スレッドへ投げる。
//!
//! RegisterHotKey を使わないのは、Win+Q（検索）・Win+W（ウィジェット）を Windows 自身が登録していて
//! 1409（ERROR_HOTKEY_ALREADY_REGISTERED）になるため（DisabledHotkeys でも Win+W は空かない）。
//! 代償として、管理者権限のウィンドウが前面にある間はフックにキーが来ず、ホットキーが効かない

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::sync::{Arc, PoisonError, RwLock};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::logic::hotkey_filter::{
    HotkeyAction, HotkeyBinding, HotkeyFilter, KeyEvent, ModifierState,
};

/// スタートメニューを抑えるために送る、割り当ての無い仮想キー
const VK_MASK: u16 = 0xE8;

/// 自分が送ったキーの印（dwExtraInfo）。"YOMI"
const MARKER: usize = 0x594F_4D49;

type OnFire = dyn Fn(HotkeyAction) + Send + Sync;

struct Shared {
    /// 設定の再読み込みで丸ごと差し替える（フックのスレッドが読む）
    filter: RwLock<Arc<HotkeyFilter>>,
    on_fire: Box<OnFire>,
}

/// フックの手続きは引数で状態を受け取れないので、動いている 1 つをここに置く
static ACTIVE: RwLock<Option<Arc<Shared>>> = RwLock::new(None);

pub struct KeyboardHook {
    shared: Arc<Shared>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl KeyboardHook {
    /// フックを設置する。on_fire はフックのスレッドから呼ばれるので、重い処理をせず UI スレッドへ投げること
    pub fn start(
        bindings: Vec<HotkeyBinding>,
        on_fire: impl Fn(HotkeyAction) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            filter: RwLock::new(Arc::new(HotkeyFilter::new(bindings))),
            on_fire: Box::new(on_fire),
        });
        {
            let mut active = ACTIVE.write().unwrap_or_else(PoisonError::into_inner);
            if active.is_some() {
                return Err("キーボードフックは 1 つだけ設置できます".into());
            }
            *active = Some(Arc::clone(&shared));
        }

        let (installed, result) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("yomitori keyboard hook".into())
            .spawn(move || run(&installed));
        let outcome = match spawned {
            Ok(thread) => match result.recv() {
                Ok(Ok(thread_id)) => Ok((thread, thread_id)),
                Ok(Err(e)) => Err(e),
                Err(_) => Err("フックのスレッドが止まりました".into()),
            },
            Err(e) => Err(e.to_string()),
        };
        match outcome {
            Ok((thread, thread_id)) => Ok(Self {
                shared,
                thread_id,
                thread: Some(thread),
            }),
            Err(e) => {
                *ACTIVE.write().unwrap_or_else(PoisonError::into_inner) = None;
                Err(e)
            }
        }
    }

    fn filter(&self) -> Arc<HotkeyFilter> {
        Arc::clone(
            &self
                .shared
                .filter
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// 範囲指定のオーバーレイを出している間 true（Esc をフックで拾う）
    pub fn set_capture_active(&self, active: bool) {
        self.filter().set_capture_active(active);
    }

    /// ホットキーを差し替える（設定の再読み込み）。押したままのキーの状態は引き継がない
    pub fn set_bindings(&self, bindings: Vec<HotkeyBinding>) {
        let mut filter = self
            .shared
            .filter
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let replaced = HotkeyFilter::new(bindings);
        replaced.set_capture_active(filter.capture_active());
        *filter = Arc::new(replaced);
    }
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let mut active = ACTIVE.write().unwrap_or_else(PoisonError::into_inner);
        if active
            .as_ref()
            .is_some_and(|a| Arc::ptr_eq(a, &self.shared))
        {
            *active = None;
        }
    }
}

fn run(installed: &mpsc::Sender<Result<u32, String>>) {
    let thread_id = unsafe { GetCurrentThreadId() };
    let hook = unsafe {
        GetModuleHandleW(None).and_then(|module| {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module.into()), 0)
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

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // フックの中の panic は FFI の境界を越えられずプロセスが落ちる。判定できなければ素通しする
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if let Ok(true) = catch_unwind(AssertUnwindSafe(|| handle(wparam.0 as u32, info))) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// true ならキーを握りつぶす
fn handle(message: u32, info: &KBDLLHOOKSTRUCT) -> bool {
    let is_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
    if !is_down && message != WM_KEYUP && message != WM_SYSKEYUP {
        return false;
    }
    if info.flags.contains(LLKHF_INJECTED) && info.dwExtraInfo == MARKER {
        return false;
    }
    let Some(shared) = ACTIVE
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
    else {
        return false;
    };
    let filter = Arc::clone(&shared.filter.read().unwrap_or_else(PoisonError::into_inner));

    let modifiers = ModifierState {
        win: is_down_now(VK_LWIN) || is_down_now(VK_RWIN),
        ctrl: is_down_now(VK_CONTROL),
        shift: is_down_now(VK_SHIFT),
        alt: is_down_now(VK_MENU),
    };
    let decision = filter.process(
        KeyEvent {
            virtual_key: info.vkCode,
            is_down,
        },
        modifiers,
    );

    if decision.send_mask_key {
        send_mask_key();
    }
    if let Some(action) = decision.fire {
        (shared.on_fire)(action);
    }
    decision.suppress
}

fn is_down_now(vk: VIRTUAL_KEY) -> bool {
    (unsafe { GetAsyncKeyState(i32::from(vk.0)) } as u16 & 0x8000) != 0
}

/// 発火の時点（修飾がまだ押されている間）に送る。Win / Alt の離しを見てから送ると、送ったキーは
/// その離しより後ろに並ぶので間に合わず、スタートメニュー / メニューバーが開いてしまう
fn send_mask_key() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_MASK),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MARKER,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}
