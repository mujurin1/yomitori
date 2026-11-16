//! Win32 / WinRT の呼び出し。判定は logic に置き、ここは OS との受け渡しだけにする

pub mod capture;
pub mod gdiplus;
pub mod http;
pub mod icon;
pub mod keyboard_hook;
pub mod ocr;
pub mod single_instance;
pub mod system;
pub mod tray;
