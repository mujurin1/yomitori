use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::settings::{
    AppSettings, ResultWindowPlacement, SettingsParseResult, SettingsParseStatus,
};
use super::settings_json;

/// 設定ファイルの置き場所への操作（テストで差し替える）
pub trait SettingsFiles {
    fn exists(&self, path: &Path) -> bool;
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, text: &str) -> io::Result<()>;
    /// 名前を変える。移動先が既にあれば失敗する
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RealSettingsFiles;

impl SettingsFiles for RealSettingsFiles {
    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    /// UTF-8 として読む。先頭の BOM は外し、UTF-8 として不正な並びは U+FFFD に置き換える
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        let bytes = fs::read(path)?;
        let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        Ok(String::from_utf8_lossy(body).into_owned())
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        fs::write(path, text)
    }

    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        // std の rename は Windows で移動先を上書きするので先に確かめる（確かめてから移すまでの競合は残る）
        if to.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "移動先が既にあります",
            ));
        }
        fs::rename(from, to)
    }
}

/// 退避ファイルの名前に使うローカル時刻
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

/// 読み込みの結果。notification が Some なら user に知らせる
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsLoadResult {
    pub settings: AppSettings,
    pub notification: Option<String>,
}

impl SettingsLoadResult {
    fn new(settings: AppSettings, notification: Option<String>) -> Self {
        Self {
            settings,
            notification,
        }
    }
}

/// settings.json の読み込みと保存。
/// 壊れたファイルの退避か既定値のファイルの作成に失敗したら、そのプロセスの間は一切書き込まない
pub struct SettingsStore<F: SettingsFiles> {
    path: PathBuf,
    files: F,
    now: Box<dyn Fn() -> LocalDateTime>,
    writes_enabled: bool,
}

impl<F: SettingsFiles> SettingsStore<F> {
    pub const FILE_NAME: &'static str = "settings.json";

    pub fn new(path: PathBuf, files: F, now: impl Fn() -> LocalDateTime + 'static) -> Self {
        Self {
            path,
            files,
            now: Box::new(now),
            writes_enabled: true,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// false になったら、このプロセスの間は設定ファイルに書き込まない（戻らない）
    pub fn writes_enabled(&self) -> bool {
        self.writes_enabled
    }

    /// 起動時と「設定を再読み込み」で同じ処理
    pub fn load(&mut self) -> SettingsLoadResult {
        let defaults = AppSettings::default();

        if !self.files.exists(&self.path) {
            if !self.writes_enabled || self.try_write(&defaults) {
                return SettingsLoadResult::new(defaults, None);
            }
            return SettingsLoadResult::new(
                defaults,
                Some(
                    "設定ファイルを作れませんでした。既定値で動いています（設定は保存されません）"
                        .into(),
                ),
            );
        }

        let parsed = match self.files.read_to_string(&self.path) {
            Ok(text) => settings_json::parse(&text),
            Err(e) => SettingsParseResult {
                status: SettingsParseStatus::Broken,
                settings: defaults,
                problem: format!("読めません: {e}"),
            },
        };

        match parsed.status {
            SettingsParseStatus::Valid => SettingsLoadResult::new(parsed.settings, None),
            SettingsParseStatus::MissingFilled => {
                if !self.writes_enabled || self.try_write(&parsed.settings) {
                    return SettingsLoadResult::new(parsed.settings, None);
                }
                SettingsLoadResult::new(
                    parsed.settings,
                    Some("欠けた設定を補ったファイルを保存できませんでした。以後、設定は保存されません".into()),
                )
            }
            SettingsParseStatus::Broken => self.handle_broken(&parsed.problem),
        }
    }

    fn handle_broken(&mut self, problem: &str) -> SettingsLoadResult {
        let defaults = AppSettings::default();
        if !self.writes_enabled {
            return SettingsLoadResult::new(
                defaults,
                Some(format!(
                    "設定ファイルが壊れています（{problem}）。書き込みを止めているので退避せず、既定値で動いています"
                )),
            );
        }

        let backup = self.backup_path();
        if self.files.rename_new(&self.path, &backup).is_err() {
            self.writes_enabled = false;
            return SettingsLoadResult::new(
                defaults,
                Some("設定ファイルを読めず、退避もできませんでした。既定値で動いています".into()),
            );
        }

        let backup_name = backup
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !self.try_write(&defaults) {
            return SettingsLoadResult::new(
                defaults,
                Some(format!(
                    "設定ファイルが壊れていたので {backup_name} に退避しましたが、既定値のファイルを作れませんでした。既定値で動いています（設定は保存されません）"
                )),
            );
        }
        SettingsLoadResult::new(
            defaults,
            Some(format!(
                "設定ファイルが壊れていたので {backup_name} に退避し、既定値に戻しました（{problem}）"
            )),
        )
    }

    /// `settings.json.bak_yyyyMMdd_HHmmss`。同じ名前があれば末尾に _2, _3 … を付ける
    fn backup_path(&self) -> PathBuf {
        let t = (self.now)();
        let suffix = format!(
            ".bak_{:04}{:02}{:02}_{:02}{:02}{:02}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        );
        let mut base: OsString = self.path.clone().into_os_string();
        base.push(suffix);
        let base = PathBuf::from(base);
        if !self.files.exists(&base) {
            return base;
        }
        (2..)
            .map(|i| {
                let mut candidate = base.clone().into_os_string();
                candidate.push(format!("_{i}"));
                PathBuf::from(candidate)
            })
            .find(|candidate| !self.files.exists(candidate))
            .expect("番号は尽きない")
    }

    /// 結果ウィンドウの位置と大きさを保存する。読み込んだ後に user がファイルを書き換えていてもそれを残すため、
    /// 今のファイルを読み直して resultWindow だけ差し替える。ファイルが壊れていたら触らない
    pub fn save_result_window(&mut self, placement: ResultWindowPlacement) {
        if !self.writes_enabled {
            return;
        }

        let mut current = if self.files.exists(&self.path) {
            let Ok(text) = self.files.read_to_string(&self.path) else {
                return;
            };
            let parsed = settings_json::parse(&text);
            if parsed.status == SettingsParseStatus::Broken {
                return;
            }
            parsed.settings
        } else {
            AppSettings::default()
        };

        // 最小値未満を書くと次の読み込みで壊れている扱いになるので切り上げる
        current.result_window = ResultWindowPlacement {
            width: placement.width.max(ResultWindowPlacement::MIN_WIDTH),
            height: placement.height.max(ResultWindowPlacement::MIN_HEIGHT),
            ..placement
        };
        self.try_write(&current);
    }

    fn try_write(&mut self, settings: &AppSettings) -> bool {
        let ok = self
            .files
            .write(&self.path, &settings_json::serialize(settings))
            .is_ok();
        if !ok {
            self.writes_enabled = false;
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    const PATH: &str = r"C:\app\settings.json";
    const NOW: LocalDateTime = LocalDateTime {
        year: 2026,
        month: 11,
        day: 16,
        hour: 21,
        minute: 5,
        second: 9,
    };

    #[derive(Default)]
    struct FakeState {
        files: HashMap<PathBuf, String>,
        fail_move: bool,
        fail_write: bool,
        writes: usize,
    }

    #[derive(Clone, Default)]
    struct FakeFiles(Rc<RefCell<FakeState>>);

    impl FakeFiles {
        fn get(&self, path: &str) -> Option<String> {
            self.0.borrow().files.get(Path::new(path)).cloned()
        }

        fn set(&self, path: &str, text: &str) {
            self.0
                .borrow_mut()
                .files
                .insert(PathBuf::from(path), text.into());
        }

        fn state(&self) -> std::cell::RefMut<'_, FakeState> {
            self.0.borrow_mut()
        }
    }

    impl SettingsFiles for FakeFiles {
        fn exists(&self, path: &Path) -> bool {
            self.0.borrow().files.contains_key(path)
        }

        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.0
                .borrow()
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }

        fn write(&self, path: &Path, text: &str) -> io::Result<()> {
            let mut s = self.0.borrow_mut();
            if s.fail_write {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            s.writes += 1;
            s.files.insert(path.to_path_buf(), text.into());
            Ok(())
        }

        fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
            let mut s = self.0.borrow_mut();
            if s.fail_move {
                return Err(io::Error::other("locked"));
            }
            if s.files.contains_key(to) {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            let text = s.files.remove(from).ok_or(io::ErrorKind::NotFound)?;
            s.files.insert(to.to_path_buf(), text);
            Ok(())
        }
    }

    fn create(content: Option<&str>) -> (SettingsStore<FakeFiles>, FakeFiles) {
        let files = FakeFiles::default();
        if let Some(c) = content {
            files.set(PATH, c);
        }
        (
            SettingsStore::new(PathBuf::from(PATH), files.clone(), || NOW),
            files,
        )
    }

    fn with_region(region: &str) -> AppSettings {
        AppSettings {
            region_hotkey: region.into(),
            ..AppSettings::default()
        }
    }

    fn backup(suffix: &str) -> String {
        format!("{PATH}{suffix}")
    }

    const fn placement(left: f64, top: f64, width: f64, height: f64) -> ResultWindowPlacement {
        ResultWindowPlacement {
            left: Some(left),
            top: Some(top),
            width,
            height,
        }
    }

    #[test]
    fn ファイルが無ければ既定値で作り通知しない() {
        let (mut store, files) = create(None);

        let r = store.load();

        assert_eq!(SettingsLoadResult::new(AppSettings::default(), None), r);
        assert_eq!(
            Some(settings_json::serialize(&AppSettings::default())),
            files.get(PATH)
        );
    }

    #[test]
    fn 正しいファイルはそのまま使い書き直さない() {
        let (mut store, files) = create(Some(&settings_json::serialize(&with_region("Ctrl+Q"))));

        let r = store.load();

        assert_eq!("Ctrl+Q", r.settings.region_hotkey);
        assert_eq!(None, r.notification);
        assert_eq!(0, files.state().writes);
    }

    #[test]
    fn 欠けた項目は補って全項目を書いたファイルに保存し直し通知しない() {
        let (mut store, files) = create(Some(r#"{ "hotkeys": { "region": "Ctrl+Q" } }"#));

        let r = store.load();

        let expected = with_region("Ctrl+Q");
        assert_eq!(SettingsLoadResult::new(expected.clone(), None), r);
        assert_eq!(Some(settings_json::serialize(&expected)), files.get(PATH));
    }

    #[test]
    fn 壊れたファイルは日時付きの名前に退避して既定値を作り退避先を通知する() {
        let (mut store, files) = create(Some("broken"));

        let r = store.load();

        assert_eq!(
            Some("broken".into()),
            files.get(&backup(".bak_20261116_210509"))
        );
        assert_eq!(
            Some(settings_json::serialize(&AppSettings::default())),
            files.get(PATH)
        );
        assert_eq!(AppSettings::default(), r.settings);
        assert!(
            r.notification
                .unwrap()
                .contains("settings.json.bak_20261116_210509")
        );
        assert!(store.writes_enabled());
    }

    #[test]
    fn 退避先の名前が既にあれば番号を付ける() {
        let (mut store, files) = create(Some("broken"));
        files.set(&backup(".bak_20261116_210509"), "older");

        store.load();

        assert_eq!(
            Some("older".into()),
            files.get(&backup(".bak_20261116_210509"))
        );
        assert_eq!(
            Some("broken".into()),
            files.get(&backup(".bak_20261116_210509_2"))
        );
    }

    #[test]
    fn 退避に失敗したら壊れたファイルに触らず既定値で動き以後は書き込まない() {
        let (mut store, files) = create(Some("broken"));
        files.state().fail_move = true;

        let r = store.load();

        assert_eq!(AppSettings::default(), r.settings);
        assert_eq!(
            Some("設定ファイルを読めず、退避もできませんでした。既定値で動いています".into()),
            r.notification
        );
        assert!(!store.writes_enabled());
        assert_eq!(Some("broken".into()), files.get(PATH));
        assert_eq!(0, files.state().writes);

        store.save_result_window(placement(1.0, 2.0, 600.0, 200.0));
        files.state().fail_move = false;
        store.load();

        assert_eq!(0, files.state().writes);
        assert_eq!(Some("broken".into()), files.get(PATH));
        assert_eq!(1, files.state().files.len());
    }

    #[test]
    fn 既定値のファイルを作れなければ通知し以後は書き込まない() {
        let (mut store, files) = create(None);
        files.state().fail_write = true;

        let r = store.load();

        assert_eq!(AppSettings::default(), r.settings);
        assert!(r.notification.is_some());
        assert!(!store.writes_enabled());

        files.state().fail_write = false;
        store.save_result_window(placement(1.0, 2.0, 600.0, 200.0));
        store.load();
        assert!(files.state().files.is_empty());
    }

    #[test]
    fn 退避の後に既定値のファイルを作れなければ通知し以後は書き込まない() {
        let (mut store, files) = create(Some("broken"));
        files.state().fail_write = true;

        let r = store.load();

        assert!(
            r.notification
                .unwrap()
                .contains("settings.json.bak_20261116_210509")
        );
        assert!(!store.writes_enabled());
        assert_eq!(None, files.get(PATH));
    }

    #[test]
    fn 結果ウィンドウの位置は今のファイルを読み直して他の項目を残したまま保存する() {
        let (mut store, files) = create(None);
        store.load();
        // 読み込んだ後に user がファイルを書き換えた
        let edited = AppSettings {
            line_hotkey: "F8".into(),
            ..AppSettings::default()
        };
        files.set(PATH, &settings_json::serialize(&edited));

        store.save_result_window(placement(10.0, 20.0, 640.0, 260.0));

        let saved = settings_json::parse(&files.get(PATH).unwrap()).settings;
        assert_eq!("F8", saved.line_hotkey);
        assert_eq!(placement(10.0, 20.0, 640.0, 260.0), saved.result_window);
    }

    #[test]
    fn 今のファイルが壊れていたら位置を保存しない() {
        let (mut store, files) = create(None);
        store.load();
        files.set(PATH, "{ editing");

        store.save_result_window(placement(10.0, 20.0, 640.0, 260.0));

        assert_eq!(Some("{ editing".into()), files.get(PATH));
    }

    #[test]
    fn 最小値未満の大きさは最小値に切り上げて保存する() {
        let (mut store, files) = create(None);
        store.load();

        store.save_result_window(placement(10.0, 20.0, 100.0, 50.0));

        let r = settings_json::parse(&files.get(PATH).unwrap());
        assert_eq!(SettingsParseStatus::Valid, r.status);
        assert_eq!(
            placement(10.0, 20.0, 300.0, 200.0),
            r.settings.result_window
        );
    }

    /// 実ファイルの検証用の一時ディレクトリ。drop で消す
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("yomitori-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn 実ファイルの名前の変更は移動先があれば失敗し両方を残す() {
        let dir = TempDir::new("rename");
        let from = dir.0.join("settings.json");
        let to = dir.0.join("settings.json.bak");
        fs::write(&from, "new").unwrap();
        fs::write(&to, "old").unwrap();

        assert!(RealSettingsFiles.rename_new(&from, &to).is_err());
        assert_eq!("new", fs::read_to_string(&from).unwrap());
        assert_eq!("old", fs::read_to_string(&to).unwrap());

        fs::remove_file(&to).unwrap();
        RealSettingsFiles.rename_new(&from, &to).unwrap();
        assert!(!RealSettingsFiles.exists(&from));
        assert_eq!("new", fs::read_to_string(&to).unwrap());
    }

    #[test]
    fn 実ファイルは先頭のbomを外して読む() {
        let dir = TempDir::new("bom");
        let path = dir.0.join("settings.json");
        fs::write(&path, b"\xEF\xBB\xBF{}").unwrap();

        assert_eq!("{}", RealSettingsFiles.read_to_string(&path).unwrap());
    }
}
