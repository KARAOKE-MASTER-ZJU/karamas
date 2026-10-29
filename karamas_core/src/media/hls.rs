//! 将有 sidx 索引的 DASH 轨道描述为 HLS 清单，支持直连或通过会话代理下载。
use futures_util::StreamExt;
use std::{fmt::Write, io, ops::Range, time::Duration};

#[derive(Clone)]
pub(crate) struct Track {
    url: String,
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
            url: parsed.into(),
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
    sources: Option<[String; 2]>,
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
            "video.mp4" => Some(&sources[0]),
            "audio.mp4" => Some(&sources[1]),
            _ => None,
        }
    }

    pub(crate) async fn prepare(
        client: &reqwest::Client,
        video: &Track,
        audio: &Track,
        proxy: bool,
    ) -> io::Result<Self> {
        let started = std::time::Instant::now();
        let (video_playlist, audio_playlist) = tokio::try_join!(
            load_playlist(client, video, if proxy { "video.mp4" } else { &video.url }),
            load_playlist(client, audio, if proxy { "audio.mp4" } else { &audio.url }),
        )?;
        let master = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"audio\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"{},{}\",AUDIO=\"audio\"\nvideo.m3u8\n",
            video.bandwidth.saturating_add(audio.bandwidth).max(1),
            video.codecs,
            audio.codecs,
        );
        let headers = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("Referer", super::upstream::referer(&video.url))
            .append_pair("User-Agent", super::upstream::user_agent(&video.url))
            .finish();
        log::info!(target: "HLS", "HLS 清单就绪: proxy={}, elapsed_ms={}", proxy, started.elapsed().as_millis());
        Ok(Self {
            master,
            video: video_playlist,
            audio: audio_playlist,
            headers,
            sources: proxy.then(|| [video.url.clone(), audio.url.clone()]),
        })
    }
}

async fn load_playlist(
    client: &reqwest::Client,
    track: &Track,
    media_url: &str,
) -> io::Result<String> {
    let expected = track.index.end - track.index.start;
    let response = client
        .get(&track.url)
        .header(
            "Range",
            format!("bytes={}-{}", track.index.start, track.index.end - 1),
        )
        .header("Referer", super::upstream::referer(&track.url))
        .header("User-Agent", super::upstream::user_agent(&track.url))
        .header("Accept-Encoding", "identity")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| io::Error::other(e.without_url()))?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        let status = response.status();
        if !status.is_success() {
            return Err(io::Error::other(format!(
                "读取 HLS 索引失败：上游返回 HTTP {status}"
            )));
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
        let chunk = chunk.map_err(|e| io::Error::other(e.without_url()))?;
        if data.len() + chunk.len() > expected as usize {
            return Err(invalid("媒体索引响应过长"));
        }
        data.extend_from_slice(&chunk);
    }
    if data.len() != expected as usize {
        return Err(invalid("媒体索引响应不完整"));
    }
    let segments = parse_index(&data, track.index.start, total)?;
    let target = segments
        .iter()
        .map(|(_, d)| d.ceil() as u64)
        .max()
        .unwrap_or(1);
    let mut playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"{}\",BYTERANGE=\"{}@{}\"\n",
        media_url,
        track.initialization.end - track.initialization.start,
        track.initialization.start,
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
    Ok(playlist)
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

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

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
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(move || {
            let index = index.clone();
            let seen = seen.clone();
            App::new().default_service(web::to(move |req: HttpRequest| {
                let index = index.clone();
                let seen = seen.clone();
                async move {
                    assert_eq!(
                        req.headers().get("Range").unwrap().to_str().unwrap(),
                        format!("bytes=16-{}", end - 1)
                    );
                    assert_eq!(
                        req.headers().get("Referer").unwrap().to_str().unwrap(),
                        "https://www.bilibili.com/"
                    );
                    seen.fetch_add(1, Ordering::Relaxed);
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
        let proxy = Playlists::prepare(&client, &video, &audio, true)
            .await
            .unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        assert_eq!(proxy.source("video.mp4"), Some(video.url.as_str()));
        assert_eq!(proxy.source("audio.mp4"), Some(audio.url.as_str()));
        for (name, path) in [("video.m3u8", "video.mp4"), ("audio.m3u8", "audio.mp4")] {
            let playlist = proxy.playlist(name).unwrap();
            assert_eq!(playlist.matches("#EXTINF:").count(), count as usize);
            assert!(playlist.contains(&format!("URI=\"{path}\"")));
            assert!(!playlist.contains("token=private"));
        }
        let direct = Playlists::prepare(&client, &video, &audio, false)
            .await
            .unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 4);
        assert!(direct.source("video.mp4").is_none());
        assert!(direct.playlist("video.m3u8").unwrap().contains(&video.url));
        assert!(direct.request_headers().contains("Referer="));
        handle.stop(true).await;
    }
}
