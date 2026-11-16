use super::hotkey::Hotkey;
use super::hotkey_filter::{HotkeyAction, HotkeyBinding};

/// 結果ウィンドウの位置と大きさ（物理ピクセル）。left / top が None なら画面中央
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResultWindowPlacement {
    pub left: Option<f64>,
    pub top: Option<f64>,
    pub width: f64,
    pub height: f64,
}

impl ResultWindowPlacement {
    pub const MIN_WIDTH: f64 = 300.0;
    pub const MIN_HEIGHT: f64 = 200.0;
    pub const DEFAULT: Self = Self {
        left: None,
        top: None,
        width: 600.0,
        height: 200.0,
    };
}

impl Default for ResultWindowPlacement {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// settings.json の中身。ホットキーは書かれた文字列のまま持ち、使う時に解釈する
#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub region_hotkey: String,
    pub line_hotkey: String,
    pub show_startup_notification: bool,
    /// OCR の複数行を改行のまま残すか（false なら空白で 1 段落にする）
    pub keep_line_breaks: bool,
    pub result_window: ResultWindowPlacement,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            region_hotkey: "Win+Q".into(),
            line_hotkey: "Win+W".into(),
            show_startup_notification: true,
            keep_line_breaks: true,
            result_window: ResultWindowPlacement::DEFAULT,
        }
    }
}

impl AppSettings {
    pub fn region(&self) -> Result<Hotkey, String> {
        Hotkey::parse(&self.region_hotkey)
    }

    pub fn line(&self) -> Result<Hotkey, String> {
        Hotkey::parse(&self.line_hotkey)
    }

    pub fn bindings(&self) -> Result<Vec<HotkeyBinding>, String> {
        Ok(vec![
            HotkeyBinding {
                hotkey: self.region()?,
                action: HotkeyAction::RegionCapture,
            },
            HotkeyBinding {
                hotkey: self.line()?,
                action: HotkeyAction::CursorLine,
            },
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsParseStatus {
    /// 全項目が揃っていて正しい
    Valid,
    /// 欠けた項目があるだけ（ある項目は全部正しい）。既定値で補った
    MissingFilled,
    /// JSON として読めない・型が違う・値が不正
    Broken,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsParseResult {
    pub status: SettingsParseStatus,
    pub settings: AppSettings,
    pub problem: String,
}
