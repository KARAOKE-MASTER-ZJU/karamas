//! 识别媒体来源、选择可用清晰度并准备 DASH、HLS 或直播媒体。
use super::{MediaSession, PlaybackMediaType, SessionMedia, cache_root, stopped};
use crate::{
    cast::{MediaFormat, Quality, select_media_quality},
    media::{cache::SongCache, mp4::dash},
    sources::bilibili::{BilibiliMedia, get_bilibili_media_with_formats, parse_bilibili_video},
};
use std::{io, sync::Arc, time::Duration};

impl MediaSession {
    pub(crate) async fn detect_live(&self, client: &reqwest::Client) -> io::Result<bool> {
        tokio::select! {
            biased;
            _ = self.cancelled() => Err(stopped()),
            result = self.live_source.get_or_try_init(|| crate::sources::live::resolve(client, &self.song)) => {
                result.map(|source| source.is_some())
            }
        }
    }

    /// 解析歌曲媒体源，按需准备可定位的 DASH 媒体。
    pub(super) async fn resolve(&self, client: &reqwest::Client) -> io::Result<SessionMedia> {
        if url::Url::parse(&self.song).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            && !crate::sources::youtube::is_youtube_url(&self.song)
            && parse_bilibili_video(&self.song).is_err()
        {
            use crate::sources::http::Source;
            let media = match crate::sources::http::resolve(client, &self.song).await? {
                Source::Native(url) => SessionMedia::Direct(url.into()),
                Source::Mp4(url) => self.prepare_mp4(client, url.into()).await?,
                Source::Flv { url, filesize } => SessionMedia::Flv { url, filesize },
                Source::Live(source) => SessionMedia::Live(
                    crate::media::live::LiveMedia::prepare(
                        client,
                        source,
                        self.quality,
                        self.audio_delay_ms,
                    )
                    .await?,
                ),
            };
            let format = if media.playback_type() == PlaybackMediaType::Hls {
                MediaFormat::Hls
            } else {
                MediaFormat::Mp4
            };
            let quality = match &media {
                SessionMedia::Live(media) => media.quality(),
                _ => self.quality,
            };
            *self.resolved_quality.lock().unwrap() = Some((quality, format));
            return Ok(media);
        }
        // 直播页先取源，再进入通用 HTTP 直链分支，避免把网页 HTML 当作 MP4 转发。
        if self.detect_live(client).await? {
            let source = self
                .live_source
                .get()
                .and_then(Option::as_ref)
                .unwrap()
                .clone();
            let media = crate::media::live::LiveMedia::prepare(
                client,
                source,
                self.quality,
                self.audio_delay_ms,
            )
            .await?;
            *self.resolved_quality.lock().unwrap() = Some((media.quality(), MediaFormat::Hls));
            return Ok(SessionMedia::Live(media));
        }
        if crate::sources::youtube::is_youtube_url(&self.song) {
            let selected = select_media_quality(
                (self.quality, self.format),
                self.allowed_formats
                    .iter()
                    .copied()
                    .filter(|format| *format != MediaFormat::Mp4)
                    .flat_map(|format| {
                        [Quality::P720, Quality::P1080].map(|quality| (quality, format))
                    }),
            )
            .ok_or_else(|| io::Error::other("YouTube 没有符合目标的可选清晰度"))?;
            let max_height = match selected.0 {
                Quality::P1080 => 1080,
                _ => 720,
            };
            let tracks = crate::sources::youtube::resolve(&self.song, max_height).await?;
            log::info!(target: "DLNA1080", "YouTube 双轨就绪: generation={}, duration={}s", self.generation, tracks.duration_secs);
            let media = self
                .prepare_dash(
                    client,
                    &tracks.video_url,
                    &tracks.audio_url,
                    tracks.hls_tracks.as_ref(),
                    selected.1,
                )
                .await?;
            *self.resolved_quality.lock().unwrap() = Some(selected);
            return Ok(media);
        }
        let bilibili = parse_bilibili_video(&self.song);
        let (bvid, page) = bilibili.map_err(io::Error::other)?;
        let resolved = get_bilibili_media_with_formats(
            &bvid,
            Some(page),
            self.quality.as_qn(),
            self.format,
            &self.allowed_formats,
        )
        .await
        .map_err(io::Error::other)?;
        let media = match resolved.media {
            BilibiliMedia::Direct { url } => self.prepare_mp4(client, url).await?,
            BilibiliMedia::Dash {
                video_url,
                audio_url,
                ..
            } => {
                self.prepare_dash(
                    client,
                    &video_url,
                    &audio_url,
                    resolved.hls_tracks.as_ref(),
                    resolved.format,
                )
                .await?
            }
        };
        *self.resolved_quality.lock().unwrap() = Some((resolved.quality, resolved.format));
        Ok(media)
    }

    /// 构建 DASH 媒体的 MP4 索引和歌曲缓存。
    async fn prepare_dash(
        &self,
        client: &reqwest::Client,
        video_url: &str,
        audio_url: &str,
        hls_tracks: Option<&(crate::media::hls::Track, crate::media::hls::Track)>,
        format: MediaFormat,
    ) -> io::Result<SessionMedia> {
        if format == MediaFormat::Hls {
            let (video, audio) =
                hls_tracks.ok_or_else(|| io::Error::other("媒体源缺少 HLS 所需的分段信息"))?;
            return Ok(SessionMedia::Hls(
                crate::media::hls::Playlists::prepare(
                    client,
                    video,
                    audio,
                    !self.hls_direct,
                    if self.hls_direct {
                        0
                    } else {
                        self.audio_delay_ms
                    },
                )
                .await?,
            ));
        }
        let prepared = tokio::time::timeout(
            Duration::from_secs(60),
            dash::prepare_with_audio_delay(client, video_url, audio_url, self.audio_delay_ms),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MP4 preparation timeout"))??;
        cache_media(client, prepared)
    }

    async fn prepare_mp4(&self, client: &reqwest::Client, url: String) -> io::Result<SessionMedia> {
        if self.audio_delay_ms == 0 || self.hls_direct {
            return Ok(SessionMedia::Mp4(url));
        }
        let prepared = tokio::time::timeout(
            Duration::from_secs(60),
            crate::media::mp4::indexed::prepare_progressive(client, &url, self.audio_delay_ms),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MP4 偏移处理超时"))??;
        cache_media(client, prepared)
    }
}

fn cache_media(client: &reqwest::Client, prepared: dash::PreparedDash) -> io::Result<SessionMedia> {
    let cache = Arc::new(SongCache::new(
        &cache_root()?,
        client.clone(),
        prepared.sources,
        256 * 1024 * 1024,
    )?);
    Ok(SessionMedia::Seekable {
        mp4: Arc::new(prepared.mp4),
        cache,
    })
}
