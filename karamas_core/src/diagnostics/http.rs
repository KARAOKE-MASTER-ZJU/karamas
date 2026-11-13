//! 记录 HTTP 请求、响应与耗时。
use crate::CoreError;
use reqwest::Client;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::OnceLock,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResponse {
    pub time: i64,
    pub url: String,
    pub request: String,
    pub response_headers: String,
    pub response_url: Option<String>,
    pub status: Option<u16>,
    pub elapsed_ms: u64,
    pub fields: BTreeMap<String, String>,
    pub raw: String,
    pub error: Option<String>,
}

pub fn http_url(value: &str) -> Result<Url, CoreError> {
    let url = Url::parse(value)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("请输入有效的 HTTP 或 HTTPS 地址".into());
    }
    Ok(url)
}

pub(super) fn client(lan: bool) -> &'static Client {
    static LAN: OnceLock<Client> = OnceLock::new();
    static WEB: OnceLock<Client> = OnceLock::new();
    (if lan { &LAN } else { &WEB }).get_or_init(|| {
        // 自动解压会移除响应头中的 Content-Encoding 和 Content-Length。
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(5))
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd();
        if lan {
            builder = builder.no_proxy();
        }
        builder.build().expect("创建诊断 HTTP 客户端失败")
    })
}

pub(super) async fn exchange(
    request: reqwest::RequestBuilder,
    url: &str,
    payload: String,
    time_body: bool,
) -> TestResponse {
    let started = Instant::now();
    let mut result = TestResponse {
        time: chrono::Utc::now().timestamp_millis(),
        url: url.to_owned(),
        request: String::new(),
        response_headers: String::new(),
        response_url: None,
        status: None,
        elapsed_ms: 0,
        fields: BTreeMap::new(),
        raw: String::new(),
        error: None,
    };
    let mut bytes = Vec::new();
    let response = async {
        let (client, request) = request
            .header(reqwest::header::ACCEPT, "*/*")
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .build_split();
        let mut request = request?;
        if let Some(length) = request
            .body()
            .and_then(|body| body.as_bytes())
            .map(<[u8]>::len)
        {
            request
                .headers_mut()
                .insert(reqwest::header::CONTENT_LENGTH, length.into());
        }
        result.request = format!("{} {}\n", request.method(), request.url());
        if !request.headers().contains_key(reqwest::header::HOST) {
            result.request.push_str(&format!(
                "host: {}\n",
                &request.url()[url::Position::BeforeHost..url::Position::AfterPort]
            ));
        }
        for (name, value) in request.headers() {
            result.request.push_str(&format!(
                "{name}: {}\n",
                String::from_utf8_lossy(value.as_bytes())
            ));
        }
        if !payload.is_empty() {
            result.request.push('\n');
            result.request.push_str(&payload);
        }
        let mut response = client.execute(request).await?;
        result.status = Some(response.status().as_u16());
        result.response_url = Some(response.url().to_string());
        result.response_headers = format!("{:?} {}\n", response.version(), response.status());
        for (name, value) in response.headers() {
            result.response_headers.push_str(&format!(
                "{name}: {}\n",
                String::from_utf8_lossy(value.as_bytes())
            ));
        }
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                result.error = Some("响应超过 2 MiB".to_owned());
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok::<_, reqwest::Error>(())
    }
    .await;
    if let Err(error) = response {
        result.error = Some(error.without_url().to_string());
    }
    result.raw = String::from_utf8_lossy(&bytes).into_owned();
    if time_body || result.status.is_none() {
        result.elapsed_ms = started.elapsed().as_millis() as u64;
    }
    result
}

pub async fn latency(url: &str, lan: bool) -> Result<TestResponse, CoreError> {
    let url = http_url(url)?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => {
            host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        }
        None => false,
    };
    Ok(exchange(
        client(lan || loopback).get(url.clone()),
        url.as_str(),
        String::new(),
        false,
    )
    .await)
}
