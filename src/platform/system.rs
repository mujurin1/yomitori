use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, MonitorFromRect,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, GetWindowRect, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOWNORMAL,
};
use windows::core::{HSTRING, w};

use crate::logic::geometry::{PixelPoint, PixelRect};
use crate::logic::settings_store::LocalDateTime;

/// カーソル位置（物理ピクセル。プロセスが Per-Monitor DPI v2 のため）。読めなければ (0, 0)
pub fn cursor_position() -> PixelPoint {
    let mut p = POINT::default();
    match unsafe { GetCursorPos(&mut p) } {
        Ok(()) => PixelPoint::new(p.x, p.y),
        Err(_) => PixelPoint::new(0, 0),
    }
}

/// 全モニターを覆う仮想スクリーン（物理ピクセル）
pub fn virtual_screen() -> PixelRect {
    unsafe {
        PixelRect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

pub fn local_now() -> LocalDateTime {
    let t = unsafe { GetLocalTime() };
    LocalDateTime {
        year: t.wYear,
        month: t.wMonth,
        day: t.wDay,
        hour: t.wHour,
        minute: t.wMinute,
        second: t.wSecond,
    }
}

/// exe のあるディレクトリ
pub fn exe_dir() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(exe.parent().map(Path::to_path_buf).unwrap_or_default())
}

/// 関連付けられたアプリでファイルを開く。失敗したら理由
pub fn open_with_shell(path: &Path) -> Result<(), String> {
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(path),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // 32 以下はエラー（ShellExecute の仕様）
    if result.0 as usize > 32 {
        return Ok(());
    }
    Err(windows::core::Error::from_thread().message())
}

/// 全モニターの矩形（物理ピクセル）
pub fn monitors() -> Vec<PixelRect> {
    unsafe extern "system" fn collect(
        _monitor: HMONITOR,
        _dc: HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<PixelRect>) };
        let r = unsafe { *rect };
        list.push(PixelRect::from_edges(r.left, r.top, r.right, r.bottom));
        true.into()
    }
    let mut list: Vec<PixelRect> = Vec::new();
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut list as *mut _ as isize),
        )
    };
    list
}

/// 点のあるモニター（無ければ最も近いモニター）の作業領域（物理ピクセル）
pub fn work_area_at(point: PixelPoint) -> PixelRect {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: point.x,
                y: point.y,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return virtual_screen();
        }
    }
    let r = info.rcWork;
    PixelRect::from_edges(r.left, r.top, r.right, r.bottom)
}

/// 矩形のあるモニター（無ければ最も近いモニター）の DPI。読めなければ 96
pub fn dpi_at(rect: PixelRect) -> u32 {
    let r = RECT {
        left: rect.left(),
        top: rect.top(),
        right: rect.right(),
        bottom: rect.bottom(),
    };
    let (mut x, mut y) = (96, 96);
    unsafe {
        let monitor = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_err() {
            return 96;
        }
    }
    x
}

/// ウィンドウの矩形（物理ピクセル）。読めなければ None（閉じた後のウィンドウ等。0 にするとそのまま保存されるので分ける）
pub fn window_rect(hwnd: HWND) -> Option<PixelRect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    Some(PixelRect::from_edges(r.left, r.top, r.right, r.bottom))
}
