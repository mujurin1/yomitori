use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// 取り消しの合図。複製は同じ合図を共有する
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 翻訳器。将来 LLM API 等に差し替えるための口。
/// ブロックしてよい（呼び出し側が UI 以外のスレッドで呼ぶ）。失敗は user に見せる文
pub trait Translator: Send + Sync {
    fn translate(
        &self,
        text: &str,
        from: &str,
        to: &str,
        cancel: &CancelToken,
    ) -> Result<String, String>;
}

/// Google 翻訳の非公式の窓口の URL と応答
pub mod google {
    use serde_json::Value;

    /// 送る窓口。GoogleTranslator は ORDER の順に試す
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Endpoint {
        GtxHttps,
        DictChromeExHttps,
        /// 平文で送る。許された時だけ、https の窓口が全部制限された後に使う。ORDER の最後に置く
        GtxHttp,
    }

    impl Endpoint {
        pub const ORDER: [Endpoint; 3] = [
            Endpoint::GtxHttps,
            Endpoint::DictChromeExHttps,
            Endpoint::GtxHttp,
        ];

        /// クエリ値はすべて URL エンコードする（# + & 改行・非 ASCII が壊れないように）
        pub fn build_uri(self, text: &str, from: &str, to: &str) -> String {
            let (base, client, extra) = match self {
                Endpoint::GtxHttps => (
                    "https://translate.googleapis.com/translate_a/single",
                    "gtx",
                    "&dt=t",
                ),
                Endpoint::DictChromeExHttps => (
                    "https://clients5.google.com/translate_a/t",
                    "dict-chrome-ex",
                    "",
                ),
                Endpoint::GtxHttp => (
                    "http://translate.googleapis.com/translate_a/single",
                    "gtx",
                    "&dt=t",
                ),
            };
            format!(
                "{base}?client={client}&sl={}&tl={}{extra}&q={}",
                escape_data_string(from),
                escape_data_string(to),
                escape_data_string(text)
            )
        }

        pub fn parse_response(self, json: &str) -> Result<String, String> {
            let root: Value = serde_json::from_str(json)
                .map_err(|_| "翻訳の応答が JSON ではありません".to_string())?;
            match self {
                Endpoint::GtxHttps | Endpoint::GtxHttp => parse_gtx(&root),
                Endpoint::DictChromeExHttps => parse_dict_chrome_ex(&root),
            }
        }
    }

    /// RFC 3986 の非予約文字（英数字と `-._~`）以外を UTF-8 の %XX にする
    fn escape_data_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                out.push(char::from(b));
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    const UNEXPECTED_SHAPE: &str = "翻訳の応答の形が想定と違います";

    /// [0][i][0] を順に連結する
    fn parse_gtx(root: &Value) -> Result<String, String> {
        let segments = root
            .as_array()
            .and_then(|a| a.first())
            .ok_or(UNEXPECTED_SHAPE)?;
        if segments.is_null() {
            return Ok(String::new());
        }
        let segments = segments.as_array().ok_or(UNEXPECTED_SHAPE)?;
        Ok(segments
            .iter()
            .filter_map(|segment| segment.as_array()?.first()?.as_str())
            .collect())
    }

    /// 言語を指定すると ["訳"]、sl=auto だと [["訳","検出した言語"]]
    fn parse_dict_chrome_ex(root: &Value) -> Result<String, String> {
        let first = root
            .as_array()
            .and_then(|a| a.first())
            .ok_or(UNEXPECTED_SHAPE)?;
        let text = match first {
            Value::String(s) => Some(s.as_str()),
            Value::Array(a) => a.first().and_then(Value::as_str),
            _ => None,
        };
        text.map(str::to_string)
            .ok_or_else(|| UNEXPECTED_SHAPE.to_string())
    }
}

/// HTTP の応答。status は数値のステータスコード
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// timeout の間に応答の本文まで読み終わらなかった
    Timeout,
    /// それ以外の失敗。user に見せる文
    Other(String),
}

/// HTTP の GET（テストで差し替える）
pub trait HttpGet: Send + Sync {
    fn get(&self, url: &str, timeout: Duration) -> Result<HttpResponse, HttpError>;
}

/// Google 翻訳の窓口を順に試す。制限（429・503）を返されたら次の窓口へ移る。
/// 時間切れ等の他の失敗では移らない（窓口の数だけ待たせない）
pub struct GoogleTranslator<H: HttpGet> {
    http: H,
    /// 翻訳のスレッドから読み、設定の再読み込みで UI スレッドから書く
    allow_http: AtomicBool,
}

impl<H: HttpGet> GoogleTranslator<H> {
    /// 窓口 1 つあたり
    pub const TIMEOUT: Duration = Duration::from_secs(5);

    /// http の窓口は set_allow_http_as_last_resort で許すまで使わない
    pub fn new(http: H) -> Self {
        Self {
            http,
            allow_http: AtomicBool::new(false),
        }
    }

    pub fn set_allow_http_as_last_resort(&self, allow: bool) {
        self.allow_http.store(allow, Ordering::SeqCst);
    }

    fn endpoints(&self) -> &'static [google::Endpoint] {
        let all = &google::Endpoint::ORDER;
        if self.allow_http.load(Ordering::SeqCst) {
            all
        } else {
            &all[..2]
        }
    }

    fn translate_at(
        &self,
        endpoint: google::Endpoint,
        text: &str,
        from: &str,
        to: &str,
    ) -> Result<String, Failure> {
        let response = self
            .http
            .get(&endpoint.build_uri(text, from, to), Self::TIMEOUT)
            .map_err(|e| match e {
                HttpError::Timeout => Failure::Other(format!(
                    "翻訳が {} 秒以内に終わりませんでした",
                    Self::TIMEOUT.as_secs()
                )),
                HttpError::Other(message) => Failure::Other(message),
            })?;
        if !(200..300).contains(&response.status) {
            let message = format!(
                "翻訳サーバーがエラーを返しました: {} {}",
                response.status, response.reason
            );
            return Err(if matches!(response.status, 429 | 503) {
                Failure::Limited(message)
            } else {
                Failure::Other(message)
            });
        }
        endpoint
            .parse_response(&response.body)
            .map_err(Failure::Other)
    }
}

enum Failure {
    Limited(String),
    Other(String),
}

impl<H: HttpGet> Translator for GoogleTranslator<H> {
    /// 送り始めた要求は止められないので、取り消しは窓口を移る前にだけ見る。取り消した翻訳の結果は LatestTranslation が捨てる
    fn translate(
        &self,
        text: &str,
        from: &str,
        to: &str,
        cancel: &CancelToken,
    ) -> Result<String, String> {
        let mut last = String::new();
        for &endpoint in self.endpoints() {
            if cancel.is_cancelled() {
                break;
            }
            match self.translate_at(endpoint, text, from, to) {
                Ok(translated) => return Ok(translated),
                Err(Failure::Limited(message)) => last = message,
                Err(Failure::Other(message)) => return Err(message),
            }
        }
        Err(last)
    }
}

/// 翻訳 1 回の結果
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranslationOutcome {
    Success(String),
    Failure(String),
    /// 新しい翻訳に置き換えられた。表示を更新してはいけない
    Superseded,
}

/// 新しい翻訳を始めたら前の翻訳を取り消す
pub struct LatestTranslation {
    translator: Arc<dyn Translator>,
    from: String,
    to: String,
    current: Mutex<Option<CancelToken>>,
}

impl LatestTranslation {
    pub fn new(translator: Arc<dyn Translator>, from: &str, to: &str) -> Self {
        Self {
            translator,
            from: from.into(),
            to: to.into(),
            current: Mutex::new(None),
        }
    }

    /// 進行中の翻訳を取り消す（その結果は Superseded になる）
    pub fn cancel(&self) {
        if let Some(token) = self.lock_current().take() {
            token.cancel();
        }
    }

    /// 前の翻訳を取り消し、新しい翻訳を用意する。表示の順を決めるので UI スレッドで呼ぶ。
    /// 翻訳そのものは返した値の run を別スレッドで呼んで走らせる
    pub fn start(&self, text: &str) -> PendingTranslation {
        let token = CancelToken::new();
        if let Some(previous) = self.lock_current().replace(token.clone()) {
            previous.cancel();
        }
        PendingTranslation {
            translator: Arc::clone(&self.translator),
            text: text.into(),
            from: self.from.clone(),
            to: self.to.clone(),
            token,
        }
    }

    fn lock_current(&self) -> std::sync::MutexGuard<'_, Option<CancelToken>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// start で用意した 1 回の翻訳
pub struct PendingTranslation {
    translator: Arc<dyn Translator>,
    text: String,
    from: String,
    to: String,
    token: CancelToken,
}

impl PendingTranslation {
    /// 翻訳を走らせる（ブロックする）。走っている間に取り消されたら結果に関わらず Superseded
    pub fn run(&self) -> TranslationOutcome {
        let result = self
            .translator
            .translate(&self.text, &self.from, &self.to, &self.token);
        if self.token.is_cancelled() {
            return TranslationOutcome::Superseded;
        }
        match result {
            Ok(text) => TranslationOutcome::Success(text),
            Err(message) => TranslationOutcome::Failure(message),
        }
    }

    /// run が返った後、UI スレッドで結果を表示する直前に確かめる。
    /// run が返ってから届くまでの間に新しい翻訳が始まっていれば true
    pub fn is_superseded(&self) -> bool {
        self.token.is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::thread;

    const WAIT: Duration = Duration::from_secs(5);

    fn query_value(uri: &str, key: &str) -> String {
        let (_, query) = uri.split_once('?').expect("クエリがある");
        query
            .split('&')
            .find_map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (k == key).then(|| v.to_string())
            })
            .unwrap_or_else(|| panic!("{key} が無い: {uri}"))
    }

    fn unescape(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn qの値はデコードすると元の文字列に戻る() {
        let cases = [
            "C# is fun",
            "1 + 1 = 2",
            "Tom & Jerry",
            "日本語の文",
            "line1\nline2\r\nline3",
            "100% sure? a=b;c/d",
        ];
        for text in cases {
            for endpoint in google::Endpoint::ORDER {
                let uri = endpoint.build_uri(text, "en", "ja");

                assert_eq!(
                    text,
                    unescape(&query_value(&uri, "q")),
                    "{endpoint:?} {text:?}"
                );
            }
        }
    }

    #[test]
    fn 区切りとして働く記号は生のまま入らない() {
        for endpoint in google::Endpoint::ORDER {
            let uri = endpoint.build_uri("a#b+c&d=e f", "en", "ja");

            let q = query_value(&uri, "q");
            for c in ['#', '+', '&', '=', ' '] {
                assert!(!q.contains(c), "{endpoint:?} {c:?} in {q}");
            }
            assert!(!uri.contains('#'), "{uri}");
        }
    }

    #[test]
    fn 窓口ごとに決まった場所と言語のパラメータで送る() {
        use google::Endpoint::*;
        let cases = [
            (
                GtxHttps,
                "https://translate.googleapis.com/translate_a/single",
                "gtx",
            ),
            (
                DictChromeExHttps,
                "https://clients5.google.com/translate_a/t",
                "dict-chrome-ex",
            ),
            (
                GtxHttp,
                "http://translate.googleapis.com/translate_a/single",
                "gtx",
            ),
        ];
        for (endpoint, base, client) in cases {
            let uri = endpoint.build_uri("hello", "en", "ja");

            assert_eq!(Some(base), uri.split('?').next(), "{endpoint:?}");
            assert_eq!(client, query_value(&uri, "client"), "{endpoint:?}");
            assert_eq!("en", query_value(&uri, "sl"), "{endpoint:?}");
            assert_eq!("ja", query_value(&uri, "tl"), "{endpoint:?}");
        }
        assert_eq!("t", query_value(&GtxHttps.build_uri("x", "en", "ja"), "dt"));
    }

    #[test]
    fn 応答から訳文を取り出す() {
        use google::Endpoint::*;
        let cases = [
            (
                GtxHttps,
                r#"[[["こんにちは。","Hello.",null,null,10],["世界。","World.",null,null,10]],null,"en"]"#,
                "こんにちは。世界。",
            ),
            (
                GtxHttp,
                r#"[[["A",null],[null,"x"],[],["B"]],null,"en"]"#,
                "AB",
            ),
            (GtxHttps, r#"[null,null,"en"]"#, ""),
            (DictChromeExHttps, r#"["一行目\n二行目"]"#, "一行目\n二行目"),
            (DictChromeExHttps, r#"[["こんにちは","en"]]"#, "こんにちは"),
        ];
        for (endpoint, json, expected) in cases {
            assert_eq!(
                Ok(expected.to_string()),
                endpoint.parse_response(json),
                "{endpoint:?} {json}"
            );
        }
    }

    #[test]
    fn 形が違う応答は失敗にする() {
        for endpoint in google::Endpoint::ORDER {
            for json in ["not json", "{}", "[]", "[1]"] {
                assert!(
                    endpoint.parse_response(json).is_err(),
                    "{endpoint:?} {json}"
                );
            }
        }
    }

    /// 呼ばれた順に responses を返す
    struct StubHttp {
        responses: Mutex<Vec<Result<HttpResponse, HttpError>>>,
        requests: Mutex<Vec<(String, Duration)>>,
    }

    impl StubHttp {
        fn new(responses: Vec<Result<HttpResponse, HttpError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().rev().collect()),
                requests: Mutex::default(),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|(url, _)| url.clone())
                .collect()
        }
    }

    impl HttpGet for &StubHttp {
        fn get(&self, url: &str, timeout: Duration) -> Result<HttpResponse, HttpError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_string(), timeout));
            self.responses
                .lock()
                .unwrap()
                .pop()
                .expect("想定より多く送った")
        }
    }

    fn ok(status: u16, reason: &str, body: &str) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status,
            reason: reason.into(),
            body: body.into(),
        })
    }

    const GTX_OK: &str = r#"[[["訳","src"]],null,"en"]"#;

    fn uris(endpoints: &[google::Endpoint], text: &str) -> Vec<String> {
        endpoints
            .iter()
            .map(|e| e.build_uri(text, "en", "ja"))
            .collect()
    }

    #[test]
    fn 最初はhttpsのgtxに送り応答の訳文を返す() {
        let http = StubHttp::new(vec![ok(200, "OK", GTX_OK)]);

        let result =
            GoogleTranslator::new(&http).translate("C# & F#", "en", "ja", &CancelToken::new());

        assert_eq!(Ok("訳".to_string()), result);
        assert_eq!(uris(&[google::Endpoint::GtxHttps], "C# & F#"), http.urls());
    }

    fn translator(http: &StubHttp, allow_http: bool) -> GoogleTranslator<&StubHttp> {
        let translator = GoogleTranslator::new(http);
        translator.set_allow_http_as_last_resort(allow_http);
        translator
    }

    #[test]
    fn 制限を返されたら次の窓口へ順に移る() {
        use google::Endpoint::*;
        let cases = [
            (
                vec![ok(429, "Too Many Requests", ""), ok(200, "OK", r#"["訳"]"#)],
                2,
            ),
            (
                vec![
                    ok(503, "Service Unavailable", ""),
                    ok(429, "Too Many Requests", ""),
                    ok(200, "OK", GTX_OK),
                ],
                3,
            ),
        ];
        for (responses, sent) in cases {
            let http = StubHttp::new(responses);

            let result = translator(&http, true).translate("x", "en", "ja", &CancelToken::new());

            assert_eq!(Ok("訳".to_string()), result, "{sent}");
            assert_eq!(
                uris(&[GtxHttps, DictChromeExHttps, GtxHttp][..sent], "x"),
                http.urls()
            );
        }
    }

    #[test]
    fn 全部の窓口に制限されたら最後のステータスを添えた失敗にする() {
        for (allow_http, sent) in [(true, 3), (false, 2)] {
            let http = StubHttp::new(vec![ok(429, "Too Many Requests", ""); sent]);

            let error = translator(&http, allow_http)
                .translate("x", "en", "ja", &CancelToken::new())
                .unwrap_err();

            assert!(error.contains("429"), "{allow_http} {error}");
            assert_eq!(sent, http.urls().len(), "{allow_http}");
        }
    }

    #[test]
    fn httpは許されるまでどの窓口でも使わない() {
        let http = StubHttp::new(vec![ok(429, "Too Many Requests", ""); 2]);

        let _ = GoogleTranslator::new(&http).translate("x", "en", "ja", &CancelToken::new());

        let urls = http.urls();
        assert_eq!(2, urls.len());
        assert!(urls.iter().all(|u| u.starts_with("https://")), "{urls:?}");
    }

    #[test]
    fn 制限以外の失敗では次の窓口へ移らない() {
        let cases = [
            ok(500, "Internal Server Error", ""),
            ok(200, "OK", "not json"),
            Err(HttpError::Timeout),
            Err(HttpError::Other("接続できません".into())),
        ];
        for response in cases {
            let http = StubHttp::new(vec![response.clone()]);

            let result =
                GoogleTranslator::new(&http).translate("x", "en", "ja", &CancelToken::new());

            assert!(result.is_err(), "{response:?}");
            assert_eq!(1, http.urls().len(), "{response:?}");
        }
    }

    #[test]
    fn 取り消された後はどの窓口へも送らない() {
        let http = StubHttp::new(vec![ok(429, "Too Many Requests", "")]);
        let cancel = CancelToken::new();
        cancel.cancel();

        let _ = GoogleTranslator::new(&http).translate("x", "en", "ja", &cancel);

        assert!(http.urls().is_empty());
    }

    #[test]
    fn 時間切れは5秒で区切り秒数を添えた失敗にする() {
        let http = StubHttp::new(vec![Err(HttpError::Timeout)]);

        let error = GoogleTranslator::new(&http)
            .translate("x", "en", "ja", &CancelToken::new())
            .unwrap_err();

        assert_eq!("翻訳が 5 秒以内に終わりませんでした", error);
        assert_eq!(Duration::from_secs(5), http.requests.lock().unwrap()[0].1);
    }

    /// 呼ばれるたびに Call を送り、テストが reply で答えるか取り消されるまでブロックする
    struct ControlledTranslator {
        calls: Mutex<mpsc::Sender<Call>>,
    }

    struct Call {
        text: String,
        token: CancelToken,
        reply: mpsc::Sender<Result<String, String>>,
    }

    impl Translator for ControlledTranslator {
        fn translate(
            &self,
            text: &str,
            _from: &str,
            _to: &str,
            cancel: &CancelToken,
        ) -> Result<String, String> {
            let (reply, replies) = mpsc::channel();
            self.calls
                .lock()
                .unwrap()
                .send(Call {
                    text: text.into(),
                    token: cancel.clone(),
                    reply,
                })
                .unwrap();
            loop {
                if cancel.is_cancelled() {
                    return Err("canceled".into());
                }
                match replies.recv_timeout(Duration::from_millis(5)) {
                    Ok(result) => return result,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return Err("abandoned".into()),
                }
            }
        }
    }

    fn controlled() -> (LatestTranslation, mpsc::Receiver<Call>) {
        let (calls, received) = mpsc::channel();
        let translator = Arc::new(ControlledTranslator {
            calls: Mutex::new(calls),
        });
        (LatestTranslation::new(translator, "en", "ja"), received)
    }

    fn spawn_run(pending: PendingTranslation) -> thread::JoinHandle<TranslationOutcome> {
        thread::spawn(move || pending.run())
    }

    struct FailingTranslator(&'static str);

    impl Translator for FailingTranslator {
        fn translate(&self, _: &str, _: &str, _: &str, _: &CancelToken) -> Result<String, String> {
            Err(self.0.into())
        }
    }

    #[test]
    fn 成功すれば訳文を返す() {
        let (latest, calls) = controlled();

        let running = spawn_run(latest.start("hello"));
        let call = calls.recv_timeout(WAIT).unwrap();
        assert_eq!("hello", call.text);
        call.reply.send(Ok("こんにちは".into())).unwrap();

        assert_eq!(
            TranslationOutcome::Success("こんにちは".into()),
            running.join().unwrap()
        );
    }

    #[test]
    fn 新しい翻訳を始めると前の要求はキャンセルされ置き換え扱いになる() {
        let (latest, calls) = controlled();

        let first = spawn_run(latest.start("one"));
        let first_call = calls.recv_timeout(WAIT).unwrap();
        let second = spawn_run(latest.start("two"));
        let second_call = calls.recv_timeout(WAIT).unwrap();

        assert!(first_call.token.is_cancelled());
        assert!(!second_call.token.is_cancelled());
        assert_eq!(TranslationOutcome::Superseded, first.join().unwrap());

        second_call.reply.send(Ok("二".into())).unwrap();
        assert_eq!(
            TranslationOutcome::Success("二".into()),
            second.join().unwrap()
        );
    }

    #[test]
    fn 取り消した翻訳は置き換え扱いになる() {
        let (latest, calls) = controlled();

        let running = spawn_run(latest.start("one"));
        let call = calls.recv_timeout(WAIT).unwrap();
        latest.cancel();

        assert!(call.token.is_cancelled());
        assert_eq!(TranslationOutcome::Superseded, running.join().unwrap());
    }

    #[test]
    fn 結果が届くまでに新しい翻訳を始めていれば届いた結果は置き換え扱いと分かる() {
        let (latest, calls) = controlled();
        let pending = latest.start("one");
        let running = thread::spawn(move || {
            let outcome = pending.run();
            (pending, outcome)
        });
        calls
            .recv_timeout(WAIT)
            .unwrap()
            .reply
            .send(Ok("一".into()))
            .unwrap();
        let (pending, outcome) = running.join().unwrap();
        assert_eq!(TranslationOutcome::Success("一".into()), outcome);

        let _newer = latest.start("two");

        assert!(pending.is_superseded());
    }

    #[test]
    fn 失敗すれば理由の文を返す() {
        let latest = LatestTranslation::new(Arc::new(FailingTranslator("時間切れ")), "en", "ja");

        assert_eq!(
            TranslationOutcome::Failure("時間切れ".into()),
            latest.start("x").run()
        );
    }
}
