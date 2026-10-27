use super::{BilibiliMedia, bili_api_client, get_page_info};
use serde_json::Value;

/// 获取指定清晰度的 B站媒体。
///
/// `qn=80` 使用实测可用的匿名 DASH 接口。`try_look=1` 会在没有 Cookie 时
/// 返回可试看高画质轨道；DASH 模式下必须检查 `dash.video[].id`，不能依赖
/// 顶层 `quality` 字段。
pub async fn get_bilibili_media(
    bv_id: &str,
    page: Option<u32>,
    qn: u32,
) -> Result<BilibiliMedia, String> {
    let page = page.unwrap_or(0);

    //如果bv_id本来就是一个URL，直接返回
    if bv_id.starts_with("http") {
        return Ok(BilibiliMedia::Direct {
            url: bv_id.to_string(),
        });
    }

    // 第一步：获取CID
    let started = std::time::Instant::now();
    let cid = get_video_cid(bv_id, page).await?;
    let cid_ms = started.elapsed().as_millis();

    // 第二步：获取视频直链
    let playurl_started = std::time::Instant::now();
    let result = get_video_url(bv_id, &cid, qn).await;
    log::info!(target: "DLNA1080", "B站媒体解析完成: bvid={}, qn={}, cid_ms={}, playurl_ms={}, success={}", bv_id, qn, cid_ms, playurl_started.elapsed().as_millis(), result.is_ok());
    result
}

/// 获取视频的CID（分集ID）
async fn get_video_cid(bv_id: &str, page: u32) -> Result<String, String> {
    get_page_info(bv_id, page)
        .await
        .map(|(cid, _)| cid.to_string())
}

/// 获取视频播放链接
async fn get_video_url(bv_id: &str, cid: &str, qn: u32) -> Result<BilibiliMedia, String> {
    let wants_1080 = qn >= 80;
    log::info!(
        target: "DLNA1080",
        "请求 Bilibili 播放地址: bvid={}, cid={}, qn={}, mode={}",
        bv_id,
        cid,
        qn,
        if wants_1080 { "DASH" } else { "完整MP4" }
    );
    let url = if wants_1080 {
        format!(
            "https://api.bilibili.com/x/player/playurl?bvid={}&cid={}&qn=80&fnval=4048&fnver=0&fourk=0&try_look=1",
            bv_id, cid
        )
    } else {
        format!(
            "https://api.bilibili.com/x/player/playurl?bvid={}&cid={}&qn=64&type=&otype=json&platform=html5&high_quality=1",
            bv_id, cid
        )
    };

    let response = bili_api_client()
        .get(&url)
        .header("Referer", format!("https://www.bilibili.com/video/{bv_id}"))
        .send()
        .await
        .map_err(|e| format!("请求视频链接失败: {}", e))?;

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("解析JSON失败:  {}", e))?;

    // 检查API返回状态
    if json["code"].as_i64() != Some(0) {
        let error = format!(
            "API错误: {}",
            json.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("未知错误")
        );
        log::error!(target: "DLNA1080", "播放地址 API 失败: bvid={}, {}", bv_id, error);
        return Err(error);
    }

    let data = &json["data"];
    if !wants_1080 {
        let video_url = data["durl"][0]["url"]
            .as_str()
            .ok_or_else(|| "无法获取 720P 视频链接".to_string())?;
        log::info!(target: "DLNA1080", "选中 720P 完整 MP4: bvid={}, cid={}", bv_id, cid);
        return Ok(BilibiliMedia::Direct {
            url: video_url.to_string(),
        });
    }

    let videos = data["dash"]["video"]
        .as_array()
        .ok_or_else(|| "1080P 接口没有返回 DASH 视频轨".to_string())?;
    let audios = data["dash"]["audio"]
        .as_array()
        .ok_or_else(|| "1080P 接口没有返回 DASH 音频轨".to_string())?;

    // DLNA 设备对 AVC/H.264 的支持远好于 HEVC/AV1，优先选择 AVC 的 id=80。
    let video = videos
        .iter()
        .filter(|v| v["id"].as_u64() == Some(80))
        .max_by_key(|v| {
            let avc_bonus = v["codecs"].as_str().is_some_and(|c| c.starts_with("avc1")) as u64;
            avc_bonus * u64::MAX / 2 + v["bandwidth"].as_u64().unwrap_or(0)
        })
        .ok_or_else(|| {
            let ids: Vec<u64> = videos.iter().filter_map(|v| v["id"].as_u64()).collect();
            format!("B站未返回 1080P DASH 轨道，可用 id={ids:?}")
        })?;
    let audio = audios
        .iter()
        .max_by_key(|a| a["bandwidth"].as_u64().unwrap_or(0))
        .ok_or_else(|| "1080P DASH 音频轨为空".to_string())?;

    log::info!(
        target: "DLNA1080",
        "选中 DASH 轨道: bvid={}, video_id={}, codec={}, {}x{}, video_bw={}, audio_id={}, audio_codec={}, audio_bw={}",
        bv_id,
        video["id"].as_u64().unwrap_or(0),
        video["codecs"].as_str().unwrap_or("unknown"),
        video["width"].as_u64().unwrap_or(0),
        video["height"].as_u64().unwrap_or(0),
        video["bandwidth"].as_u64().unwrap_or(0),
        audio["id"].as_u64().unwrap_or(0),
        audio["codecs"].as_str().unwrap_or("unknown"),
        audio["bandwidth"].as_u64().unwrap_or(0),
    );

    let media_url = |value: &Value| {
        value["base_url"]
            .as_str()
            .or_else(|| value["baseUrl"].as_str())
            .map(str::to_string)
    };
    Ok(BilibiliMedia::Dash {
        video_url: media_url(video).ok_or_else(|| "DASH 视频轨没有 URL".to_string())?,
        audio_url: media_url(audio).ok_or_else(|| "DASH 音频轨没有 URL".to_string())?,
        width: video["width"].as_u64().unwrap_or(0) as u32,
        height: video["height"].as_u64().unwrap_or(0) as u32,
    })
}
