//! 按解码时间交错输出音视频 fMP4 分片，不重新编码媒体。
use bytes::Bytes;
use futures_util::Stream;
use std::{io, pin::Pin};
use tokio::sync::mpsc;

mod boxes;
mod metadata;
mod source;

pub(crate) use metadata::delay_fragment;
use metadata::{merge_initialization, patch_moof, track_timescale};
use source::TrackSource;

const VIDEO_TRACK_ID: u32 = 1;
const AUDIO_TRACK_ID: u32 = 2;

pub type MuxByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>;

pub struct MuxedMp4 {
    pub stream: MuxByteStream,
    pub content_length: Option<u64>,
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

/// 按解码时间交错输出音视频分片。
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

/// 判断分片是否位于目标播放时间或之后。
fn fragment_is_at_or_after(decode_time: u64, timescale: u32, start_secs: u32) -> bool {
    decode_time >= u64::from(start_secs) * u64::from(timescale)
}

/// 生成日志使用的媒体地址，省略查询参数。

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    /// Full network fixture used manually before releases. The output stays in
    /// target/ and is verified with ffprobe by the release workflow/operator.
    #[tokio::test]
    #[ignore = "downloads the complete public Bilibili test video"]
    async fn live_mux_target_video() {
        let media = crate::sources::bilibili::get_bilibili_media_with_format(
            "BV1dqj16aECG",
            Some(0),
            80,
            crate::cast::MediaFormat::Dash,
            true,
        )
        .await
        .unwrap();
        let crate::sources::bilibili::BilibiliMedia::Dash {
            video_url,
            audio_url,
            ..
        } = media.media
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
}
