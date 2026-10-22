// 使用示例
use crate::SharedState;
use crate::bilibili_parser::{BilibiliMedia, get_bilibili_media};
use crate::mp4_util::get_mp4_duration;
use actix_web::{HttpRequest, HttpResponse, route, web};
use futures_util::StreamExt;
use log::{debug, info};

#[route("/{url:.*}", method = "GET", method = "HEAD")]
pub async fn proxy_handler(
    req: HttpRequest,
    path: web::Path<(String,)>,
    client: web::Data<reqwest::Client>,
    shared_state: web::Data<SharedState>,
) -> Result<HttpResponse, actix_web::Error> {
    let (mut origin_url,) = path.into_inner();
    let mut duration_cache_key = None;
    // Session paths are immutable. Reject obsolete generations before resolving
    // Bilibili URLs or allocating any cache; an old renderer retry cannot switch
    // the current song back to an earlier one.
    let is_session_path = origin_url.starts_with("__ktv_media/");
    if is_session_path {
        let Some(session) = crate::media_session::generation_from_path(&origin_url)
            .and_then(|generation| shared_state.media_sessions.get(generation)) else {
                return Ok(HttpResponse::Gone().finish());
            };
        let media = match session.prepare(client.get_ref()).await {
            Ok(media) => media,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Ok(HttpResponse::Gone().finish()),
            Err(error) => {
                log::error!(target: "DLNA1080", "媒体会话准备失败: generation={}, error={}", session.generation, error);
                return Err(actix_web::error::ErrorBadGateway(error));
            }
        };
        match media.as_ref() {
            crate::media_session::SessionMedia::Seekable { mp4, cache } => {
                return Ok(crate::virtual_mp4_http::serve(&req, mp4.clone(), cache.clone(), session.generation));
            }
            crate::media_session::SessionMedia::Direct(url) => {
                // Progress reads durations by playlist song, including its page suffix.
                duration_cache_key = Some(session.song.clone());
                origin_url = url.clone();
            }
        }
    }
    let query_string = req.query_string();
    if !is_session_path && !query_string.is_empty() {
        origin_url.push('?');
        origin_url.push_str(query_string);
    }

    // 针对 eplus 的特殊鉴权参数保存与补全逻辑
    if origin_url.contains("eplus") {
        let mut session = shared_state.eplus_auth.lock().await;
        if origin_url.contains("Policy=") && origin_url.contains("Signature=") {
            // 如果链接带了完整鉴权参数（如 index.m3u8），保存它供后续切片使用
            if let Some(query_start) = origin_url.find('?') {
                *session = Some(origin_url[query_start + 1..].to_string());
                info!("Eplus: Saved auth parameters from playlist request");
            }
        } else if (origin_url.ends_with(".ts") || origin_url.contains(".ts?"))
            && !origin_url.contains("Signature=")
        {
            // 如果是切片请求（.ts）且缺少鉴权，将保存的参数拼接到切片 URL 后
            if let Some(ref auth_params) = *session {
                let connector = if origin_url.contains('?') { "&" } else { "?" };
                origin_url.push_str(connector);
                origin_url.push_str(auth_params);
                info!("Eplus: Injected saved auth parameters into segment request");
            }
        }
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
        "Proxy request: method={} path={} origin_url={} Range={} If-Range={}",
        req.method(),
        req.path(),
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
        .unwrap_or_else(|| crate::get_dlna_quality().as_qn());
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
        match get_bilibili_media(bv_id, page, requested_qn).await {
            Ok(media) => media,
            // Not every public video exposes an anonymous id=80 DASH track. Do not
            // leave a renderer waiting on a failed 1080p request: preserve the
            // established 720p single-file route for that title.
            Err(error) if requested_qn >= 80 => {
                log::warn!(
                    target: "DLNA1080",
                    "1080P 不可用，自动回退 720P: bvid={}, page={:?}, reason={}",
                    bv_id,
                    page,
                    error
                );
                get_bilibili_media(bv_id, page, 64)
                    .await
                    .map_err(|fallback_error| {
                        log::error!(
                            target: "DLNA1080",
                            "720P 回退也失败: bvid={}, error={}",
                            bv_id,
                            fallback_error
                        );
                        actix_web::error::ErrorInternalServerError(fallback_error)
                    })?
            }
            Err(error) => return Err(actix_web::error::ErrorInternalServerError(error)),
        }
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
        let muxed = crate::fmp4_mux::mux_dash_from(
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

    if target_url.contains("eplus") {
        upstream = upstream
            .header("accept", "*/*")
            .header("accept-language", "zh-CN,zh;q=0.9,en;q=0.8")
            .header("cache-control", "no-cache")
            .header("origin", "https://live.nulla.top")
            .header("pragma", "no-cache")
            .header("priority", "u=1, i")
            .header("sec-ch-ua", "\"Not:A-Brand\";v=\"99\", \"Google Chrome\";v=\"145\", \"Chromium\";v=\"145\"")
            .header("sec-ch-ua-mobile", "?1")
            .header("sec-ch-ua-platform", "\"iOS\"")
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "cross-site")
            .header("user-agent", "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1");
    } else {
        upstream = upstream
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36")
            .header("Referer", "https://www.bilibili.com/");
    }

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
    use crate::media_server::proxy_handler;
    use actix_web::{App, HttpServer, web};
    use reqwest::Client;

    #[actix_web::test]
    async fn obsolete_session_head_returns_gone_without_upstream_access() {
        use crate::{SharedState, media_session::MediaSessions, cast::Quality};
        use std::sync::Arc;
        let sessions = Arc::new(MediaSessions::default());
        let old = sessions.activate("https://example.invalid/old.mp4", Quality::P720);
        sessions.activate("https://example.invalid/current.mp4", Quality::P720);
        let state = web::Data::new(SharedState {
            duration_cache: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            eplus_auth: Arc::new(tokio::sync::Mutex::new(None)),
            media_sessions: sessions,
        });
        let app = actix_web::test::init_service(App::new().app_data(state).app_data(web::Data::new(Client::new())).service(proxy_handler)).await;
        for method in [actix_web::http::Method::GET, actix_web::http::Method::HEAD] {
            let request = actix_web::test::TestRequest::with_uri(&format!("/{}", old.path())).method(method).to_request();
            let response = actix_web::test::call_service(&app, request).await;
            assert_eq!(response.status(), actix_web::http::StatusCode::GONE);
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
            .use_rustls_tls() // 强制使用 rustls
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
