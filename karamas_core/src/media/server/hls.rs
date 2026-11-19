//! 代理 HLS 轨道的字节范围，由播放器按需下载初始化信息和媒体分片。
use crate::media::{session::MediaSession, upstream};
use actix_web::{HttpRequest, HttpResponse, HttpResponseBuilder, http::Method};
use futures_util::StreamExt;
use std::sync::Arc;

pub(super) fn cors(response: &mut HttpResponseBuilder) -> &mut HttpResponseBuilder {
    response
        .insert_header(("Access-Control-Allow-Origin", "*"))
        .insert_header(("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS"))
        .insert_header(("Access-Control-Allow-Headers", "Range, If-Range"))
        .insert_header((
            "Access-Control-Expose-Headers",
            "Content-Length, Content-Range, Accept-Ranges",
        ))
}

pub(super) async fn serve_track(
    req: &HttpRequest,
    client: &reqwest::Client,
    session: Arc<MediaSession>,
    source: Arc<crate::media::hls::Source>,
) -> Result<HttpResponse, actix_web::Error> {
    use crate::media::mp4::http::{RequestedRange, resolve_range};
    if session.is_stopped() {
        return Ok(cors(&mut HttpResponse::Gone()).finish());
    }
    let etag = format!("\"ktv-{}\"", session.generation);
    let header = |name| {
        req.headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let requested = if header("If-Range").is_none_or(|value| value == etag) {
        header("Range").map_or(RequestedRange::Full, |value| {
            resolve_range(value, source.total)
        })
    } else {
        RequestedRange::Full
    };
    let mut response;
    let range = match requested {
        RequestedRange::Unsatisfiable => {
            return Ok(cors(&mut HttpResponse::RangeNotSatisfiable())
                .insert_header(("Content-Range", format!("bytes */{}", source.total)))
                .finish());
        }
        RequestedRange::Partial(range) => {
            response = HttpResponse::PartialContent();
            response.insert_header((
                "Content-Range",
                format!("bytes {}-{}/{}", range.start, range.end - 1, source.total),
            ));
            range
        }
        RequestedRange::Full => {
            response = HttpResponse::Ok();
            0..source.total
        }
    };
    cors(&mut response)
        .insert_header((
            "Content-Type",
            if req.path().ends_with("audio.mp4") {
                "audio/mp4"
            } else {
                "video/mp4"
            },
        ))
        .insert_header(("Accept-Ranges", "bytes"))
        .insert_header(("ETag", etag));
    let length = range.end - range.start;
    if req.method() == Method::HEAD {
        return Ok(response.body(actix_web::body::SizedStream::new(
            length,
            futures_util::stream::empty::<Result<bytes::Bytes, std::io::Error>>(),
        )));
    }
    let client = client.clone();
    let stream = futures_util::stream::try_unfold(range, move |range| {
        let client = client.clone();
        let source = source.clone();
        async move {
            if range.is_empty() {
                return Ok(None);
            }
            let end = range.end.min(
                range
                    .start
                    .saturating_add(crate::media::cache::BLOCK_BYTES),
            );
            let data = source.read(&client, range.start..end).await?;
            Ok::<_, std::io::Error>(Some((data, end..range.end)))
        }
    })
    .take_until(async move { session.cancelled().await });
    Ok(response.body(actix_web::body::SizedStream::new(length, stream)))
}

pub(super) async fn serve_track_with_referer(
    req: &HttpRequest,
    client: &reqwest::Client,
    session: Arc<MediaSession>,
    url: &str,
    referer: &str,
) -> Result<HttpResponse, actix_web::Error> {
    let mut request = if req.method() == Method::HEAD {
        client.head(url)
    } else {
        client.get(url)
    }
    .header("Referer", referer)
    .header("User-Agent", upstream::user_agent(url))
    .header("Accept-Encoding", "identity");
    for name in ["Range", "If-Range"] {
        if let Some(value) = req.headers().get(name) {
            request = request.header(name, value.as_bytes());
        }
    }
    let upstream = tokio::select! {
        biased;
        _ = session.cancelled() => return Ok(cors(&mut HttpResponse::Gone()).finish()),
        result = request.send() => result.map_err(|error| {
            actix_web::error::ErrorBadGateway(error.without_url())
        })?,
    };
    let status = actix_web::http::StatusCode::from_u16(upstream.status().as_u16())
        .map_err(actix_web::error::ErrorBadGateway)?;
    let mut response = HttpResponse::build(status);
    // 只保留媒体响应头，不转发上游 Cookie 或重定向目标。
    for name in [
        "Content-Type",
        "Content-Length",
        "Content-Range",
        "Accept-Ranges",
        "ETag",
        "Last-Modified",
    ] {
        if let Some(value) = upstream.headers().get(name) {
            response.insert_header((name, value.as_bytes()));
        }
    }
    cors(&mut response);
    if req.method() == Method::HEAD {
        // 空响应体会让 actix 把 Content-Length 改成 0；HEAD 仍需保留媒体长度。
        if let Some(length) = upstream
            .headers()
            .get("Content-Length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
        {
            return Ok(response.body(actix_web::body::SizedStream::new(
                length,
                futures_util::stream::empty::<Result<bytes::Bytes, std::io::Error>>(),
            )));
        }
        return Ok(response.finish());
    }
    let stream = upstream
        .bytes_stream()
        .map(|result| result.map_err(|error| std::io::Error::other(error.without_url())))
        .take_until(async move { session.cancelled().await });
    Ok(response.streaming(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cast::{MediaFormat, Quality},
        media::session::MediaSessions,
    };
    use actix_web::{App, HttpServer, body::to_bytes, test::TestRequest, web};

    // 播放器按固定偏移读取；恢复断流时不能交付残块、重复字节或重新激活旧会话。
    #[actix_web::test]
    async fn interrupted_hls_ranges_recover_without_changing_the_response_bytes() {
        use crate::media::hls::{Source, Track};
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let data = Arc::new(
            (0..crate::media::cache::BLOCK_BYTES + 64)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let upstream_data = data.clone();
        let server = tokio::spawn(async move {
            let mut interrupted = false;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                }
                let request = String::from_utf8(request).unwrap();
                let range = request
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_owned)
                    })
                    .unwrap();
                let (start, end) = range.split_once('-').unwrap();
                let (start, end) = (
                    start.parse::<usize>().unwrap(),
                    end.parse::<usize>().unwrap(),
                );
                if request.starts_with("GET /primary") && interrupted {
                    socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    continue;
                }
                socket.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", upstream_data.len(), end - start + 1).as_bytes()).await.unwrap();
                if request.starts_with("GET /primary") {
                    socket
                        .write_all(&upstream_data[start..start + 3])
                        .await
                        .unwrap();
                    interrupted = true;
                } else {
                    socket.write_all(&upstream_data[start..=end]).await.unwrap();
                }
            }
        });
        let mut track = Track::new(
            format!("http://{address}/primary"),
            "avc1.640033",
            1000,
            "0-15",
            "16-59",
        )
        .unwrap();
        track.urls.push(format!("http://{address}/backup"));
        let source = Arc::new(Source::new(track, data.len() as u64, Vec::new(), None));
        let sessions = MediaSessions::default();
        let session =
            sessions.activate_with_format("BV1NTWn6aEoY-page0", Quality::P1080, MediaFormat::Hls);
        let request = TestRequest::get()
            .uri("/video.mp4")
            .insert_header(("Range", format!("bytes=4-{}", data.len() - 1)))
            .to_http_request();
        let client = reqwest::Client::new();
        let response = serve_track(&request, &client, session.clone(), source.clone())
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(
            response.headers().get("Content-Range").unwrap(),
            format!("bytes 4-{}/{}", data.len() - 1, data.len()).as_str()
        );
        assert_eq!(
            to_bytes(response.into_body()).await.unwrap().as_ref(),
            &data[4..]
        );
        let head = TestRequest::default()
            .method(Method::HEAD)
            .to_http_request();
        let response = serve_track(&head, &client, session.clone(), source.clone())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            actix_web::body::MessageBody::size(response.body()),
            actix_web::body::BodySize::Sized(data.len() as u64)
        );
        sessions.clear();
        assert_eq!(
            serve_track(&request, &client, session, source)
                .await
                .unwrap()
                .status(),
            410
        );
        server.abort();
    }

    // Range 的响应字节和会话失效是播放器契约，错误转发会破坏定位或恢复旧曲目。
    #[actix_web::test]
    async fn track_proxy_preserves_ranges_headers_and_session_cancellation() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(|| {
            App::new().default_service(web::to(|req: HttpRequest| async move {
                assert_eq!(
                    req.headers().get("Range").unwrap().to_str().unwrap(),
                    "bytes=4-7"
                );
                assert_eq!(
                    req.headers().get("If-Range").unwrap().to_str().unwrap(),
                    "\"track\""
                );
                assert_eq!(
                    req.headers().get("Referer").unwrap().to_str().unwrap(),
                    "https://www.bilibili.com/"
                );
                assert_eq!(
                    req.headers().get("User-Agent").unwrap().to_str().unwrap(),
                    "Mozilla/5.0"
                );
                HttpResponse::PartialContent()
                    .insert_header(("Content-Range", "bytes 4-7/12"))
                    .insert_header(("Accept-Ranges", "bytes"))
                    .insert_header(("Set-Cookie", "private=value"))
                    .body("data")
            }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let sessions = MediaSessions::default();
        let session = sessions.activate_with_format(
            "http://example.invalid/video",
            Quality::P720,
            MediaFormat::Hls,
        );
        let request = TestRequest::get()
            .insert_header(("Range", "bytes=4-7"))
            .insert_header(("If-Range", "\"track\""))
            .to_http_request();
        let client = reqwest::Client::new();
        let url = format!("http://{address}/track");
        let response = serve_track_with_referer(
            &request,
            &client,
            session.clone(),
            &url,
            upstream::referer(&url),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(
            response
                .headers()
                .get("Content-Range")
                .unwrap()
                .to_str()
                .unwrap(),
            "bytes 4-7/12"
        );
        assert_eq!(
            response
                .headers()
                .get("Access-Control-Allow-Origin")
                .unwrap()
                .to_str()
                .unwrap(),
            "*"
        );
        assert!(!response.headers().contains_key("Set-Cookie"));
        assert_eq!(to_bytes(response.into_body()).await.unwrap(), "data");
        sessions.clear();
        // 已取消的会话即使指向不可达地址，也必须在联网前返回 Gone。
        let response = serve_track_with_referer(
            &request,
            &client,
            session,
            "http://example.invalid/track",
            "https://www.bilibili.com/",
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 410);
        handle.stop(true).await;
    }
}
