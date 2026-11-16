//! settings.json の解釈と書き出し。`//` コメントと末尾のカンマは JSON の誤りとして扱う

use serde_json::{Map, Value};

use super::hotkey::Hotkey;
use super::settings::{
    AppSettings, ResultWindowPlacement, SettingsParseResult, SettingsParseStatus,
};

/// 値が 1 つでもおかしければ全体を Broken とする（その時の settings は既定値）
pub fn parse(json: &str) -> SettingsParseResult {
    let root: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => return broken(format!("JSON として読めません: {e}")),
    };
    let mut reader = Reader { missing: false };
    match reader.read(&root) {
        Ok(settings) => SettingsParseResult {
            status: if reader.missing {
                SettingsParseStatus::MissingFilled
            } else {
                SettingsParseStatus::Valid
            },
            settings,
            problem: String::new(),
        },
        Err(problem) => broken(problem),
    }
}

fn broken(problem: String) -> SettingsParseResult {
    SettingsParseResult {
        status: SettingsParseStatus::Broken,
        settings: AppSettings::default(),
        problem,
    }
}

/// 全項目を決まった順で書く。改行は CRLF、末尾にも改行
pub fn serialize(s: &AppSettings) -> String {
    let p = &s.result_window;
    let lines = [
        "{".to_string(),
        "  \"hotkeys\": {".to_string(),
        format!("    \"region\": {},", string(&s.region_hotkey)),
        format!("    \"line\": {}", string(&s.line_hotkey)),
        "  },".to_string(),
        format!(
            "  \"showStartupNotification\": {},",
            s.show_startup_notification
        ),
        format!("  \"keepLineBreaks\": {},", s.keep_line_breaks),
        "  \"resultWindow\": {".to_string(),
        format!("    \"left\": {},", nullable_number(p.left)),
        format!("    \"top\": {},", nullable_number(p.top)),
        format!("    \"width\": {},", number(p.width)),
        format!("    \"height\": {}", number(p.height)),
        "  }".to_string(),
        "}".to_string(),
    ];
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

fn string(s: &str) -> String {
    serde_json::to_string(s).expect("文字列の書き出しは失敗しない")
}

fn nullable_number(v: Option<f64>) -> String {
    v.map_or_else(|| "null".to_string(), number)
}

/// 整数の値は `600.0` ではなく `600` と書く（user が手で読み書きするファイルのため）
fn number(v: f64) -> String {
    const EXACT_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0; // 2^53
    if v.fract() == 0.0 && v.abs() < EXACT_INTEGER_LIMIT {
        return format!("{}", v as i64);
    }
    // 非有限の値は JSON に書けないので null になり、次の読み込みで壊れている扱いになる
    serde_json::Number::from_f64(v).map_or_else(|| "null".to_string(), |n| n.to_string())
}

struct Reader {
    missing: bool,
}

type ReadResult<T> = Result<T, String>;

impl Reader {
    fn read(&mut self, root: &Value) -> ReadResult<AppSettings> {
        let root = require_object(root, "設定全体")?;
        let d = AppSettings::default();

        let mut region = d.region_hotkey.clone();
        let mut line = d.line_hotkey.clone();
        if let Some(hotkeys) = self.child(root, "hotkeys") {
            let hotkeys = require_object(hotkeys, "hotkeys")?;
            let (r, region_key) = self.hotkey_value(hotkeys, "region", &d.region_hotkey)?;
            let (l, line_key) = self.hotkey_value(hotkeys, "line", &d.line_hotkey)?;
            if region_key == line_key {
                return Err(format!(
                    "hotkeys.region と hotkeys.line が同じです（{r} / {l}）"
                ));
            }
            region = r;
            line = l;
        }

        let mut notify = d.show_startup_notification;
        if let Some(n) = self.child(root, "showStartupNotification") {
            notify = n
                .as_bool()
                .ok_or("showStartupNotification は true か false にしてください")?;
        }

        let mut keep_line_breaks = d.keep_line_breaks;
        if let Some(k) = self.child(root, "keepLineBreaks") {
            keep_line_breaks = k
                .as_bool()
                .ok_or("keepLineBreaks は true か false にしてください")?;
        }

        let mut placement = d.result_window;
        if let Some(rw) = self.child(root, "resultWindow") {
            let rw = require_object(rw, "resultWindow")?;
            placement = ResultWindowPlacement {
                left: self.nullable_number(rw, "left", placement.left)?,
                top: self.nullable_number(rw, "top", placement.top)?,
                width: self.size(
                    rw,
                    "width",
                    placement.width,
                    ResultWindowPlacement::MIN_WIDTH,
                )?,
                height: self.size(
                    rw,
                    "height",
                    placement.height,
                    ResultWindowPlacement::MIN_HEIGHT,
                )?,
            };
        }

        Ok(AppSettings {
            region_hotkey: region,
            line_hotkey: line,
            show_startup_notification: notify,
            keep_line_breaks,
            result_window: placement,
        })
    }

    fn child<'a>(&mut self, parent: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
        let value = parent.get(name);
        if value.is_none() {
            self.missing = true;
        }
        value
    }

    fn hotkey_value(
        &mut self,
        parent: &Map<String, Value>,
        name: &str,
        fallback: &str,
    ) -> ReadResult<(String, Hotkey)> {
        let text = match self.child(parent, name) {
            Some(e) => e
                .as_str()
                .ok_or_else(|| format!("hotkeys.{name} は文字列にしてください"))?
                .to_string(),
            None => fallback.to_string(),
        };
        let hotkey = Hotkey::parse(&text).map_err(|e| format!("hotkeys.{name}: {e}"))?;
        Ok((text, hotkey))
    }

    fn nullable_number(
        &mut self,
        parent: &Map<String, Value>,
        name: &str,
        fallback: Option<f64>,
    ) -> ReadResult<Option<f64>> {
        match self.child(parent, name) {
            None => Ok(fallback),
            Some(Value::Null) => Ok(None),
            Some(e) => finite_number(e)
                .map(Some)
                .ok_or_else(|| format!("resultWindow.{name} は数か null にしてください")),
        }
    }

    fn size(
        &mut self,
        parent: &Map<String, Value>,
        name: &str,
        fallback: f64,
        min: f64,
    ) -> ReadResult<f64> {
        let Some(e) = self.child(parent, name) else {
            return Ok(fallback);
        };
        let v =
            finite_number(e).ok_or_else(|| format!("resultWindow.{name} は数にしてください"))?;
        if v < min {
            return Err(format!(
                "resultWindow.{name} は {min} 以上にしてください（{v}）"
            ));
        }
        Ok(v)
    }
}

fn require_object<'a>(e: &'a Value, name: &str) -> ReadResult<&'a Map<String, Value>> {
    e.as_object()
        .ok_or_else(|| format!("{name} は {{ }} で囲んだオブジェクトにしてください"))
}

fn finite_number(e: &Value) -> Option<f64> {
    e.as_f64().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
        {
          "hotkeys": { "region": "Ctrl+Alt+Q", "line": "F8" },
          "showStartupNotification": false,
          "keepLineBreaks": true,
          "resultWindow": { "left": 100, "top": -20.5, "width": 700, "height": 250 }
        }
        "#;

    fn settings(
        region: &str,
        line: &str,
        notify: bool,
        placement: ResultWindowPlacement,
    ) -> AppSettings {
        AppSettings {
            region_hotkey: region.into(),
            line_hotkey: line.into(),
            show_startup_notification: notify,
            keep_line_breaks: true,
            result_window: placement,
        }
    }

    const fn placement(
        left: Option<f64>,
        top: Option<f64>,
        width: f64,
        height: f64,
    ) -> ResultWindowPlacement {
        ResultWindowPlacement {
            left,
            top,
            width,
            height,
        }
    }

    #[test]
    fn 全項目が正しければそのまま使う() {
        let r = parse(FULL);

        assert_eq!(SettingsParseStatus::Valid, r.status);
        assert_eq!(
            settings(
                "Ctrl+Alt+Q",
                "F8",
                false,
                placement(Some(100.0), Some(-20.5), 700.0, 250.0)
            ),
            r.settings
        );
    }

    #[test]
    fn 位置のnullは正しい値として扱う() {
        let r = parse(&serialize(&AppSettings::default()));

        assert_eq!(SettingsParseStatus::Valid, r.status);
        assert_eq!(AppSettings::default(), r.settings);
    }

    #[test]
    fn 書き出した設定は読み直すと同じになる() {
        let s = settings(
            "Shift+F1",
            "Win+Alt+W",
            false,
            placement(Some(-1920.0), Some(30.0), 640.5, 300.0),
        );

        assert_eq!(
            SettingsParseResult {
                status: SettingsParseStatus::Valid,
                settings: s.clone(),
                problem: String::new()
            },
            parse(&serialize(&s))
        );
    }

    #[test]
    fn 整数の値は小数点を付けずに書く() {
        let text = serialize(&AppSettings::default());

        assert!(text.contains("\"width\": 600,"), "{text}");
        assert!(text.contains("\"height\": 200\r\n"), "{text}");
    }

    #[test]
    fn 改行を残す設定は既定でtrueで書いた値はそのまま() {
        assert!(parse("{}").settings.keep_line_breaks);
        let r = parse(r#"{ "keepLineBreaks": false }"#);
        assert_eq!(SettingsParseStatus::MissingFilled, r.status);
        assert!(!r.settings.keep_line_breaks);
    }

    #[test]
    fn 改行を残す設定は起動時の案内の次に書き読み直すと同じになる() {
        let s = AppSettings {
            keep_line_breaks: false,
            ..AppSettings::default()
        };

        let text = serialize(&s);

        assert!(
            text.contains(
                "  \"showStartupNotification\": true,\r\n  \"keepLineBreaks\": false,\r\n  \"resultWindow\""
            ),
            "{text}"
        );
        assert_eq!(s, parse(&text).settings);
        assert_eq!(SettingsParseStatus::Valid, parse(&text).status);
    }

    #[test]
    fn 欠けた項目は既定値で補う() {
        let cases = [
            r#"{}"#,
            r#"{ "hotkeys": { "region": "Ctrl+Q" } }"#,
            r#"{ "showStartupNotification": false, "resultWindow": { "width": 800 } }"#,
        ];
        for json in cases {
            assert_eq!(
                SettingsParseStatus::MissingFilled,
                parse(json).status,
                "{json}"
            );
        }
    }

    #[test]
    fn 補った値は既定値で書いた値はそのまま() {
        let r = parse(r#"{ "hotkeys": { "region": "Ctrl+Q" }, "resultWindow": { "width": 800 } }"#);

        assert_eq!(
            settings("Ctrl+Q", "Win+W", true, placement(None, None, 800.0, 200.0)),
            r.settings
        );
    }

    #[test]
    fn 値が1つでもおかしければ全体を壊れている扱いにする() {
        let cases = [
            "not json",
            r#"[]"#,
            r#"{ "hotkeys": "Win+Q" }"#,
            r#"{ "hotkeys": { "region": 1 } }"#,
            r#"{ "hotkeys": { "region": "Win+?" } }"#,
            r#"{ "hotkeys": { "region": "Win+W" } }"#,
            r#"{ "hotkeys": { "region": "ctrl+q", "line": "Ctrl+Q" } }"#,
            r#"{ "showStartupNotification": "yes" }"#,
            r#"{ "keepLineBreaks": "yes" }"#,
            r#"{ "keepLineBreaks": null }"#,
            r#"{ "resultWindow": null }"#,
            r#"{ "resultWindow": { "left": "10" } }"#,
            r#"{ "resultWindow": { "width": 299 } }"#,
            r#"{ "resultWindow": { "height": 199.5 } }"#,
            r#"{ "resultWindow": { "width": null } }"#,
        ];
        for json in cases {
            let r = parse(json);

            assert_eq!(SettingsParseStatus::Broken, r.status, "{json}");
            assert_eq!(AppSettings::default(), r.settings, "{json}");
            assert!(!r.problem.is_empty(), "{json}");
        }
    }

    #[test]
    fn コメントと末尾のカンマはjsonの誤りとして壊れている扱いにする() {
        let cases = [
            "{ // コメント\n \"showStartupNotification\": true }",
            "{ /* コメント */ \"showStartupNotification\": true }",
            r#"{ "showStartupNotification": true, }"#,
            r#"{ "hotkeys": { "region": "Win+Q", "line": "Win+W", } }"#,
        ];
        for json in cases {
            let r = parse(json);

            assert_eq!(SettingsParseStatus::Broken, r.status, "{json}");
            assert!(
                r.problem.starts_with("JSON として読めません"),
                "{json}: {}",
                r.problem
            );
        }
    }

    #[test]
    fn 欠けた項目があっても他の項目がおかしければ壊れている() {
        assert_eq!(
            SettingsParseStatus::Broken,
            parse(r#"{ "resultWindow": { "width": 10 } }"#).status
        );
    }

    #[test]
    fn 最小の大きさちょうどは正しい() {
        let json = r#"{ "hotkeys": { "region": "Win+Q", "line": "Win+W" }, "showStartupNotification": true, "keepLineBreaks": true, "resultWindow": { "left": null, "top": null, "width": 300, "height": 200 } }"#;

        assert_eq!(SettingsParseStatus::Valid, parse(json).status);
    }
}
