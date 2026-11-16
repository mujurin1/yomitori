use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_INFOTIP_FLAGS, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, HMENU, MF_SEPARATOR, MF_STRING,
    PostMessageW, SetForegroundWindow, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::HSTRING;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BalloonIcon {
    Info,
    Warning,
    Error,
}

/// タスクトレイのアイコン。drop で消す
pub struct Tray {
    hwnd: HWND,
    callback_message: u32,
    icon: HICON,
    tip: String,
}

const ICON_ID: u32 = 1;

/// 固定長の UTF-16 の欄に書く。入りきらなければ切り詰める（末尾の NUL を残す）
fn copy_into(dest: &mut [u16], text: &str) {
    let units: Vec<u16> = text.encode_utf16().take(dest.len() - 1).collect();
    dest[..units.len()].copy_from_slice(&units);
    dest[units.len()] = 0;
}

impl Tray {
    /// hwnd が callback_message でマウスの操作を受け取る（lParam の下位がマウスのメッセージ）
    pub fn add(hwnd: HWND, callback_message: u32, icon: HICON, tip: &str) -> Result<Self, String> {
        let tray = Self {
            hwnd,
            callback_message,
            icon,
            tip: tip.into(),
        };
        tray.register()?;
        Ok(tray)
    }

    fn base(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        }
    }

    fn register(&self) -> Result<(), String> {
        let mut data = self.base();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = self.callback_message;
        data.hIcon = self.icon;
        copy_into(&mut data.szTip, &self.tip);
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            Ok(())
        } else {
            Err("タスクトレイにアイコンを置けませんでした".into())
        }
    }

    /// アイコンを差し替える（DPI が変わった時）。古いアイコンは呼び出し側が、これの後で捨てる
    pub fn set_icon(&mut self, icon: HICON) {
        self.icon = icon;
        let mut data = self.base();
        data.uFlags = NIF_ICON;
        data.hIcon = icon;
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    /// エクスプローラーが再起動した（TaskbarCreated）。アイコンを置き直す
    pub fn restore(&self) {
        let _ = self.register();
    }

    /// バルーン通知。後に出したものが前を上書きする
    pub fn notify(&self, title: &str, text: &str, icon: BalloonIcon) {
        let mut data = self.base();
        data.uFlags = NIF_INFO;
        copy_into(&mut data.szInfoTitle, title);
        copy_into(&mut data.szInfo, text);
        data.Anonymous.uTimeout = 5000;
        data.dwInfoFlags = match icon {
            BalloonIcon::Info => NIIF_INFO,
            BalloonIcon::Warning => NIIF_WARNING,
            BalloonIcon::Error => NIIF_ERROR,
        } | NOTIFY_ICON_INFOTIP_FLAGS(0);
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.base();
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }
}

/// トレイのメニューの 1 項目。None は区切り線
pub type MenuItem<'a> = Option<(u32, &'a str)>;

/// カーソルの位置にメニューを出す。選ばれた項目は hwnd に WM_COMMAND（wParam の下位が id）で届く
pub fn show_menu(hwnd: HWND, items: &[MenuItem]) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        append_items(menu, items);
        let mut cursor = Default::default();
        let _ = GetCursorPos(&mut cursor);
        // 前面にしておかないと、メニューの外をクリックしてもメニューが閉じない（TrackPopupMenu の仕様）
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, cursor.x, cursor.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

fn append_items(menu: HMENU, items: &[MenuItem]) {
    for item in items {
        let _ = unsafe {
            match item {
                Some((id, text)) => {
                    AppendMenuW(menu, MF_STRING, *id as usize, &HSTRING::from(*text))
                }
                None => AppendMenuW(menu, MF_SEPARATOR, 0, None),
            }
        };
    }
}
