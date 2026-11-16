//! 画面上の英語を OCR で読んで日本語に訳す常駐ツール

/// 常駐アプリの組み立て（UI スレッド）
pub mod app;
/// 画面・Win32・WinRT に依存しないロジック。テストはここに置く
pub mod logic;
/// Win32 / WinRT の呼び出し
pub mod platform;
