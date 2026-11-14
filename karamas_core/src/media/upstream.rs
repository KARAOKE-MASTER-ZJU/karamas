//! 媒体上游客户端、来源请求头及严格的 HTTP Range 读取。
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use std::{io, ops::Range, time::Duration};

const MAX_RANGE_BYTES: u64 = 16 * 1024 * 1024;

/// 媒体请求自行指定来源 Referer，CDN 重定向时继续保留该请求头。
pub fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(crate::config::PROXY_CONNECT_TIMEOUT)
        .referer(false)
        .build()
}

/// 根据媒体来源选择上游请求的 User-Agent。
pub fn user_agent(url: &str) -> &'static str {
    // Keep the media request compatible with youtube_parser's selected client.
    // A generic Mozilla/5.0 UA can read the start but fails on later ranges.
    if referer(url) == "https://www.youtube.com/" {
        innertube_rs::constants::clients::VISIONOS_USER_AGENT
    } else {
        "Mozilla/5.0"
    }
}

/// 严格读取指定字节范围，拒绝忽略 Range、文件大小不匹配及响应不完整的上游。
pub async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    range: Range<u64>,
    total: u64,
) -> io::Result<Bytes> {
    if range.start >= range.end || range.end > total || range.end - range.start > MAX_RANGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid upstream range",
        ));
    }
    let response = client
        .get(url)
        .header("User-Agent", crate::media::upstream::user_agent(url))
        .header("Referer", crate::media::upstream::referer(url))
        .header("Accept-Encoding", "identity")
        .header("Range", format!("bytes={}-{}", range.start, range.end - 1))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| io::Error::other(e.without_url()))?;
    let expected = format!("bytes {}-{}/{}", range.start, range.end - 1, total);
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "upstream HTTP {} while requesting {expected}",
            response.status()
        )));
    }
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        || response
            .headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            != Some(expected.as_str())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "upstream did not return the exact requested range",
        ));
    }
    let mut result = BytesMut::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| io::Error::other(e.without_url()))?;
        if result.len() as u64 + chunk.len() as u64 > range.end - range.start {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized upstream range",
            ));
        }
        result.extend_from_slice(&chunk);
    }
    if result.len() as u64 != range.end - range.start {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated upstream range",
        ));
    }
    Ok(result.freeze())
}

#[cfg(test)]
mod tests {
    // CDN 重定向后仍须保留来源鉴权和 Range；改回默认客户端会让分轨媒体返回 403。
    #[actix_web::test]
    async fn media_redirect_preserves_source_headers_and_byte_range() {
        use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(|| {
            App::new().default_service(web::to(|req: HttpRequest| async move {
                if req.path() == "/redirect" {
                    return HttpResponse::Found()
                        .insert_header(("Location", "/media"))
                        .finish();
                }
                assert_eq!(
                    req.headers().get("Referer").unwrap(),
                    "https://www.bilibili.com/"
                );
                assert_eq!(req.headers().get("User-Agent").unwrap(), "Mozilla/5.0");
                assert_eq!(req.headers().get("Range").unwrap(), "bytes=4-7");
                HttpResponse::PartialContent()
                    .insert_header(("Content-Range", "bytes 4-7/12"))
                    .body("data")
            }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let source = "https://upos-sz-mirrorbd.bilivideo.com/track";
        let response = super::client()
            .unwrap()
            .get(format!("http://{address}/redirect?token=private"))
            .header("Referer", super::referer(source))
            .header("User-Agent", super::user_agent(source))
            .header("Range", "bytes=4-7")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(response.bytes().await.unwrap(), "data");
        handle.stop(true).await;
    }

    #[test]
    fn youtube_headers_apply_only_to_googlevideo_hosts() {
        for url in [
            "https://rr1.googlevideo.com/videoplayback",
            "https://googlevideo.com/videoplayback",
        ] {
            assert_eq!(
                super::user_agent(url),
                innertube_rs::constants::clients::VISIONOS_USER_AGENT
            );
            assert_eq!(super::referer(url), "https://www.youtube.com/");
        }
        for url in [
            "https://upos.bilivideo.com/video",
            "https://googlevideo.com.evil.test/video",
            "invalid",
        ] {
            assert_eq!(super::user_agent(url), "Mozilla/5.0");
            assert_eq!(super::referer(url), "https://www.bilibili.com/");
        }
    }
}

/// 根据媒体来源选择上游请求的 Referer。
pub fn referer(url: &str) -> &'static str {
    match url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
    {
        Some(host) if host == "googlevideo.com" || host.ends_with(".googlevideo.com") => {
            "https://www.youtube.com/"
        }
        _ => "https://www.bilibili.com/",
    }
}
