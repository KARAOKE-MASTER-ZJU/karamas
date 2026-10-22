//! Minimal streaming fragmented-MP4 muxer for Bilibili DASH tracks.
//!
//! Bilibili serves video and audio as separate single-track fMP4 files. Both
//! files consist of `ftyp + moov + sidx + (moof + mdat)*`. We build a new
//! two-track initialization segment, assign track id 2 to audio, then emit the
//! original fragments in decode-time order while patching only `mfhd` and
//! `tfhd`. Sample bytes are never decoded or copied through a codec.

use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt};
use std::io;
use std::ops::Range;
use std::pin::Pin;
use tokio::sync::mpsc;

const MAX_BOX_SIZE: usize = 64 * 1024 * 1024;
const VIDEO_TRACK_ID: u32 = 1;
const AUDIO_TRACK_ID: u32 = 2;

pub type MuxByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>;

pub struct MuxedMp4 {
    pub stream: MuxByteStream,
    pub content_length: Option<u64>,
}

type UpstreamStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

struct BoxReader {
    stream: UpstreamStream,
    buffered: BytesMut,
    eof: bool,
}

impl BoxReader {
    async fn open(client: &reqwest::Client, url: &str, track: &'static str) -> io::Result<Self> {
        Self::open_at(client, url, track, None).await
    }

    async fn open_at(
        client: &reqwest::Client,
        url: &str,
        track: &'static str,
        offset: Option<u64>,
    ) -> io::Result<Self> {
        let endpoint = safe_endpoint(url);
        if let Some(offset) = offset {
            log::info!(target: "DLNA1080", "连接 {} 上游定位流: {}, offset={}B", track, endpoint, offset);
        } else {
            log::info!(target: "DLNA1080", "连接 {} 上游: {}", track, endpoint);
        }
        let mut request = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0")
            .header("Referer", crate::upstream::referer(url));
        if let Some(offset) = offset {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let response = request
            .send()
            .await
            .map_err(|err| {
                log::error!(target: "DLNA1080", "连接 {} 上游失败: {}, error={}", track, endpoint, err);
                io::Error::other(err)
            })?
            .error_for_status()
            .map_err(|err| {
                log::error!(target: "DLNA1080", "{} 上游 HTTP 失败: {}, error={}", track, endpoint, err);
                io::Error::other(err)
            })?;
        if offset.is_some() && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(invalid(format!(
                "{track} upstream ignored byte range: status={}",
                response.status()
            )));
        }
        log::info!(target: "DLNA1080", "{} 上游连接成功: status={}", track, response.status());
        Ok(Self {
            stream: Box::pin(response.bytes_stream()),
            buffered: BytesMut::new(),
            eof: false,
        })
    }

    async fn next_box(&mut self) -> io::Result<Option<Bytes>> {
        loop {
            if self.buffered.len() >= 8 {
                let header_len = if read_u32(&self.buffered, 0)? == 1 {
                    16
                } else {
                    8
                };
                if self.buffered.len() >= header_len {
                    let size = box_size_prefix(&self.buffered)?;
                    if size < header_len || size > MAX_BOX_SIZE {
                        return Err(invalid(format!("invalid fMP4 box size {size}")));
                    }
                    if self.buffered.len() >= size {
                        return Ok(Some(self.buffered.split_to(size).freeze()));
                    }
                }
            }

            if self.eof {
                if self.buffered.is_empty() {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated fMP4 box",
                ));
            }
            match self.stream.next().await {
                Some(Ok(chunk)) => self.buffered.extend_from_slice(&chunk),
                Some(Err(err)) => return Err(io::Error::other(err)),
                None => self.eof = true,
            }
        }
    }
}

struct TrackSource {
    reader: BoxReader,
    moov: Bytes,
    pending_moof: Option<Bytes>,
    sidx: Option<SidxInfo>,
    url: String,
    track: &'static str,
}

#[derive(Clone, Debug)]
struct SidxInfo {
    referenced_bytes: u64,
    duration: u64,
    timescale: u32,
    earliest_presentation_time: u64,
    first_media_offset: u64,
    entries: Vec<SidxEntry>,
}

#[derive(Clone, Copy, Debug)]
struct SidxEntry {
    referenced_size: u32,
    subsegment_duration: u32,
}

impl TrackSource {
    async fn open(
        client: &reqwest::Client,
        url: &str,
        track: &'static str,
    ) -> io::Result<(Bytes, Self)> {
        let mut reader = BoxReader::open(client, url, track).await?;
        let mut ftyp = None;
        let mut moov = None;
        let mut pending_moof = None;
        let mut sidx = None;
        let mut bytes_before_atom = 0u64;

        for _ in 0..32 {
            let Some(atom) = reader.next_box().await? else {
                break;
            };
            let atom_len = atom.len() as u64;
            match atom_type(&atom)? {
                b"ftyp" => ftyp = Some(atom),
                b"moov" => moov = Some(atom),
                b"sidx" => {
                    sidx = parse_sidx(&atom).ok().map(|mut index| {
                        index.first_media_offset =
                            bytes_before_atom + atom.len() as u64 + index.first_media_offset;
                        index
                    });
                }
                b"moof" => {
                    pending_moof = Some(atom);
                    break;
                }
                _ => {}
            }
            bytes_before_atom += atom_len;
        }

        let ftyp = ftyp.ok_or_else(|| invalid("fMP4 source has no ftyp"))?;
        let moov = moov.ok_or_else(|| invalid("fMP4 source has no moov"))?;
        log::info!(
            target: "DLNA1080",
            "{} 初始化段读取完成: ftyp={}B, moov={}B, indexed_media={}B",
            track,
            ftyp.len(),
            moov.len(),
            sidx.as_ref().map_or_else(|| "unknown".to_string(), |info| info.referenced_bytes.to_string())
        );
        Ok((
            ftyp,
            Self {
                reader,
                moov,
                pending_moof,
                sidx,
                url: url.to_owned(),
                track,
            },
        ))
    }

    async fn seek_to(&mut self, client: &reqwest::Client, start_secs: u32) -> io::Result<()> {
        let Some(sidx) = self.sidx.as_ref() else {
            return Err(invalid("fMP4 source has no sidx seek index"));
        };
        let Some((offset, segment_secs)) = sidx_offset_for_start(sidx, start_secs) else {
            return Err(invalid(format!(
                "seek position {start_secs}s is outside sidx index"
            )));
        };
        let reader = BoxReader::open_at(client, &self.url, self.track, Some(offset)).await?;
        self.reader = reader;
        self.pending_moof = None;
        log::info!(
            target: "DLNA1080",
            "{} 定位索引命中: requested={}s, segment_start={:.3}s, upstream_offset={}B",
            self.track,
            start_secs,
            segment_secs,
            offset
        );
        Ok(())
    }

    async fn next_fragment(&mut self) -> io::Result<Option<Fragment>> {
        let moof = if let Some(moof) = self.pending_moof.take() {
            moof
        } else {
            loop {
                let Some(atom) = self.reader.next_box().await? else {
                    return Ok(None);
                };
                if atom_type(&atom)? == b"moof" {
                    break atom;
                }
            }
        };
        let decode_time = fragment_decode_time(&moof)?;
        loop {
            let atom =
                self.reader.next_box().await?.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "moof without mdat")
                })?;
            if atom_type(&atom)? == b"mdat" {
                return Ok(Some(Fragment {
                    moof,
                    mdat: atom,
                    decode_time,
                }));
            }
        }
    }
}

struct Fragment {
    moof: Bytes,
    mdat: Bytes,
    decode_time: u64,
}

/// Start an online mux of two Bilibili DASH fMP4 tracks.
pub async fn mux_dash(
    client: &reqwest::Client,
    video_url: &str,
    audio_url: &str,
) -> io::Result<MuxedMp4> {
    mux_dash_from(client, video_url, audio_url, 0).await
}

/// Start an online mux at the closest subsequent fMP4 fragment boundary.
///
/// DLNA renderers commonly seek with a byte `Range`, which cannot address an
/// interleaved stream we produce on the fly. Reopening a URL with `start` lets
/// the caster seek by media time instead, while keeping all muxing in Rust.
pub async fn mux_dash_from(
    client: &reqwest::Client,
    video_url: &str,
    audio_url: &str,
    start_secs: u32,
) -> io::Result<MuxedMp4> {
    log::info!(target: "DLNA1080", "开始纯 Rust 在线 fMP4 混流: start={}s", start_secs);
    let ((video_ftyp, mut video), (_audio_ftyp, mut audio)) = tokio::try_join!(
        TrackSource::open(client, video_url, "视频"),
        TrackSource::open(client, audio_url, "音频"),
    )?;

    let video_timescale = track_timescale(&video.moov)?;
    let audio_timescale = track_timescale(&audio.moov)?;
    let init = merge_initialization(
        &video_ftyp,
        &video.moov,
        &audio.moov,
        video.sidx.as_ref(),
        audio.sidx.as_ref(),
    )?;
    let content_length = if start_secs == 0 {
        video
            .sidx
            .as_ref()
            .zip(audio.sidx.as_ref())
            .map(|(v, a)| init.len() as u64 + v.referenced_bytes + a.referenced_bytes)
    } else {
        None
    };
    log::info!(
        target: "DLNA1080",
        "双轨初始化段合并完成: init={}B, video_timescale={}, audio_timescale={}, content_length={}",
        init.len(),
        video_timescale,
        audio_timescale,
        content_length.map_or_else(|| "unknown".to_string(), |size| size.to_string())
    );

    if start_secs > 0 {
        if let Err(error) = video.seek_to(client, start_secs).await {
            log::warn!(target: "DLNA1080", "视频无法按索引定位，将顺序跳过片段: {}", error);
        }
        if let Err(error) = audio.seek_to(client, start_secs).await {
            log::warn!(target: "DLNA1080", "音频无法按索引定位，将顺序跳过片段: {}", error);
        }
    }

    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(4);
    tokio::spawn(async move {
        if tx.send(Ok(init)).await.is_err() {
            log::warn!(target: "DLNA1080", "下游在初始化段发送前已断开");
            return;
        }
        let result = mux_fragments(
            &tx,
            &mut video,
            &mut audio,
            video_timescale,
            audio_timescale,
            start_secs,
        )
        .await;
        if let Err(err) = result {
            log::error!(target: "DLNA1080", "在线混流失败: {}", err);
            let _ = tx.send(Err(err)).await;
        }
    });

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    Ok(MuxedMp4 {
        stream: Box::pin(stream),
        content_length,
    })
}

async fn mux_fragments(
    tx: &mpsc::Sender<Result<Bytes, io::Error>>,
    video: &mut TrackSource,
    audio: &mut TrackSource,
    video_timescale: u32,
    audio_timescale: u32,
    start_secs: u32,
) -> io::Result<()> {
    let mut next_video = video.next_fragment().await?;
    let mut next_audio = audio.next_fragment().await?;
    let mut sequence = 1u32;
    let mut video_fragments = 0u64;
    let mut audio_fragments = 0u64;
    let mut emitted_bytes = 0u64;
    let mut skipped_video = 0u64;
    let mut skipped_audio = 0u64;
    let mut logged_seek_start = start_secs == 0;

    while next_video.is_some() || next_audio.is_some() {
        let take_video = match (&next_video, &next_audio) {
            (Some(v), Some(a)) => {
                (v.decode_time as u128) * (audio_timescale as u128)
                    <= (a.decode_time as u128) * (video_timescale as u128)
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };

        let (mut fragment, track_id) = if take_video {
            (next_video.take().expect("checked"), VIDEO_TRACK_ID)
        } else {
            (next_audio.take().expect("checked"), AUDIO_TRACK_ID)
        };
        let track_timescale = if take_video {
            video_timescale
        } else {
            audio_timescale
        };
        if !fragment_is_at_or_after(fragment.decode_time, track_timescale, start_secs) {
            if take_video {
                skipped_video += 1;
                next_video = video.next_fragment().await?;
            } else {
                skipped_audio += 1;
                next_audio = audio.next_fragment().await?;
            }
            continue;
        }
        if !logged_seek_start {
            log::info!(
                target: "DLNA1080",
                "1080P 定位流开始输出: requested={}s, skipped_video={}, skipped_audio={}",
                start_secs,
                skipped_video,
                skipped_audio
            );
            logged_seek_start = true;
        }
        fragment.moof = patch_moof(&fragment.moof, track_id, sequence)?;
        let fragment_bytes = (fragment.moof.len() + fragment.mdat.len()) as u64;
        emitted_bytes += fragment_bytes;
        if take_video {
            video_fragments += 1;
        } else {
            audio_fragments += 1;
        }
        sequence = sequence.wrapping_add(1);
        if tx.send(Ok(fragment.moof)).await.is_err() || tx.send(Ok(fragment.mdat)).await.is_err() {
            log::warn!(
                target: "DLNA1080",
                "DLNA 下游中途断开: video_fragments={}, audio_fragments={}, emitted={}B",
                video_fragments,
                audio_fragments,
                emitted_bytes
            );
            return Ok(());
        }

        let total_fragments = video_fragments + audio_fragments;
        if total_fragments % 100 == 0 {
            log::info!(
                target: "DLNA1080",
                "混流进度: fragments={} (video={}, audio={}), emitted={}B",
                total_fragments,
                video_fragments,
                audio_fragments,
                emitted_bytes
            );
        }

        if take_video {
            next_video = video.next_fragment().await?;
        } else {
            next_audio = audio.next_fragment().await?;
        }
    }
    log::info!(
        target: "DLNA1080",
        "在线混流完成: video_fragments={}, audio_fragments={}, emitted={}B",
        video_fragments,
        audio_fragments,
        emitted_bytes
    );
    Ok(())
}

fn fragment_is_at_or_after(decode_time: u64, timescale: u32, start_secs: u32) -> bool {
    decode_time >= u64::from(start_secs) * u64::from(timescale)
}

fn safe_endpoint(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| {
            format!(
                "{}://{}{}",
                url.scheme(),
                url.host_str().unwrap_or("unknown"),
                url.path()
            )
        })
        .unwrap_or_else(|_| "<invalid-url>".to_string())
}

fn merge_initialization(
    video_ftyp: &Bytes,
    video_moov: &Bytes,
    audio_moov: &Bytes,
    video_sidx: Option<&SidxInfo>,
    audio_sidx: Option<&SidxInfo>,
) -> io::Result<Bytes> {
    let mut mvhd = child(video_moov, b"mvhd")?.to_vec();
    let mut video_trak = child(video_moov, b"trak")?.to_vec();
    let mut audio_trak = child(audio_moov, b"trak")?.to_vec();
    let video_mvex = child(video_moov, b"mvex")?;
    let audio_mvex = child(audio_moov, b"mvex")?;
    let mut video_trex = child(video_mvex, b"trex")?.to_vec();
    let mut audio_trex = child(audio_mvex, b"trex")?.to_vec();

    patch_mvhd_next_track_id(&mut mvhd, 3)?;
    patch_trak_track_id(&mut video_trak, VIDEO_TRACK_ID)?;
    patch_trak_track_id(&mut audio_trak, AUDIO_TRACK_ID)?;
    patch_full_box_u32(&mut video_trex, 12, VIDEO_TRACK_ID)?;
    patch_full_box_u32(&mut audio_trex, 12, AUDIO_TRACK_ID)?;

    // Bilibili 的 fragmented MP4 会把总时长放在 sidx，而 moov 中的
    // duration 通常是 0。我们不会把单轨 sidx 原样拼进双轨文件，但必须
    // 将其中的时长写入初始化段，否则部分 DLNA renderer（Kodi）只会根据
    // 第一个片段推断出约 5 秒的总时长，进而触发错误的自动切歌。
    if let (Some(video_sidx), Some(audio_sidx)) = (video_sidx, audio_sidx) {
        let movie_timescale = mvhd_timescale(&mvhd)?;
        let video_duration =
            scale_duration(video_sidx.duration, video_sidx.timescale, movie_timescale)?;
        let audio_duration =
            scale_duration(audio_sidx.duration, audio_sidx.timescale, movie_timescale)?;
        patch_header_duration(&mut mvhd, video_duration.max(audio_duration))?;
        patch_trak_duration(&mut video_trak, video_sidx, movie_timescale)?;
        patch_trak_duration(&mut audio_trak, audio_sidx, movie_timescale)?;
        log::info!(
            target: "DLNA1080",
            "已写入 DASH 完整时长: video={:.3}s, audio={:.3}s",
            video_sidx.duration as f64 / video_sidx.timescale as f64,
            audio_sidx.duration as f64 / audio_sidx.timescale as f64
        );
    } else {
        log::warn!(target: "DLNA1080", "DASH 初始化段缺少 sidx 时长索引，设备可能暂时显示错误总时长");
    }

    let mvex = make_box(b"mvex", &[&video_trex, &audio_trex])?;
    let moov = make_box(b"moov", &[&mvhd, &video_trak, &audio_trak, &mvex])?;
    let mut init = BytesMut::with_capacity(video_ftyp.len() + moov.len());
    init.extend_from_slice(video_ftyp);
    init.extend_from_slice(&moov);
    Ok(init.freeze())
}

fn patch_moof(moof: &Bytes, track_id: u32, sequence: u32) -> io::Result<Bytes> {
    let mut out = moof.to_vec();
    let mfhd = child_range(&out, 0..out.len(), b"mfhd")?;
    patch_at(&mut out, mfhd.start + 12, sequence)?;
    let traf = child_range(&out, 0..out.len(), b"traf")?;
    let tfhd = child_range(&out, traf, b"tfhd")?;
    patch_at(&mut out, tfhd.start + 12, track_id)?;
    Ok(Bytes::from(out))
}

fn fragment_decode_time(moof: &[u8]) -> io::Result<u64> {
    let traf = child_range(moof, 0..moof.len(), b"traf")?;
    let tfdt = child_range(moof, traf, b"tfdt")?;
    let version = *moof
        .get(tfdt.start + 8)
        .ok_or_else(|| invalid("short tfdt"))?;
    if version == 1 {
        read_u64(moof, tfdt.start + 12)
    } else {
        Ok(read_u32(moof, tfdt.start + 12)? as u64)
    }
}

fn track_timescale(moov: &[u8]) -> io::Result<u32> {
    let trak = child_range(moov, 0..moov.len(), b"trak")?;
    let mdia = child_range(moov, trak, b"mdia")?;
    let mdhd = child_range(moov, mdia, b"mdhd")?;
    let version = *moov
        .get(mdhd.start + 8)
        .ok_or_else(|| invalid("short mdhd"))?;
    let offset = if version == 1 {
        mdhd.start + 28
    } else {
        mdhd.start + 20
    };
    let timescale = read_u32(moov, offset)?;
    if timescale == 0 {
        Err(invalid("zero track timescale"))
    } else {
        Ok(timescale)
    }
}

fn mvhd_timescale(mvhd: &[u8]) -> io::Result<u32> {
    let header = box_header_len(mvhd)?;
    let version = *mvhd.get(header).ok_or_else(|| invalid("short mvhd"))?;
    let offset = if version == 1 {
        header + 20
    } else {
        header + 12
    };
    let timescale = read_u32(mvhd, offset)?;
    if timescale == 0 {
        Err(invalid("zero movie timescale"))
    } else {
        Ok(timescale)
    }
}

fn patch_trak_duration(trak: &mut [u8], sidx: &SidxInfo, movie_timescale: u32) -> io::Result<()> {
    let tkhd = child_range(trak, 0..trak.len(), b"tkhd")?;
    let mdia = child_range(trak, 0..trak.len(), b"mdia")?;
    let mdhd = child_range(trak, mdia, b"mdhd")?;
    let media_timescale = {
        let header = box_header_len(&trak[mdhd.clone()])?;
        let version = *trak
            .get(mdhd.start + header)
            .ok_or_else(|| invalid("short mdhd"))?;
        read_u32(
            trak,
            mdhd.start
                + if version == 1 {
                    header + 20
                } else {
                    header + 12
                },
        )?
    };
    if media_timescale == 0 {
        return Err(invalid("zero media timescale"));
    }
    let media_duration = scale_duration(sidx.duration, sidx.timescale, media_timescale)?;
    patch_header_duration(&mut trak[mdhd], media_duration)?;
    let movie_duration = scale_duration(sidx.duration, sidx.timescale, movie_timescale)?;
    patch_tkhd_duration(&mut trak[tkhd], movie_duration)?;
    Ok(())
}

fn scale_duration(duration: u64, source_timescale: u32, target_timescale: u32) -> io::Result<u64> {
    if source_timescale == 0 || target_timescale == 0 {
        return Err(invalid("zero duration timescale"));
    }
    u64::try_from((duration as u128) * (target_timescale as u128) / (source_timescale as u128))
        .map_err(|_| invalid("duration overflow"))
}

fn patch_header_duration(atom: &mut [u8], duration: u64) -> io::Result<()> {
    let header = box_header_len(atom)?;
    let version = *atom.get(header).ok_or_else(|| invalid("short full box"))?;
    let offset = if version == 1 {
        header + 24
    } else {
        header + 16
    };
    if version == 1 {
        patch_u64(atom, offset, duration)
    } else {
        patch_at(
            atom,
            offset,
            u32::try_from(duration).map_err(|_| invalid("v0 duration overflow"))?,
        )
    }
}

fn patch_tkhd_duration(atom: &mut [u8], duration: u64) -> io::Result<()> {
    let header = box_header_len(atom)?;
    let version = *atom.get(header).ok_or_else(|| invalid("short tkhd"))?;
    let offset = if version == 1 {
        header + 28
    } else {
        header + 20
    };
    if version == 1 {
        patch_u64(atom, offset, duration)
    } else {
        patch_at(
            atom,
            offset,
            u32::try_from(duration).map_err(|_| invalid("v0 duration overflow"))?,
        )
    }
}

fn patch_trak_track_id(trak: &mut [u8], track_id: u32) -> io::Result<()> {
    let tkhd = child_range(trak, 0..trak.len(), b"tkhd")?;
    let version = *trak
        .get(tkhd.start + 8)
        .ok_or_else(|| invalid("short tkhd"))?;
    let offset = if version == 1 {
        tkhd.start + 28
    } else {
        tkhd.start + 20
    };
    patch_at(trak, offset, track_id)
}

fn patch_mvhd_next_track_id(mvhd: &mut [u8], next_track_id: u32) -> io::Result<()> {
    if mvhd.len() < 12 {
        return Err(invalid("short mvhd"));
    }
    patch_at(mvhd, mvhd.len() - 4, next_track_id)
}

fn patch_full_box_u32(atom: &mut [u8], offset: usize, value: u32) -> io::Result<()> {
    patch_at(atom, offset, value)
}

fn patch_at(data: &mut [u8], offset: usize, value: u32) -> io::Result<()> {
    let dst = data
        .get_mut(offset..offset + 4)
        .ok_or_else(|| invalid("box patch out of bounds"))?;
    dst.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

fn patch_u64(data: &mut [u8], offset: usize, value: u64) -> io::Result<()> {
    let dst = data
        .get_mut(offset..offset + 8)
        .ok_or_else(|| invalid("box patch out of bounds"))?;
    dst.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

fn child<'a>(parent: &'a [u8], wanted: &[u8; 4]) -> io::Result<&'a [u8]> {
    let range = child_range(parent, 0..parent.len(), wanted)?;
    Ok(&parent[range])
}

fn child_range(data: &[u8], parent: Range<usize>, wanted: &[u8; 4]) -> io::Result<Range<usize>> {
    let parent_header = box_header_len(&data[parent.clone()])?;
    let mut offset = parent.start + parent_header;
    while offset + 8 <= parent.end {
        let size = box_size_prefix(&data[offset..parent.end])?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| invalid("box size overflow"))?;
        if end > parent.end {
            return Err(invalid("child box exceeds parent"));
        }
        if data.get(offset + 4..offset + 8) == Some(wanted.as_slice()) {
            return Ok(offset..end);
        }
        offset = end;
    }
    Err(invalid(format!(
        "missing {} box",
        String::from_utf8_lossy(wanted)
    )))
}

fn make_box(kind: &[u8; 4], children: &[&[u8]]) -> io::Result<Vec<u8>> {
    let payload: usize = children.iter().map(|c| c.len()).sum();
    let size = 8usize
        .checked_add(payload)
        .ok_or_else(|| invalid("box size overflow"))?;
    let size32 = u32::try_from(size).map_err(|_| invalid("initialization box too large"))?;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&size32.to_be_bytes());
    out.extend_from_slice(kind);
    for child in children {
        out.extend_from_slice(child);
    }
    Ok(out)
}

fn parse_sidx(sidx: &[u8]) -> io::Result<SidxInfo> {
    let header = box_header_len(sidx)?;
    let version = *sidx.get(header).ok_or_else(|| invalid("short sidx"))?;
    let timescale = read_u32(sidx, header + 8)?;
    if timescale == 0 {
        return Err(invalid("zero sidx timescale"));
    }
    let mut offset = header + 4 + 4 + 4;
    let earliest_presentation_time = if version == 0 {
        read_u32(sidx, offset)? as u64
    } else {
        read_u64(sidx, offset)?
    };
    offset += if version == 0 { 4 } else { 8 };
    let first_media_offset = if version == 0 {
        read_u32(sidx, offset)? as u64
    } else {
        read_u64(sidx, offset)?
    };
    offset += if version == 0 { 4 } else { 8 };
    offset += 2;
    let count = read_u16(sidx, offset)? as usize;
    offset += 2;
    let mut total = 0u64;
    let mut duration = 0u64;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let reference = read_u32(sidx, offset)?;
        if reference >> 31 != 0 {
            return Err(invalid("nested sidx reference unsupported"));
        }
        let referenced_size = reference & 0x7fff_ffff;
        let subsegment_duration = read_u32(sidx, offset + 4)?;
        total += referenced_size as u64;
        duration += subsegment_duration as u64;
        entries.push(SidxEntry {
            referenced_size,
            subsegment_duration,
        });
        offset += 12;
    }
    Ok(SidxInfo {
        referenced_bytes: total,
        duration,
        timescale,
        earliest_presentation_time,
        first_media_offset,
        entries,
    })
}

fn sidx_offset_for_start(sidx: &SidxInfo, start_secs: u32) -> Option<(u64, f64)> {
    let target_time = (u128::from(start_secs) * u128::from(sidx.timescale)) as u64;
    let mut offset = sidx.first_media_offset;
    let mut segment_time = sidx.earliest_presentation_time;
    for entry in &sidx.entries {
        if segment_time >= target_time {
            return Some((offset, segment_time as f64 / sidx.timescale as f64));
        }
        offset += u64::from(entry.referenced_size);
        segment_time += u64::from(entry.subsegment_duration);
    }
    None
}

fn atom_type(data: &[u8]) -> io::Result<&[u8]> {
    data.get(4..8).ok_or_else(|| invalid("short box header"))
}

fn box_header_len(data: &[u8]) -> io::Result<usize> {
    Ok(if read_u32(data, 0)? == 1 { 16 } else { 8 })
}

fn box_size_prefix(data: &[u8]) -> io::Result<usize> {
    let size32 = read_u32(data, 0)?;
    if size32 == 0 {
        return Err(invalid("box extending to EOF unsupported"));
    }
    if size32 == 1 {
        usize::try_from(read_u64(data, 8)?).map_err(|_| invalid("box too large"))
    } else {
        Ok(size32 as usize)
    }
}

fn read_u16(data: &[u8], offset: usize) -> io::Result<u16> {
    let b: [u8; 2] = data
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("short u16"))?
        .try_into()
        .unwrap();
    Ok(u16::from_be_bytes(b))
}

fn read_u32(data: &[u8], offset: usize) -> io::Result<u32> {
    let b: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("short u32"))?
        .try_into()
        .unwrap();
    Ok(u32::from_be_bytes(b))
}

fn read_u64(data: &[u8], offset: usize) -> io::Result<u64> {
    let b: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or_else(|| invalid("short u64"))?
        .try_into()
        .unwrap();
    Ok(u64::from_be_bytes(b))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn makes_and_finds_boxes() {
        let child_a = make_box(b"aaaa", &[]).unwrap();
        let child_b = make_box(b"bbbb", &[]).unwrap();
        let parent = make_box(b"test", &[&child_a, &child_b]).unwrap();
        assert_eq!(child(&parent, b"bbbb").unwrap(), child_b);
    }

    #[test]
    fn patches_fragment_ids_without_changing_size() {
        let mfhd = make_box(b"mfhd", &[&[0, 0, 0, 0, 0, 0, 0, 9]]).unwrap();
        let tfhd = make_box(b"tfhd", &[&[0, 0, 0, 0, 0, 0, 0, 1]]).unwrap();
        let traf = make_box(b"traf", &[&tfhd]).unwrap();
        let moof = Bytes::from(make_box(b"moof", &[&mfhd, &traf]).unwrap());
        let patched = patch_moof(&moof, 2, 7).unwrap();
        assert_eq!(patched.len(), moof.len());
        let mfhd = child_range(&patched, 0..patched.len(), b"mfhd").unwrap();
        let traf = child_range(&patched, 0..patched.len(), b"traf").unwrap();
        let tfhd = child_range(&patched, traf, b"tfhd").unwrap();
        assert_eq!(read_u32(&patched, mfhd.start + 12).unwrap(), 7);
        assert_eq!(read_u32(&patched, tfhd.start + 12).unwrap(), 2);
    }

    #[test]
    fn log_endpoint_does_not_expose_query_credentials() {
        let endpoint =
            safe_endpoint("https://upos-sz.example.com/path/video.m4s?deadline=123&token=secret");
        assert_eq!(endpoint, "https://upos-sz.example.com/path/video.m4s");
        assert!(!endpoint.contains("secret"));
    }

    #[test]
    fn parses_sidx_media_size_and_full_duration() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        body.extend_from_slice(&1u32.to_be_bytes()); // reference_ID
        body.extend_from_slice(&1_000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u32.to_be_bytes()); // earliest_presentation_time
        body.extend_from_slice(&0u32.to_be_bytes()); // first_offset
        body.extend_from_slice(&0u16.to_be_bytes()); // reserved
        body.extend_from_slice(&2u16.to_be_bytes()); // reference_count
        for (size, duration) in [(120u32, 5_000u32), (80u32, 4_000u32)] {
            body.extend_from_slice(&size.to_be_bytes());
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&0u32.to_be_bytes()); // SAP flags
        }
        let sidx = make_box(b"sidx", &[&body]).unwrap();

        let parsed = parse_sidx(&sidx).unwrap();
        assert_eq!(parsed.referenced_bytes, 200);
        assert_eq!(parsed.duration, 9_000);
        assert_eq!(parsed.timescale, 1_000);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(sidx_offset_for_start(&parsed, 0), Some((0, 0.0)));
        assert_eq!(sidx_offset_for_start(&parsed, 5), Some((120, 5.0)));
        assert_eq!(sidx_offset_for_start(&parsed, 9), None);
    }

    #[test]
    fn seek_start_is_measured_in_each_track_timescale() {
        assert!(fragment_is_at_or_after(80_000, 16_000, 5));
        assert!(!fragment_is_at_or_after(79_999, 16_000, 5));
        assert!(fragment_is_at_or_after(220_500, 44_100, 5));
        assert!(!fragment_is_at_or_after(220_499, 44_100, 5));
    }

    #[test]
    fn patches_v0_movie_header_duration() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        body.extend_from_slice(&0u32.to_be_bytes()); // creation time
        body.extend_from_slice(&0u32.to_be_bytes()); // modification time
        body.extend_from_slice(&1_000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u32.to_be_bytes()); // duration
        body.resize(100, 0);
        let mut mvhd = make_box(b"mvhd", &[&body]).unwrap();

        assert_eq!(mvhd_timescale(&mvhd).unwrap(), 1_000);
        patch_header_duration(&mut mvhd, 285_000).unwrap();
        assert_eq!(read_u32(&mvhd, 24).unwrap(), 285_000);
    }

    #[test]
    fn patches_v0_track_header_duration() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        body.extend_from_slice(&0u32.to_be_bytes()); // creation time
        body.extend_from_slice(&0u32.to_be_bytes()); // modification time
        body.extend_from_slice(&1u32.to_be_bytes()); // track id
        body.extend_from_slice(&0u32.to_be_bytes()); // reserved
        body.extend_from_slice(&0u32.to_be_bytes()); // duration
        body.resize(100, 0);
        let mut tkhd = make_box(b"tkhd", &[&body]).unwrap();

        patch_tkhd_duration(&mut tkhd, 285_000).unwrap();
        assert_eq!(read_u32(&tkhd, 28).unwrap(), 285_000);
    }

    /// Full network fixture used manually before releases. The output stays in
    /// target/ and is verified with ffprobe by the release workflow/operator.
    #[tokio::test]
    #[ignore = "downloads the complete public Bilibili test video"]
    async fn live_mux_target_video() {
        let media = crate::bilibili_parser::get_bilibili_media("BV1dqj16aECG", Some(0), 80)
            .await
            .unwrap();
        let crate::bilibili_parser::BilibiliMedia::Dash {
            video_url,
            audio_url,
            ..
        } = media
        else {
            panic!("expected DASH");
        };
        let client = reqwest::Client::builder().build().unwrap();
        let muxed = mux_dash(&client, &video_url, &audio_url).await.unwrap();
        let expected = muxed.content_length;
        let path = "target/bilibili-1080-mux-test.mp4";
        let mut file = tokio::fs::File::create(path).await.unwrap();
        let mut stream = muxed.stream;
        let mut written = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            file.write_all(&chunk).await.unwrap();
            written += chunk.len() as u64;
        }
        file.flush().await.unwrap();
        if let Some(expected) = expected {
            assert_eq!(written, expected);
        }
        assert!(written > 10 * 1024 * 1024);
    }

    #[tokio::test]
    #[ignore = "opens a public Bilibili 1080P stream and verifies indexed seeking"]
    async fn live_mux_seeks_with_sidx_range() {
        let media = crate::bilibili_parser::get_bilibili_media("BV1dqj16aECG", Some(0), 80)
            .await
            .unwrap();
        let crate::bilibili_parser::BilibiliMedia::Dash {
            video_url,
            audio_url,
            ..
        } = media
        else {
            panic!("expected DASH");
        };
        let client = reqwest::Client::builder().build().unwrap();
        let mut stream = mux_dash_from(&client, &video_url, &audio_url, 120)
            .await
            .unwrap()
            .stream;
        let init = stream.next().await.unwrap().unwrap();
        assert_eq!(atom_type(&init).unwrap(), b"ftyp");
        let first_fragment = stream.next().await.unwrap().unwrap();
        assert_eq!(atom_type(&first_fragment).unwrap(), b"moof");
    }
}
