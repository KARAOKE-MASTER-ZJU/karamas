use reqwest::Client;
use serde_json::Value;
use std::sync::OnceLock;

/// 全局共享 B站 API 客户端：连接池复用 + 超时。
/// 请求超时由 `crate::config::API_TIMEOUT` 设置。
fn bili_api_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(crate::config::API_TIMEOUT)
            .user_agent("Mozilla/5.0")
            .build()
            .expect("创建 B站 API 客户端失败")
    })
}

// 新算法 (2020-03+): https://socialsisteryi.github.io/bilibili-API-collect/docs/misc/bvid_desc.html
const BV_XOR_CODE: u64 = 23442827791579;
const BV_MASK_CODE: u64 = 2251799813685247;
const BV_BASE: u64 = 58;
const BV_ALPHABET: &str = "FcwAPNKTMug3GV5Lj7EJnHpWsx4tb8haYeviqBz6rkCy12mUSDQX9RdoZf";
// ENCODE_MAP = [8,7,0,5,1,3,2,4,6]; DECODE_MAP = reversed(ENCODE_MAP)
const BV_DECODE_MAP: [usize; 9] = [6, 4, 2, 3, 1, 5, 0, 7, 8];

/// 将 B站 BV号 转换为 AV号
pub fn bv_to_aid(bvid: &str) -> u64 {
    let s = bvid.strip_prefix("BV1").unwrap_or(bvid);
    let alpha: Vec<char> = BV_ALPHABET.chars().collect();
    let chars: Vec<char> = s.chars().collect();
    let mut tmp: u64 = 0;
    for i in 0..9 {
        let idx = alpha
            .iter()
            .position(|&a| a == chars[BV_DECODE_MAP[i]])
            .unwrap_or(0) as u64;
        tmp = tmp * BV_BASE + idx;
    }
    (tmp & BV_MASK_CODE) ^ BV_XOR_CODE
}

/// Public page metadata. `page_index` is zero-based; `page` is for display.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliPart {
    pub page_index: u32,
    pub page: u32,
    pub cid: u64,
    pub part: String,
    pub duration_secs: u32,
}

/// 获取 B站视频的所有分 P 页数据。
pub async fn get_bilibili_parts(bvid: &str) -> Result<Vec<BilibiliPart>, String> {
    let json: Value = bili_api_client()
        .get("https://api.bilibili.com/x/player/pagelist")
        .query(&[("bvid", bvid)])
        .header("Referer", "https://www.bilibili.com/")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if json["code"].as_i64() != Some(0) {
        return Err(format!(
            "B站 API 错误: {} ({})",
            json["message"], json["code"]
        ));
    }
    let data = json["data"]
        .as_array()
        .filter(|data| !data.is_empty())
        .ok_or("该视频没有可用的分 P 数据")?;
    data.iter()
        .enumerate()
        .map(|(index, item)| {
            let page = item["page"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n == index as u32 + 1)
                .ok_or("无效的分 P 页码")?;
            Ok(BilibiliPart {
                page_index: page - 1,
                page,
                cid: item["cid"]
                    .as_u64()
                    .filter(|&n| n > 0)
                    .ok_or("无效的 CID")?,
                part: item["part"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("P{page}")),
                duration_secs: item["duration"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or("无效的分 P 时长")?,
            })
        })
        .collect()
}

/// 获取 B站视频的指定分 P 页的 CID 和时长。
pub async fn get_page_info(bv_id: &str, page: u32) -> Result<(u64, u32), String> {
    let parts = get_bilibili_parts(bv_id).await?;
    let part = parts
        .get(page as usize)
        .ok_or_else(|| format!("分 P 超出范围: {page}"))?;
    Ok((part.cid, part.duration_secs))
}

/// 当前视频分 P 支持的清晰度。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BilibiliQuality {
    pub qn: u32,
    pub label: String,
}

/// 通过播放接口读取清晰度列表，不下载媒体内容。
pub async fn get_bilibili_qualities(video: &str) -> Result<Vec<BilibiliQuality>, String> {
    let (bvid, page) = parse_bilibili_video(video)?;
    let (cid, _) = get_page_info(&bvid, page).await?;
    let json = bili_api_client()
        .get("https://api.bilibili.com/x/player/playurl")
        .query(&[
            ("bvid", bvid.as_str()),
            ("cid", &cid.to_string()),
            ("qn", "80"),
            ("fnval", "4048"),
            ("fourk", "1"),
        ])
        .header("Referer", format!("https://www.bilibili.com/video/{bvid}"))
        .send()
        .await
        .map_err(|e| format!("获取清晰度失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("获取清晰度失败：{e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("解析清晰度响应失败：{e}"))?;
    parse_bilibili_qualities(&json)
}

fn parse_bilibili_qualities(json: &Value) -> Result<Vec<BilibiliQuality>, String> {
    if json["code"].as_i64() != Some(0) {
        return Err(format!(
            "获取清晰度失败：{} ({})",
            json["message"].as_str().unwrap_or("B站接口返回错误"),
            json["code"]
        ));
    }
    let data = &json["data"];
    let accepted = data["accept_quality"]
        .as_array()
        .ok_or("B站未返回清晰度列表")?;
    let mut qualities = Vec::<BilibiliQuality>::new();
    for (index, value) in accepted.iter().enumerate() {
        let Some(quality) = value
            .as_u64()
            .and_then(|qn| u32::try_from(qn).ok())
            // 仅展示现有投屏控制支持的档位，避免选中后被 set_quality 拒绝。
            .and_then(crate::cast::Quality::from_qn)
        else {
            continue;
        };
        let qn = quality.as_qn();
        if qualities.iter().any(|option| option.qn == qn) {
            continue;
        }
        let format = data["support_formats"].as_array().and_then(|formats| {
            formats
                .iter()
                .find(|item| item["quality"].as_u64() == Some(qn as u64))
        });
        let label = format
            .and_then(|format| {
                ["new_description", "description", "display_desc"]
                    .into_iter()
                    .find_map(|key| {
                        format[key]
                            .as_str()
                            .filter(|label| !label.trim().is_empty())
                    })
            })
            .or_else(|| {
                data["accept_description"][index]
                    .as_str()
                    .filter(|label| !label.trim().is_empty())
            })
            .unwrap_or_else(|| quality.label());
        qualities.push(BilibiliQuality {
            qn,
            label: label.to_owned(),
        });
    }
    if qualities.is_empty() {
        return Err("该分 P 没有可用的投屏清晰度".into());
    }
    Ok(qualities)
}

/// 解析 B站视频输入，支持 BV号、BV URL、bilibili://video/URL、以及带分 P 的格式。
pub fn parse_bilibili_video(input: &str) -> Result<(String, u32), String> {
    let (bvid, page) = if input.starts_with("bilibili://")
        || input.starts_with("https://")
        || input.starts_with("http://")
    {
        let url = url::Url::parse(input).map_err(|_| "无效的视频 URL")?;
        let app = url.scheme() == "bilibili";
        if app {
            if url.host_str() != Some("video") {
                return Err("仅支持普通 BV 视频".into());
            }
        } else if !matches!(
            url.host_str(),
            Some("www.bilibili.com" | "bilibili.com" | "m.bilibili.com")
        ) {
            return Err("仅支持普通 BV 视频".into());
        }
        let path = url.path().trim_matches('/');
        let bvid = if app {
            path
        } else {
            path.strip_prefix("video/").ok_or("仅支持普通 BV 视频")?
        };
        let pairs: Vec<_> = url.query_pairs().collect();
        let page = if app {
            pairs.iter().find(|(k, _)| k == "page")
        } else {
            None
        };
        let index = if let Some((_, value)) = page {
            value.parse::<u32>().map_err(|_| "无效的分 P 索引")?
        } else if let Some((_, value)) = pairs.iter().find(|(k, _)| k == "p") {
            value
                .parse::<u32>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .ok_or("无效的分 P 页码")?
        } else {
            0
        };
        (bvid.to_owned(), index)
    } else {
        let (bvid, page) = input.split_once("-page").unwrap_or((input, "0"));
        (
            bvid.to_owned(),
            page.parse::<u32>().map_err(|_| "无效的分 P 索引")?,
        )
    };
    if bvid.len() != 12
        || !bvid.starts_with("BV")
        || !bvid.bytes().skip(2).all(|c| c.is_ascii_alphanumeric())
    {
        return Err("仅支持普通 BV 视频".into());
    }
    Ok((bvid, page))
}

/// 获取 B站视频的指定分 P 页的时长。
pub async fn get_page_duration(bv_id: &str, page: u32) -> Result<u32, String> {
    get_page_info(bv_id, page).await.map(|(_, d)| d)
}

/// 获取BiliBili视频直链
///
/// # Arguments
/// * `bv_id` - 视频BV号（例如："BV1AP411x7YW"）
/// * `page` - 分P页码，默认为0
///
/// # Returns
/// * `Result<String, String>` - 返回直链URL或错误信息
pub async fn get_bilibili_direct_link(bv_id: &str, page: Option<u32>) -> Result<String, String> {
    match get_bilibili_media(bv_id, page, 64).await? {
        BilibiliMedia::Direct { url } => Ok(url),
        BilibiliMedia::Dash { .. } => Err("720P 接口意外返回 DASH".to_string()),
    }
}

/// B站媒体地址。720P 保持单文件 MP4；1080P 使用独立的 DASH 视频、音频轨。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BilibiliMedia {
    Direct {
        url: String,
    },
    Dash {
        video_url: String,
        audio_url: String,
        width: u32,
        height: u32,
    },
}

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
    let cid = get_video_cid(bv_id, page).await?;

    // 第二步：获取视频直链
    get_video_url(bv_id, &cid, qn).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires public Bilibili access"]
    async fn test_get_bilibili_direct_link() {
        // 示例：测试获取视频直链
        match get_bilibili_direct_link("BV1LS4MzKE8y", Some(2)).await {
            Ok(url) => println!("视频直链: {}", url),
            Err(e) => println!("错误: {}", e),
        }
    }

    #[tokio::test]
    #[ignore = "public Bilibili network fixture; run manually before release"]
    async fn test_anonymous_1080_dash() {
        let media = get_bilibili_media("BV1dqj16aECG", Some(0), 80)
            .await
            .expect("目标视频应提供匿名 1080P DASH");
        match media {
            BilibiliMedia::Dash {
                width,
                height,
                video_url,
                audio_url,
            } => {
                assert_eq!((width, height), (1920, 1080));
                assert!(video_url.starts_with("https://"));
                assert!(audio_url.starts_with("https://"));
            }
            BilibiliMedia::Direct { .. } => panic!("1080P 不应退回单文件 720P"),
        }
    }
}
