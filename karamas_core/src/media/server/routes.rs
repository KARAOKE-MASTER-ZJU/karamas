use super::SharedState;
use crate::media::mp4::duration::get_mp4_duration;
use crate::sources::bilibili::{BilibiliMedia, get_bilibili_media_with_format};
use actix_web::{HttpRequest, HttpResponse, route, web};
use futures_util::StreamExt;
use log::{debug, info};

/// 处理媒体代理请求，提供会话媒体或转发上游响应。
#[route("/{url:.*}", method = "GET", method = "HEAD", method = "OPTIONS")]
pub async fn proxy_handler(
    req: HttpRequest,
    path: web::Path<(String,)>,
    client: web::Data<reqwest::Client>,
    shared_state: web::Data<SharedState>,
) -> Result<HttpResponse, actix_web::Error> {
    // 只使用 TCP 对端地址，转发请求头不能作为设备身份。
    if crate::cast::dlna::settings::get_proxy_device_only()
        && req.peer_addr().map(|peer| peer.ip()) != Some(shared_state.proxy_device_ip)
    {
        return Ok(HttpResponse::Forbidden().finish());
    }
    let (path,) = path.into_inner();
    let Some(resource) = shared_state.media_sessions.resource_path(&path) else {
        return Ok(HttpResponse::Forbidden().finish());
    };
    let mut origin_url = resource.to_owned();
    let mut duration_cache_key = None;
    // Session paths are immutable. Reject obsolete generations before resolving
    // Bilibili URLs or allocating any cache; an old renderer retry cannot switch
    // the current song back to an earlier one.
    let is_session_path = origin_url.starts_with("__ktv_media/");
    if is_session_path {
        let Some(session) = crate::media::session::generation_from_path(&origin_url)
            .and_then(|generation| shared_state.media_sessions.get(generation)) else {
                return Ok(HttpResponse::Gone().finish());
            };
        if req.method() == actix_web::http::Method::OPTIONS {
            return Ok(super::hls::cors(&mut HttpResponse::NoContent()).finish());
        }
        let media = match session.prepare(client.get_ref()).await {
            Ok(media) => media,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Ok(HttpResponse::Gone().finish()),
            Err(error) => {
                log::error!(target: "DLNA1080", "媒体会话准备失败: generation={}, error={}", session.generation, error);
                return Err(actix_web::error::ErrorBadGateway(error));
            }
        };
        match media.as_ref() {
            crate::media::session::SessionMedia::Live(live) => {
                let name = origin_url.rsplit('/').next().unwrap_or_default();
                return super::live::serve(&req, client.get_ref(), session.clone(), live, name).await;
            }
            crate::media::session::SessionMedia::Hls(playlists) => {
                let name = origin_url.rsplit('/').next().unwrap_or_default();
                if let Some(url) = playlists.source(name) {
                    return super::hls::serve_track(&req, client.get_ref(), session.clone(), url).await;
                }
                let Some(playlist) = playlists.playlist(name) else {
                    return Ok(HttpResponse::NotFound().finish());
                };
                let mut response = HttpResponse::Ok();
                response
                    .insert_header(("Content-Type", "application/vnd.apple.mpegurl"))
                    .insert_header(("Cache-Control", "no-store"))
                    .insert_header(("Content-Length", playlist.len().to_string()));
                super::hls::cors(&mut response);
                if req.method() == actix_web::http::Method::HEAD {
                    return Ok(response.finish());
                }
                return Ok(response.body(playlist.to_owned()));
            }
            crate::media::session::SessionMedia::Seekable { mp4, cache } => {
                return Ok(crate::media::mp4::http::serve(&req, mp4.clone(), cache.clone(), session.generation));
            }
            crate::media::session::SessionMedia::Direct(url) => {
                // Progress reads durations by playlist song, including its page suffix.
                duration_cache_key = Some(session.song.clone());
                origin_url = url.clone();
            }
        }
    }
    if req.method() == actix_web::http::Method::OPTIONS {
        return Ok(HttpResponse::MethodNotAllowed().finish());
    }
    let query_string = req.query_string();
    if !is_session_path && !query_string.is_empty() {
        origin_url.push('?');
        origin_url.push_str(query_string);
    }

    debug!("Received proxy request for URL: {}", origin_url);
    let range_hdr = req
        .headers()
        .get(actix_web::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let if_range_hdr = req
        .headers()
        .get(actix_web::http::header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");

    debug!(
        "Proxy request: method={} origin_url={} Range={} If-Range={}",
        req.method(),
        origin_url,
        range_hdr,
        if_range_hdr
    );

    let is_direct = origin_url.starts_with("http://") || origin_url.starts_with("https://");
    let (bv_id, page) = if is_direct {
        (origin_url.as_str(), None)
    } else {
        let path_without_query = origin_url.split('?').next().unwrap_or(origin_url.as_str());
        let bv_id = &path_without_query[..path_without_query
            .find('-')
            .unwrap_or(path_without_query.len())];
        let page: Option<u32> = if let Some(pos) = path_without_query.find("-page") {
            path_without_query[pos + 5..].parse().ok()
        } else {
            None
        };
        (bv_id, page)
    };

    debug!("Proxy parsed: bv_id={} page={:?}", bv_id, page);

    let requested_qn = req
        .query_string()
        .split('&')
        .find_map(|part| {
            part.strip_prefix("qn=")
                .and_then(|value| value.parse::<u32>().ok())
        })
        .unwrap_or_else(|| crate::cast::dlna::settings::get_dlna_media_quality().0.as_qn());
    let start_secs = req
        .query_string()
        .split('&')
        .find_map(|part| {
            part.strip_prefix("start=")
                .and_then(|value| value.parse::<u32>().ok())
        })
        .unwrap_or(0);

    let media = if is_direct {
        BilibiliMedia::Direct {
            url: origin_url.clone(),
        }
    } else {
        let qn = crate::cast::Quality::from_qn(requested_qn)
            .ok_or_else(|| actix_web::error::ErrorBadRequest("不支持的清晰度"))?;
        let format = crate::cast::dlna::settings::get_dlna_media_quality().1;
        let resolved = get_bilibili_media_with_format(bv_id, page, qn.as_qn(), format, true)
            .await.map_err(actix_web::error::ErrorBadGateway)?;
        crate::cast::dlna::settings::set_dlna_actual_quality((resolved.quality, resolved.format));
        resolved.media
    };

    if let BilibiliMedia::Dash {
        video_url,
        audio_url,
        width,
        height,
    } = media
    {
        info!(target: "DLNA1080", "媒体请求进入 DASH 混流: {}x{}, qn={}, start={}s, method={}", width, height, requested_qn, start_secs, req.method());
        if let Some(range) = req.headers().get(actix_web::http::header::RANGE) {
            log::info!(
                target: "DLNA1080",
                "DLNA renderer 请求字节 Range；1080P 定位通过重开 start 流处理: {:?}",
                range
            );
        }
        if *req.method() == actix_web::http::Method::HEAD {
            info!(target: "DLNA1080", "响应 DLNA HEAD 探测: video/mp4, ranges=none");
            return Ok(HttpResponse::Ok()
                .insert_header(("Content-Type", "video/mp4"))
                .insert_header(("Accept-Ranges", "none"))
                .insert_header(("transferMode.dlna.org", "Streaming"))
                .finish());
        }
        let muxed = crate::media::mp4::fragmented::mux_dash_from(
            client.get_ref(),
            &video_url,
            &audio_url,
            start_secs,
        )
            .await
            .map_err(|error| {
                log::error!(target: "DLNA1080", "创建混流响应失败: {}", error);
                actix_web::error::ErrorInternalServerError(error)
            })?;
        let mut response = HttpResponse::Ok();
        response.insert_header(("Content-Type", "video/mp4"));
        response.insert_header(("Accept-Ranges", "none"));
        response.insert_header(("transferMode.dlna.org", "Streaming"));
        response.insert_header(("contentFeatures.dlna.org", "DLNA.ORG_OP=00;DLNA.ORG_CI=0"));
        if let Some(content_length) = muxed.content_length {
            response.insert_header(("Content-Length", content_length.to_string()));
        }
        info!(target: "DLNA1080", "开始向 DLNA 设备输出混流 MP4: content_length={}", muxed.content_length.map_or_else(|| "unknown".to_string(), |size| size.to_string()));
        return Ok(response.streaming(muxed.stream));
    }

    let BilibiliMedia::Direct { url: target_url } = media else {
        unreachable!()
    };

    debug!("Proxy resolved target_url={}", target_url);

    // 异步获取视频时长并存入缓存（m3u8 直播流跳过，为了避免抓取 HLS 分片时产生大量无用且失败的网络请求，直接地址也需要跳过对 ts/m4s/fmp4 分片或带有结尾查询串切片的探测）
    let is_hls = origin_url
        .split('?')
        .next()
        .is_some_and(|url| url.ends_with(".m3u8"))
        || target_url
            .split('?')
            .next()
            .is_some_and(|url| url.ends_with(".m3u8"));

    // 对于 HLS 或者是显然属于 FMP4 / HLS segment 的分片文件，避免去请求其时长
    let is_segment = origin_url.contains("fmp4")
        || origin_url.contains(".m4s")
        || origin_url.contains(".ts")
        || (is_direct
            && req.headers().contains_key(actix_web::http::header::RANGE)
            && !range_hdr.starts_with("bytes=0-"));

    if !is_hls && !is_segment {
        let duration_cache = shared_state.duration_cache.clone();
        let origin_url_clone = duration_cache_key.unwrap_or_else(|| origin_url.clone());
        let target_url_clone = target_url.clone();
        tokio::spawn(async move {
            // 先检查缓存中是否已有该视频的时长，若无则先用 0 占位，防止并发产生大量多余下载请求
            {
                let mut cache = duration_cache.lock().await;
                if cache.contains_key(&origin_url_clone) {
                    return;
                }
                cache.insert(origin_url_clone.clone(), 0);
            }

            match get_mp4_duration(&target_url_clone).await {
                Ok(duration) => {
                    let mut cache = duration_cache.lock().await;
                    cache.insert(origin_url_clone, duration.as_secs() as u32);
                    info!(
                        "成功获取并缓存视频时长: {} -> {}s",
                        target_url_clone,
                        duration.as_secs()
                    );
                }
                Err(e) => {
                    let mut cache = duration_cache.lock().await;
                    cache.insert(origin_url_clone, 0); // 缓存失败结果，避免后续 HLS 分片反复请求下载 2MB
                    if is_direct {
                        log::debug!("无法获取(直链/分片)视频时长: {} (静默跳过并缓存为0)", e);
                    } else {
                        log::warn!("无法获取 bilibili 视频时长: {}", e);
                    }
                }
            }
        });
    } else {
        info!("检测到 m3u8 等切片或直播流，跳过时长解析: {}", target_url);
    }

    // DLNA renderers often probe with HEAD and/or send Range requests.
    let mut upstream = match *req.method() {
        actix_web::http::Method::HEAD => client.head(&target_url),
        _ => client.get(&target_url),
    };

    upstream = upstream
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36")
        .header("Referer", "https://www.bilibili.com/");

    // Forward Range-related headers to support seek/probe.
    if let Some(range) = req.headers().get(actix_web::http::header::RANGE) {
        upstream = upstream.header("Range", range.as_bytes());
    }
    if let Some(if_range) = req.headers().get(actix_web::http::header::IF_RANGE) {
        upstream = upstream.header("If-Range", if_range.as_bytes());
    }

    let response = upstream
        .send()
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    let ct = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let cl = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let ar = response
        .headers()
        .get("accept-ranges")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    let cr = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");

    info!(
        "Proxy upstream: status={} Content-Type={} Content-Length={} Accept-Ranges={} Content-Range={}",
        response.status(),
        ct,
        cl,
        ar,
        cr
    );

    let status_u16 = response.status().as_u16();
    let mut client_resp = HttpResponse::build(
        actix_web::http::StatusCode::from_u16(status_u16)
            .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR),
    );

    for (name, value) in response.headers().iter() {
        let name_str = name.as_str();
        if name_str != "connection"
            && name_str != "content-encoding"
            && name_str != "transfer-encoding"
        {
            client_resp.insert_header((name_str, value.as_bytes()));
        }
    }

    // Some renderers require this header to decide whether they can seek.
    if !response.headers().contains_key("accept-ranges") {
        client_resp.insert_header(("accept-ranges", "bytes"));
    }

    // HEAD should not include a body.
    if *req.method() == actix_web::http::Method::HEAD {
        return Ok(client_resp.finish());
    }

    let body_stream = response
        .bytes_stream()
        .map(|item| item.map_err(std::io::Error::other));

    Ok(client_resp.streaming(body_stream))
}

#[cfg(test)]
mod tests {
    use crate::media::server::proxy_handler;
    use actix_web::{App, HttpServer, web};
    use reqwest::Client;

    // token 与设备 IP 共同保护会话和直链入口，转发头不能作为设备身份。
    #[actix_web::test]
    async fn proxy_uses_connected_device_ip_instead_of_forwarded_headers() {
        use crate::{SharedState, media::session::MediaSessions};
        use actix_web::{
            http::{Method, StatusCode},
            test,
        };
        use std::sync::Arc;

        let state = web::Data::new(SharedState {
            proxy_device_ip: "192.0.2.10".parse().unwrap(),
            duration_cache: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            media_sessions: Arc::new(MediaSessions::default()),
        });
        let app = test::init_service(
            App::new()
                .app_data(state.clone())
                .app_data(web::Data::new(Client::new()))
                .service(proxy_handler),
        )
        .await;
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            for peer in [None, Some("192.0.2.20:50000"), Some("127.0.0.1:50000")] {
                for resource in ["__ktv_media/1.mp4", "https://example.invalid/segment.ts"] {
                    let path = format!("/{}", state.media_sessions.proxy_path(resource));
                    let mut request = test::TestRequest::with_uri(&path)
                        .method(method.clone())
                        .insert_header(("X-Forwarded-For", "192.0.2.10"))
                        .insert_header(("Forwarded", "for=192.0.2.10"));
                    if let Some(peer) = peer {
                        request = request.peer_addr(peer.parse().unwrap());
                    }
                    let response = test::call_service(&app, request.to_request()).await;
                    assert_eq!(response.status(), StatusCode::FORBIDDEN);
                }
            }
            for resource in ["__ktv_media/1.mp4", "https://example.invalid/segment.ts"] {
                for path in [format!("/{resource}"), format!("/wrong-token/{resource}")] {
                    let request = test::TestRequest::with_uri(&path)
                        .method(method.clone())
                        .peer_addr("192.0.2.10:50000".parse().unwrap())
                        .to_request();
                    assert_eq!(test::call_service(&app, request).await.status(), StatusCode::FORBIDDEN);
                }
            }
            let path = format!("/{}", state.media_sessions.proxy_path("__ktv_media/1.mp4"));
            let request = test::TestRequest::with_uri(&path)
                .method(method)
                .peer_addr("192.0.2.10:50000".parse().unwrap())
                .insert_header(("X-Forwarded-For", "192.0.2.20"))
                .to_request();
            assert_eq!(
                test::call_service(&app, request).await.status(),
                StatusCode::GONE
            );
        }
    }

    #[actix_web::test]
    async fn obsolete_session_head_returns_gone_without_upstream_access() {
        use crate::{SharedState, media::session::MediaSessions, cast::Quality};
        use std::sync::Arc;
        let sessions = Arc::new(MediaSessions::default());
        let old = sessions.activate_with_format(
            "https://example.invalid/old.mp4",
            Quality::P720, crate::cast::MediaFormat::Hls,
        );
        sessions.activate_with_format(
            "https://example.invalid/current.mp4",
            Quality::P720, crate::cast::MediaFormat::Mp4,
        );
        let state = web::Data::new(SharedState {
            proxy_device_ip: "127.0.0.1".parse().unwrap(),
            duration_cache: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            media_sessions: sessions.clone(),
        });
        let app = actix_web::test::init_service(App::new().app_data(state).app_data(web::Data::new(Client::new())).service(proxy_handler)).await;
        for path in [old.path(), format!("__ktv_media/{}.mp4", old.generation), format!("__ktv_media/{}/video.mp4", old.generation), format!("__ktv_media/{}/audio.mp4", old.generation)] {
            for method in [actix_web::http::Method::GET, actix_web::http::Method::HEAD, actix_web::http::Method::OPTIONS] {
                let request = actix_web::test::TestRequest::with_uri(&format!("/{}", sessions.proxy_path(&path)))
                    .peer_addr("127.0.0.1:50000".parse().unwrap()).method(method).to_request();
                let response = actix_web::test::call_service(&app, request).await;
                assert_eq!(response.status(), actix_web::http::StatusCode::GONE);
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires public Bilibili access"]
    async fn test_https() {
        let client = reqwest::Client::new();

        match client
            .get("https://www.bilibili.com/")
            .header("User-Agent", "Mozilla/5.0 ...")
            .send()
            .await
        {
            Ok(res) => println!("成功连接! 状态码: {}", res.status()),
            Err(e) => println!("连接失败: {:?}. 请检查网络连接。", e),
        }
    }
    #[tokio::test]
    #[ignore = "manual long-running proxy server"]
    async fn test_proxy() -> std::io::Result<()> {
        // 在外面创建全局唯一的 Client，内部已配置好纯 Rustls
        let client = Client::builder()
            .tls_backend_rustls() // 强制使用 rustls
            .build()
            .expect("Failed to create client");

        let client_data = web::Data::new(client);

        HttpServer::new(move || {
            App::new()
                .app_data(client_data.clone())
                .service(proxy_handler)
        })
        .bind(("127.0.0.1", 8080))?
        .run()
        .await
    }
}
