use std::time::Duration;

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::logic::translation::{HttpError, HttpGet, HttpResponse};

/// ureq による HttpGet。TLS は SChannel（native-tls）
pub struct UreqHttp {
    agent: Agent,
}

impl UreqHttp {
    pub fn new() -> Self {
        let config = Agent::config_builder()
            // 成功以外のステータスも応答として受け取り、判定は GoogleGtxTranslator に任せる
            .http_status_as_error(false)
            .user_agent("Mozilla/5.0")
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    // 既定の WebPki は OS の根証明書を外して同梱の証明書だけにする（同梱していないので全部失敗する）
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpGet for UreqHttp {
    fn get(&self, url: &str, timeout: Duration) -> Result<HttpResponse, HttpError> {
        // timeout_global は接続から本文を読み終わるまでの全体にかかる
        let mut response = self
            .agent
            .get(url)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .call()
            .map_err(to_http_error)?;
        let status = response.status();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(to_http_error)?;
        Ok(HttpResponse {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or_default().to_string(),
            body,
        })
    }
}

fn to_http_error(e: ureq::Error) -> HttpError {
    match e {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        other => HttpError::Other(other.to_string()),
    }
}
