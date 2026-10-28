//! 将有 sidx 索引的 DASH 轨道描述为 HLS 清单，媒体仍由接收端直连下载。
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

    pub(crate) async fn prepare(
        client: &reqwest::Client,
        video: &Track,
        audio: &Track,
    ) -> io::Result<Self> {
        let started = std::time::Instant::now();
        let (video_playlist, audio_playlist) =
            tokio::try_join!(load_playlist(client, video), load_playlist(client, audio),)?;
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
        log::info!(target: "DLNA_HLS", "HLS 直链清单就绪: elapsed_ms={}", started.elapsed().as_millis());
        Ok(Self {
            master,
            video: video_playlist,
            audio: audio_playlist,
            headers,
        })
    }
}

async fn load_playlist(client: &reqwest::Client, track: &Track) -> io::Result<String> {
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
        return Err(invalid("HLS 直链源不支持分段读取"));
    }
    let content_range = response
        .headers()
        .get("Content-Range")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| invalid("HLS 直链源缺少范围信息"))?;
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
        track.url,
        track.initialization.end - track.initialization.start,
        track.initialization.start,
    );
    for (range, duration) in segments {
        writeln!(
            playlist,
            "#EXTINF:{duration:.8},\n#EXT-X-BYTERANGE:{}@{}\n{}",
            range.end - range.start,
            range.start,
            track.url
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
