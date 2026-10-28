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
    url: &str,
) -> Result<HttpResponse, actix_web::Error> {
    let mut request = if req.method() == Method::HEAD {
        client.head(url)
    } else {
        client.get(url)
    }
    .header("Referer", upstream::referer(url))
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
        let response = serve_track(&request, &client, session.clone(), &url)
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
        let response = serve_track(&request, &client, session, "http://example.invalid/track")
            .await
            .unwrap();
        assert_eq!(response.status(), 410);
        handle.stop(true).await;
    }
}
