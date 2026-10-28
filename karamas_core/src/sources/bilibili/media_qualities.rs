use crate::cast::bilibili::{self, BilibiliSession};
use serde_json::Value;

use super::{
    BilibiliQuality, bili_api_client, get_page_info, parse_bilibili_qualities, parse_bilibili_video,
};

/// 本机播放和 DLNA 使用的媒体能力；不代表接收设备支持对应编码。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliMediaQuality {
    pub qn: u32,
    pub label: String,
    /// HTML5 返回的 MP4 清晰度列表是否包含此档位；查询失败时为 null。
    pub direct_mp4: Option<bool>,
    /// 是否同时返回该档位的 DASH 视频轨和可用音频轨。
    pub dash: bool,
}

/// 只请求播放元数据，不下载媒体，不影响当前播放器的选档和取源。
pub async fn get_bilibili_media_qualities(
    video: &str,
) -> Result<Vec<BilibiliMediaQuality>, String> {
    let started = std::time::Instant::now();
    let (bvid, page) = parse_bilibili_video(video)?;
    let (cid, _) = get_page_info(&bvid, page).await?;
    let cid = cid.to_string();
    let session = playback_session();
    // 每个分 P 固定查一次 DASH、一次 MP4，不按清晰度逐档请求。
    let (dash_result, mp4_result) = tokio::join!(
        request_playurl(
            &bvid,
            &cid,
            &[
                ("qn", "80"),
                ("fnval", "4048"),
                ("fourk", "1"),
                ("try_look", "1"),
            ],
            session.as_ref(),
        ),
        request_playurl(
            &bvid,
            &cid,
            &[
                ("qn", "80"),
                ("type", ""),
                ("otype", "json"),
                ("platform", "html5"),
                ("high_quality", "1"),
            ],
            session.as_ref(),
        ),
    );
    let json = dash_result?;
    let qualities = parse_media_qualities(&json, mp4_result.as_ref().map_err(Clone::clone))?;
    log::info!(
        "B站媒体清晰度查询完成: bvid={}, page={}, 档位数={}, 未确认MP4数={}, 耗时={}ms",
        bvid,
        page + 1,
        qualities.len(),
        qualities
            .iter()
            .filter(|quality| quality.direct_mp4.is_none())
            .count(),
        started.elapsed().as_millis()
    );
    Ok(qualities)
}

pub(super) fn parse_media_qualities(
    json: &Value,
    mp4_result: Result<&Value, String>,
) -> Result<Vec<BilibiliMediaQuality>, String> {
    let mp4_qualities = match mp4_result.and_then(parse_mp4_qualities) {
        Ok(qualities) => Some(qualities),
        Err(error) => {
            log::debug!("B站 MP4 清晰度未确认: {error}");
            None
        }
    };
    let supports_mp4 = |qn| {
        mp4_qualities
            .as_ref()
            .map(|qualities| qualities.iter().any(|quality| quality.qn == qn))
    };
    let advertised = parse_bilibili_qualities(json)?;
    let mut qualities: Vec<_> = advertised
        .into_iter()
        .map(|quality| BilibiliMediaQuality {
            qn: quality.qn,
            label: quality.label,
            direct_mp4: supports_mp4(quality.qn),
            dash: has_dash(&json["data"], quality.qn),
        })
        .collect();
    // 部分响应的 accept_quality 会漏掉实际返回的轨道，以轨道为准补齐。
    if let Some(tracks) = json["data"]["dash"]["video"].as_array() {
        for track in tracks {
            let Some(quality) = track["id"]
                .as_u64()
                .and_then(|qn| u32::try_from(qn).ok())
                .and_then(crate::cast::Quality::from_qn)
            else {
                continue;
            };
            let qn = quality.as_qn();
            if has_dash(&json["data"], qn) && !qualities.iter().any(|item| item.qn == qn) {
                qualities.push(BilibiliMediaQuality {
                    qn,
                    label: quality.label().to_owned(),
                    direct_mp4: supports_mp4(qn),
                    dash: true,
                });
            }
        }
    }

    // MP4 独有的档位也要保留，不能只取 DASH 候选列表。
    if let Some(mp4_qualities) = mp4_qualities {
        for quality in mp4_qualities {
            if !qualities.iter().any(|item| item.qn == quality.qn) {
                qualities.push(BilibiliMediaQuality {
                    qn: quality.qn,
                    label: quality.label,
                    direct_mp4: Some(true),
                    dash: has_dash(&json["data"], quality.qn),
                });
            }
        }
    }
    Ok(qualities)
}

pub(super) fn playback_session() -> Option<BilibiliSession> {
    bilibili::load_session().filter(|session| {
        !session.access_token.is_empty() && !bilibili::is_session_expired(session)
    })
}

pub(super) async fn request_playurl(
    bvid: &str,
    cid: &str,
    params: &[(&str, &str)],
    session: Option<&BilibiliSession>,
) -> Result<Value, String> {
    let endpoint = "https://api.bilibili.com/x/player/playurl";
    let request = if let Some(session) = session {
        let params = [("bvid", bvid), ("cid", cid)]
            .iter()
            .chain(params)
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        let query = bilibili::sign_playurl_params(params, session);
        bili_api_client().get(format!("{endpoint}?{query}"))
    } else {
        bili_api_client()
            .get(endpoint)
            .query(&[("bvid", bvid), ("cid", cid)])
            .query(params)
    };
    let json = request
        .header("Referer", format!("https://www.bilibili.com/video/{bvid}"))
        .send()
        .await
        .map_err(|error| format!("获取播放信息失败：{}", error.without_url()))?
        .error_for_status()
        .map_err(|error| format!("获取播放信息失败：{}", error.without_url()))?
        .json::<Value>()
        .await
        .map_err(|error| format!("解析播放信息失败：{}", error.without_url()))?;
    if json["code"].as_i64() != Some(0) || !json["data"].is_object() {
        return Err(format!(
            "获取播放信息失败：{} ({})",
            json["message"].as_str().unwrap_or("B站接口返回错误"),
            json["code"]
        ));
    }
    Ok(json)
}

fn is_media_url(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        url::Url::parse(value)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
    })
}

fn has_track_url(track: &Value) -> bool {
    is_media_url(&track["baseUrl"]) || is_media_url(&track["base_url"])
}

fn has_dash(data: &Value, qn: u32) -> bool {
    let dash = &data["dash"];
    dash["audio"]
        .as_array()
        .is_some_and(|tracks| tracks.iter().any(has_track_url))
        && dash["video"].as_array().is_some_and(|tracks| {
            tracks
                .iter()
                .any(|track| track["id"].as_u64() == Some(qn as u64) && has_track_url(track))
        })
}

fn parse_mp4_qualities(json: &Value) -> Result<Vec<BilibiliQuality>, String> {
    let data = &json["data"];
    if !data["format"]
        .as_str()
        .is_some_and(|format| format.starts_with("mp4"))
        || !data["durl"]
            .as_array()
            .is_some_and(|files| files.len() == 1 && is_media_url(&files[0]["url"]))
    {
        return Err("HTML5 接口未返回单文件 MP4".into());
    }
    let mut qualities = parse_bilibili_qualities(json)?;
    // 登录后的 HTML5 列表也可能包含 FLV 档位，不能把它们当作 MP4。
    if let Some(formats) = data["support_formats"].as_array() {
        qualities.retain(|quality| {
            formats
                .iter()
                .find(|format| format["quality"].as_u64() == Some(quality.qn as u64))
                .and_then(|format| format["format"].as_str())
                .is_none_or(|format| format.starts_with("mp4"))
        });
    }
    Ok(qualities)
}
