//! 根据 HTTP 响应识别本地播放源，URL 只用作资源地址。
use super::live::{self, Resolved};
use std::{io, time::Duration};
use url::Url;

pub(crate) enum Source {
    Native(Url),
    Mp4(Url),
    Flv { url: Url, filesize: Option<u64> },
    Live(Resolved),
}

pub(crate) async fn resolve(client: &reqwest::Client, value: &str) -> io::Result<Source> {
    let url = Url::parse(value).map_err(|_| live::invalid("媒体地址无效"))?;
    let url = live::media_url(&url, value)?;
    let referer = format!("{}/", url.origin().ascii_serialization());
    let mut response = client
        .get(url.clone())
        .header("User-Agent", crate::media::upstream::user_agent(value))
        .header("Referer", &referer)
        .header("Accept-Encoding", "identity")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| io::Error::other(error.without_url()))?
        .error_for_status()
        .map_err(|error| io::Error::other(error.without_url()))?;
    let effective = response.url().clone();
    let filesize = response.content_length();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(content_type.as_str(), "text/html" | "application/xhtml+xml") {
        let body = live::read_body(response, 4 * 1024 * 1024).await?;
        return live::resolve_page(&effective, &body).map(Source::Live);
    }
    // 只读识别所需的数据头，直播和大文件都不等待完整下载。
    let mut prefix = Vec::with_capacity(9);
    while prefix.len() < 9 {
        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| io::Error::other(error.without_url()))?
        else {
            break;
        };
        prefix.extend_from_slice(&chunk[..chunk.len().min(9 - prefix.len())]);
    }
    if prefix.starts_with(b"FLV\x01") {
        return Ok(Source::Flv { url, filesize });
    }
    if prefix.starts_with(b"#EXTM3U")
        || matches!(
            content_type.as_str(),
            "application/vnd.apple.mpegurl" | "application/x-mpegurl"
        )
    {
        return Ok(Source::Live(Resolved {
            source: live::Source::Hls(effective),
            referer,
        }));
    }
    if prefix.get(4..8) == Some(b"ftyp") { return Ok(Source::Mp4(url)); }
    Ok(Source::Native(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};

    // 直播识别必须处理分块数据头，不能等待流结束；HLS 和错误响应也不依赖 URL 后缀。
    #[actix_web::test]
    async fn response_detection_handles_streaming_headers_and_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(|| {
            App::new().default_service(web::to(|request: HttpRequest| async move {
                match request.path() {
                    "/channel.mp4" => {
                        let chunks = futures_util::stream::iter([
                            Ok::<_, io::Error>(bytes::Bytes::from_static(b"FLV")),
                            Ok(bytes::Bytes::from_static(b"\x01\x05\x00\x00\x00\x09")),
                        ]);
                        use futures_util::StreamExt;
                        HttpResponse::Ok()
                            .content_type("video/mp4")
                            .streaming(chunks.chain(futures_util::stream::pending()))
                    }
                    "/playlist" => HttpResponse::Ok().body("#EXTM3U\n#EXT-X-VERSION:3\n"),
                    _ => HttpResponse::NotFound().finish(),
                }
            }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let client = crate::media::upstream::client().unwrap();
        let source = tokio::time::timeout(
            Duration::from_secs(3),
            resolve(&client, &format!("http://{address}/channel.mp4")),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(source, Source::Flv { filesize: None, .. }));
        let source = resolve(&client, &format!("http://{address}/playlist"))
            .await
            .unwrap();
        assert!(matches!(
            source,
            Source::Live(Resolved {
                source: live::Source::Hls(_),
                ..
            })
        ));
        assert!(
            resolve(&client, &format!("http://{address}/missing.flv"))
                .await
                .is_err()
        );
        handle.stop(false).await;
    }
}
