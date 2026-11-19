//! 将有 sidx 索引的 DASH 轨道描述为 HLS 清单，支持直连或通过会话代理下载。
use futures_util::StreamExt;
use std::{fmt::Write, io, ops::Range, sync::Arc, time::Duration};

mod delay;
mod recovery;
use delay::FragmentDelay;
pub(crate) use recovery::Source;

#[derive(Clone)]
pub(crate) struct Track {
    pub(crate) urls: Vec<String>,
    pub(crate) refresh: Option<crate::sources::bilibili::HlsRefresh>,
    codecs: String,
    bandwidth: u64,
    initialization: Range<u64>,
    index: Range<u64>,
}

impl Track {
    pub(crate) fn new(
        url: String,
        codecs: &str,
        bandwidth: u64,
        initialization: &str,
        index: &str,
    ) -> Option<Self> {
        fn range(value: &str) -> Option<Range<u64>> {
            let (start, end) = value.split_once('-')?;
            let start = start.parse::<u64>().ok()?;
            let end = end.parse::<u64>().ok()?.checked_add(1)?;
            (end > start && end - start <= 1024 * 1024).then_some(start..end)
        }
        let parsed = url::Url::parse(&url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https")
            || url.contains(['\r', '\n', '"'])
            || codecs.is_empty()
            || !codecs
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".,-".contains(&c))
        {
            return None;
        }
        Some(Self {
            urls: vec![parsed.into()],
            refresh: None,
            codecs: codecs.into(),
            bandwidth,
            initialization: range(initialization)?,
            index: range(index)?,
        })
    }
}

pub struct Playlists {
    master: String,
    video: String,
    audio: String,
    headers: String,
    sources: Option<[Arc<Source>; 2]>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Playlists {
    pub fn playlist(&self, name: &str) -> Option<&str> {
        match name {
            "master.m3u8" => Some(&self.master),
            "video.m3u8" => Some(&self.video),
            "audio.m3u8" => Some(&self.audio),
            _ => None,
        }
    }

    /// Kodi 等接收端通过 URL 请求选项将防盗链请求头传给上游媒体。
    pub fn request_headers(&self) -> &str {
        &self.headers
    }

    /// 本机播放使用固定会话路径，避免向 WebView 暴露上游地址和鉴权参数。
    pub fn source(&self, name: &str) -> Option<&str> {
        let sources = self.sources.as_ref()?;
        match name {
            "video.mp4" => Some(&sources[0].url),
            "audio.mp4" => Some(&sources[1].url),
            _ => None,
        }
    }

    pub(crate) fn proxy_source(&self, name: &str) -> Option<Arc<Source>> {
        let sources = self.sources.as_ref()?;
        match name {
            "video.mp4" => Some(sources[0].clone()),
            "audio.mp4" => Some(sources[1].clone()),
            _ => None,
        }
    }

    pub(crate) async fn prepare(
        client: &reqwest::Client,
        video: &Track,
        audio: &Track,
        proxy: bool,
        audio_delay_ms: i32,
    ) -> io::Result<Self> {
        let started = std::time::Instant::now();
        let (video_playlist, audio_playlist) = tokio::try_join!(
            load_playlist(
                client,
                video,
                proxy.then_some("video.mp4"),
                if proxy && audio_delay_ms < 0 {
                    audio_delay_ms.unsigned_abs()
                } else {
                    0
                }
            ),
            load_playlist(
                client,
                audio,
                proxy.then_some("audio.mp4"),
                if proxy && audio_delay_ms > 0 {
                    audio_delay_ms as u32
                } else {
                    0
                }
            ),
        )?;
        let master = master_playlist(
            &video.codecs,
            &audio.codecs,
            video.bandwidth.saturating_add(audio.bandwidth),
        )?;
        let headers = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("Referer", super::upstream::referer(&video.urls[0]))
            .append_pair("User-Agent", super::upstream::user_agent(&video.urls[0]))
            .finish();
        log::info!(target: "HLS", "HLS 清单就绪: proxy={}, elapsed_ms={}", proxy, started.elapsed().as_millis());
        Ok(Self {
            master,
            video: video_playlist.0,
            audio: audio_playlist.0,
            headers,
            sources: proxy.then(|| [Arc::new(video_playlist.1), Arc::new(audio_playlist.1)]),
        })
    }
}

async fn load_playlist(
    client: &reqwest::Client,
    track: &Track,
    media_url: Option<&str>,
    delay_ms: u32,
) -> io::Result<(String, Source)> {
    let mut selected = track.clone();
    let (data, total, segments, delay) = selected
        .recover(Duration::from_secs(60), |url, _| async move {
            let (data, total) = load_index(client, track, &url).await?;
            let segments = parse_index(&data, track.index.start, total)?;
            let delay = if delay_ms == 0 {
                None
            } else {
                let init =
                    super::upstream::fetch_range(client, &url, track.initialization.clone(), total)
                        .await?;
                Some(FragmentDelay::new(delay_ms, &init, &segments)?)
            };
            Ok((data, total, segments, delay))
        })
        .await?;
    let media_url = media_url.unwrap_or(&selected.urls[0]);
    let playlist = track.playlist(media_url, segments);
    Ok((playlist, Source::new(selected, total, data, delay)))
}

fn network_error(error: reqwest::Error) -> io::Error {
    let kind = if error.is_timeout() {
        io::ErrorKind::TimedOut
    } else {
        io::ErrorKind::ConnectionReset
    };
    io::Error::new(
        kind,
        format!("读取 HLS 索引失败：{:?}", error.without_url()),
    )
}

async fn load_index(
    client: &reqwest::Client,
    track: &Track,
    url: &str,
) -> io::Result<(Vec<u8>, u64)> {
    let expected = track.index.end - track.index.start;
    let response = client
        .get(url)
        .header(
            "Range",
            format!("bytes={}-{}", track.index.start, track.index.end - 1),
        )
        .header("Referer", super::upstream::referer(url))
        .header("User-Agent", super::upstream::user_agent(url))
        .header("Accept-Encoding", "identity")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(network_error)?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        let status = response.status();
        if !status.is_success() {
            return Err(io::Error::new(
                if status.is_server_error() {
                    io::ErrorKind::ConnectionRefused
                } else {
                    io::ErrorKind::PermissionDenied
                },
                format!("读取 HLS 索引失败：上游返回 HTTP {status}"),
            ));
        }
        return Err(invalid(&format!(
            "上游未按范围返回 HLS 索引（HTTP {status}）"
        )));
    }
    let content_range = response
        .headers()
        .get("Content-Range")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| invalid("HLS 媒体源缺少范围信息"))?;
    let (range, total) = content_range
        .rsplit_once('/')
        .ok_or_else(|| invalid("无效的媒体范围"))?;
    let total = total
        .parse::<u64>()
        .map_err(|_| invalid("无效的媒体长度"))?;
    if range != format!("bytes {}-{}", track.index.start, track.index.end - 1)
        || track.index.end > total
        || track.initialization.end > total
    {
        return Err(invalid("媒体索引范围不匹配"));
    }
    let mut data = Vec::with_capacity(expected as usize);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network_error)?;
        if data.len() + chunk.len() > expected as usize {
            return Err(invalid("媒体索引响应过长"));
        }
        data.extend_from_slice(&chunk);
    }
    if data.len() != expected as usize {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "媒体索引响应不完整",
        ));
    }
    Ok((data, total))
}

impl Track {
    fn playlist(&self, media_url: &str, segments: Vec<(Range<u64>, f64)>) -> String {
        let target = segments
            .iter()
            .map(|(_, d)| d.ceil() as u64)
            .max()
            .unwrap_or(1);
        let mut playlist = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"{}\",BYTERANGE=\"{}@{}\"\n",
            media_url,
            self.initialization.end - self.initialization.start,
            self.initialization.start,
        );
        for (range, duration) in segments {
            writeln!(
                playlist,
                "#EXTINF:{duration:.8},\n#EXT-X-BYTERANGE:{}@{}\n{}",
                range.end - range.start,
                range.start,
                media_url
            )
            .unwrap();
        }
        playlist.push_str("#EXT-X-ENDLIST\n");
        playlist
    }
}

fn parse_index(data: &[u8], position: u64, total: u64) -> io::Result<Vec<(Range<u64>, f64)>> {
    let u32_at = |p: usize| -> io::Result<u32> {
        Ok(u32::from_be_bytes(
            data.get(p..p + 4)
                .ok_or_else(|| invalid("sidx 不完整"))?
                .try_into()
                .unwrap(),
        ))
    };
    let u64_at = |p: usize| -> io::Result<u64> {
        Ok(u64::from_be_bytes(
            data.get(p..p + 8)
                .ok_or_else(|| invalid("sidx 不完整"))?
                .try_into()
                .unwrap(),
        ))
    };
    let (size, header) = if u32_at(0)? == 1 {
        (u64_at(8)?, 16)
    } else {
        (u32_at(0)? as u64, 8)
    };
    if size != data.len() as u64 || data.get(4..8) != Some(b"sidx") {
        return Err(invalid("无效的 sidx"));
    }
    let version = *data.get(header).ok_or_else(|| invalid("sidx 不完整"))?;
    let scale = u32_at(header + 8)?;
    if version > 1 || scale == 0 {
        return Err(invalid("不支持的 sidx"));
    }
    let (first, mut p) = if version == 0 {
        (u32_at(header + 16)? as u64, header + 20)
    } else {
        (u64_at(header + 20)?, header + 28)
    };
    let count = (u32_at(p)? & 0xffff) as usize;
    p += 4;
    if count == 0 || count > 8192 {
        return Err(invalid("不支持的媒体分段数量"));
    }
    let mut offset = position
        .checked_add(size)
        .and_then(|n| n.checked_add(first))
        .ok_or_else(|| invalid("分段偏移溢出"))?;
    let mut segments = Vec::with_capacity(count);
    for _ in 0..count {
        let reference = u32_at(p)?;
        let duration = u32_at(p + 4)?;
        u32_at(p + 8)?;
        if reference >> 31 != 0 || reference < 8 || duration == 0 {
            return Err(invalid("不支持的媒体分段"));
        }
        let end = offset
            .checked_add(reference as u64)
            .ok_or_else(|| invalid("分段偏移溢出"))?;
        if end > total {
            return Err(invalid("分段超出源文件"));
        }
        segments.push((offset..end, duration as f64 / scale as f64));
        offset = end;
        p += 12;
    }
    if p != data.len() {
        return Err(invalid("sidx 长度不匹配"));
    }
    Ok(segments)
}

/// 点播与直播共用同一份音视频主清单格式。
pub(crate) fn master_playlist(video: &str, audio: &str, bandwidth: u64) -> io::Result<String> {
    if [video, audio].iter().any(|codec| {
        codec.is_empty()
            || !codec
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".,-".contains(&c))
    }) {
        return Err(invalid("HLS 编码描述无效"));
    }
    Ok(format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"audio\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"{},{}\",AUDIO=\"audio\"\nvideo.m3u8\n",
        bandwidth.max(1),
        video,
        audio
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};

    // 索引读取失败后仍须能开播；直连清单必须使用成功的 CDN，代理清单必须保留本机路径。
    #[actix_web::test]
    async fn playlists_use_the_working_cdn_after_index_failure() {
        let mut index = Vec::new();
        index.extend_from_slice(&44u32.to_be_bytes());
        index.extend_from_slice(b"sidx");
        for value in [0u32, 1, 1000, 0, 0, 1, 100, 5000, 0x9000_0000] {
            index.extend_from_slice(&value.to_be_bytes());
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(move || {
            let index = index.clone();
            App::new().default_service(web::to(move |req: HttpRequest| {
                let index = index.clone();
                async move {
                    if req.path() == "/primary" {
                        return HttpResponse::Forbidden().finish();
                    }
                    assert_eq!(req.headers().get("Range").unwrap(), "bytes=16-59");
                    HttpResponse::PartialContent()
                        .insert_header(("Content-Range", "bytes 16-59/160"))
                        .body(index)
                }
            }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let mut track = Track::new(
            format!("http://{address}/primary"),
            "avc1.640033",
            1000,
            "0-15",
            "16-59",
        )
        .unwrap();
        let backup = format!("http://{address}/backup");
        track.urls.push(backup.clone());
        let client = reqwest::Client::new();
        let proxy = Playlists::prepare(&client, &track, &track, true, 0)
            .await
            .unwrap();
        assert_eq!(proxy.source("video.mp4"), Some(backup.as_str()));
        assert!(
            proxy
                .playlist("video.m3u8")
                .unwrap()
                .contains("URI=\"video.mp4\"")
        );
        let direct = Playlists::prepare(&client, &track, &track, false, 0)
            .await
            .unwrap();
        assert!(direct.playlist("video.m3u8").unwrap().contains(&backup));
        assert!(!direct.playlist("video.m3u8").unwrap().contains("/primary"));
        handle.stop(true).await;
    }

    // HLS 开播只读分段目录；视频变长时不能退回逐片扫描，也不能把本机路径改回上游地址。
    #[actix_web::test]
    async fn long_tracks_prepare_from_indexes_for_proxy_and_direct_playback() {
        let count = 2048u32;
        let mut index = Vec::new();
        index.extend_from_slice(&(32 + count * 12).to_be_bytes());
        index.extend_from_slice(b"sidx");
        for value in [0u32, 1, 1000, 0, 0, count] {
            index.extend_from_slice(&value.to_be_bytes());
        }
        for _ in 0..count {
            for value in [100u32, 5000, 0x9000_0000] {
                index.extend_from_slice(&value.to_be_bytes());
            }
        }
        let end = 16 + index.len() as u64;
        let total = end + u64::from(count) * 100;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(move || {
            let index = index.clone();
            App::new().default_service(web::to(move |req: HttpRequest| {
                let index = index.clone();
                async move {
                    assert_eq!(
                        req.headers().get("Range").unwrap().to_str().unwrap(),
                        format!("bytes=16-{}", end - 1)
                    );
                    assert_eq!(
                        req.headers().get("Referer").unwrap().to_str().unwrap(),
                        "https://www.bilibili.com/"
                    );
                    HttpResponse::PartialContent()
                        .insert_header(("Content-Range", format!("bytes 16-{}/{total}", end - 1)))
                        .body(index)
                }
            }))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let track = |name| {
            Track::new(
                format!("http://{address}/{name}?token=private"),
                if name == "video" {
                    "avc1.640033"
                } else {
                    "mp4a.40.2"
                },
                1000,
                "0-15",
                &format!("16-{}", end - 1),
            )
            .unwrap()
        };
        let (video, audio) = (track("video"), track("audio"));
        let client = reqwest::Client::new();
        let proxy = Playlists::prepare(&client, &video, &audio, true, 0)
            .await
            .unwrap();
        assert_eq!(proxy.source("video.mp4"), Some(video.urls[0].as_str()));
        assert_eq!(proxy.source("audio.mp4"), Some(audio.urls[0].as_str()));
        for (name, path) in [("video.m3u8", "video.mp4"), ("audio.m3u8", "audio.mp4")] {
            let playlist = proxy.playlist(name).unwrap();
            assert_eq!(playlist.matches("#EXTINF:").count(), count as usize);
            assert!(playlist.contains(&format!("URI=\"{path}\"")));
            assert!(!playlist.contains("token=private"));
        }
        let direct = Playlists::prepare(&client, &video, &audio, false, 0)
            .await
            .unwrap();
        assert!(direct.source("video.mp4").is_none());
        assert!(
            direct
                .playlist("video.m3u8")
                .unwrap()
                .contains(&video.urls[0])
        );
        assert!(direct.request_headers().contains("Referer="));
        handle.stop(true).await;
    }
}
