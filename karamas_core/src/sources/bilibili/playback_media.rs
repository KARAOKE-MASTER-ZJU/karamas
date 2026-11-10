use super::{
    BilibiliMedia, BilibiliMediaQuality,
    media_qualities::{parse_media_qualities, playback_session, request_playurl},
};
use crate::cast::bilibili::BilibiliSession;
use crate::cast::{MediaFormat, Quality, select_media_quality};
use serde_json::Value;

pub struct ResolvedBilibiliMedia {
    pub media: BilibiliMedia,
    pub quality: Quality,
    pub format: MediaFormat,
    pub(crate) hls_tracks: Option<(crate::media::hls::Track, crate::media::hls::Track)>,
}

pub(super) async fn resolve(
    bvid: &str,
    cid: &str,
    qn: u32,
    format: MediaFormat,
    formats: &[MediaFormat],
) -> Result<ResolvedBilibiliMedia, String> {
    let preferred = (Quality::from_qn(qn).ok_or("不支持的清晰度")?, format);
    let session = playback_session();
    let qn_string = qn.to_string();
    let dash_params = [
        ("qn", qn_string.as_str()),
        ("fnval", "4048"),
        ("fnver", "0"),
        ("fourk", "1"),
        ("try_look", "1"),
    ];
    let (dash_result, mp4_result) = tokio::join!(
        request_playurl(bvid, cid, &dash_params, session.as_ref()),
        request_mp4(bvid, cid, preferred.0.as_qn(), session.as_ref()),
    );
    let base = match (&dash_result, &mp4_result) {
        (Ok(json), _) | (_, Ok(json)) => json,
        (Err(dash), Err(mp4)) => return Err(format!("MP4：{mp4}；DASH：{dash}")),
    };
    let qualities = parse_media_qualities(base, mp4_result.as_ref().map_err(Clone::clone))?;
    let points = available_points(&qualities, dash_result.as_ref().ok(), formats);
    let selected = select_media_quality(preferred, points).ok_or_else(|| {
        format!(
            "没有符合模式和清晰度要求的媒体，目标为 {}",
            preferred.0.label()
        )
    })?;
    if selected != preferred {
        log::info!(target: "DLNA1080", "选档回退: bvid={}, 目标=({}, {:?}), 本首=({}, {:?})", bvid,
            preferred.0.as_qn(), preferred.1, selected.0.as_qn(), selected.1);
    }
    let resolved = if selected.1 != MediaFormat::Mp4 {
        let mut resolved = parse_dash(
            bvid,
            dash_result.as_ref().map_err(Clone::clone)?,
            selected.0.as_qn(),
        )
        .await?;
        resolved.format = selected.1;
        resolved
    } else {
        let current = mp4_result.as_ref().map_err(Clone::clone)?;
        if current["data"]["quality"].as_u64() == Some(selected.0.as_qn() as u64) {
            parse_mp4(bvid, current).await?
        } else {
            // 仅在最终选中其他 MP4 档位时补取该地址，不逐档探测。
            parse_mp4(
                bvid,
                &request_mp4(bvid, cid, selected.0.as_qn(), session.as_ref()).await?,
            )
            .await?
        }
    };
    if (resolved.quality, resolved.format) != selected {
        return Err("B站取源返回的清晰度已变化，请重试".into());
    }
    Ok(resolved)
}

fn available_points(
    qualities: &[BilibiliMediaQuality],
    dash_json: Option<&Value>,
    formats: &[MediaFormat],
) -> Vec<(Quality, MediaFormat)> {
    let mut points = Vec::new();
    for quality in qualities {
        let Some(qn) = Quality::from_qn(quality.qn) else {
            continue;
        };
        if formats.contains(&MediaFormat::Mp4) && quality.direct_mp4 == Some(true) {
            points.push((qn, MediaFormat::Mp4));
        }
        if quality.dash {
            if formats.contains(&MediaFormat::Dash) {
                points.push((qn, MediaFormat::Dash));
            }
            if formats.contains(&MediaFormat::Hls)
                && dash_json.is_some_and(|json| {
                    dash_tracks(json, qn.as_qn())
                        .ok()
                        .and_then(|(video, audio)| hls_track(video).zip(hls_track(audio)))
                        .is_some()
                })
            {
                points.push((qn, MediaFormat::Hls));
            }
        }
    }
    points
}

async fn request_mp4(
    bvid: &str,
    cid: &str,
    qn: u32,
    session: Option<&BilibiliSession>,
) -> Result<Value, String> {
    let qn_string = qn.to_string();
    request_playurl(
        bvid,
        cid,
        &[
            ("qn", qn_string.as_str()),
            ("type", ""),
            ("otype", "json"),
            ("platform", "html5"),
            ("high_quality", "1"),
        ],
        session,
    )
    .await
}

async fn parse_mp4(bvid: &str, json: &Value) -> Result<ResolvedBilibiliMedia, String> {
    let data = &json["data"];
    if !data["format"]
        .as_str()
        .is_some_and(|format| format.starts_with("mp4"))
    {
        return Err("B站未返回 MP4 格式".into());
    }
    let files = data["durl"]
        .as_array()
        .filter(|files| files.len() == 1)
        .ok_or("B站未返回单文件 MP4")?;
    let quality = data["quality"]
        .as_u64()
        .and_then(|qn| u32::try_from(qn).ok())
        .and_then(Quality::from_qn)
        .ok_or("MP4 没有有效的清晰度")?;
    let client = crate::media::upstream::client().map_err(|error| error.to_string())?;
    let url = select_media_url(&client, &files[0]).await?;
    log::info!(target: "DLNA1080", "选中完整 MP4: bvid={}, qn={}", bvid, quality.as_qn());
    Ok(ResolvedBilibiliMedia {
        media: BilibiliMedia::Direct { url },
        quality,
        format: MediaFormat::Mp4,
        hls_tracks: None,
    })
}

async fn parse_dash(bvid: &str, json: &Value, qn: u32) -> Result<ResolvedBilibiliMedia, String> {
    let quality = Quality::from_qn(qn).ok_or("不支持的清晰度")?;
    let (video, audio) = dash_tracks(json, qn)?;
    let client = crate::media::upstream::client().map_err(|error| error.to_string())?;
    let (video_url, audio_url) = tokio::try_join!(
        select_media_url(&client, video),
        select_media_url(&client, audio),
    )?;
    let hls_tracks = hls_track_with_url(video, video_url.clone())
        .zip(hls_track_with_url(audio, audio_url.clone()));
    log::info!(target: "DLNA1080", "选中 DASH 轨道: bvid={}, video_id={}, codec={}, {}x{}, audio_id={}",
        bvid, qn, video["codecs"].as_str().unwrap_or("unknown"),
        video["width"].as_u64().unwrap_or(0), video["height"].as_u64().unwrap_or(0),
        audio["id"].as_u64().unwrap_or(0));
    Ok(ResolvedBilibiliMedia {
        media: BilibiliMedia::Dash {
            video_url,
            audio_url,
            width: video["width"].as_u64().unwrap_or(0) as u32,
            height: video["height"].as_u64().unwrap_or(0) as u32,
        },
        quality,
        format: MediaFormat::Dash,
        hls_tracks,
    })
}

fn dash_tracks(json: &Value, qn: u32) -> Result<(&Value, &Value), String> {
    let quality = Quality::from_qn(qn).ok_or("不支持的清晰度")?;
    let data = &json["data"]["dash"];
    let videos = data["video"].as_array().ok_or("B站未返回 DASH 视频轨")?;
    let audios = data["audio"].as_array().ok_or("B站未返回 DASH 音频轨")?;
    let video = videos
        .iter()
        .filter(|video| video["id"].as_u64() == Some(qn as u64) && track_url(video).is_some())
        // 在同档位内优先 H.264，以兼容本地播放器及更多 DLNA 设备。
        .max_by_key(|video| {
            (
                video["codecs"]
                    .as_str()
                    .is_some_and(|codec| codec.starts_with("avc1")),
                video["bandwidth"].as_u64().unwrap_or(0),
            )
        })
        .ok_or_else(|| format!("B站未返回 {} 的 DASH 视频轨", quality.label()))?;
    let audio = audios
        .iter()
        .filter(|audio| track_url(audio).is_some())
        .max_by_key(|audio| audio["bandwidth"].as_u64().unwrap_or(0))
        .ok_or("B站未返回有效的 DASH 音频轨")?;
    Ok((video, audio))
}

fn hls_track(track: &Value) -> Option<crate::media::hls::Track> {
    hls_track_with_url(track, track_url(track)?)
}

fn hls_track_with_url(track: &Value, url: String) -> Option<crate::media::hls::Track> {
    let segment = track
        .get("SegmentBase")
        .or_else(|| track.get("segment_base"))?;
    crate::media::hls::Track::new(
        url,
        track["codecs"].as_str()?,
        track["bandwidth"].as_u64()?,
        segment
            .get("Initialization")
            .or_else(|| segment.get("initialization"))?
            .as_str()?,
        segment
            .get("indexRange")
            .or_else(|| segment.get("index_range"))?
            .as_str()?,
    )
}

fn track_url(track: &Value) -> Option<String> {
    media_urls(track).into_iter().next()
}

pub(super) fn media_urls(track: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    let primary = ["baseUrl", "base_url", "url"].map(|key| &track[key]);
    let backups = ["backupUrl", "backup_url"]
        .into_iter()
        .filter_map(|key| track[key].as_array())
        .flatten();
    for value in primary.into_iter().chain(backups) {
        if let Some(url) = valid_url(value) {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

// 在取源时选定 CDN，索引准备、后续分片和设备直连共用该地址。
async fn select_media_url(client: &reqwest::Client, track: &Value) -> Result<String, String> {
    let mut last_error = "没有有效的媒体地址".to_owned();
    for url in media_urls(track) {
        match probe_media_url(client, &url).await {
            Ok(()) => return Ok(url),
            Err(error) => {
                let endpoint = url::Url::parse(&url).unwrap();
                log::warn!(target: "DLNA1080", "B站媒体 CDN 不可用: endpoint={}, error={}", endpoint.origin().ascii_serialization(), error);
                last_error = error;
            }
        }
    }
    Err(format!("B站媒体 CDN 均不可用：{last_error}"))
}

async fn probe_media_url(client: &reqwest::Client, url: &str) -> Result<(), String> {
    let mut response = client
        .get(url)
        .header("Range", "bytes=0-0")
        .header("Referer", crate::media::upstream::referer(url))
        .header("User-Agent", crate::media::upstream::user_agent(url))
        .header("Accept-Encoding", "identity")
        .timeout(crate::config::PROXY_CONNECT_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("{:?}", error.without_url()))?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        || !response
            .headers()
            .get("Content-Range")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes 0-0/"))
            .and_then(|length| length.parse::<u64>().ok())
            .is_some_and(|length| length > 0)
    {
        return Err(format!(
            "媒体未返回有效的字节范围：HTTP {}",
            response.status()
        ));
    }
    let mut length = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("{:?}", error.without_url()))?
    {
        length += chunk.len();
        if length > 1 {
            return Err("媒体范围响应过长".into());
        }
    }
    if length != 1 {
        return Err("媒体范围响应不完整".into());
    }
    Ok(())
}

fn valid_url(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| value.to_owned())
}
