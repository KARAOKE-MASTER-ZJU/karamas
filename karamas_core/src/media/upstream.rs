//! Source-specific request headers for media fetched by the local DLNA proxy.

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
