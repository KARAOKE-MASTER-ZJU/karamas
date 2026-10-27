use super::{BilibiliMedia, media_qualities::request_playurl};
use crate::cast::{MediaFormat, Quality};
use serde_json::Value;

pub struct ResolvedBilibiliMedia {
    pub media: BilibiliMedia,
    pub quality: Quality,
    pub format: MediaFormat,
}

pub(super) async fn resolve(
    bvid: &str,
    cid: &str,
    qn: u32,
    format: MediaFormat,
) -> Result<ResolvedBilibiliMedia, String> {
    match format {
        MediaFormat::Mp4 => mp4(bvid, cid, qn).await,
        MediaFormat::Dash => dash(bvid, cid, qn).await,
        MediaFormat::Auto => {
            if qn >= 80 {
                dash(bvid, cid, qn).await
            } else {
                mp4(bvid, cid, qn).await
            }
        }
    }
}

async fn mp4(bvid: &str, cid: &str, qn: u32) -> Result<ResolvedBilibiliMedia, String> {
    let qn_string = qn.to_string();
    let json = request_playurl(
        bvid,
        cid,
        &[
            ("qn", qn_string.as_str()),
            ("type", ""),
            ("otype", "json"),
            ("platform", "html5"),
            ("high_quality", "1"),
        ],
    )
    .await?;
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
    let url = valid_url(&files[0]["url"]).ok_or("MP4 没有有效的 URL")?;
    let quality = data["quality"]
        .as_u64()
        .and_then(|qn| u32::try_from(qn).ok())
        .and_then(Quality::from_qn)
        .ok_or("MP4 没有有效的清晰度")?;
    log::info!(target: "DLNA1080", "选中完整 MP4: bvid={}, 请求qn={}, 返回qn={}", bvid, qn, quality.as_qn());
    Ok(ResolvedBilibiliMedia {
        media: BilibiliMedia::Direct { url },
        quality,
        format: MediaFormat::Mp4,
    })
}

async fn dash(bvid: &str, cid: &str, qn: u32) -> Result<ResolvedBilibiliMedia, String> {
    let quality = Quality::from_qn(qn).ok_or("不支持的清晰度")?;
    let qn_string = qn.to_string();
    let json = request_playurl(
        bvid,
        cid,
        &[
            ("qn", qn_string.as_str()),
            ("fnval", "4048"),
            ("fnver", "0"),
            ("fourk", "1"),
            ("try_look", "1"),
        ],
    )
    .await?;
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
    log::info!(target: "DLNA1080", "选中 DASH 轨道: bvid={}, video_id={}, codec={}, {}x{}, audio_id={}",
        bvid, qn, video["codecs"].as_str().unwrap_or("unknown"),
        video["width"].as_u64().unwrap_or(0), video["height"].as_u64().unwrap_or(0),
        audio["id"].as_u64().unwrap_or(0));
    Ok(ResolvedBilibiliMedia {
        media: BilibiliMedia::Dash {
            video_url: track_url(video).unwrap(),
            audio_url: track_url(audio).unwrap(),
            width: video["width"].as_u64().unwrap_or(0) as u32,
            height: video["height"].as_u64().unwrap_or(0) as u32,
        },
        quality,
        format: MediaFormat::Dash,
    })
}

fn track_url(track: &Value) -> Option<String> {
    valid_url(&track["baseUrl"]).or_else(|| valid_url(&track["base_url"]))
}

fn valid_url(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| value.to_owned())
}
