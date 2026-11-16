//! Windows.Media.Ocr で英語を認識する。エンジンは専用スレッド 1 本で持ち、要求を 1 件ずつ処理する
//! （C# 版は SemaphoreSlim で 1 件ずつにしていた）

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::core::HSTRING;

use super::capture::BgraImage;
use super::gdiplus::{GdiPlus, scale_image};
use crate::logic::cursor_line::OcrLineInfo;
use crate::logic::geometry::PixelRect;
use crate::logic::ocr_support;

pub type OcrResult = Result<Vec<OcrLineInfo>, String>;
type Done = Box<dyn FnOnce(OcrResult) + Send>;

pub struct OcrService {
    jobs: Option<mpsc::Sender<(BgraImage, Done)>>,
    thread: Option<JoinHandle<()>>,
    error: Option<String>,
}

impl OcrService {
    /// エンジンを用意する。使えなければ error() に理由が入り、recognize は理由で失敗する
    pub fn start() -> Self {
        let (jobs, received) = mpsc::channel::<(BgraImage, Done)>();
        let (ready, ready_result) = mpsc::channel::<Option<String>>();
        let spawned = thread::Builder::new()
            .name("yomitori ocr".into())
            .spawn(move || {
                let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
                let gdiplus = GdiPlus::start();
                match (create_engine(), gdiplus) {
                    (Ok(engine), Ok(_gdiplus)) => {
                        let _ = ready.send(None);
                        for (image, done) in received {
                            done(recognize(&engine, &image));
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        let _ = ready.send(Some(e));
                    }
                }
                if initialized {
                    unsafe { RoUninitialize() };
                }
            });
        match spawned {
            Ok(thread) => {
                let error = ready_result
                    .recv()
                    .unwrap_or_else(|_| Some("OCR のスレッドが止まりました".into()));
                Self {
                    jobs: error.is_none().then_some(jobs),
                    thread: Some(thread),
                    error,
                }
            }
            Err(e) => Self {
                jobs: None,
                thread: None,
                error: Some(format!("OCR のスレッドを作れませんでした: {e}")),
            },
        }
    }

    /// OCR が使えない理由。使えるなら None
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 画像を認識する。done は OCR のスレッドから呼ばれる。行の矩形は元の画像の物理ピクセル座標
    pub fn recognize(&self, image: BgraImage, done: impl FnOnce(OcrResult) + Send + 'static) {
        let done: Done = Box::new(done);
        let unavailable = || {
            self.error
                .clone()
                .unwrap_or_else(|| "OCR のスレッドが止まりました".into())
        };
        match &self.jobs {
            Some(jobs) => {
                if let Err(mpsc::SendError((_, done))) = jobs.send((image, done)) {
                    done(Err(unavailable()));
                }
            }
            None => done(Err(unavailable())),
        }
    }
}

impl OcrService {
    /// recognize の結果を待って返す（UI スレッドで呼ぶと OCR の間 UI が止まる）
    pub fn recognize_blocking(&self, image: BgraImage) -> OcrResult {
        let (sender, result) = mpsc::channel();
        self.recognize(image, move |r| {
            let _ = sender.send(r);
        });
        result
            .recv()
            .unwrap_or_else(|_| Err("OCR のスレッドが止まりました".into()))
    }
}

impl Drop for OcrService {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn create_engine() -> Result<OcrEngine, String> {
    let tags: Vec<String> = OcrEngine::AvailableRecognizerLanguages()
        .and_then(|languages| {
            languages
                .into_iter()
                .map(|l| l.LanguageTag().map(|t| t.to_string()))
                .collect()
        })
        .map_err(|e| format!("OCR の言語を調べられませんでした: {}", e.message()))?;
    let Some(tag) = ocr_support::pick_english(&tags) else {
        return Err(
            "英語の OCR が使えません。Windows の設定 > 時刻と言語 > 言語と地域 で英語を追加してください".into(),
        );
    };
    Language::CreateLanguage(&HSTRING::from(tag.as_str()))
        .and_then(|language| OcrEngine::TryCreateFromLanguage(&language))
        .map_err(|_| format!("OCR エンジンを作れませんでした（{tag}）"))
}

fn recognize(engine: &OcrEngine, image: &BgraImage) -> OcrResult {
    let max = OcrEngine::MaxImageDimension().map_err(|e| e.message())?;
    let scale = ocr_support::compute_scale(image.width, image.height, max as i32);
    let scaled = scale_image(image, scale)?;
    lines(engine, &scaled, scale).map_err(|e| e.message())
}

fn lines(
    engine: &OcrEngine,
    image: &BgraImage,
    scale: i32,
) -> windows::core::Result<Vec<OcrLineInfo>> {
    let buffer = CryptographicBuffer::CreateFromByteArray(&image.pixels)?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        image.width,
        image.height,
        BitmapAlphaMode::Premultiplied,
    )?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;

    // OcrLine には矩形が無いので、行の矩形は Words の BoundingRect の外接矩形で作る。
    // Win+W の行選択と枠の表示はこの矩形に頼る
    let scale = f64::from(scale);
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let words = line.Words()?;
        if words.Size()? == 0 {
            continue;
        }
        let (mut left, mut top, mut right, mut bottom) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for word in words {
            let r = word.BoundingRect()?;
            left = left.min(f64::from(r.X));
            top = top.min(f64::from(r.Y));
            right = right.max(f64::from(r.X + r.Width));
            bottom = bottom.max(f64::from(r.Y + r.Height));
        }
        let bounds = PixelRect::from_edges(
            (left / scale).floor() as i32,
            (top / scale).floor() as i32,
            (right / scale).ceil() as i32,
            (bottom / scale).ceil() as i32,
        );
        lines.push(OcrLineInfo {
            text: line.Text()?.to_string(),
            bounds,
        });
    }
    Ok(lines)
}
