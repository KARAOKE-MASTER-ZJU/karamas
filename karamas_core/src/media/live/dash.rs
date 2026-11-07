//! 将动态 DASH 清单映射为 fMP4 HLS，按需解密分片并保留媒体时间戳。
mod manifest;
use super::Resource;
use crate::{
    cast::Quality,
    media::{hls::master_playlist, mp4::cenc},
    sources::live::{Keys, fetch, invalid},
};
use manifest::{Manifest, Representation};
use std::{
    collections::VecDeque,
    fmt::Write,
    io::{self, Write as _},
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::NamedTempFile;
use tokio::sync::{Mutex as AsyncMutex, OnceCell};
use url::Url;

type File = Arc<NamedTempFile>;
type Entry = ((usize, u64), Arc<OnceCell<File>>);
struct Track {
    crypto: Arc<cenc::Track>,
    initialization: File,
}
pub(super) struct Dash {
    url: Url,
    referer: String,
    requested: Quality,
    quality: Quality,
    keys: Arc<Keys>,
    tracks: [Track; 2],
    manifest: AsyncMutex<Manifest>,
    cache: Mutex<VecDeque<Entry>>,
}

fn file(bytes: &[u8]) -> io::Result<File> {
    let mut file = tempfile::Builder::new()
        .prefix("live-")
        .tempfile_in(crate::media::session::cache_root()?)?;
    file.write_all(bytes)?;
    Ok(Arc::new(file))
}

impl Dash {
    pub async fn new(
        client: &reqwest::Client,
        url: Url,
        referer: &str,
        keys: Keys,
        quality: Quality,
    ) -> io::Result<Self> {
        let (effective, xml) = fetch(client, &url, referer, 2 * 1024 * 1024).await?;
        let manifest = manifest::parse(&xml, &effective, quality)?;
        async fn track(
            client: &reqwest::Client,
            rep: &Representation,
            referer: &str,
        ) -> io::Result<Track> {
            let (_, init) = fetch(client, &rep.init, referer, 1024 * 1024).await?;
            let crypto = Arc::new(cenc::Track::new(&init)?);
            if crypto.timescale != rep.timescale {
                return Err(invalid("清单与 MP4 时间基不一致"));
            }
            Ok(Track {
                initialization: file(&crypto.initialization)?,
                crypto,
            })
        }
        let (video, audio) = tokio::try_join!(
            track(client, &manifest.video, referer),
            track(client, &manifest.audio, referer)
        )?;
        let actual = match manifest.video.height {
            0..=240 => Quality::P240,
            241..=360 => Quality::P360,
            361..=480 => Quality::P480,
            481..=720 => Quality::P720,
            _ => Quality::P1080,
        };
        let warm = [&manifest.video, &manifest.audio].map(|rep| {
            // 优先避开直播边缘两片；短清单使用已有分片，不要求至少三片。
            rep.segments
                .keys()
                .rev()
                .nth(2.min(rep.segments.len().saturating_sub(1)))
                .copied()
        });
        let [video_warm, audio_warm] = warm;
        let video_warm = video_warm.ok_or_else(|| invalid("视频分片尚未就绪"))?;
        let audio_warm = audio_warm.ok_or_else(|| invalid("音频分片尚未就绪"))?;
        let result = Self {
            url,
            referer: referer.into(),
            requested: quality,
            quality: actual,
            keys: Arc::new(keys),
            tracks: [video, audio],
            manifest: AsyncMutex::new(manifest),
            cache: Mutex::new(VecDeque::new()),
        };
        // 交给播放端前读取两条轨道的真实分片，提前发现缺失密钥或不支持的格式。
        tokio::try_join!(
            result.segment(client, 0, video_warm),
            result.segment(client, 1, audio_warm)
        )?;
        Ok(result)
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    async fn refresh(
        &self,
        client: &reqwest::Client,
    ) -> io::Result<tokio::sync::MutexGuard<'_, Manifest>> {
        let mut current = self.manifest.lock().await;
        if current.refreshed.elapsed() >= Duration::from_secs(1) {
            let (effective, xml) = fetch(client, &self.url, &self.referer, 2 * 1024 * 1024).await?;
            let next = manifest::parse(&xml, &effective, self.requested)?;
            for (old, new) in [(&current.video, &next.video), (&current.audio, &next.audio)] {
                if current.period != next.period
                    || old.init != new.init
                    || old.kid != new.kid
                    || old.timescale != new.timescale
                    || old.codecs != new.codecs
                    || old.segments.last_key_value().map(|(n, _)| n)
                        > new.segments.last_key_value().map(|(n, _)| n)
                {
                    return Err(invalid("直播初始化信息或时间轴已变化，请重新播放"));
                }
            }
            *current = next;
        }
        Ok(current)
    }

    pub async fn resource(&self, client: &reqwest::Client, name: &str) -> io::Result<Resource> {
        match name {
            "video-init.mp4" => return Ok(Resource::File(self.tracks[0].initialization.clone())),
            "audio-init.mp4" => return Ok(Resource::File(self.tracks[1].initialization.clone())),
            "master.m3u8" | "video.m3u8" | "audio.m3u8" => {
                let current = self.refresh(client).await?;
                return Ok(Resource::Playlist(match name {
                    "master.m3u8" => master_playlist(
                        &current.video.codecs,
                        &current.audio.codecs,
                        current
                            .video
                            .bandwidth
                            .saturating_add(current.audio.bandwidth),
                    )?,
                    "video.m3u8" => playlist(&current.video, "video")?,
                    _ => playlist(&current.audio, "audio")?,
                }));
            }
            _ => {}
        }
        for (index, prefix) in ["video-", "audio-"].iter().enumerate() {
            if let Some(number) = name
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(".m4s"))
                .and_then(|n| n.parse().ok())
            {
                return Ok(Resource::File(self.segment(client, index, number).await?));
            }
        }
        Err(io::ErrorKind::NotFound.into())
    }

    async fn segment(
        &self,
        client: &reqwest::Client,
        track: usize,
        number: u64,
    ) -> io::Result<File> {
        let entry = {
            let mut cache = self.cache.lock().unwrap();
            if let Some((_, entry)) = cache.iter().find(|(key, _)| *key == (track, number)) {
                entry.clone()
            } else {
                let entry = Arc::new(OnceCell::new());
                if cache.len() >= 32 {
                    cache.pop_front();
                }
                cache.push_back(((track, number), entry.clone()));
                entry
            }
        };
        entry
            .get_or_try_init(|| async {
                let url = {
                    let current = self.manifest.lock().await;
                    let rep = if track == 0 {
                        &current.video
                    } else {
                        &current.audio
                    };
                    rep.segments
                        .get(&number)
                        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "直播分片已过期"))?
                        .url
                        .clone()
                };
                let (_, data) = fetch(client, &url, &self.referer, 32 * 1024 * 1024).await?;
                let crypto = self.tracks[track].crypto.clone();
                let keys = self.keys.clone();
                tokio::task::spawn_blocking(move || file(&crypto.decrypt(&data, &keys)?))
                    .await
                    .map_err(io::Error::other)?
            })
            .await
            .cloned()
    }
}

fn playlist(track: &Representation, prefix: &str) -> io::Result<String> {
    let mut parts: Vec<_> = track.segments.iter().rev().take(10).collect();
    parts.reverse();
    let first = parts.first().ok_or_else(|| invalid("直播时间轴为空"))?.0;
    let target = parts
        .iter()
        .map(|(_, p)| p.duration.div_ceil(u64::from(track.timescale)))
        .max()
        .unwrap()
        .max(1);
    let mut output = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-MAP:URI=\"{prefix}-init.mp4\"\n"
    );
    for (number, part) in parts {
        let _ = writeln!(
            output,
            "#EXTINF:{:.6},\n{prefix}-{number}.m4s",
            part.duration as f64 / f64::from(track.timescale)
        );
    }
    Ok(output)
}
