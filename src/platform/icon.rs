//! exe に埋め込んだアプリのアイコン（assets/icon/yomitori.ico。build.rs が埋め込む）を、DPI に合う大きさで読む

use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::LoadIconWithScaleDown;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, HICON, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SYSTEM_METRICS_INDEX,
};
use windows::core::PCWSTR;

/// アイコンのリソース ID。build.rs の ICON_GROUP_ID と合わせる
const ICON_ID: u16 = 1;

/// 読んだアイコン。drop で捨てる（LoadIconWithScaleDown のアイコンは共有されない）
pub struct AppIcon(pub HICON);

impl Drop for AppIcon {
    fn drop(&mut self) {
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

fn load(cx: SYSTEM_METRICS_INDEX, cy: SYSTEM_METRICS_INDEX, dpi: u32) -> Option<AppIcon> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let (w, h) = (
            GetSystemMetricsForDpi(cx, dpi),
            GetSystemMetricsForDpi(cy, dpi),
        );
        // MAKEINTRESOURCE: 数値の ID をポインタの値として渡す
        LoadIconWithScaleDown(
            Some(module.into()),
            PCWSTR(ICON_ID as usize as *const u16),
            w,
            h,
        )
        .ok()
        .map(AppIcon)
    }
}

/// 小さいアイコン（トレイ・タイトルバー。96 DPI で 16 px）
pub fn small(dpi: u32) -> Option<AppIcon> {
    load(SM_CXSMICON, SM_CYSMICON, dpi)
}

/// 大きいアイコン（Alt+Tab 等。96 DPI で 32 px）
pub fn large(dpi: u32) -> Option<AppIcon> {
    load(SM_CXICON, SM_CYICON, dpi)
}
